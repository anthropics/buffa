# Vtable-mode `ReflectMessage` for message views

**Status:** Pre-implementation analysis, partially landed
**Builds on:** `reflect-prototype` branch (`docs/investigations/reflection-prototype-2026-05.md`)
**Scope:** Generate `impl ReflectMessage` directly on view types (and optionally
owned types), eliminating the bridge-mode encode → decode → `DynamicMessage`
round-trip. This is the deferred `ReflectMode::VTable` deliverable.

## Progress

- **2026-05-22** — Component 3 (the `ValueRef::List`/`Map` trait-object
  refactor) has **landed.** This was flagged below as the only component with
  real design risk and the reason vtable mode was deferred. `ValueRef` now
  carries `List(&'a dyn ReflectList)` and `Map(&'a dyn ReflectMap)`, the
  `ReflectList` / `ReflectMap` traits exist (with the no-alloc `get_str` CEL
  path), `DynamicMessage` implements them for `Vec<Value>` / `MapValue`, and
  conformance passes with the new shape. The API-breaking, conformance-gated
  refactor that would have been painful to do after a release is therefore
  behind us. Everything that remains is additive codegen and additive runtime
  trait impls — no consumer-facing breaking change. See the revised §3 and the
  revised sequencing for what this means for the remaining work.

## Why

Bridge mode (`ReflectMode::Bridge`, the only mode codegen currently emits) is:

```rust
fn reflect(&self) -> ReflectCow<'_> {
    // self → encode_to_vec() → DynamicMessage::decode() → Box
    ReflectCow::Owned(Box::new(DynamicMessage::from_message(self, pool, idx)))
}
```

Every `reflect()` call pays one full encode pass, one full decode pass, and a
heap allocation per string/bytes/repeated/map field. For consumers that read a
single field from a large message — the interceptor and field-mask use cases
named in the design — that cost is paid for every field they *don't* read.

Vtable mode generates `impl ReflectMessage for FooView<'a>` directly. `get()`
becomes a `match` over `field.number()` reading struct fields. No encode, no
decode, no `DynamicMessage`, no per-field allocation for fields not accessed.
The `ReflectCow` / `ReflectMode` contract was designed in advance of this:
flipping a message from bridge to vtable must be a zero-diff change at every
call site (`foo.reflect().get(fd)` is the only pattern).

## Components

### 1. `impl ReflectMessage for FooView<'a>` codegen — the core deliverable

A new `reflect_message_impl_for_view()` function in
`buffa-codegen/src/reflect.rs`, parallel to the existing `reflectable_impl()`.
Per generated view type, emit:

```rust
impl<'a> ::buffa_descriptor::reflect::ReflectMessage for FooView<'a> {
    fn message_descriptor(&self) -> &MessageDescriptor {
        // memoized — see component 2
    }

    fn pool(&self) -> &Arc<DescriptorPool> {
        #buffa_path::reflect::descriptor_pool()
    }

    fn get(&self, field: &FieldDescriptor) -> ValueRef<'_> {
        match field.number() {
            // string/bytes — borrow wire bytes, the actual zero-copy win
            1 => ValueRef::String(self.ans_uri),
            2 => ValueRef::Bytes(self.payload),

            // scalars — copy, trivially cheap
            3 => ValueRef::I32(self.count),

            // proto3-implicit-presence optional scalar — return default if absent.
            // `ReflectMessage::get` contract: absent singular fields return the
            // type's default value; presence is queried via `has()`.
            4 => ValueRef::String(self.label.as_deref().unwrap_or("")),

            // enum — number only, matching DynamicMessage
            5 => ValueRef::EnumNumber(self.kind as i32),

            // singular message — borrow via MessageFieldView<V> or default.
            // `MessageFieldView<V>` is a struct field, so `&self.workload`
            // is a real borrow tied to `&self`. `DefaultViewInstance` already
            // provides a static default for the unset case (no allocation).
            6 => ValueRef::Message(ReflectCow::Borrowed(
                self.workload
                    .get()
                    .map(|w| w as &dyn ReflectMessage)
                    .unwrap_or(WorkloadView::default_view_instance()),
            )),

            // repeated/map — just borrow the view container; the blanket
            // ReflectList/ReflectMap impls (§3) do the rest. No per-field codegen.
            7 => ValueRef::List(&self.tags),
            8 => ValueRef::Map(&self.labels),

            _ => {
                debug_assert!(false, "field {} not in {}", field.number(), Self::FULL_NAME);
                ValueRef::Bool(false) // arbitrary, unreachable in correct code
            }
        }
    }

    fn has(&self, field: &FieldDescriptor) -> bool {
        // Parallel match. Implicit-presence scalars: != default.
        // Explicit-presence (optional, message): is_set().
        // Repeated/map: !is_empty().
        match field.number() { /* ... */ }
    }

    fn for_each_set(&self, f: &mut dyn FnMut(&FieldDescriptor, ValueRef<'_>)) {
        // Iterate all field descriptors, call has() then get() for each set field.
        // Or unroll inline — codegen can emit a flat sequence of
        // `if has { f(fd, get) }` blocks, avoiding the descriptor iteration.
    }

    fn to_dynamic(&self) -> DynamicMessage {
        // Fall back to bridge-style materialization for this one call.
        // Used by `ReflectCow::to_dynamic()` and by consumers that need an
        // owned snapshot. Acceptable cost — it's an explicit opt-in.
        DynamicMessage::from_message(&self.to_owned_message(), pool, idx)
    }
}
```

What's already in place:

- View types are structs with named fields (`buffa-codegen/src/view.rs`), so
  borrows are real.
- `MessageFieldView<V>` boxes the inner view but `Deref`s to `&V`
  (`buffa/src/view.rs:432`).
- `DefaultViewInstance` (`buffa/src/view.rs:399`) provides the static default
  for absent message fields.
- The `get()` contract (absent singular → default, absent repeated/map → empty)
  is documented in the trait (`buffa-descriptor/src/reflect/message.rs:35`).

What to verify during implementation:

- `MessageFieldView<V>::get() -> Option<&V>` exists (or the equivalent accessor
  name) — needed for the `unwrap_or(default_view_instance())` pattern.
- Oneof handling: for a set oneof member, `get()` returns its value; for unset
  members, `get()` returns the type default and `has()` returns false. Verify
  the codegen path through the generated `FooKindView<'a>` enum.
- proto2 `required` fields: always present, `get()` never falls through to a
  default. Confirm the codegen knows the presence kind.

### 2. Per-message `MessageIndex` memoization

`message_descriptor()` is on the per-field-access hot path. Without
memoization, every call does a string lookup against the pool by `FULL_NAME`.

Generate, per message, alongside the `impl`:

```rust
#[doc(hidden)]
mod __reflect_foo {
    use std::sync::OnceLock;
    static MESSAGE_INDEX: OnceLock<::buffa_descriptor::MessageIndex> = OnceLock::new();

    pub(super) fn message_index() -> ::buffa_descriptor::MessageIndex {
        *MESSAGE_INDEX.get_or_init(|| {
            super::__buffa::reflect::descriptor_pool()
                .message_index(<super::Foo as ::buffa::MessageName>::FULL_NAME)
                .expect("generated message is in the embedded descriptor pool")
        })
    }
}
```

Then `message_descriptor()` is `pool().message_descriptor(__reflect_foo::message_index())`.
The `expect` is a codegen invariant, not consumer-facing — same justification
as the bridge-mode impl's expect (`buffa-codegen/src/reflect.rs:55`).

Constraints:

- `OnceLock` is `std`-only. The bridge-mode impl already requires `std` for the
  same reason (the `descriptor_pool()` accessor uses `OnceLock`). Document that
  vtable mode shares this requirement; `no_std` consumers stay on
  `ReflectMode::Off`.
- One `OnceLock<MessageIndex>` per message type (4 bytes inner + sync overhead),
  per package descriptor module. Negligible.

Alternative: cache the `MessageIndex` inside the per-package
`__buffa::reflect` module as a `OnceLock<HashMap<&'static str, MessageIndex>>`
populated once when the pool is built. One lock instead of N, but adds a
`HashMap` lookup per call. The per-message `OnceLock` is faster after warmup
and keeps the per-message codegen self-contained. Prefer per-message.

### 3. `ValueRef::List` and `ValueRef::Map` — RESOLVED (landed)

**This was the reason vtable mode was deferred and the only component with
real design risk. It has been resolved: option (a) below was adopted and has
landed.** The remaining text records the decision and the runtime that shipped,
and then specifies the *view-side* container impls that the vtable codegen
needs — which were not part of the landed change.

`ValueRef` now carries trait objects rather than materialized storage:

```rust
pub enum ValueRef<'a> {
    // ... scalars, String, Bytes, EnumNumber, Message unchanged ...
    List(&'a dyn ReflectList),
    Map(&'a dyn ReflectMap),
}

pub trait ReflectList: core::fmt::Debug {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn get(&self, idx: usize) -> Option<ValueRef<'_>>;
    fn for_each(&self, f: &mut dyn FnMut(ValueRef<'_>));
}

pub trait ReflectMap: core::fmt::Debug {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>>;
    fn get_str(&self, key: &str) -> Option<ValueRef<'_>>; // no-alloc CEL path
    fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>));
}
```

Bridge mode implements `ReflectList for Vec<Value>` and `ReflectMap for
MapValue`, so `DynamicMessage` returns a borrow with one extra vtable
indirection per element access. The `size_of::<ValueRef>() <= 32` and
`size_of::<ReflectCow>() <= 24` assertions in `value.rs` still hold: a
`&dyn ReflectList` is a 16-byte fat pointer, the same width the old `&[Value]`
would have been. Conformance passes with this shape (the bridge tests in
`buffa-test/tests/reflect_bridge.rs` exercise `List`/`Map`/`get_str`).

The two rejected alternatives are kept here for the record, because the
reasoning still constrains future changes:

- **`OnceCell<Vec<Value>>` cache per repeated field on the view.** Materialize
  on first access, cache, return the slice. Rejected: it grows the view struct
  (a wire-format type) with one cell per repeated/map field, still allocates
  once per field per view instance — defeating the field-mask use case — and
  `OnceCell` is `!Sync`, which breaks `Send + Sync` `OwnedView`.
- **A separate `LazyValueRef` for vtable, `ValueRef` for bridge.** Rejected:
  dual API surface forever, and it breaks the `ReflectMode` zero-diff promise
  (a call site matching `ValueRef::List` would not handle `LazyValueRef::List`).
  The `ReflectCow` design explicitly chose unified return types over parallel
  APIs; this stays consistent with it.

#### What still has to be built: container impls for the view types

The landed change covers the *bridge* side (`Vec<Value>` / `MapValue`). The
vtable side needs `ReflectList` / `ReflectMap` implemented for the view
containers — `RepeatedView<'a, T>` and `MapView<'a, K, V>` — so a view's
`get()` can return `ValueRef::List(&self.tags)` directly.

**One generic container impl per container; per-element conversion through a
helper trait.** The container impls are fully generic and live in
`buffa-descriptor` — because `ReflectList` / `ReflectMap` are local there and
the view containers (`RepeatedView` / `MapView`) are foreign from `buffa`, the
orphan rule permits an impl of the local trait for the foreign type with no
restriction. Codegen emits *zero* per-field container boilerplate — the
repeated/map arm of `get()` is just `ValueRef::List(&self.tags)`.

```rust
// in buffa-descriptor.

/// Per-element conversion to a borrowed reflective value.
pub trait ReflectElement: core::fmt::Debug {
    fn as_value_ref(&self) -> ValueRef<'_>;
}
impl ReflectElement for i32 { /* I32 */ }   // + i64/u32/u64/bool/f32/f64
impl ReflectElement for &str { /* String */ }
impl ReflectElement for &[u8] { /* Bytes */ }
impl<E: Enumeration> ReflectElement for EnumValue<E> { /* EnumNumber(to_i32) */ }

/// Per-key conversion (the spec-valid map-key types only).
pub trait ReflectMapKey: core::fmt::Debug {
    fn as_map_key_ref(&self) -> MapKeyRef<'_>;
}
impl ReflectMapKey for i32 { /* I32 */ }    // + i64/u32/u64/bool
impl ReflectMapKey for &str { /* String */ }

impl<'a, T: ReflectElement> ReflectList for RepeatedView<'a, T> { /* … */ }
impl<'a, K: ReflectMapKey, V: ReflectElement> ReflectMap for MapView<'a, K, V> { /* … */ }
```

**The message and enum element cases need a one-line codegen impl, not a
blanket — this is a coherence constraint, not a choice.** The tempting shape is
`impl<M: ReflectMessage> ReflectElement for M`, letting repeated-of-message fall
out automatically. Rust rejects it (E0119): a trait-bound blanket
`impl<M: ReflectMessage> ReflectElement for M` overlaps with every concrete
impl — `impl ReflectElement for i32` included — because the compiler cannot
prove `i32: !ReflectMessage` (nothing forbids `buffa-descriptor` from later
adding `impl ReflectMessage for i32`, and sealing does not factor into the
overlap check). The same argument kills `impl<E: Enumeration> ReflectElement
for E` for bare (closed) enums. So:

- **Scalars, `&str`, `&[u8]`, `EnumValue<E>`** get concrete (or single-type-
  constructor) impls in `buffa-descriptor`. `EnumValue<E>` is fine because it is
  a distinct type constructor, not a bare type parameter — it cannot overlap a
  scalar.
- **Message views (`FooView<'a>`) and bare closed enums (`SomeEnum`)** get a
  one-line `impl ReflectElement` emitted by codegen — an impl of a foreign
  trait for a local type, which the orphan rule allows in the consumer crate.
  This is per *type*, not per *field*: a message used in ten repeated fields
  still gets one impl. The repeated-of-message and map-of-message cases the
  original draft flagged for an early spike then resolve through that
  per-type impl plus the generic container impl — no per-field codegen, but not
  the zero-codegen the blanket would have given.

Two further wrinkles to handle in the impls:

1. **`get_str` on a generic `MapView<K, V>`.** It cannot dispatch on `K` at the
   type level, so `get_str` does a linear scan and matches
   `MapKeyRef::String(_)` per entry, returning `None` for a non-string-keyed
   map. This is acceptable: `MapView` lookup is already documented as `O(n)`
   (the runtime comment calls this fine for the small maps protobuf produces),
   so vtable maps match bridge maps in behavior, not in asymptotics. A consumer
   that needs `O(1)` collects into a `HashMap`.
2. **`&[u8]` map keys (the exotic case).** A `string` map key with editions
   `utf8_validation = NONE` is typed `&'a [u8]` in the view, but `MapKeyRef`
   has no bytes variant (bytes are not a spec-valid map key). The proto type
   *is* `string`, so the bytes are normally valid UTF-8; `ReflectMapKey for
   &[u8]` converts via `from_utf8` and falls back to `""` on invalid input,
   with a `debug_assert`. This keeps every generated map satisfiable by the
   generic impl. Flagged as a low-severity correctness nuance, not a blocker.
3. **Coherence with the bridge impls.** The bridge `impl ReflectList for
   Vec<Value>` does not collide with the view impls, because the view containers
   are `RepeatedView` / `MapView`, not `Vec` / `MapValue`. This only becomes a
   problem for **owned-message vtable** (component 6), where the owned repeated
   field is a `Vec<T>` — see that component for the resolution
   (`impl ReflectElement for Value`, dropping the bespoke `Vec<Value>` impl).

### 4. `ReflectMode::VTable` plumbing

`ReflectMode` already has a `VTable` variant marked `**Deferred**`
(`buffa-descriptor/src/reflect/mod.rs:43`). Wire it through:

- `BuildConfig::reflect_mode(ReflectMode)` (or per-message override via
  `message_attribute`-style targeting).
- `buffa-codegen/src/feature_gates.rs` and `lib.rs`: when `VTable`, emit
  `impl ReflectMessage` for the view *and* the owned message. (The owned-message
  impl is also valuable — it gives `&dyn ReflectMessage` over an in-memory
  struct without the encode/decode round-trip, which is the interceptor use
  case.)
- The bridge-mode `Reflectable::reflect()` impl is unchanged; vtable mode adds
  a second `reflect()` body: `ReflectCow::Borrowed(self)`. The codegen picks
  the body by mode. Consumers that already hold a generated type can reflect
  without a `DynamicMessage` ever existing.
- Default mode: keep `Bridge` for 0.7.0 (it's conformance-tested and works).
  Flip the default to `VTable` in a later release once it's exercised.

### 5. `OwnedView` → `dyn ReflectMessage` entry point — verify, no work

`OwnedView<V>` is `'static + Send + Sync` (`buffa/src/view.rs:981`).
`OwnedView::reborrow()` gives `&'b V::Reborrowed<'b>` where the lifetime is
narrowed by covariance (`ViewReborrow` trait, soundness argument in the doc
comment at `buffa/src/view.rs:160-190`). With `impl<'a> ReflectMessage for
FooView<'a>`, `owned_view.reborrow() as &dyn ReflectMessage` falls out.

Verify with a test: decode an `OwnedView` from `Bytes`, reborrow, call
`get()`/`has()`/`for_each_set()` through `&dyn ReflectMessage`. This is the
entry point a CEL adapter (or any reflection consumer with raw wire bytes)
will use.

## Sequencing

**All steps below have landed.** The feature set the original plan called for —
view *and* owned vtable reflection, the public `ReflectMode` selector, and the
acceptance tests — is complete and conformance-validated. One unanticipated
prerequisite surfaced during implementation (WKT view reflection, step 3.5
below). The remaining open items are noted under "Not yet done" at the end.

1. ✅ **`ValueRef::List`/`Map` trait-object refactor.** `ValueRef` carries
   `&dyn ReflectList` / `&dyn ReflectMap`; `DynamicMessage` implements the
   bridge side.
2. ✅ **Runtime container impls (§3).** `ReflectElement` / `ReflectMapKey` traits
   plus generic `ReflectList` / `ReflectMap` for `RepeatedView` / `MapView`
   (and, for owned vtable, `Vec<T>` / `HashMap<K, V>`), in `buffa-descriptor`.
3. ✅ **`impl ReflectMessage for FooView<'a>` codegen + memoized `MessageIndex`
   (§1, §2).** Behind an internal mode flag; oneof dispatch and enum-number
   extraction handled.
3.5. ✅ **WKT view reflection (unplanned prerequisite).** The conformance corpus
   references well-known types, whose views live in `buffa-types` with no path
   to `ReflectMessage`. Added a `reflect` feature to `buffa-types` (gated via the
   new `gate_reflect_on_crate_feature` codegen flag) so WKT views/owned types
   implement `ReflectMessage`. Without this, vtable mode only worked for
   WKT-free protos.
4. ✅ **`ReflectMode` plumbing (§4).** Public `ReflectMode` enum
   (Off/Bridge/VTable), exposed as `buffa_build::Config::reflect_mode` and
   `protoc-gen-buffa`'s `reflect_mode=` option. Vtable mode emits
   `impl ReflectMessage` for the view *and* the owned message and switches the
   owned `Reflectable::reflect()` body to `ReflectCow::Borrowed(self)`. Default
   stays `Off`/`Bridge`; flipping the default to `VTable` is deferred.
5. ✅ **Acceptance: conformance via-vtable run + `OwnedView` entry-point test
   (§5).** `BUFFA_VIA_VTABLE` runner mode (decode_view → reflect → rebuild →
   JSON) passes 1246 binary→JSON cases across proto2/proto3/editions with zero
   failures, parallel to `BUFFA_VIEW_JSON` / `BUFFA_VIA_REFLECT`. The
   `OwnedView` → `&dyn ReflectMessage` test and an owned-vtable↔bridge parity
   test (every field kind) lock in correctness.
6. ✅ **Owned-message vtable (§6).** `impl ReflectMessage` for the owned struct;
   `owned.reflect()` borrows `self` with no round-trip (the interceptor use
   case). Required the coherence resolution noted in §3: `ReflectElement for
   Value` and dropping the bespoke `impl ReflectList for Vec<Value>`. owned
   `unknown_fields()` is overridden to preserve unknowns (bridge parity).

### Done since

- ✅ **`VTable` is the default reflection mode.** `generate_reflection(true)` (and
  `protoc-gen-buffa`'s `reflection=true`) now select `VTable`; `Bridge` is opt-in
  via `reflect_mode(ReflectMode::Bridge)`. Vtable no longer requires views — the
  owned `impl ReflectMessage` is self-contained, so views-off builds get
  owned-only vtable reflection rather than an error.
- ✅ **README reflection charts regenerated.** The `reflect` bench's new cases are
  charted: a `view` series on reflection-decode (zero-copy decode is the floor,
  +25% to +153% over the generated owned codec) and a new reflection-read chart
  (`vtable` vs `bridge` vs `dynamic` — vtable runs 4–7× faster than the bridge
  round-trip). Regenerated through the Docker bench harness; see the README note
  that these reflection charts run on the dev host, not the pinned Xeon runner.

## What this does *not* solve

Reflection consumers whose own value model requires `'static` (e.g., a
scripting host whose value trait is `Any`-bound) cannot hold a borrowed
`&'a dyn ReflectMessage` directly — they hit a `'static` wall at their own
boundary and must either snapshot (`to_dynamic()`) or hold the `OwnedView`
and reborrow per access. Vtable mode is a real win for consumers that do
short-lived borrowed reflection (interceptors, field masks, debug printing).
For consumers that hold reflected values for the duration of an evaluation,
the win is reduced to "fewer allocations for fields not accessed" — which is
zero if the consumer reads every field. Set expectations accordingly: the
0.7.0 bridge path is correct and adequate for those consumers; vtable mode
helps them only at the margins. The architecture supports a transparent swap
later, which is the right call — don't gate downstream work on this.

## Risks

- **Conformance regression from the `ValueRef` refactor.** Retired — the
  refactor landed behind the conformance gate with no unexpected failures.
- **Repeated-of-message and map-with-message-value.** Downgraded from "most
  fiddly codegen case, spike it early" to a non-issue for codegen: the blanket
  `impl<M: ReflectMessage> ReflectElement for M` (§3) handles them with no
  per-field codegen. The residual risk is purely in the generic impls
  themselves — the per-element borrow lifetime ties to `&self` (the
  `RepeatedView`), and covariance makes it work. Cover it with a test in PR 2,
  not a codegen spike.
- **Oneof reflection.** Verify the `FooKindView<'a>` enum codegen exposes the
  active variant in a way the `get()` match can dispatch on. May need a small
  accessor on the generated oneof view enum. This is now the sharpest remaining
  codegen risk (PR 3).
- **Enum-number extraction.** Views store enum fields as
  `buffa::EnumValue<E>`, not a bare `i32`; `get()` must extract the wire number.
  Confirm the `EnumValue` accessor and add the `ReflectElement for EnumValue<E>`
  impl in PR 2.
- **Recursive message types.** `MessageFieldView<V>` boxes precisely to break
  recursion; verify `default_view_instance()` for a self-recursive message does
  not recurse infinitely (it should not — the default has no set fields — but
  check).

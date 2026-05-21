# Vtable-mode `ReflectMessage` for message views

**Status:** Recommendation / pre-implementation analysis
**Builds on:** `reflect-prototype` branch (`docs/investigations/reflection-prototype-2026-05.md`)
**Scope:** Generate `impl ReflectMessage` directly on view types (and optionally
owned types), eliminating the bridge-mode encode → decode → `DynamicMessage`
round-trip. This is the deferred `ReflectMode::VTable` deliverable.

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

            // repeated/map — see component 3 (this is the open design decision)
            7 => /* ValueRef::List(...) — see §3 */,

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

### 3. `ValueRef::List` and `ValueRef::Map` — open design decision

**This is the reason vtable mode was deferred and the only component with
real design risk. It needs a decision before 0.7.0 ships.**

The current variants require materialized storage:

```rust
pub enum ValueRef<'a> {
    // ...
    List(&'a [Value]),
    Map(&'a MapValue),
}
```

`DynamicMessage` stores repeated/map fields as `Value::List(Vec<Value>)` and
`Value::Map(MapValue)` already, so it can return a slice/borrow.

A view holds `RepeatedView<'a, T>` (a `Vec<T>` where `T` is the per-element
view type — `&'a str`, `i32`, `BarView<'a>`, etc.) and `MapView<'a, K, V>`.
**There is no `&[Value]` to hand out.** A vtable `get()` would have to allocate
a `Vec<Value>` and find somewhere to store it so the borrow outlives the call.

#### Option (a) — change to trait objects (recommended)

```rust
pub enum ValueRef<'a> {
    // ... scalars, String, Bytes, EnumNumber, Message unchanged ...
    List(&'a dyn ReflectList),
    Map(&'a dyn ReflectMap),
}

pub trait ReflectList {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn get(&self, idx: usize) -> Option<ValueRef<'_>>;
    fn for_each(&self, f: &mut dyn FnMut(ValueRef<'_>));
}

pub trait ReflectMap {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>>;
    /// Visit entries. Iteration order is unspecified — callers must not
    /// depend on it (matches DynamicMessage and protobuf semantics).
    fn for_each(&self, f: &mut dyn FnMut(ValueRef<'_>, ValueRef<'_>));
}
```

`Vec<Value>` and `MapValue` impl these trivially (`Value::as_ref()` already
exists). `RepeatedView<'a, T>` and `MapView<'a, K, V>` impl them per element
type — codegen emits these or a small set of generic impls covers the scalar
cases. This is structurally symmetric with the existing
`ReflectCow::Borrowed(&dyn ReflectMessage)` pattern, which is already accepted
in the design.

Size: `&dyn ReflectList` is a 16-byte fat pointer. `&'a [Value]` is also 16
bytes. `&'a MapValue` is 8 bytes → 16. `ValueRef`'s 32-byte budget assertion
still holds (largest variant remains `String`/`Bytes` at 16 + tag, or
`Message(ReflectCow)` at 16 + tag).

Cost: one virtual dispatch per element access. The conformance JSON serializer
and the WKT codecs iterate via `for_each_set` already — they switch from a
slice loop to a `for_each` callback. Mechanical.

Break: this is a breaking change to `ValueRef`. The prototype is on a local
unpushed branch — there are zero released consumers. **Make the change before
`buffa-descriptor` 0.7.0 ships.** If 0.7.0 ships with `&[Value]` and vtable
mode lands in 0.8.0, this becomes a breaking `ValueRef` change with downstream
consumers, or a permanent parallel `LazyValueRef` API. Neither is good.

#### Option (b) — `OnceCell<Vec<Value>>` cache per repeated field on the view

Materialize on first access, cache, return the slice. Works without touching
`ValueRef`. Costs:

- View struct grows by one `OnceCell<Vec<Value>>` per repeated/map field.
- `OnceCell` is `!Sync` — `OnceLock` would be needed for `Send + Sync` views,
  which `OwnedView` requires.
- Repeated-field reflection still allocates, just once per field per view
  instance. Defeats the point for field-mask consumers.
- Pollutes the view struct (a wire-format type) with reflection state.

**Reject this option.** It optimizes for not changing `ValueRef` at the cost
of every other axis.

#### Option (c) — separate `LazyValueRef` for vtable, keep `ValueRef` for bridge

A second value type returned by a second trait method. Avoids the break,
costs: dual API surface forever, every reflection consumer needs two code
paths or an adapter, the `ReflectMode` zero-diff promise is broken (call
sites that handle `ValueRef::List(&[Value])` don't handle
`LazyValueRef::List(&dyn ReflectList)`).

**Reject this option.** The `ReflectCow` design explicitly chose unified
return types over parallel APIs (see the module doc in
`buffa-descriptor/src/reflect/message.rs`). Be consistent.

#### Decision needed

Adopt option (a). Land the `ValueRef::List`/`Map` change as its own commit
*ahead of* the vtable codegen, with conformance run before and after to prove
no regression. The vtable codegen then targets the new shape from the start.

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

1. **`ValueRef::List`/`Map` change first, as its own commit.** Update
   `DynamicMessage`, the JSON serde, the WKT codecs, the conformance runner.
   Run conformance — must remain at 0 unexpected failures. This is a focused
   refactor with a clear pass/fail gate.
2. **Per-message `MessageIndex` memoization.** Small, independent.
3. **`impl ReflectMessage for FooView<'a>` codegen.** The main deliverable.
   Test against the existing `reflect_e2e.rs` tests by running them through
   both bridge and vtable mode.
4. **`ReflectMode::VTable` wiring.** Once (3) is solid.
5. **`OwnedView` entry-point test.** Trivial once (3)–(4) land; do it as the
   acceptance test.

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

- **Conformance regression from the `ValueRef` refactor.** Mitigated by
  landing it first with the conformance gate.
- **Codegen complexity for repeated-of-message and map-with-message-value.**
  These need `ReflectList`/`ReflectMap` impls that yield
  `ValueRef::Message(ReflectCow::Borrowed(&BarView))` per element. The
  borrow lifetime ties to `&self` (the `RepeatedView`); covariance makes it
  work but it's the most fiddly codegen case. Spike it early.
- **Oneof reflection.** Verify the `FooKindView<'a>` enum codegen exposes
  the active variant in a way that the `get()` match can dispatch on. May
  need a small accessor on the generated oneof enum.
- **Recursive message types.** `MessageFieldView<V>` boxes precisely to
  break recursion; verify `default_view_instance()` for a self-recursive
  message doesn't recurse infinitely (it shouldn't — the default has no
  set fields — but check).

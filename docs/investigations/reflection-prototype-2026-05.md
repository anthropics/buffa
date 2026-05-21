# Reflection prototype — viability report

**Status:** Release-prep for 0.7.0 — phases 1b–2 + JSON + conformance + dual code review
**Date:** May 2026
**Branch:** `reflect-prototype` (local only, not pushed)
**Design:** [`reflection.md`](reflection.md) (the March 2026 design doc; relocated from `~/home/tmp/`)
**Conformance:** 2764 successes, 7 expected failures, 0 unexpected (binary + JSON, proto2/proto3/editions 2023)

> This document started as a viability report for a local prototype. After
> the prototype passed conformance, the reflection runtime was promoted to
> a 0.7.0 release deliverable (it will be load-bearing for a CEL embedding
> in the Minos Rust port). The "What was built" and "Findings" sections
> below describe the prototype phase; the "0.7.0 release-prep" section at
> the end records the hardening work done after the dual code review.

## Summary

The runtime reflection design is implementable as written, with one
architectural correction. The prototype builds phases 1b through 2 plus
descriptor-driven JSON serde plus a conformance runner mode in ~4,400 lines
of new code, and the result passes the protobuf conformance suite:

- **Binary round-trip: 1409 successes, 0 unexpected failures** across
  proto2, proto3, and editions 2023.
- **Binary + JSON: 2725 successes, 46 expected failures, 0 unexpected
  failures** — every failure is a documented gap (`google.protobuf.Any`
  is a phase-4 deliverable; the rest are strict-validation rejections
  the codec accepts where the spec says reject).

The implementation surfaced one architectural finding (the design doc's crate
layout is impossible), validated the design's hardest decisions (the
`SingularKind`/`FieldKind` split, pool indices, the `Reflectable`/`ReflectCow`
contract, editions feature resolution at pool-build time), and validated the
"is descriptor-driven JSON mechanical?" question — yes, ~700 lines for the
serde impls, ~600 more for the reflective WKT codecs.

## What was built

| Component | Lines | Coverage |
|---|---|---|
| `buffa-descriptor/src/features.rs` | ~210 | Edition feature resolution (file → message → field chain). |
| `buffa-descriptor/src/pool.rs` | ~620 | `DescriptorPool`: two-pass register/link, FQN lookup, map detection, packed/delimited/presence resolution, u16 field-cap validation. |
| `buffa-descriptor/src/reflect/value.rs` | ~150 | `Value` (owned), `ValueRef<'_>` (borrowed, ≤32 B compile-time-asserted), `MapKey`. |
| `buffa-descriptor/src/reflect/message.rs` | ~170 | `ReflectMessage`/`ReflectMessageMut` (dyn-safe, storage-agnostic), `ReflectCow::{Borrowed,Owned}`, `Reflectable`. |
| `buffa-descriptor/src/reflect/dynamic.rs` | ~1000 | `DynamicMessage`: descriptor-driven encode/decode, generated↔dynamic bridge, get/set/has/clear/for_each_set. |
| `buffa-descriptor/src/reflect/json.rs` | ~700 | `impl Serialize for DynamicMessage`, `DynamicMessageSeed` (`DeserializeSeed`), per-`SingularKind` serde dispatch, base64. |
| `buffa-descriptor/src/reflect/json_wkt.rs` | ~600 | Reflective WKT codecs: Timestamp (RFC 3339), Duration, FieldMask, Empty, Struct/Value/ListValue, NullValue, the nine wrappers. |
| `conformance/` (`BUFFA_VIA_REFLECT=1` mode) | ~150 | Fifth runner suite: binary and JSON input/output through `DynamicMessage`. |
| Tests | ~570 | 12 pool/feature-resolution tests, 4 generated↔dynamic bridge tests, 3 `Reflectable` trait tests. |

PR #9 (`desc.rs` — `MessageDescriptor`, `FieldDescriptor`, `FieldKind`,
`SingularKind`, pool-index newtypes) is the foundation; it cherry-picked
cleanly onto post-v0.6.0 `main` and is unchanged by the prototype work.

## Findings

### 1. The crate layout in the design doc has a dependency cycle

The design doc places the reflection runtime in `buffa/src/reflect/`, with
`buffa-descriptor` providing the descriptor types it consumes. But
`buffa-descriptor` already depends on `buffa` — its generated descriptor
types use `buffa::Message`. The reverse arrow (`buffa` → `buffa-descriptor`)
is a cycle.

**Resolution: the reflection runtime lives in `buffa-descriptor/src/reflect/`.**
This is a clean home: the pool already lives there, reflection consumers
already declare `buffa-descriptor` post-#118, and `buffa-descriptor` already
depends on `buffa::encoding` for the wire primitives `DynamicMessage` needs.
The design doc's stated *reason* for placing reflection in `buffa` ("so the
runtime can depend on descriptor types without pulling the codegen toolchain")
is satisfied either way — `buffa-descriptor` is dependency-free of
`buffa-codegen`.

The one downstream consequence: codegen emits
`impl ::buffa_descriptor::reflect::Reflectable for Foo`, which means
reflection consumers must declare `buffa-descriptor` directly (not just
transitively through `buffa`). That's already the model for descriptor types
post-#118, so it's not a new requirement.

### 2. The design's hard decisions hold

Four specific design points were testable in this prototype and all hold:

- **`FieldKind` is `Copy` with no `Box`.** The `SingularKind` sub-enum split
  works exactly as sketched. `DynamicMessage::merge_one_field` does
  `match fd.kind { ... }` with no borrowing or cloning. The static assertion
  `size_of::<ValueRef<'_>>() <= 32` is wired into `value.rs` and passes.

- **Pool indices, not `Arc`.** `MessageIndex(u32)` / `EnumIndex(u32)` opaque
  newtypes work. The pool stores flat `Vec`s; `DynamicMessage` holds an
  `Arc<DescriptorPool>` and dereferences indices through it. No cycles, no
  leaks.

- **Editions feature resolution at pool-build time.** Presence, packed,
  delimited, and enum openness are pre-resolved into `FieldDescriptor` flags.
  The `editions_feature_resolution` test specifically validates that
  editions 2023 packs by default and that field-level overrides
  (`features.repeated_field_encoding = EXPANDED`) work. This is the
  correctness gap the external `buffa-reflect` crate has — the conformance
  suite would have caught it on the first run.

- **`Reflectable`/`ReflectCow` mode-switching contract.** The hand-written
  `impl Reflectable for ReflectablePerson<'_>` (`buffa-test/tests/reflectable.rs`)
  is ~10 LOC per message — exactly the design's "minimal codegen" estimate.
  The call site `foo.reflect().get(fd)` works through `ReflectCow::Deref` to
  `&dyn ReflectMessage`, which is the contract that makes the future vtable
  mode a zero-diff change. The `generic_function_over_dyn_reflect_message`
  test demonstrates a `&dyn ReflectMessage`-keyed interceptor reading a field
  by name from any reflectable message — the connect-rust use case.

### 3. The conformance suite is the right verification loop

The first `BUFFA_VIA_REFLECT=1` conformance run found 6 unexpected failures
out of 1371 binary→binary tests (99.6% pass rate from a from-scratch
descriptor-driven decoder). The 6 failures were three real bugs in standard
protobuf merge semantics:

1. **Singular message merge.** When the same message field appears multiple
   times on the wire, the parser must merge the messages (each sub-field
   merged), not replace wholesale. Fixed by `merge_into_existing_message`.
2. **Oneof last-wins.** Setting any oneof member must clear all other
   members of the same oneof. Fixed by `clear_other_oneof_members`.
3. **Oneof presence.** Oneof members always have explicit presence on the
   wire — a oneof field set to its type's default value is still present
   because the oneof discriminant carries that information. The pool's
   presence resolution incorrectly mapped proto3 oneof members to `Implicit`,
   which made the implicit-presence default-skip strip them. Fixed by special-
   casing `oneof_index.is_some()` in `link_field`'s presence resolution.

None of these would have surfaced from the unit tests. All three are textbook
protobuf semantics that protobuf-go and protobuf-es get right and that any
reimplementation gets wrong on the first try. This validates the design's
choice to make the conformance runner mode a phase deliverable rather than
an aspiration.

### 4. Bridge cost is what the design predicts

`DynamicMessage::from_message` is one `encode_to_vec()` plus one
`decode()` — exactly the encode→decode round-trip the design accepts as the
v1 bridge cost. `for_each_set` after the bridge walks a `BTreeMap`. There's
no profiling yet, but the structure matches the ~40 B/field-in-map analysis;
the bridge is correct and the vtable optimization remains a separable future
phase.

### 5. The `Deref` lifetime needs `+ 'a`

A small implementation detail not in the design doc:
`impl Deref for ReflectCow<'a>` needs `type Target = dyn ReflectMessage + 'a`
(with the explicit lifetime). Without it, the `Borrowed` variant can't be
returned because the trait object's data lives for `'a`, not the implicit
`'static` that bare `dyn ReflectMessage` carries. This is correct and
intentional Rust behavior, but it's a one-character footgun that the design
doc should mention so future implementers don't hit a confusing borrow-checker
error.

## What's not built

These were explicitly out of scope for the viability prototype but are part
of the phased plan:

- **Phase 3 codegen** (`generate_reflection(true)` → `FILE_DESCRIPTOR_SET_BYTES`
  plus `impl Reflectable`). The hand-written impl in `tests/reflectable.rs` is
  the codegen target; what's missing is the `buffa-codegen` plumbing to emit
  it. This is mechanical work — the pattern is identical to `MessageName`
  emission — but it's the most fragile part of the codegen and warrants its
  own focused PR.
- **`MergeSink` / `ReflectCowMut`** — the bridge-mode write-back path. The
  design's `clear() + merge_from_slice()` on `Drop` mechanism is sketched in
  the trait module's comments but not implemented. Mutable reflection works
  through `DynamicMessage`'s `set`/`clear` directly; what's missing is the
  ergonomic path back into a generated struct.
- **Phase 4** (`ReflectRegistry`, `Any` via reflection, dynamic extensions).
  `Any` JSON form is the largest tail in `known_failures_reflect.txt`
  (~20 of the 46 expected failures) — it requires resolving the inner
  `@type` to a descriptor and recursing, which is the registry's job.
- **Text format on `DynamicMessage`** — 883 conformance tests skipped.
  Same shape as JSON (mechanical descriptor walk) but with the
  textproto tokenizer rather than serde.
- **Strict-validation rejections** (~26 of the 46 expected failures) —
  duplicate JSON keys, float/timestamp/duration bounds checks,
  FieldMask round-trip-safe validation. Each is a small isolated
  check; deferred until the codec API stabilizes so they can be
  configurable (the existing typed JSON path has the same lenient
  defaults).
- **`no_std`** — the prototype uses `OnceLock` in the conformance runner
  and `std::error::Error` in `PoolError`, both of which are `std`-only. The
  reflection runtime itself (`reflect/`) is `alloc`-only; the `std`
  dependencies are at the edges and can be replaced.

## Was descriptor-driven JSON "actually fairly simple"?

Yes. The serde `Serialize` impl is a loop over `md.fields` with a
per-`SingularKind` dispatch — `json_name` and presence are pre-resolved
on `FieldDescriptor`, so the walk is entirely mechanical (~150 LOC for
the core, ~250 more for the per-scalar special cases: 64-bit ints as
quoted strings, bytes as base64, NaN/±Inf as strings, enum names).

The deserialize side needed `DeserializeSeed` rather than `Deserialize`
because the descriptor must travel with the call — that's a known serde
pattern, ~250 LOC for the visitor stack. The conformance suite caught
the same kind of small bugs the binary codec had: scalar `null`
handling, lowercase RFC 3339 separators, integer-string overflow
saturation. Each fix was a few lines once the failure was named.

The reflective WKT codecs are the largest single chunk (~600 LOC),
mostly because `Timestamp` needs an RFC 3339 formatter/parser and
`Struct`/`Value`/`ListValue` are recursive. None of it is hard — the
design tradeoff is that this duplicates the WKT JSON logic that's
already hand-written in `buffa-types`. The right "restructure for
merge" move is probably to extract the shared formatting helpers
(`fmt_rfc3339`, `fmt_duration`, the camelCase converters) into a
`buffa::json_helpers::wkt` module that both the typed and reflective
paths call, rather than having two implementations.

## Open question carried forward

The design's open question #4 (subsume `AnyRegistry`?) remains open. The
existing `TypeRegistry` is a tier-0 fn-pointer registry — the reflection-
backed `Any` fallback would compete with it. The instinct ("keep both, with
reflection as the fallback for unregistered types") is sound, but the
registry trait shapes need a deliberate decision before phase 4 lands so they
don't fight each other.

## Recommendation

The design is viable and the prototype validates the hard decisions. The
sequencing for upstream work:

1. **Land PR #9** — the linked descriptor types are the foundation and are
   already conformance-validated.
2. **Land the pool + reflection runtime** behind a `reflect` feature on
   `buffa-descriptor`. Split into focused PRs (pool, value/trait,
   `DynamicMessage`).
3. **Land the conformance runner mode** alongside, so every PR after step 2
   has the corpus as a regression bar.
4. **`generate_reflection(true)` codegen** — `FILE_DESCRIPTOR_SET_BYTES`
   per package mod plus `impl Reflectable` per message. This is also the
   upstream collaboration point with the external `buffa-reflect` crate,
   which currently retrofits the same thing via text-rewriting of
   buffa-codegen output.
5. **`DynamicMessage` JSON/text transcoding** — unblocks the connect-rust
   transcoding gateway.

The prototype branch (`reflect-prototype`) is local only. The commits are
focused enough that several could become PRs directly with minimal
restructuring; the conformance commit and the merge-semantics fixes are
already PR-shaped.

## 0.7.0 release-prep (2026-05-19)

After the prototype passed conformance, the reflection runtime was promoted
to a 0.7.0 release deliverable. The hardening work since:

### JSON serde + Any + bounds checks

The descriptor-driven JSON serde shipped (see "Was descriptor-driven JSON
'actually fairly simple'?" above), then the `google.protobuf.Any` codec, the
Timestamp/Duration bounds checks, the `FieldMask` round-trip-safe conversion,
the oneof-duplicate detection, and the `DynamicMessage::has()` implicit-
presence default semantics. Conformance expected-failures dropped from 46 to
**7** — six `Recommended.*` duplicate-JSON-key tests (a `serde_json`
limitation the typed JSON path also has) and one proto2 extension-key test
(extensions are phase-4).

### Dual code review (rust-code-reviewer + rust-api-ergonomics-reviewer)

Both reviews ran against the full diff. Critical and High findings fixed:

- **Untrusted-input panics** — `DescriptorPool::decode` no longer
  `.expect()`s on malformed `FileDescriptorSet` bytes; `parse_rfc3339` no
  longer panics on non-ASCII byte boundaries (switched to byte-indexed
  parsing); negative `extension_range` bounds error rather than rolling
  over via `as u32`; field numbers validate against `[1, 2^29 - 1]`.
- **Spec correctness** — `ReflectMessageMut::set` clears oneof siblings
  (the third write path after the wire decoder and JSON deserializer; CEL
  mutation through the trait surface would otherwise produce non-spec
  output); `parse_rfc3339` validates per-month day counts (Hinnant's
  algorithm silently shifts impossible dates); the pass-1/pass-2 walk
  order assertion is promoted to a release-mode `assert_eq!` because a
  desync corrupts every cross-reference in the pool.
- **API ergonomics for the CEL hot path** — `MessageDescriptor::field_by_name`
  added (`O(log n)`, indexes both proto and JSON names); `ReflectMessage::pool()`
  returns `&Arc<DescriptorPool>` so consumers can clone it to construct
  sibling messages; `ReflectMessage::get` debug-asserts the
  `FieldDescriptor` belongs to this message (a foreign descriptor with a
  colliding number silently returns the wrong value, which is worse than a
  panic); reflection types re-exported at the crate root.
- **Diagnostic quality** — `PoolError` gains `Decode(DecodeError)`,
  `MissingTypeName`, `InvalidFieldNumber`, `Clone`, and a `source()` impl;
  `decode_packed_element`/`decode_map_key` carry the real field number in
  `WireTypeMismatch` errors instead of sentinel zeros; `fmt_duration` uses
  `expect()` instead of `unwrap_or(0)` so a regression panics loudly.

### Findings deferred to user triage

Medium/Low findings from the review left for explicit decision:

- **`#[non_exhaustive]` + `pub` fields on descriptors** (ergonomics M8). The
  module docstring says "read the `pub` fields directly"; `#[non_exhaustive]`
  prevents downstream construction. CEL unit tests can't fabricate a
  `MessageDescriptor` literal — the only path is through `DescriptorPool`.
  Options: drop `#[non_exhaustive]` (accept the breakage cost), or add a
  `for_testing()` constructor under a `test-fixtures` feature, or accept
  that test fixtures are FDS files. The current state is the worst of both
  ("read-only-pub-fields" communicates "you can construct this" but
  `#[non_exhaustive]` denies it).
- **`MapKey::String` allocation per map lookup** (ergonomics M11). CEL
  evaluating `m[name]` constructs `MapKey::String(name.to_owned())` per
  access. A `lookup_map_str(&self, number, key: &str)` helper would avoid
  it. Performance, not correctness; defer until CEL benchmarks land.
- **Absent message-typed field reads allocate** (code-review M3). Reading
  an absent singular-message field returns
  `ReflectCow::Owned(Box::new(DynamicMessage::new(...)))`, the one path
  where an absent-field read allocates. CEL workloads that frequently
  check `has(x.foo)` before `x.foo.bar` can avoid it. Documented in
  `default_value_ref`'s comment; consider a `ReflectCow` variant carrying
  just `(Arc<DescriptorPool>, MessageIndex)` if it shows in profiles.
- **`empty_map` AtomicPtr leak pattern Stacked Borrows concern** — the
  `Box::leak` + `compare_exchange` pattern holds a `&'static` reference
  across the CAS-loss `Box::from_raw().drop()`. The `&'static` is never
  used after the drop, but Stacked Borrows / Miri may flag it. The
  rust-code-reviewer assessed it as "production-quality unsafe" with
  correct memory ordering; running Miri on the conformance corpus before
  tagging would close the question.
- **`MergeSink`/`ReflectCowMut` write-back path not implemented.** The
  design's bridge-mode mutable reflection. CEL is read-only over messages
  so this isn't blocking for the Minos use case; revisit when a
  field-mask-application or interceptor mutation use case appears.

### Restructure-for-merge

The branch is local-only. Restructuring for merge would split into:

1. PR #9 (`desc.rs`) — already on its own branch, force-pushed onto main.
2. `features.rs` move + `pool.rs` — pool linking and the shared feature
   resolution.
3. `reflect/{value,message,dynamic}.rs` — the reflection runtime.
4. `reflect/{json,json_wkt}.rs` — JSON serde.
5. Conformance `BUFFA_VIA_REFLECT=1` mode.
6. The tests in `buffa-test`.

Each step's regression bar is the conformance suite. The dedup of the
shared WKT formatting helpers (Timestamp RFC 3339, Duration, FieldMask
camelCase) between the typed `buffa-types` JSON impls and the reflective
`json_wkt.rs` codecs is a separate cleanup that should land before 0.7.0
ships, since drift between them is a user-visible inconsistency.

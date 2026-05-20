# Runtime Reflection for buffa — Design

**Status:** In progress (PRs #8, #9 up as drafts)
**Date:** March 2026

## Motivation

buffa currently has no way to work with protobuf messages whose schema is only known at runtime. Every operation — encode, decode, JSON transcoding, extension access — requires compile-time generated code for the specific message type. This blocks several use cases that are table-stakes for a protobuf runtime:

- **Schema registries / dynamic Any** — unpack a `google.protobuf.Any` whose payload type is discovered at runtime from a schema registry, not known at compile time.
- **gRPC server reflection** — implement `grpc.reflection.v1.ServerReflection`, which requires serving `FileDescriptorProto` bytes and supporting name-based type lookup.
- **Transcoding gateways** — convert between binary protobuf and JSON/text for arbitrary types, e.g. a connectrpc-rs HTTP/JSON gateway.
- **Dynamic extensions** — register and access extension fields whose descriptors are loaded at runtime, not baked into a codegen'd `Extension<C>` const.
- **Generic utilities** — field-mask application, deep diff/merge, validation rules — written once against a reflective API rather than reimplemented per message type.

This document captures the research comparing reference implementations, the resulting design, and the phased implementation plan.

## Prior art

Two mature reflection implementations were surveyed in depth: **protobuf-go** (the official Go runtime) and **protobuf-es** (Buf's TypeScript runtime, v2). They sit at opposite ends of a design spectrum, and Rust's constraints land buffa somewhere between.

### protobuf-go — message carries its own descriptor

Every generated struct implements `ProtoReflect() → protoreflect.Message`, yielding a reflective view. The API is descriptor-keyed: `Get(FieldDescriptor) → Value`, `Set`, `Has`, `Clear`, `Range`.

**Key implementation details:**

- **Field accessors built lazily via runtime `reflect`.** On first reflective access to a message type, Go's `reflect` package walks the struct to discover field offsets, then caches closures (`fieldInfo` structs in `internal/impl/message_reflect_field.go`) that do unsafe pointer arithmetic. No per-field codegen — the table is derived at runtime.
- **Zero-alloc `ProtoReflect()`** via an unsafe first-field pointer pun. Every generated struct has `state protoimpl.MessageState` as field zero; `ProtoReflect()` casts `&foo` to `*messageState`, which itself implements the reflection interface.
- **`Value` is a hand-rolled 24-byte tagged union** (`reflect/protoreflect/value_unsafe.go`) to avoid `interface{}` heap-boxing scalars. Explicit rationale in the source: boxing every int64 through `any` is too slow.
- **Two-level lazy descriptor init** (`internal/filedesc/`): L1 parsed at `init()` time (names, hierarchy), L2 on first access (cross-file linking, options). Keeps startup fast for binaries linking thousands of `.proto` files. Raw `FileDescriptorProto` bytes retained for process lifetime.
- **`dynamicpb.Message`** is a `map[FieldNumber]Value` + descriptor — no struct, no offsets. Simple, ~2 allocs per field access.

### protobuf-es — schema entirely external

Messages are plain `{$typeName, ...fields}` objects. **Every operation** takes `(schema, message)` as two arguments — the message does not carry its descriptor. There is **zero generated encode/decode code**; all wire I/O is a descriptor-driven interpreter loop (`from-binary.ts`, `to-binary.ts`).

**Key implementation details:**

- **`FieldKind` discriminated union** (`descriptors.ts`): `scalar | enum | message | list | map`. Flattens protobuf's orthogonal type × label × map-entry axes into one discriminant that maps 1:1 to runtime representation. TypeScript narrows exhaustively on it.
- **Edition features pre-resolved** at registry-build time. `DescField.presence`, `.packed`, `.delimitedEncoding` are direct values; consumers never touch `FeatureSet`.
- **Reflection is string-keyed property access**: `target[field.localName]`. Works because JS objects are dictionaries.
- **Generated code is minimal**: base64-encoded `FileDescriptorProto` (with `source_code_info` stripped) + TypeScript type declarations + a path index (`messageDesc(file, 0)`). Per-message overhead: one path-lookup call.
- **No separate DynamicMessage** — generated and dynamic messages use the same plain-object + interpreter path. The only difference is TypeScript-level (phantom type parameters on generated schemas).
- **Extensions stored lazily in unknown fields**, re-parsed on every `getExtension()` via a clever "synthetic one-field message" container trick.

### Where Rust lands

Rust can't sit at either pole:

| Constraint | Implication |
|---|---|
| No runtime `reflect` | Go's lazy struct-layout discovery is unavailable. Field-accessor tables must be codegen'd if they exist at all. |
| No string-keyed struct access | ES's `target[field.localName]` trick doesn't apply. |
| Monomorphized encode/decode is the perf win | ES's pure-interpreter model throws away the ~2× throughput buffa has over prost on binary. |
| Enum is a first-class tagged union | Go's 24B unsafe `Value` struct is trivially a Rust enum — zero unsafe, zero heap for scalars. |

The landing point: **Go's API shape** (descriptor-keyed get/set/has/clear), **ES's descriptor representation** (`FieldKind` enum, pre-resolved features), **Rust enum for `Value`**, with field-accessor codegen deferred as an optional optimization.

## Design

### DynamicMessage-only as v1, vtable deferred

The deferred-codegen insight: reflection use cases split into two categories.

**Dynamic-schema cases** (schema registries, gRPC reflection, transcoding, dynamic extensions) never touch a generated struct — the type is unknown at compile time by definition. A `DynamicMessage` backed by `BTreeMap<u32, Value>` + descriptor reference covers all of these. No per-type codegen required.

**Generated-instance cases** (field-mask application, middleware, generic diff/merge) want to reflectively access a `&Foo` you already hold. Without a codegen'd accessor table, the bridge is encode→decode: serialize `Foo` to bytes, decode into `DynamicMessage`, reflect. Slow (full round-trip, ~40B/field in the map vs packed struct), but correct and zero unsafe.

**v1 ships DynamicMessage-only.** The vtable — a static `&[(u32, FieldAccessor)]` table emitted by codegen with four fn-pointers (get/set/has/clear) per field, ~32B/field — is a performance optimization deferrable until a field-mask or interceptor hot path profiles badly.

**Why this differs from Go:** Go gets the vtable nearly free via runtime `reflect` — lazy discovery, zero codegen bloat. Rust pays in generated code size. The asymmetry justifies deferring until there's a concrete demand.

### Zero-diff mode switching

When the vtable lands, flipping a message between slow (bridge) and fast (vtable) reflection must require **no call-site changes**. The mechanism is a `Reflectable` trait that codegen always emits (whenever any reflection is enabled), with a body that varies by mode:

```rust
pub trait Reflectable: Message {
    fn reflect(&self) -> ReflectCow<'_>;
    fn reflect_mut(&mut self) -> ReflectCowMut<'_>;
}

pub enum ReflectCow<'a> {
    Borrowed(&'a dyn ReflectMessage),   // vtable path — 16B fat ptr, zero alloc
    Owned(Box<dyn ReflectMessage>),     // bridge path — boxed DynamicMessage
}
impl Deref for ReflectCow<'_> { type Target = dyn ReflectMessage; ... }
```

Call site is always `foo.reflect().get(fd)`. Bridge mode's body does `ReflectCow::Owned(Box::new(DynamicMessage::from_message(self)))`; vtable mode's body does `ReflectCow::Borrowed(self)` (where `Foo` directly implements `ReflectMessage`). Same source, different performance.

**Boxing the `Owned` variant is load-bearing.** Inlining `DynamicMessage` (~56B: `Arc` + `BTreeMap` + `UnknownFields`) would make `ReflectCow` ~64B. Since `ValueRef::Message(ReflectCow)` sets the floor for `ValueRef`'s size, that would triple it from ~32B to ~72B — every `get()` including scalar reads would move 72B across two cache lines. Boxing keeps it at 32B. The one extra alloc fires only at entry points and mixed-mode boundaries, where a full encode/decode is already happening — it's noise against that backdrop.

`ReflectCowMut` handles write-back for bridge mode via a `MergeSink` trait (blanket-impl'd over `Message` via the existing `clear()` + `merge_from_slice()`), with the reverse encode/decode on `Drop`.

### Per-message granularity

Reflection mode is selectable per message via FQN glob patterns, first-match-wins — the same model as prost's `type_attribute`:

```rust
// build.rs
.reflection(ReflectMode::Bridge)                       // default
.reflection_for("mypkg.HotPath", ReflectMode::VTable)
.reflection_for("mypkg.Internal.**", ReflectMode::Off)

// protoc
--buffa_opt=reflect=bridge
--buffa_opt=reflect_for=mypkg.HotPath:vtable
```

`ReflectMode` is three-state: `Off` (no `Reflectable` impl), `Bridge` (slow body), `VTable` (fast body + static table). Mixed modes are handled by `ValueRef::Message` holding a `ReflectCow` rather than a bare `&dyn ReflectMessage` — a vtable `Foo` containing a bridge `Bar` degrades at the boundary but stays source-compatible.

### Descriptor representation

Follow ES: **linked, immutable, feature-resolved** wrappers over the raw proto. Not the raw `DescriptorProto` directly.

```rust
pub enum SingularKind {
    Scalar(ScalarType),
    Enum(EnumIndex),
    Message(MessageIndex),
}

pub enum FieldKind {              // Copy — no Box needed
    Singular(SingularKind),
    List(SingularKind),
    Map { key: ScalarType, value: SingularKind },
}

pub struct FieldDescriptor {
    pub name: String,
    pub json_name: String,
    pub number: u32,
    pub kind: FieldKind,
    pub presence: FieldPresence,  // pre-resolved from edition features
    pub packed: bool,             // pre-resolved
    pub delimited: bool,          // pre-resolved (GROUP encoding)
    pub oneof_index: Option<u16>,
}
```

The `SingularKind` split makes `List(List(...))` and nested maps **unrepresentable at the type level** — protobuf doesn't allow them — and keeps `FieldKind` `Copy` with no heap allocation.

**Cross-references use pool indices**, not `Arc`. `MessageIndex(u32)` and `EnumIndex(u32)` are opaque `Copy` newtypes. This avoids `Arc` cycles (message A has field of type B, B has field of type A would leak), keeps descriptors `Send + Sync` trivially, and the `pub(crate)` inner prevents forgery.

`DescriptorPool` owns the flat arrays of `MessageDescriptor`/`EnumDescriptor` and does name linking (resolve `type_name: ".pkg.Foo"` → `MessageIndex`) plus edition feature resolution at build time.

### no_std

The entire reflection layer is `alloc`-only: `Box`, `Vec`, `hashbrown::HashMap`, `Arc`. Global registry uses the existing `AtomicPtr` + leak-on-replace pattern. The `reflect` feature is orthogonal to `std`.

## Crate layout

```text
buffa-descriptor/           NEW — runtime descriptor types + pool
  src/generated/            FileDescriptorProto etc, moved from buffa-codegen
  src/desc.rs               MessageDescriptor, FieldDescriptor, FieldKind
  src/pool.rs               DescriptorPool: link, feature-resolve, lookup

buffa/src/reflect/          behind feature = "reflect"
  value.rs                  Value, ValueRef
  message.rs                ReflectMessage trait (dyn-safe, storage-agnostic)
  reflectable.rs            Reflectable trait, ReflectCow, MergeSink
  dynamic.rs                DynamicMessage (BTreeMap<u32, Value> + descriptor)
  registry.rs               ReflectRegistry (FQN → MessageType)
  vtable.rs                 DEFERRED — MessageVTable, FieldAccessor
```

The `buffa-descriptor` crate has only `buffa` as a dependency — no quote/syn/prettyplease — so the runtime can depend on descriptor types without pulling the codegen toolchain.

## Phased plan

| PR | Scope | Status |
|---|---|---|
| **1a** | `buffa-descriptor` crate extraction — move generated descriptor types + vendored protos out of `buffa-codegen`. Structural only, `pub use` re-export for zero downstream churn. | [#8](https://github.com/anthropics/buffa/pull/8), draft |
| **1b** | Linked descriptor types: `MessageDescriptor`, `FieldDescriptor`, `FieldKind` + `SingularKind`, `EnumDescriptor`, pool indices. | [#9](https://github.com/anthropics/buffa/pull/9), draft |
| **1c** | `DescriptorPool`: name linking, feature resolution, `u16` field-count validation, by-name lookup. | next |
| **2** | `DynamicMessage` + `Value` + `ReflectMessage` trait + `ReflectCow`/`MergeSink`. Map-backed reflection works end-to-end for runtime-loaded schemas. | |
| **3** | Codegen: `FILE_DESCRIPTOR_BYTES` per file, `Reflectable` impl per type (bridge-mode body, ~8 LOC/message), `ReflectMode` + FQN-glob filter in `CodeGenContext` + protoc opts. Uniform `foo.reflect()` entry point. | |
| **4** | `ReflectRegistry`, `Any` via reflection, dynamic extensions (register `FieldDescriptor` at runtime, get/set by descriptor). | |
| *deferred* | VTable emission (`ReflectMode::VTable`). Swap `Reflectable` body + emit static accessor table. Zero downstream churn by design. | |
| *deferred* | JSON/text transcoding on `DynamicMessage`. Enables schema-registry-style format conversion for arbitrary descriptors. | |

**Conformance:** add a fourth runner mode `BUFFA_VIA_REFLECT=1` routing binary input through `decode → DynamicMessage → encode` to verify reflection round-trips.

## Open questions

1. **Pool lifetime** — cross-references within the pool are indices (resolved in PR #9). Remaining question: how is the pool *itself* held by `DynamicMessage`? `&'static` (leak-on-link) for codegen-embedded descriptors vs `Arc<DescriptorPool>` for runtime-loaded. A small enum unifies at the `DynamicMessage` level.

2. **VTable encoding** (deferred with vtable) — `*const ()` type-erasure is the minimal-unsafe approach. `core::mem::offset_of!` (stable since 1.77) + a typed-kind enum is the alternative, potentially shrinking the table and reducing closure-body count. Worth prototyping both when the vtable lands.

3. **`ValueMut` ergonomics** — Go's `Mutable()` is get-or-create for composites. Rust's borrow checker complicates nested mutation. Less acute while v1 is DynamicMessage-only (map mutation is simpler than struct-field mutation). Revisit when vtable lands.

4. **Subsume `AnyRegistry`?** — the current `AnyRegistry` stores per-type `to_json`/`from_json` fn-pointers. A reflection-backed path could replace it: look up descriptor, use `DynamicMessage`. Simpler, but adds `reflect` as a dep of `Any` JSON handling. The fn-pointer model stays leaner for json-only builds. Likely keep both, with reflection as the fallback for unregistered types.

## References

- protobuf-go: `reflect/protoreflect/`, `internal/impl/`, `types/dynamicpb/` at <https://github.com/protocolbuffers/protobuf-go>
- protobuf-es v2: `packages/protobuf/src/{descriptors,reflect,registry}.ts` at <https://github.com/bufbuild/protobuf-es>
- Protobuf editions feature resolution: <https://protobuf.dev/editions/features/>

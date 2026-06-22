# custom-types — pluggable owned types end to end

This example shows how to replace every owned representation in a generated message — strings, bytes, repeated collections, the map container, and the message-field pointer — with crate-local types, and then round-trip the result through both binary and proto3 JSON. It is the copy-paste reference for an integrator who wants to bring their own storage (for example `flexstr`, `smallvec`, `indexmap`, or `smallbox`) into a buffa-generated schema.

Run it with:

```sh
cargo run -p example-custom-types
```

## The moving parts

The example is three small files, and they are easiest to read in this order:

- [`build.rs`](build.rs) wires each `buffa_build::Config` knob to a type defined in this crate.
- [`src/types/`](src/types) defines the five newtypes those knobs point at — one file per knob.
- [`src/main.rs`](src/main.rs) builds a `Record`, encodes and decodes it, and proves at compile time that every field has the custom type.

## Pointing the knobs at crate-local types

Every override is set in `build.rs`. The string and bytes knobs take a complete type path; the repeated and box knobs take a *template* with a literal `*` that codegen substitutes for the element or pointee type; and the map knob takes a bare path that codegen applies as `path<K, V>`.

```rust
buffa_build::Config::new()
    .files(&["proto/record.proto"])
    .generate_json(true)
    .string_type_custom("crate::types::FlexStr")
    .bytes_type_custom("crate::types::SmallBytes")
    .repeated_type_custom("crate::types::SmallVec<*>")
    .box_type_custom("crate::types::SmallBox<*>")
    .map_type_custom("crate::types::IndexMap")
    .compile()?;
```

If all you want from a custom map is deterministic key order, the built-in `MapRepr::BTreeMap` preset gives that without a newtype — `.map_type(MapRepr::BTreeMap)` instead of `.map_type_custom(...)`.

[`proto/record.proto`](proto/record.proto) is a single `Record` message with one field per knob — including a `repeated int64`, a `map<int64, string>`, and a oneof with a message variant, so the non-trivial proto3 JSON paths are exercised too.

## The newtype pattern

Each custom type wraps a *foreign* storage type. The orphan rule forbids implementing `buffa::ProtoString` (or `ProtoBytes`, `ProtoList`, `ProtoBox`) on `flexstr::SharedStr` directly, because both the trait and the type are defined outside this crate — so a thin `#[repr(transparent)]` newtype in this crate is the bridge. `FlexStr` is the template; the other three follow the same shape.

```rust
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct FlexStr(pub flexstr::SharedStr);

impl buffa::ProtoString for FlexStr {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        core::str::from_utf8(payload.as_slice())
            .map(|s| Self(flexstr::SharedStr::from_ref(s)))
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}
```

The remaining trait surface (`Deref<Target = str>`, `AsRef<str>`, `From<String>`, `From<&str>`) is `ProtoString`'s supertrait set, and each impl is a one-line forward to the inner type. The `assert_transparent!` macro in [`src/types/mod.rs`](src/types/mod.rs) freezes the zero-cost guarantee — if a second field ever sneaks into the wrapper, the build fails.

## What each newtype needs for JSON

Under `generate_json(true)`, the five traits have different serde requirements, and getting them wrong is the most common stumbling point. The doc comment on each newtype explains its specific case; the summary is:

| Newtype | Needs its own `Serialize`/`Deserialize`? | Why |
| --- | --- | --- |
| `FlexStr` (`ProtoString`) | Yes — `#[serde(transparent)]` | A singular `string` routes through buffa's with-module, but a `repeated string` element or a map value serializes through the type's native serde. |
| `SmallBytes` (`ProtoBytes`) | No | Codegen routes all bytes positions through buffa's base64 with-module, which only needs `AsRef<[u8]>` / `From<Vec<u8>>`. |
| `SmallVec<T>` (`ProtoList`) | Yes — `#[serde(transparent)]` | A repeated field whose element type is proto-JSON-compliant on its own (string, int32, message, …) is serialized through the collection's native serde. |
| `IndexMap<K, V>` (`MapStorage`) | Yes — `#[serde(transparent)]` | An integer-keyed map routes through buffa's `string_key_map` with-module (which only needs `MapStorage`), but a string-keyed map serializes through the container's native serde. |
| `SmallBox<T>` (`ProtoBox`) | `Serialize` only | An optional message field goes through `MessageField`'s blanket serde, and every deserialize path constructs via `ProtoBox::new` — so only the oneof *serialize* arm reaches the pointer's own `Serialize`. |

## The compile-time guard

`assert_field_types` in [`src/main.rs`](src/main.rs) coerces a reference to each generated field to the expected custom type. The guarantee comes from the function being type-checked, not from anything it does at runtime — so if a knob ever silently regresses to the default representation, this example stops compiling.

```rust
fn assert_field_types(r: &Record) {
    let _: &FlexStr = &r.id;
    let _: &SmallBytes = &r.payload;
    let _: &SmallVec<i64> = &r.samples;
    let _: &SmallVec<FlexStr> = &r.tags;
    let _: &IndexMap<i64, FlexStr> = &r.attributes;
    let _: &buffa::MessageField<Metadata, SmallBox<Metadata>> = &r.metadata;
}
```

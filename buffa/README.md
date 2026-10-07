# buffa

[![crates.io](https://img.shields.io/crates/v/buffa.svg)](https://crates.io/crates/buffa)
[![docs.rs](https://img.shields.io/docsrs/buffa)](https://docs.rs/buffa)

The runtime crate for **buffa** — a pure-Rust Protocol Buffers implementation
with first-class [editions] support, zero-copy views, and `no_std` compatibility.

This crate contains the `Message` trait, wire-format encode/decode primitives,
`MessageField` / `EnumValue` / `UnknownFields` container types, and the
zero-copy view layer. Generated code emitted by `buffa-build` or
`protoc-gen-buffa` depends on this crate.

[editions]: https://protobuf.dev/editions/overview/

## Quick start

See the [workspace README] for end-to-end setup with `buf generate` or
`build.rs`. In short:

```rust,ignore
use buffa::Message;

let bytes = person.encode_to_vec();
let decoded = Person::decode_from_slice(&bytes)?;
```

For untrusted input, tighten limits with `DecodeOptions`:

```rust,ignore
use buffa::DecodeOptions;

let msg: Person = DecodeOptions::new()
    .with_recursion_limit(50)
    .with_max_message_size(1024 * 1024)  // 1 MiB
    .decode_from_slice(&bytes)?;
```

[workspace README]: https://github.com/anthropics/buffa#readme

## Feature flags

| Flag | Default | Enables |
|------|:-------:|---------|
| `std` | ✓ | `std::io::Read` decoders, `HashMap` for map fields, thread-local JSON parse options |
| `json` |  | Proto3 JSON via `serde` |
| `text` |  | Text format (`textproto`) encode/decode |
| `arbitrary` |  | `arbitrary::Arbitrary` impls for fuzzing |

With `default-features = false` the crate is `#![no_std]` (requires `alloc`).

## Documentation

- API reference: <https://docs.rs/buffa>
- User guide: [docs/guide.md]
- Design rationale: [DESIGN.md]

[docs/guide.md]: https://github.com/anthropics/buffa/blob/main/docs/guide.md
[DESIGN.md]: https://github.com/anthropics/buffa/blob/main/DESIGN.md

## Conformance

buffa passes the full protobuf [conformance suite] for binary, JSON, and
text-format encoding (both `std` and `no_std` builds).

[conformance suite]: https://github.com/protocolbuffers/protobuf/tree/main/conformance

## License

Apache-2.0

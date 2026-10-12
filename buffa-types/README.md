# buffa-types

Rust types for Google's Protocol Buffers well-known types, including
`Timestamp`, `Duration`, `Any`, `FieldMask`, `Struct`, and wrapper types.
The generated message code is checked in, so using this crate does not require
`protoc` or a build script.

## Quick start

Add both the runtime and well-known-types crates:

```sh
cargo add buffa
cargo add buffa-types
```

Encode and decode a timestamp with buffa's `Message` trait:

```rust
use buffa::Message;
use buffa_types::Timestamp;

let timestamp = Timestamp {
    seconds: 1_700_000_000,
    nanos: 0,
    ..Default::default()
};
let bytes = timestamp.encode_to_vec();
let decoded = Timestamp::decode_from_slice(&bytes).unwrap();
assert_eq!(decoded, timestamp);
```

Common types such as `Timestamp`, `Duration`, `Any`, `FieldMask`, `Struct`,
`Value`, `ListValue`, and `Empty` are re-exported from `buffa_types`. Wrapper
messages are available under `buffa_types::google::protobuf`.

## Features

| Flag | Default | Enables |
|------|:-------:|---------|
| `std` | ✓ | `Timestamp` conversions to and from `std::time::SystemTime`, and `Timestamp::now` |
| `json` |  | Proto3 JSON via `serde` for `Timestamp`, `Duration`, `Any`, `Struct`, `Value`, `ListValue`, `FieldMask`, `Empty` and the wrappers |
| `chrono` |  | Conversions to and from `chrono` time types |
| `jiff` |  | Conversions to and from `jiff` time types |
| `reflect` |  | Runtime reflection for well-known types; also enables `std` |
| `arbitrary` |  | `arbitrary::Arbitrary` implementations |

`Api`, `Type`, `Enum`, `SourceContext` and the messages they contain have no
`serde` impls, with or without `json`.

For JSON, enable the feature and add `serde_json` to your crate:

```sh
cargo add buffa-types --features json
cargo add serde_json
```

The crate also supports `no_std` with `alloc`. A `no_std` build needs the
default features of both crates off, because the default `std` feature of
`buffa` applies to the whole build:

```sh
cargo add buffa --no-default-features
cargo add buffa-types --no-default-features
```

The conversions between `Duration` and `core::time::Duration` do not need
`std`.

See the [crate documentation](https://docs.rs/buffa-types) for conversion
details and the complete API.

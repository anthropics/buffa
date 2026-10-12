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

Encode and decode a timestamp with Buffa's `Message` trait:

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

| Feature | Default | Enables |
|---|:---:|---|
| `std` | Yes | `Timestamp` conversions to and from `std::time::SystemTime`, and `Timestamp::now` |
| `json` | No | ProtoJSON serde implementations for well-known types |
| `chrono` | No | Conversions to and from `chrono` time types |
| `jiff` | No | Conversions to and from `jiff` time types |
| `reflect` | No | Runtime reflection for well-known types; also enables `std` |
| `arbitrary` | No | `arbitrary::Arbitrary` implementations |

For JSON, enable the feature and add `serde_json` to your crate:

```sh
cargo add buffa-types --features json
cargo add serde_json
```

The crate also supports `no_std` with `alloc`. Disable the default `std`
feature with `default-features = false` when needed. The conversions between
`Duration` and `core::time::Duration` do not need `std`.

See the [crate documentation](https://docs.rs/buffa-types) for conversion
details and the complete API.

# buffa-types

Rust types for Google's Protocol Buffers well-known types, including
`Timestamp`, `Duration`, `Any`, `FieldMask`, `Struct`, and wrapper types.
The generated message code is checked in, so using this crate does not require
`protoc` or a build script.

## Quick start

Add both the runtime and well-known-types crates:

```toml
[dependencies]
buffa = "0.9"
buffa-types = "0.9"
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
| `std` | Yes | Standard-library integrations, including `SystemTime` and `Duration` conversions |
| `json` | No | ProtoJSON serde implementations for well-known types |
| `chrono` | No | Conversions to and from `chrono` time types |
| `jiff` | No | Conversions to and from `jiff` time types |
| `reflect` | No | Runtime reflection for well-known types; also enables `std` |
| `arbitrary` | No | `arbitrary::Arbitrary` implementations |

For JSON, enable the feature and add `serde_json` to your crate:

```toml
buffa-types = { version = "0.9", features = ["json"] }
serde_json = "1"
```

The crate also supports `no_std` with `alloc`. Disable the default `std`
feature with `default-features = false` when needed.

See the [crate documentation](https://docs.rs/buffa-types) for conversion
details and the complete API.

# buffa-yaml

YAML encoding and decoding for [buffa](https://github.com/anthropics/buffa)
Protocol Buffers messages. This crate uses buffa's protobuf JSON
serialization rules with a YAML carrier.

**Status:** Experimental and currently in-tree only (`publish = false`).
It is not yet available on crates.io.

## Quick start

Message types must be generated with JSON support enabled
(`json = true`) so they implement `serde::Serialize` and
`serde::Deserialize`.

```rust
# fn main() -> Result<(), buffa_yaml::Error> {
use buffa_types::Timestamp;

let timestamp = Timestamp::from_unix_secs(1_700_000_000);
let yaml = buffa_yaml::to_string(&timestamp)?;
let decoded: Timestamp = buffa_yaml::from_str(&yaml)?;

assert_eq!(decoded, timestamp);
# Ok(())
# }
```

## API

- **Owned messages:** `to_string` and `to_writer` encode to YAML;
  `from_str`, `from_slice`, and `from_reader` decode into owned messages.
- **Borrowed views:** `to_string_view` and `to_writer_view` encode generated
  zero-copy message views. To encode an `OwnedView`, pass
  `handle.reborrow()`. YAML decoding always creates an owned message.
- **Errors:** `Error::location()` returns a 1-based line and column when
  available. `Error::io_error_kind()` reports reader/writer failures.

## YAML behavior

The underlying message's protobuf JSON mapping still applies:

- Output uses camelCase field names, while input accepts camelCase and
  snake_case for generated message fields.
- 64-bit integers are emitted as strings; bytes as base64; enums as names.
- Well-known types use their protobuf JSON forms, including RFC 3339
  `Timestamp` strings and decimal-seconds `Duration` strings.
- `serde_norway` uses restricted YAML 1.1 scalar resolution: unquoted
  `name: no` is a string, not a boolean.

This initial implementation does not include the protoyaml-go extensions
for byte-size suffixes, Go-style durations, field-number addressing, or
snippet diagnostics.

## Untrusted YAML

YAML anchors and aliases are expanded while parsing. A small document with
deeply nested aliases can use disproportionate memory and CPU. Limit the
size of untrusted input before calling `from_str`, `from_slice`, or
`from_reader`; a source-byte limit alone does not bound alias expansion.

# buffa

[![crates.io](https://img.shields.io/crates/v/buffa.svg)](https://crates.io/crates/buffa)
[![docs.rs](https://img.shields.io/docsrs/buffa)](https://docs.rs/buffa)
[![CI](https://github.com/anthropics/buffa/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/anthropics/buffa/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/crates/msrv/buffa)](Cargo.toml)
[![deps.rs](https://deps.rs/repo/github/anthropics/buffa/status.svg)](https://deps.rs/repo/github/anthropics/buffa)
[![no_std](https://img.shields.io/badge/no__std-compatible-blue)](docs/guide.md#no_std-usage)
[![License](https://img.shields.io/crates/l/buffa)](LICENSE)

A pure-Rust Protocol Buffers implementation with first-class [protobuf editions](https://protobuf.dev/editions/overview/) support. Written by Claude ❣️

## Why buffa?

The Rust ecosystem lacks an actively maintained, pure-Rust library that supports [protobuf editions](https://protobuf.dev/editions/overview/). Buffa fills that gap with a ground-up design that treats editions as the core abstraction. It passes the full protobuf conformance suite — binary, JSON, and text — with zero expected failures.

## Features

- **Editions-first.** Proto2 and proto3 are understood as feature presets within the editions model. One code path, parameterized by resolved features.

- **Two-tier owned/borrowed types.** Each message generates both `MyMessage` (owned, heap-allocated) and `MyMessageView<'a>` (zero-copy from the wire). `OwnedView<V>` wraps a view with its backing `Bytes` buffer for use across async boundaries.

- **`MessageField<T>`.** Optional message fields deref to a default instance when unset -- no `Option<Box<T>>` unwrapping ceremony.

- **`EnumValue<T>`.** Type-safe open enums with proper Rust `enum` types and preservation of unknown values, instead of raw `i32`.

- **Linear-time serialization.** Cached encoded sizes prevent the exponential blowup that affects libraries without a size-caching pass.

- **Unknown field preservation.** Round-trip fidelity for proxy and middleware use cases.

- **`no_std` + `alloc`.** The core runtime works without `std`, including JSON serialization via serde. Enabling `std` adds `std::io` integration, `std::time` conversions, and thread-local JSON parse options.

## Wire formats

buffa supports **binary**, **JSON**, and **text** protobuf encodings:

- **Binary wire format** -- full support for all scalar types, nested messages, repeated/packed fields, maps, oneofs, groups, and unknown fields.

- **Proto3 JSON** -- canonical protobuf JSON mapping via optional `serde` integration. Includes well-known type serialization (Timestamp as RFC 3339, Duration as `"1.5s"`, int64/uint64 as quoted strings, bytes as base64, etc.).

- **Text format (`textproto`)** -- the human-readable debug format. Covers `Any` expansion (`[type.googleapis.com/...] { ... }`), extension bracket syntax (`[pkg.ext] { ... }`), and group/DELIMITED fields. `no_std`-compatible.

## Unsupported features

These are intentionally out of scope:

- **Runtime reflection** (`DynamicMessage`, descriptor-driven introspection) — planned for a future release. The descriptor types are now available in `buffa-descriptor` as a first step. Buffa remains a codegen-first library; if you need schema-agnostic processing today, consider preserving unknown fields or using `Any`.
- **Proto2 optional-field getter methods** — `[default = X]` on `optional` fields does not generate `fn field_name(&self) -> T` unwrap-to-default accessors. Custom defaults are applied only to `required` fields via `impl Default`. Optional fields are `Option<T>`; use pattern matching or `.unwrap_or(X)`.
- **Scoped `JsonParseOptions` in `no_std`** — serde's `Deserialize` trait has no context parameter, so runtime options must be passed through ambient state. In `std` builds, [`with_json_parse_options`] provides per-closure, per-thread scoping via a thread-local. In `no_std` builds, [`set_global_json_parse_options`] provides process-wide set-once configuration via a global atomic. The two APIs are mutually exclusive. The `no_std` global supports singular-enum accept-with-default but not repeated/map container filtering (which requires scoped strict-mode override).

[`with_json_parse_options`]: https://docs.rs/buffa/latest/buffa/json/fn.with_json_parse_options.html
[`set_global_json_parse_options`]: https://docs.rs/buffa/latest/buffa/json/fn.set_global_json_parse_options.html

## Known limitations

These are gaps we intend to address in future releases:

- **Closed-enum unknown values in packed-repeated view decode** are silently dropped (not routed to unknown fields). The owned decoder handles this correctly; the view decoder handles singular, optional, oneof, and unpacked repeated correctly. Packed blobs have no per-element tag to borrow, so the zero-copy `UnknownFieldsView<'a>` has no span to reference.
- **Closed-enum unknown values in map values** are silently dropped (not routed to unknown fields). The proto spec requires the entire map entry (key + value) to go to unknown fields, which requires re-encoding. This affects proto2 schemas with `map<K, ClosedEnum>` where an evolved sender adds new enum values.

## Semver and API stability

Buffa is pre-1.0. We follow the [Rust community convention](https://doc.rust-lang.org/cargo/reference/semver.html) for 0.x crates: breaking changes increment the **minor** version (0.1.x → 0.2.0), additive changes increment the **patch** version (0.1.0 → 0.1.1). Pin to a minor version (`buffa = "0.4"`) to avoid surprises.

The generated code API (struct shapes, `Message` trait, `MessageView` trait, `EnumValue`, `MessageField`) is considered the primary stability surface. Internal helper modules marked `#[doc(hidden)]` (`__private`, `__buffa_*` fields) may change at any time.

## Quick start

### Using `buf generate` (recommended)

Install [buf](https://buf.build/docs/installation) and [the protoc plugins](docs/guide.md#installing-the-protoc-plugins), then create a `buf.gen.yaml`:

```yaml
version: v2
plugins:
  - local: protoc-gen-buffa
    out: src/gen
  - local: protoc-gen-buffa-packaging
    out: src/gen
    strategy: all
```

```sh
buf generate
```

### Using `buffa-build` in `build.rs`

Alternatively, use `buffa-build` for a `build.rs`-based workflow (requires `protoc` on PATH):

```rust,ignore
// build.rs
fn main() {
    buffa_build::Config::new()
        .files(&["proto/my_service.proto"])
        .includes(&["proto/"])
        .compile()
        .unwrap();
}
```

### Encoding and decoding

```rust,ignore
use buffa::Message;

// Encode
let msg = MyMessage { id: 42, name: "hello".into(), ..Default::default() };
let bytes = msg.encode_to_vec();

// Decode (owned)
let decoded = MyMessage::decode_from_slice(&bytes).unwrap();

// Decode (zero-copy view)
let view = MyMessageView::decode_view(&bytes).unwrap();
println!("name: {}", view.name); // &str, no allocation

// Decode (owned view — zero-copy + 'static, for async/RPC use)
let owned_view = OwnedView::<MyMessageView>::decode(bytes.into()).unwrap();
println!("name: {}", owned_view.name); // still zero-copy, but 'static + Send
```

### JSON serialization (with `json` feature)

```rust,ignore
let json = serde_json::to_string(&msg).unwrap();
let decoded: MyMessage = serde_json::from_str(&json).unwrap();
```

## Documentation

- **[User Guide](docs/guide.md)** — comprehensive guide to buffa's API, generated code shape, encoding/decoding, views, JSON, well-known types, and editions support.
- **[Migrating from prost](docs/migration-from-prost.md)** — step-by-step migration guide with before/after code examples.
- **[Migrating from protobuf](docs/migration-from-protobuf.md)** — migration guide covering both stepancheg v3 and Google official v4.

## Workspace layout

| Crate | Purpose |
|---|---|
| `buffa` | Core runtime: `Message` trait, wire format codec, `no_std` support |
| `buffa-types` | Well-known types: Timestamp, Duration, Any, Struct, wrappers, etc. |
| `buffa-descriptor` | Protobuf descriptor types (`FileDescriptorProto`, `DescriptorProto`, ...) |
| `buffa-codegen` | Code generation from protobuf descriptors |
| `buffa-build` | `build.rs` helper for invoking codegen via `protoc` |
| `protoc-gen-buffa` | `protoc` plugin binary |

## Performance

Throughput comparison across five representative message types, measured on an Intel Xeon Platinum 8488C (x86_64). Cross-implementation benchmarks run in Docker for toolchain consistency (`task bench-cross`). Higher is better.

### Binary decode

![Binary decode — ApiResponse](benchmarks/charts/binary-decode-api_response.svg)
![Binary decode — LogRecord](benchmarks/charts/binary-decode-log_record.svg)
![Binary decode — AnalyticsEvent](benchmarks/charts/binary-decode-analytics_event.svg)
![Binary decode — GoogleMessage1](benchmarks/charts/binary-decode-google_message1_proto3.svg)
![Binary decode — MediaFrame](benchmarks/charts/binary-decode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | buffa (view) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|
| ApiResponse | 862 | 1,475 (+71%) | 756 (−12%) | 676 (−22%) | 695 (−19%) | 269 (−69%) |
| LogRecord | 722 | 1,984 (+175%) | 712 (−1%) | 676 (−6%) | 857 (+19%) | 247 (−66%) |
| AnalyticsEvent | 199 | 320 (+61%) | 254 (+28%) | 194 (−3%) | 361 (+82%) | 88 (−56%) |
| GoogleMessage1 | 1,014 | 1,341 (+32%) | 956 (−6%) | 931 (−8%) | 639 (−37%) | 338 (−67%) |
| MediaFrame | 16,816 | 73,004 (+334%) | 9,648 (−43%) | 23,516 (+40%) | 17,633 (+5%) | 1,241 (−93%) |

</details>

### Binary encode

![Binary encode — ApiResponse](benchmarks/charts/binary-encode-api_response.svg)
![Binary encode — LogRecord](benchmarks/charts/binary-encode-log_record.svg)
![Binary encode — AnalyticsEvent](benchmarks/charts/binary-encode-analytics_event.svg)
![Binary encode — GoogleMessage1](benchmarks/charts/binary-encode-google_message1_proto3.svg)
![Binary encode — MediaFrame](benchmarks/charts/binary-encode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | prost | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|
| ApiResponse | 2,543 | 1,810 (−29%) | 1,013 (−60%) | 560 (−78%) |
| LogRecord | 4,018 | 3,093 (−23%) | 1,642 (−59%) | 303 (−92%) |
| AnalyticsEvent | 656 | 357 (−46%) | 511 (−22%) | 160 (−76%) |
| GoogleMessage1 | 2,594 | 1,808 (−30%) | 869 (−67%) | 360 (−86%) |
| MediaFrame | 45,990 | 38,514 (−16%) | 10,463 (−77%) | 1,647 (−96%) |

</details>

### JSON encode

![JSON encode — ApiResponse](benchmarks/charts/json-encode-api_response.svg)
![JSON encode — LogRecord](benchmarks/charts/json-encode-log_record.svg)
![JSON encode — AnalyticsEvent](benchmarks/charts/json-encode-analytics_event.svg)
![JSON encode — GoogleMessage1](benchmarks/charts/json-encode-google_message1_proto3.svg)
![JSON encode — MediaFrame](benchmarks/charts/json-encode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | prost | Go |
|---------|------:|------:|---:|
| ApiResponse | 875 | 943 (+8%) | 114 (−87%) |
| LogRecord | 1,294 | 1,407 (+9%) | 136 (−89%) |
| AnalyticsEvent | 786 | 843 (+7%) | 51 (−93%) |
| GoogleMessage1 | 961 | 1,007 (+5%) | 122 (−87%) |
| MediaFrame | 1,423 | 1,449 (+2%) | 206 (−86%) |

</details>

### JSON decode

![JSON decode — ApiResponse](benchmarks/charts/json-decode-api_response.svg)
![JSON decode — LogRecord](benchmarks/charts/json-decode-log_record.svg)
![JSON decode — AnalyticsEvent](benchmarks/charts/json-decode-analytics_event.svg)
![JSON decode — GoogleMessage1](benchmarks/charts/json-decode-google_message1_proto3.svg)
![JSON decode — MediaFrame](benchmarks/charts/json-decode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | prost | Go |
|---------|------:|------:|---:|
| ApiResponse | 706 | 303 (−57%) | 67 (−90%) |
| LogRecord | 757 | 696 (−8%) | 107 (−86%) |
| AnalyticsEvent | 268 | 233 (−13%) | 45 (−83%) |
| GoogleMessage1 | 640 | 258 (−60%) | 70 (−89%) |
| MediaFrame | 1,942 | 1,954 (+1%) | 262 (−87%) |

</details>

**Message types:** ApiResponse (~200 B, flat scalars), LogRecord (~1 KB, strings + map + nested message), AnalyticsEvent (~10 KB, deeply nested + repeated sub-messages), GoogleMessage1 (standard protobuf benchmark message), MediaFrame (~10 KB, dominated by `bytes` fields — primary body + chunked sub-blobs + named attachments).

**Libraries:** prost 0.13 + pbjson 0.7, protobuf‑v4 (Google Rust/upb, v4.33.1), Go `google.golang.org/protobuf` v1.36.6. protobuf-v4 JSON is not included as it does not provide a JSON codec.

**`prost (bytes)`** uses `prost-build`'s `.bytes(["."])` config so every proto `bytes` field is generated as `bytes::Bytes` instead of `Vec<u8>`, and decodes from a `bytes::Bytes` input to exercise `Bytes`' zero-copy `copy_to_bytes` slicing. The substitution only affects the decode path, so only decode numbers are reported — `prost (bytes)` encode tracks default `prost` by construction. On the four non-bytes messages, `prost (bytes)` tracks default `prost` within noise (and is slightly slower on `ApiResponse` where the per-message `Bytes::clone` refcount overhead isn't offset by any actual zero-copy). On `MediaFrame` it runs ~2.4× faster than default `prost` at decode, confirming that prost's feature does land when it has bytes fields to work with. buffa views are in a different regime again: they borrow directly from the input buffer for strings, bytes, and nested message bodies, so `buffa (view)` on `MediaFrame` is ~3× the `prost (bytes)` number and ~4.3× `buffa`'s own owned decode. Views also benefit on the four non-bytes messages, where prost's `bytes` feature is inert.

**Owned decode trade-offs:** buffa's owned decode is typically within ±10% of prost, trading a small throughput cost for features prost omits: unknown-field preservation by default, typed `EnumValue<E>` wrappers (not raw `i32`), and a type-stable decode loop that supports recursive message types without manual boxing. The zero-copy view path (`MyMessageView::decode_view`) sidesteps allocation entirely and is the recommended fast decode path. protobuf-v4's decode advantage on deeply-nested messages comes from upb's arena allocator — all sub-messages are bump-allocated in one arena rather than individually boxed.

## Conformance

buffa passes the protobuf binary and JSON conformance test suite (v33.5, editions up to 2024). Both `std` and `no_std` builds pass the full suite including JSON. Run with `task conformance`.

## Compiler compatibility

**[buf](https://buf.build/docs/cli/)** is the recommended way to compile `.proto` files. The buf CLI has its own built-in compiler, so no separate `protoc` install is needed — just install buf and `protoc-gen-buffa`.

**protoc** is also fully supported. `protoc-gen-buffa` and `buffa-build` work with **protoc v21.12 and later**. The minimum version varies by feature:

| Feature | Minimum protoc |
|---|---|
| Proto2 + proto3 | v21.12 |
| Editions 2023 | v27.0 |
| Editions 2024 | v33.0 |

Note that Linux distro packages (Debian Bookworm, Ubuntu 24.04) ship protoc v21.12, which does not support editions. Install protoc v27+ from [GitHub releases](https://github.com/protocolbuffers/protobuf/releases) or use buf if you need editions support.

Compatibility is tested against protoc v21.12, v22.5, v25.5, v27.3, v29.5, and v33.5 (`task protoc-compat`).

## Minimum supported Rust version

1.85

## License

Apache-2.0

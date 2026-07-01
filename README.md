# buffa

[![crates.io](https://img.shields.io/crates/v/buffa.svg)](https://crates.io/crates/buffa)
[![docs.rs](https://img.shields.io/docsrs/buffa)](https://docs.rs/buffa)
[![CI](https://github.com/anthropics/buffa/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/anthropics/buffa/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/crates/msrv/buffa)](Cargo.toml)
[![deps.rs](https://deps.rs/repo/github/anthropics/buffa/status.svg)](https://deps.rs/repo/github/anthropics/buffa)
[![no_std](https://img.shields.io/badge/no__std-compatible-blue)](docs/guide.md#no_std-usage)
[![License](https://img.shields.io/crates/l/buffa)](LICENSE)

A pure-Rust Protocol Buffers implementation with first-class [protobuf editions](https://protobuf.dev/editions/overview/) support. Written by Claude and friends ❣️

## Why buffa?

The Rust ecosystem lacks an actively maintained, pure-Rust library that supports [protobuf editions](https://protobuf.dev/editions/overview/). Buffa fills that gap with a ground-up design that treats editions as the core abstraction. It passes the full protobuf conformance suite — binary, JSON, and text — with zero expected failures.

## Features

- **Editions-first.** Proto2 and proto3 are understood as feature presets within the editions model. One code path, parameterized by resolved features.

- **Two-tier owned/borrowed types.** Each message generates both `MyMessage` (owned, heap-allocated) and `MyMessageView<'a>` (zero-copy from the wire). `OwnedView<V>` wraps a view with its backing `Bytes` buffer for use across async boundaries.

- **`MessageField<T>`.** Optional message fields deref to a default instance when unset -- no `Option<Box<T>>` unwrapping ceremony.

- **`EnumValue<T>`.** Type-safe open enums with proper Rust `enum` types and preservation of unknown values, instead of raw `i32`.

- **Linear-time serialization.** Cached encoded sizes prevent the exponential blowup that affects libraries without a size-caching pass.

- **Unknown field preservation.** Round-trip fidelity for proxy and middleware use cases.

- **Runtime reflection.** `buffa-descriptor` (under the `reflect` feature) provides `DescriptorPool` and `DynamicMessage` for schema-driven encode, decode, and JSON without generated code — plus extensions, custom-option access, `Any` pack/unpack, and symbol→file lookup for gRPC server reflection. Generated types implement the same `ReflectMessage` trait directly (vtable mode), so `foo.reflect()` borrows in place and a CEL evaluator, transcoding gateway, or generic interceptor treats typed and dynamic messages uniformly — without a re-encode round-trip. See [Reflection](#reflection) for the cost relative to generated code.

- **`no_std` + `alloc`.** The core runtime works without `std`, including JSON serialization via serde. Enabling `std` adds `std::io` integration, `std::time` conversions, and thread-local JSON parse options.

## Wire formats

buffa supports **binary**, **JSON**, and **text** protobuf encodings:

- **Binary wire format** -- full support for all scalar types, nested messages, repeated/packed fields, maps, oneofs, groups, and unknown fields.

- **Proto3 JSON** -- canonical protobuf JSON mapping via optional `serde` integration. Includes well-known type serialization (Timestamp as RFC 3339, Duration as `"1.5s"`, int64/uint64 as quoted strings, bytes as base64, etc.).

- **Text format (`textproto`)** -- the human-readable debug format. Covers `Any` expansion (`[type.googleapis.com/...] { ... }`), extension bracket syntax (`[pkg.ext] { ... }`), and group/DELIMITED fields. `no_std`-compatible.

## Unsupported features

These are intentionally out of scope:

- **Proto2 optional-field getter methods** — `[default = X]` on `optional` fields does not generate `fn field_name(&self) -> T` unwrap-to-default accessors. Custom defaults are applied only to `required` fields via `impl Default`. Optional fields are `Option<T>`; use pattern matching or `.unwrap_or(X)`.
- **Scoped `JsonParseOptions` in `no_std`** — serde's `Deserialize` trait has no context parameter, so runtime options must be passed through ambient state. In `std` builds, [`with_json_parse_options`] provides per-closure, per-thread scoping via a thread-local. In `no_std` builds, [`set_global_json_parse_options`] provides process-wide set-once configuration via a global atomic. The two APIs are mutually exclusive. The `no_std` global supports singular-enum accept-with-default but not repeated/map container filtering (which requires scoped strict-mode override).

[`with_json_parse_options`]: https://docs.rs/buffa/latest/buffa/json/fn.with_json_parse_options.html
[`set_global_json_parse_options`]: https://docs.rs/buffa/latest/buffa/json/fn.set_global_json_parse_options.html

## Known limitations

These are gaps we intend to address in future releases:

- **Closed-enum unknown values in packed-repeated view decode** are silently dropped (not routed to unknown fields). The owned decoder handles this correctly; the view decoder handles singular, optional, oneof, unpacked repeated, and map values correctly. Packed blobs have no per-element tag to borrow, so the zero-copy `UnknownFieldsView<'a>` has no span to reference.

## Semver and API stability

Buffa is pre-1.0. We follow the [Rust community convention](https://doc.rust-lang.org/cargo/reference/semver.html) for 0.x crates: breaking changes increment the **minor** version (0.1.x → 0.2.0), additive changes increment the **patch** version (0.1.0 → 0.1.1). Pin to a minor version (`buffa = "x.y"`; the full `x.y.z` that `cargo add buffa` writes is equivalent under caret semantics) to avoid surprises.

The generated code API (struct shapes, `Message` trait, `MessageView` trait, `EnumValue`, `MessageField`) is considered the primary stability surface. Internal helper modules marked `#[doc(hidden)]` (`__private`, `__buffa_*` fields) may change at any time.

## Quick start

### Using `buf generate` (recommended)

Install [buf](https://buf.build/docs/installation), then create a `buf.gen.yaml` that uses the published [`buf.build/anthropics/buffa`](https://buf.build/anthropics/buffa) remote plugin — no local plugin install required:

```yaml
version: v2
plugins:
  - remote: buf.build/anthropics/buffa
    out: src/gen
    opt:
      - file_per_package=true
      - json=true
```

```sh
buf generate
```

This emits one `<dotted.package>.rs` file per proto package. Wire them into your crate with a small `pub mod` tree:

```rust,ignore
// src/gen/mod.rs (hand-written)
pub mod example {
    pub mod v1 {
        include!("example.v1.rs");
    }
}
```

If you'd rather have the module tree generated for you, install [`protoc-gen-buffa-packaging`](docs/guide.md#installing-the-protoc-plugins) locally and add it as a second plugin (drop the `file_per_package=true` opt):

```yaml
version: v2
plugins:
  - remote: buf.build/anthropics/buffa
    out: src/gen
    opt:
      - json=true
  - local: protoc-gen-buffa-packaging
    out: src/gen
    strategy: all
```

See [`examples/bsr-quickstart/`](examples/bsr-quickstart/) for a complete, runnable project, or the [user guide](docs/guide.md#using-buf) for the full set of build setups (local plugins, `buffa-build`/`build.rs`, BSR-generated SDKs).

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
let owned_view = MyMessageOwnedView::decode(bytes.into()).unwrap();
println!("name: {}", owned_view.name()); // still zero-copy, but 'static + Send
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
| `protoc-gen-buffa` | `protoc` plugin binary; also published as [`buf.build/anthropics/buffa`](https://buf.build/anthropics/buffa) |
| `protoc-gen-buffa-packaging` | `protoc` plugin that emits a `mod.rs` module tree (local-only) |

## Performance

Throughput comparison across five representative message shapes, measured at buffa v0.8.0 on a quiesced bare-metal Intel Xeon Platinum 8488C (turbo disabled, `performance` governor, one core per implementation, median of five passes; full methodology in [`benchmarks/charts/README.md`](benchmarks/charts/README.md)). Higher is better.

### Binary decode

![Binary decode — ApiResponse](benchmarks/charts/binary-decode-api_response.svg)
![Binary decode — LogRecord](benchmarks/charts/binary-decode-log_record.svg)
![Binary decode — AnalyticsEvent](benchmarks/charts/binary-decode-analytics_event.svg)
![Binary decode — GoogleMessage1](benchmarks/charts/binary-decode-google_message1_proto3.svg)
![Binary decode — MediaFrame](benchmarks/charts/binary-decode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 575 | 872 (+52%) | 912 (+59%) | 550 (−4%) | 546 (−5%) | 430 (−25%) | 175 (−70%) |
| LogRecord | 572 | 1,336 (+134%) | 1,716 (+200%) | 481 (−16%) | 477 (−17%) | 555 (−3%) | 161 (−72%) |
| AnalyticsEvent | 123 | 225 (+83%) | 11,873 (+9538%) | 148 (+20%) | 129 (+5%) | 222 (+80%) | 57 (−54%) |
| GoogleMessage1 | 601 | 786 (+31%) | 1,452 (+142%) | 698 (+16%) | 669 (+11%) | 373 (−38%) | 263 (−56%) |
| MediaFrame | 10,619 | 41,441 (+290%) | 40,678 (+283%) | 6,002 (−43%) | 18,432 (+74%) | 11,005 (+4%) | 1,890 (−82%) |

</details>

### Binary encode

![Binary encode — ApiResponse](benchmarks/charts/binary-encode-api_response.svg)
![Binary encode — LogRecord](benchmarks/charts/binary-encode-log_record.svg)
![Binary encode — AnalyticsEvent](benchmarks/charts/binary-encode-analytics_event.svg)
![Binary encode — GoogleMessage1](benchmarks/charts/binary-encode-google_message1_proto3.svg)
![Binary encode — MediaFrame](benchmarks/charts/binary-encode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 1,972 | 1,955 (−1%) | 1,959 (−1%) | 1,964 (−0%) | — | 639 (−68%) | 384 (−81%) |
| LogRecord | 3,053 | 3,509 (+15%) | 3,587 (+17%) | 2,758 (−10%) | — | 1,067 (−65%) | 186 (−94%) |
| AnalyticsEvent | 404 | 426 (+6%) | 12,960 (+3109%) | 238 (−41%) | — | 307 (−24%) | 105 (−74%) |
| GoogleMessage1 | 2,122 | 2,123 (+0%) | 2,965 (+40%) | 1,816 (−14%) | — | 522 (−75%) | 232 (−89%) |
| MediaFrame | 25,727 | 27,325 (+6%) | 27,441 (+7%) | 25,402 (−1%) | — | 6,671 (−74%) | 2,423 (−91%) |

</details>

### Build + binary encode

The `build + encode` measure starts from raw field values rather than a pre-built
message struct, so it counts struct construction. The `buffa (view)` path
constructs a borrowed view directly over the input slices and never allocates an
owned message at all, which is why it is consistently faster than building owned
structs and then encoding them.

![Build + binary encode — ApiResponse](benchmarks/charts/build-encode-api_response.svg)
![Build + binary encode — LogRecord](benchmarks/charts/build-encode-log_record.svg)
![Build + binary encode — AnalyticsEvent](benchmarks/charts/build-encode-analytics_event.svg)
![Build + binary encode — GoogleMessage1](benchmarks/charts/build-encode-google_message1_proto3.svg)
![Build + binary encode — MediaFrame](benchmarks/charts/build-encode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | buffa (view) |
|---------|------:|------:|
| ApiResponse | 639 | 1,224 (+91%) |
| LogRecord | 315 | 2,447 (+678%) |
| AnalyticsEvent | 267 | 802 (+200%) |
| GoogleMessage1 | 673 | 900 (+34%) |
| MediaFrame | 14,432 | 33,481 (+132%) |

</details>

### JSON encode

![JSON encode — ApiResponse](benchmarks/charts/json-encode-api_response.svg)
![JSON encode — LogRecord](benchmarks/charts/json-encode-log_record.svg)
![JSON encode — AnalyticsEvent](benchmarks/charts/json-encode-analytics_event.svg)
![JSON encode — GoogleMessage1](benchmarks/charts/json-encode-google_message1_proto3.svg)
![JSON encode — MediaFrame](benchmarks/charts/json-encode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 521 | 589 (+13%) | 70 (−87%) |
| LogRecord | 697 | 882 (+27%) | 85 (−88%) |
| AnalyticsEvent | 505 | 533 (+6%) | 33 (−94%) |
| GoogleMessage1 | 571 | 674 (+18%) | 73 (−87%) |
| MediaFrame | 702 | 937 (+33%) | 235 (−67%) |

</details>

### JSON decode

![JSON decode — ApiResponse](benchmarks/charts/json-decode-api_response.svg)
![JSON decode — LogRecord](benchmarks/charts/json-decode-log_record.svg)
![JSON decode — AnalyticsEvent](benchmarks/charts/json-decode-analytics_event.svg)
![JSON decode — GoogleMessage1](benchmarks/charts/json-decode-google_message1_proto3.svg)
![JSON decode — MediaFrame](benchmarks/charts/json-decode-media_frame.svg)

<details><summary>Raw data (MiB/s)</summary>

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 492 | 205 (−58%) | 40 (−92%) |
| LogRecord | 530 | 424 (−20%) | 63 (−88%) |
| AnalyticsEvent | 169 | 152 (−10%) | 25 (−85%) |
| GoogleMessage1 | 413 | 171 (−59%) | 41 (−90%) |
| MediaFrame | 1,235 | 1,218 (−1%) | 215 (−83%) |

</details>

**Message types:** ApiResponse (~200 B, flat scalars), LogRecord (~1 KB, strings + map + nested message), AnalyticsEvent (~10 KB, deeply nested + repeated sub-messages), GoogleMessage1 (standard protobuf benchmark message), MediaFrame (~10 KB, dominated by `bytes` fields — primary body + chunked sub-blobs + named attachments).

**Libraries:** prost 0.13 + pbjson 0.7, protobuf‑v4 (Google Rust/upb, v4.33.1), Go `google.golang.org/protobuf` v1.36.6. protobuf-v4 JSON is not included as it does not provide a JSON codec.

**`prost (bytes)`** uses `prost-build`'s `.bytes(["."])` config so every proto `bytes` field is generated as `bytes::Bytes` instead of `Vec<u8>`, and decodes from a `bytes::Bytes` input to exercise `Bytes`' zero-copy `copy_to_bytes` slicing. The substitution only affects the decode path, so only decode numbers are reported — `prost (bytes)` encode tracks default `prost` by construction. On the four non-bytes messages, `prost (bytes)` tracks default `prost` within noise (and is slightly slower on `ApiResponse` where the per-message `Bytes::clone` refcount overhead isn't offset by any actual zero-copy). On `MediaFrame` it runs ~3.1× faster than default `prost` at decode, confirming that prost's feature does land when it has bytes fields to work with. buffa views are in a different regime again: they borrow directly from the input buffer for strings, bytes, and nested message bodies, so `buffa (view)` on `MediaFrame` is ~2.2× the `prost (bytes)` number and ~3.9× `buffa`'s own owned decode. Views also benefit on the four non-bytes messages, where prost's `bytes` feature is inert.

**`buffa (lazy)`** is the opt-in `FooLazyView` family (`lazy_views(true)`): `decode_lazy` performs one non-recursive scan that records nested and repeated message fields as undecoded byte ranges, and the encode number re-encodes from that view, replaying the recorded ranges verbatim. On flat messages it tracks the eager view within noise, because there is nothing to defer. On nested-message-dominated payloads the deferral is the whole cost model — `AnalyticsEvent` decodes at ~12 GiB/s because the scan never enters the repeated `Property` sub-messages, and re-encodes at ~13 GiB/s because the deferred ranges are copied rather than re-serialized. These full-scan numbers are the *floor* of the lazy advantage: the family exists for partial-access workloads (read a few fields out of many large items), where skipping untouched sub-trees also skips their allocation entirely. The trade-off is deferred validation — malformed bytes in a deferred field surface on access rather than at decode — documented in [the guide](docs/guide.md).

**Owned decode trade-offs:** buffa's owned decode is typically within ±10% of prost, trading a small throughput cost for features prost omits: unknown-field preservation by default, typed `EnumValue<E>` wrappers (not raw `i32`), and a type-stable decode loop that supports recursive message types without manual boxing. The zero-copy view path (`MyMessageView::decode_view`) sidesteps allocation entirely and is the recommended fast decode path. protobuf-v4's decode advantage on deeply-nested messages comes from upb's arena allocator — all sub-messages are bump-allocated in one arena rather than individually boxed.

### Reflection

Reflection lets a CEL evaluator, a transcoding gateway, or a generic interceptor encode, decode, and serialize messages it has no generated type for. buffa offers two implementations, selected with `reflect_mode`: **bridge** keeps generated code small (`foo.reflect()` re-encodes the typed message and decodes the bytes into a `DynamicMessage`), while **vtable** — the default when reflection is enabled — implements `ReflectMessage` directly on the generated types so `foo.reflect()` borrows `foo` in place, with no round-trip. Both hand out the same `&dyn ReflectMessage`, so the call site does not change between modes.

These charts measure the genericity tax against the generated codec. Only the four code-generated benchmark messages are covered, because reflection needs a generated type to compare against; `MediaFrame` is omitted. They are produced by the same bare-metal run as the cross-implementation charts above (the `reflect` bench binary runs alongside buffa's), so read them as a buffa-internal comparison (generated vs. reflect vs. view vs. vtable) on the same scale.

#### Decode

- **generated** — the typed codec `buffa-codegen` emits: a Rust struct with one field per proto field, decode monomorphized to those fields. The same `buffa` baseline charted under [Binary decode](#binary-decode).
- **reflect** — `DynamicMessage`: a single `BTreeMap<u32, Value>` keyed by field number, driven entirely by a runtime `DescriptorPool`. No generated type is involved.
- **view** — zero-copy `decode_view`: strings and bytes borrow from the input buffer instead of being copied into owned `String`/`Vec`, so it decodes *faster than the generated owned codec*. This is the floor every vtable reflection read builds on.

![Reflection decode — ApiResponse](benchmarks/charts/reflect-decode-api_response.svg)
![Reflection decode — LogRecord](benchmarks/charts/reflect-decode-log_record.svg)
![Reflection decode — AnalyticsEvent](benchmarks/charts/reflect-decode-analytics_event.svg)
![Reflection decode — GoogleMessage1](benchmarks/charts/reflect-decode-google_message1_proto3.svg)

#### Read

The interceptor / field-mask workload: take a wire payload, obtain a reflective handle, and read every set field. This is where vtable mode pays off — it is dominated by the cheap zero-copy decode, so it runs several times faster than either reflection alternative.

- **vtable** — `decode_view`, then read through the borrowed `&dyn ReflectMessage`. No round-trip, no per-field allocation.
- **bridge** — decode the owned message, then round-trip it into a `DynamicMessage` (the cost the codegen `Reflectable` paid per call before vtable mode).
- **dynamic** — decode straight into a `DynamicMessage`, no typed step (pure reflection).

![Reflection read — ApiResponse](benchmarks/charts/reflect-read-api_response.svg)
![Reflection read — LogRecord](benchmarks/charts/reflect-read-log_record.svg)
![Reflection read — AnalyticsEvent](benchmarks/charts/reflect-read-analytics_event.svg)
![Reflection read — GoogleMessage1](benchmarks/charts/reflect-read-google_message1_proto3.svg)

#### Encode

![Reflection encode — ApiResponse](benchmarks/charts/reflect-encode-api_response.svg)
![Reflection encode — LogRecord](benchmarks/charts/reflect-encode-log_record.svg)
![Reflection encode — AnalyticsEvent](benchmarks/charts/reflect-encode-analytics_event.svg)
![Reflection encode — GoogleMessage1](benchmarks/charts/reflect-encode-google_message1_proto3.svg)

<details><summary>Raw decode data (MiB/s, % vs generated)</summary>

| Message | generated | reflect | view |
|---------|------:|------:|------:|
| ApiResponse | 605 | 247 (−59%) | 914 (+51%) |
| LogRecord | 596 | 315 (−47%) | 1,364 (+129%) |
| AnalyticsEvent | 135 | 53 (−61%) | 224 (+66%) |
| GoogleMessage1 | 725 | 195 (−73%) | 746 (+3%) |

</details>

<details><summary>Raw read data (MiB/s, decode + scan all fields, % vs bridge)</summary>

| Message | vtable | bridge | dynamic |
|---------|------:|------:|------:|
| ApiResponse | 784 (+586%) | 114 | 186 (+63%) |
| LogRecord | 1,221 (+763%) | 141 | 283 (+100%) |
| AnalyticsEvent | 224 (+576%) | 33 | 54 (+64%) |
| GoogleMessage1 | 471 (+300%) | 118 | 142 (+21%) |

</details>

<details><summary>Raw encode data (MiB/s, % vs generated)</summary>

| Message | generated | reflect |
|---------|------:|------:|
| ApiResponse | 1,946 | 476 (−76%) |
| LogRecord | 3,055 | 885 (−71%) |
| AnalyticsEvent | 397 | 71 (−82%) |
| GoogleMessage1 | 2,127 | 223 (−90%) |

</details>

**Why the gap, on decode (~1.6–4.6×).** Generated decode resolves each field number through a compile-time jump table and writes the value straight into a typed struct field. Reflective decode instead binary-searches the descriptor's field table for every field, matches on the field's kind at runtime, wraps the value in a `Value` enum, and inserts it into the `BTreeMap` — an ordered-map insertion that allocates a node, where `String`/`Bytes`/nested values carry their own heap allocations and each nested message becomes a fresh `DynamicMessage` with its own map. The spread across messages follows directly: `LogRecord` shows the smallest gap because its payload is dominated by string and map decoding — UTF-8 validation and allocation that *both* paths perform identically — so the fixed per-field reflection overhead is amortized over shared work. `GoogleMessage1` shows the largest gap because it is scalar-dense: the generated path is a tight jump table doing almost nothing per field, leaving the reflection per-field cost nowhere to hide.

**Why the gap is wider on encode (~3.1–6.8×).** The generated encoder threads a `SizeCache` through one size pass, so each nested message's length is computed once and reused when its length prefix is written — this is buffa's linear-time serialization. `DynamicMessage` has no such cache: `encode_to_vec` runs a full `encoded_len()` traversal and then a full `encode()` traversal, and the encode traversal recomputes each nested message's size again to emit its length prefix. On flat messages that is a constant factor on top of the `BTreeMap` walk and per-field wire-type derivation; on deeply nested ones (`AnalyticsEvent`) the repeated size computation compounds with nesting depth.

The **bridge round-trip** is the v1 cost of the encode-decode bridge; a future zero-copy reflection mode would let a generated message expose its fields without re-encoding. For now, the rule is simple: reach for reflection when the schema is only known at runtime, and stay on the generated codec when throughput is what matters.

## Conformance

buffa passes the protobuf binary and JSON conformance test suite (v33.5, editions up to 2024). Both `std` and `no_std` builds pass the full suite including JSON. Run with `task conformance`.

## Compiler compatibility

**[buf](https://buf.build/docs/cli/)** is the recommended way to compile `.proto` files. The buf CLI has its own built-in compiler and can run `protoc-gen-buffa` as a remote plugin on the [Buf Schema Registry](https://buf.build/anthropics/buffa) — `buf generate` sends your compiled proto descriptors to the BSR, which executes the plugin and returns the generated Rust source — so the only thing you need to install is buf itself.

**protoc** is also fully supported. `protoc-gen-buffa` and `buffa-build` work with **protoc v21.12 and later**. The minimum version varies by feature:

| Feature | Minimum protoc |
|---|---|
| Proto2 + proto3 | v21.12 |
| Editions 2023 | v27.0 |
| Editions 2024 | v33.0 |

Note that Linux distro packages (Debian Bookworm, Ubuntu 24.04) ship protoc v21.12, which does not support editions. Install protoc v27+ from [GitHub releases](https://github.com/protocolbuffers/protobuf/releases) or use buf if you need editions support.

Compatibility is tested against protoc v21.12, v22.5, v25.5, v27.3, v29.5, and v33.5 (`task protoc-compat`).

## Minimum supported Rust version

The current MSRV is **1.75**.

buffa is a foundational codec crate, so its `rust-version` is set to the lowest toolchain the released code actually compiles on, not to a calendar target. CI verifies the workspace builds at the MSRV and at stable on every change. With cargo's MSRV-aware resolver (`resolver = "3"`, Rust 1.84+), a downstream project on an older toolchain will automatically resolve to the newest buffa release whose `rust-version` fits — so an accurate declaration matters more than a conservative one.

We reserve the right to raise the MSRV in any minor release when a language feature, standard-library API, or dependency makes it worthwhile, but never further than roughly twelve months behind the current stable. An MSRV bump is recorded in the CHANGELOG. We do not add workarounds for rustc or cargo bugs that are already fixed in stable; if you hit one on an older toolchain, the answer is to upgrade.

## License

Apache-2.0

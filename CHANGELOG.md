# Changelog

All notable changes to buffa will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the [Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html): breaking changes increment the minor version (0.1 → 0.2), additive changes increment the patch version.

## [Unreleased]

### Breaking changes

- **`Extension::new(number)` → `Extension::new(number, extendee)`.** Same for
  `Extension::with_default`. Codegen consumers are unaffected — the `pub const`
  items are regenerated. Hand-written `Extension` consts (unusual) need the
  extendee string added.
- **`ExtensionSet` trait gained a required `const PROTO_FQN: &'static str`.**
  Codegen consumers are unaffected. Hand-written impls need the const added.
- **`extension()`, `set_extension()`, `clear_extension()` now panic on extendee
  mismatch** (previously: silently returned `None` / no-op). `has_extension()`
  returns `false` gracefully. Catches `field_options.extension(&MESSAGE_OPTION)`
  bugs at the first call site; matches protobuf-go (panics) and protobuf-es
  (throws).

### Deprecated

- **`set_any_registry`, `set_extension_registry`** — use
  `buffa::type_registry::set_type_registry` instead, which installs all maps
  in one call. The deprecated functions still work.
- **`AnyTypeEntry` → `JsonAnyEntry`, `ExtensionRegistryEntry` → `JsonExtEntry`.**
  Type aliases for one release cycle. The text-format fields have moved to
  separate `TextAnyEntry` / `TextExtEntry` structs in `type_registry`.

### Added

- **Full extension support.** `Extension<C>` typed descriptors,
  `ExtensionSet` trait with `extension`/`set_extension`/`has_extension`/
  `clear_extension`/`extension_or_default`, codec types for every proto field
  type (including `GroupCodec` for editions `DELIMITED` / proto2 groups),
  proto2 `[default = ...]` on extension declarations, and MessageSet wire
  format behind `CodeGenConfig::allow_message_set`. See the
  [Extensions section of the user guide](docs/guide.md#extensions-custom-options).
- **`TypeRegistry`** — unified registry covering `Any` type entries and
  extension entries for both JSON and text formats. Codegen emits
  `register_types(&mut TypeRegistry)` per file; call once per generated file,
  then `set_type_registry(reg)`. JSON entries (`JsonAnyEntry`, `JsonExtEntry`)
  and text entries (`TextAnyEntry`, `TextExtEntry`) live in feature-split
  maps so `json` and `text` are independently enableable.
- **`JsonParseOptions::strict_extension_keys`** — error on unregistered `"[...]"`
  JSON keys (default: silently drop, matching pre-0.3 behavior for all unknown
  keys).
- **Editions `features.message_encoding = DELIMITED`** — fully supported in
  codegen, previously parsed but ignored. Message fields with this feature use
  the group wire format (StartGroup/EndGroup) instead of length-prefixed.
- **Text format (`textproto`)** — the `buffa::text` module provides
  `TextFormat` trait, `TextEncoder`, `TextDecoder`, and `encode_to_string` /
  `decode_from_str` conveniences. Enable with `features = ["text"]`
  (zero-dependency, `no_std`-compatible) and `Config::generate_text(true)`.
  Covers `Any` expansion (`[type.googleapis.com/...] { ... }`), extension
  brackets (`[pkg.ext] { ... }`), and group/DELIMITED naming. `Any` expansion
  and extension brackets consult the text maps in `TypeRegistry` — the `json`
  and `text` features are independently enableable. Passes the full
  text-format conformance suite (883/883).
- **Conformance:** `TestAllTypesEdition2023` enabled; binary+JSON 5539 → 5549
  passing (std). Text format suite 0 → 883 passing (was entirely skipped).

## [0.2.0] - 2026-03-16

### Breaking changes

- **`protoc-gen-buffa`: the `mod_file=<name>` option is removed.** Module tree
  assembly (`mod.rs` generation) is now a separate plugin,
  `protoc-gen-buffa-packaging`. The codegen plugin emits per-file `.rs` only
  and no longer requires `strategy: all`.

  Migration - replace this:

  ```yaml
  plugins:
    - local: protoc-gen-buffa
      out: src/gen
      strategy: all
      opt: [mod_file=mod.rs]
  ```

  with this:

  ```yaml
  plugins:
    - local: protoc-gen-buffa
      out: src/gen
    - local: protoc-gen-buffa-packaging
      out: src/gen
      strategy: all
  ```

  Passing `mod_file=` to the 0.2 plugin is a hard error with a migration hint
  (not a silent no-op).

### Added

- **`protoc-gen-buffa-packaging`** - new protoc plugin that emits a `mod.rs`
  module tree for per-file output. Works with any codegen plugin that follows
  buffa's per-file naming convention (`foo/v1/bar.proto` -> `foo.v1.bar.rs`).
  Invoke once per output tree; compose via multiple buf.gen.yaml entries.
  Optional `filter=services` restricts the tree to proto files that declare
  at least one `service`, for packaging service-stub-only output from plugins
  layered on buffa. Released as standalone binaries for the same five targets
  as `protoc-gen-buffa`, with SLSA provenance and cosign signatures.

- **`buffa-codegen`: `"."` accepted as a catch-all `extern_path` prefix.**
  `extern_path = (".", "crate::proto")` maps every proto package to an
  absolute Rust path rooted at `crate::proto`. More-specific mappings (including
  the auto-injected WKT mapping) still win via longest-prefix-match.

### Library compatibility

`buffa`, `buffa-types`, `buffa-codegen`, and `buffa-build` have no breaking
API changes in this release. The version bump reflects the
`protoc-gen-buffa` CLI change; library consumers upgrading from 0.1 should
see no code changes required.

## [0.1.0] - 2026-03-07

Initial release.

### Protobuf feature coverage

| Feature | Status |
|---|---|
| Binary wire format (proto2, proto3, editions 2023/2024) | ✅ |
| Proto3 JSON canonical mapping | ✅ |
| Well-known types (Timestamp, Duration, Any, Struct, Value, FieldMask, wrappers) | ✅ |
| Unknown field preservation | ✅ (default on) |
| Zero-copy view types | ✅ |
| Open enums (`EnumValue<E>`) with unknown-value preservation | ✅ |
| Closed enums (proto2) with unknown-value routing to unknown fields | ✅¹ |
| proto2 groups (singular, repeated, oneof) | ✅ |
| proto2 custom defaults (`[default = X]`) | ✅ on `required`; `optional` stays `None` |
| Editions feature resolution (`field_presence`, `enum_type`, `repeated_field_encoding`, `utf8_validation`) | ✅ |
| Editions `message_encoding = DELIMITED` | ⚠️ Parsed but ignored — see Known Limitations in README |
| `no_std` + `alloc` (core runtime, views, JSON) | ✅ |
| Text format (`textproto`) | ❌ Not planned |
| proto2 extensions | ❌ Not planned (use `Any`) |
| Runtime reflection | ❌ Not planned for 0.1 |

¹ See Known Limitations for two closed-enum edge cases (packed-repeated in views, map values).

### Conformance

Passes the [protobuf conformance suite](https://github.com/protocolbuffers/protobuf/tree/main/conformance) (v33.5):

- **5,539 passing** binary + JSON tests (std)
- **5,519 passing** binary + JSON tests (no_std — the 20-test gap is `IgnoreUnknownEnumStringValue*` in repeated/map contexts, which requires scoped strict-mode override; `no_std` has `set_global_json_parse_options` for singular-enum accept-with-default but not container filtering)
- **2,797 passing** via-view mode (binary → `decode_view` → `to_owned_message` → encode; direct JSON decode is not supported for views)
- **0 expected failures** across all three runs
- Text-format tests (883) are skipped (not supported)

### Test coverage

- **94.3% line coverage** (workspace, including build-script codegen paths)
- **1,018 unit tests** across runtime, codegen, types, and integration
- **6 fuzz targets**: binary decode (proto2, proto3, WKT), binary encode, JSON round-trip, WKT string parsers
- **googleapis stress test**: codegen compiles all ~3,000 `.proto` files in the Google Cloud API set
- **protoc compatibility**: plugin tested against protoc v21–v33

### Benchmarks (Intel Xeon Platinum 8488C)

Comparison against `prost` 0.13 (lower = buffa faster):

| Operation | buffa vs prost |
|---|---|
| Binary encode | **0.56–0.74×** (26–44% faster) |
| Binary decode | 0.91–1.29× (mixed; deep-nested messages slower) |
| JSON encode | 0.97–1.08× (parity) |
| JSON decode | **0.40–0.88×** (12–60% faster) |

See the [README Performance section](README.md#performance) for charts and raw data.

### Crates

This release publishes:

- `buffa` — core runtime
- `buffa-types` — well-known types (Timestamp, Duration, Any, etc.)
- `buffa-codegen` — descriptor → Rust source (for downstream code generators)
- `buffa-build` — `build.rs` integration
- `protoc-gen-buffa` — protoc plugin binary (also released as standalone binaries for linux-x86_64, linux-aarch64, darwin-x86_64, darwin-aarch64, windows-x86_64)

MSRV: Rust 1.85.

[Unreleased]: https://github.com/anthropics/buffa/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/anthropics/buffa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/anthropics/buffa/releases/tag/v0.1.0

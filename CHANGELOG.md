# Changelog

All notable changes to buffa will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the [Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html): breaking changes increment the minor version (0.1 → 0.2), additive changes increment the patch version.

## [Unreleased]

### Added

- **Generated message structs now include `with_<field>(value) -> Self`
  builder-style setter methods for every explicit-presence field** (proto3
  `optional`, proto2 `optional`, and editions fields with
  `field_presence = EXPLICIT`). This allows chained construction without
  `Some(...)` wrapping:

  ```rust
  let req = GetSecretRequest::default()
      .with_name("alice")
      .with_timeout_ms(30_000)
      .with_enabled(true);
  ```

  String fields accept `impl Into<String>` (`&str` works directly); bytes
  fields accept `impl Into<Vec<u8>>` or `impl Into<bytes::Bytes>` (byte
  array literals like `b"data"` work directly); enum fields accept
  `impl Into<EnumValue<E>>` (bare variant works directly, no
  `EnumValue::Known(...)` wrapper needed); plain scalars take the bare
  type to keep integer-literal inference unambiguous. Message fields
  (`MessageField<T>`), repeated fields, map fields, oneof variants,
  proto2 `required` fields, and implicit-presence fields are unaffected.
  To clear a field, assign `None` directly. Setters are pure inherent
  methods with no runtime dependency, so they're emitted unconditionally
  regardless of `gate_impls_on_crate_features`. Disable per compilation
  unit with `CodeGenConfig::generate_with_setters = false`,
  `buffa_build::Config::generate_with_setters(false)`, or the
  `with_setters=false` plugin opt. **Consumers with checked-in generated
  code** will see new methods on regen.
  ([#30](https://github.com/anthropics/buffa/issues/30),
  [#93](https://github.com/anthropics/buffa/pull/93), by @tejas-dharani)

- **`buffa::MessageName` trait exposes a generated message's protobuf
  identifiers as compile-time `&'static str` constants.** Codegen emits
  `impl MessageName for #Msg` (and `for #MsgView<'a>`) with four consts:
  `PACKAGE` (`"my.pkg"`, empty for the unnamed root package), `NAME`
  (`"Outer.Inner"` — unqualified, with `.` between nesting levels),
  `FULL_NAME` (`"my.pkg.Outer.Inner"`), and `TYPE_URL`
  (`"type.googleapis.com/my.pkg.Outer.Inner"` — the
  `google.protobuf.Any.type_url` form). All four are computed at codegen
  time as string literals, so there's no runtime allocation or
  concatenation — unlike `prost::Name`, whose `full_name()` and
  `type_url()` are runtime `format!` calls. `PACKAGE` and `NAME` are
  separate consts because the dotted `FULL_NAME` cannot be split
  unambiguously (`foo.Bar.Baz` could be package `foo.Bar` + message `Baz`
  or package `foo` + nested `Bar.Baz`).

  The trait has no supertrait — it doesn't reach into the wire codec —
  so view types implement it too: a generic event-sourcing registry can
  bound on `T: MessageName` and dispatch zero-copy views and owned
  messages identically. Useful for type-erased registries, logging, and
  any code that needs the protobuf name without the descriptor machinery.
  The inherent `Foo::TYPE_URL` const generated since 0.4.0 is unchanged
  and equal to `<Foo as MessageName>::TYPE_URL`; for messages that also
  implement `ExtensionSet`, `FULL_NAME` is equal to
  `ExtensionSet::PROTO_FQN` (all derive from the same codegen source).
  `MessageName` is **not** object-safe (associated `const` only) — use it
  as a bound, not `dyn MessageName`. Migrating from `prost::Name`: rename
  the bound and replace runtime `M::full_name()` / `M::type_url()` calls
  with the consts. ([#108](https://github.com/anthropics/buffa/pull/108),
  by @yordis)

- **`buf.build/anthropics/buffa` is published to the public Buf Schema
  Registry.** `buf generate` can now reference `protoc-gen-buffa` as a
  `remote:` plugin with no local install: `remote: buf.build/anthropics/buffa`
  with `opt: [file_per_package=true]` and a small hand-written `pub mod`
  tree, or paired with a locally-installed `protoc-gen-buffa-packaging`
  for a generated `mod.rs`. The README quick-start, `docs/guide.md`
  ["Using buf"](docs/guide.md#using-buf) section, and a new
  [`examples/bsr-quickstart/`](examples/bsr-quickstart/) project document
  the workflow. The stale in-repo `protoc-gen-buffa/buf.plugin.yaml`
  metadata file is removed — the canonical plugin definition lives in
  [bufbuild/plugins](https://github.com/bufbuild/plugins).

- **`buffa-codegen`: `CodeGenConfig::gate_impls_on_crate_features`.**
  When `true`, generated impls controlled by `generate_json`,
  `generate_views`, and `generate_text` are wrapped in
  `#[cfg(feature = "json" | "views" | "text")]` (or `#[cfg_attr(...)]` for
  derives and field attributes) instead of being emitted unconditionally.
  The consuming crate defines matching Cargo features and enables the
  corresponding runtime support (`buffa/json`, `buffa/text`, `serde`, …)
  behind them. The `generate_*` flags still control *whether* an impl kind
  is emitted; the new flag only controls *how*. Default `false` — no
  change to existing output. This is the codegen mechanism that will let
  `buffa-descriptor` and `buffa-types` ship every impl while keeping the
  codegen toolchain (`buffa-codegen` / `buffa-build` / `protoc-gen-buffa`)
  lean — it depends on them with `default-features = false`. Tracked in
  [#113](https://github.com/anthropics/buffa/issues/113). Exposed as
  `buffa_build::Config::gate_impls_on_crate_features(bool)` and the
  `gate_impls=true` plugin opt, both default-off.

- **`buffa-descriptor`: regenerated with views, JSON, text, and arbitrary
  impls behind crate features.** `descriptor.proto` and
  `compiler/plugin.proto` types now ship the full impl surface — gated on
  `views`, `json`, `text`, and `arbitrary` Cargo features so the codegen
  toolchain (`buffa-codegen` / `buffa-build` / `protoc-gen-buffa`) can
  depend on `buffa-descriptor` with `default-features = false` and stay
  free of `serde` / `serde_json` / `base64` / `arbitrary`. **Consumers
  whose protos reference a `descriptor.proto` type as a field (most
  commonly anything depending on `buf/validate/validate.proto`, or
  `buf.registry.module.v1` / `buf.alpha.image.v1` which embed
  `FileDescriptorSet` / `FileDescriptorProto`) must enable the
  `buffa-descriptor` features matching their codegen modes** —
  `views = ["buffa-descriptor/views"]`, `json = ["buffa-descriptor/json"]`,
  etc., or just `buffa-descriptor = { ..., features = ["views", "json"] }`.
  This closes [#113](https://github.com/anthropics/buffa/issues/113): the
  full `bufbuild/registry` and `bufbuild/buf` modules now generate and
  compile cleanly with `views=true` + `json=true`.

  **Migration:** if your `Cargo.toml` already declares `buffa-descriptor`
  as a dependency, add the features matching your codegen config:

  ```toml
  # build.rs uses .generate_views(true).generate_json(true)
  buffa-descriptor = { version = "0.6", features = ["views", "json"] }
  ```

  If you don't declare `buffa-descriptor` directly, the failure mode is a
  missing-impl error at the embedding type's serde / view call site (e.g.
  `the trait bound FileDescriptorSet: serde::Deserialize is not
  satisfied`); add `buffa-descriptor` with the right features.

  The `buffa_descriptor::generated` module tree now nests
  `google.protobuf.compiler` inside `google.protobuf` to mirror the proto
  package hierarchy (so cross-package `super::*` references in the view
  code resolve); the previous sibling-style
  `buffa_descriptor::generated::compiler` and
  `buffa_descriptor::generated::{FileDescriptorProto, GeneratedCodeInfo}`
  paths are preserved with `pub use` re-exports.

- `serde::Serialize` is now implemented for generated view types when `generate_json` is
  enabled, allowing zero-copy JSON serialization without `.to_owned_message()`.
  `OwnedView<V>` also gains a blanket `Serialize` impl so `serde_json::to_string(&owned_view)`
  works directly. Well-known type views (`TimestampView`, `DurationView`, `AnyView`, etc.)
  also implement `Serialize` (delegating to the owned form) when the `buffa-types/json`
  feature is enabled, so messages that nest WKT fields work out of the box. `MapView` gains
  `iter_unique()` and `len_unique()` helpers (last-write-wins deduplication) so map fields
  with duplicate wire keys serialize to a valid JSON object. The protobuf conformance suite
  gains a `BUFFA_VIEW_JSON=1` run that exercises view-side JSON output against the
  conformance reference assertions.
  **Known limitations:** (1) Extension fields are not included in view JSON output —
  serialize the owned form (`view.to_owned_message()`) to include extensions. (2) The view
  impl uses `serialize_map(None)`, which is fine for `serde_json` but will be rejected at
  runtime by length-prefixed formats like `bincode` or `postcard`; use the owned form for
  those serializers. ([#83](https://github.com/anthropics/buffa/issues/83))

### Fixed

- **`buffa` / `buffa-codegen`: `serde_json` re-exported from `buffa` for
  generated extension JSON deserialize.** Messages with `extensions N to M;`
  ranges and `json=true` codegen get a hand-written `Deserialize` impl that
  buffers `"[pkg.ext]"` JSON keys into a `serde_json::Value` before
  dispatching to `extension_registry::deserialize_extension_key`. The emitted
  path was a bare `::serde_json::Value`, which silently required every
  consumer of `json=true` codegen to declare `serde_json` directly in its own
  `Cargo.toml` — a footgun reported by Buf for `bufbuild_registry_*` SDKs
  generated against `buf/validate/validate.proto` (which has 21 extension
  ranges). `buffa` now re-exports `serde_json` (gated on the `json` feature,
  `#[doc(hidden)]`, matching the existing `bytes` re-export) and codegen
  emits `::buffa::serde_json::Value`, so consumers only need `buffa`,
  `buffa-types`, and `serde` (the latter for the `#[derive]` macro). No
  generated output exists for this path in the checked-in WKTs (none declare
  extension ranges), so no regen.

- **`buffa-codegen`: `descriptor.proto` types now resolve to
  `buffa-descriptor`, not `buffa-types`.** The auto-injected WKT
  extern_path `.google.protobuf` → `::buffa_types::google::protobuf`
  covers everything in the `google.protobuf` package, including
  `descriptor.proto` types — but `buffa-types` only ships the
  JSON-mappable WKTs. Any proto referencing a `descriptor.proto` type as
  a field — e.g. `buf/validate/validate.proto`, which has three `optional
  google.protobuf.FieldDescriptorProto.Type` fields — produced a
  generated path that doesn't exist:
  `::buffa_types::google::protobuf::field_descriptor_proto::Type`. An
  internal **file-level** extern resolution now routes
  `google/protobuf/descriptor.proto` to
  `::buffa_descriptor::generated::descriptor` and
  `google/protobuf/compiler/plugin.proto` to
  `::buffa_descriptor::generated::compiler`, taking priority over the
  package-level WKT mapping. Suppression mirrors the WKT mapping: a user
  `.google.protobuf` extern_path overrides it (preserving the long-standing
  behaviour that the override covers descriptor types too), and a file in
  `files_to_generate` resolves locally. **Consumers whose protos
  `import "google/protobuf/descriptor.proto"` and reference its types as
  fields must add `buffa-descriptor` to their `[dependencies]`** — the
  same way protos that reference WKTs require `buffa-types`. The
  user-facing `extern_path` API is unchanged (still package-prefix keyed).

- **`buffa`: closed-enum JSON helpers no longer require the enum to
  `impl Deserialize`.** `opt_closed_enum`, `repeated_closed_enum`, and
  `map_closed_enum` deserialized via `serde_json::from_value::<E>()`,
  which bound `E: DeserializeOwned`. That meant a closed-enum field whose
  enum type lives in an externally-generated crate built *without*
  `generate_json` — e.g. `google.protobuf.FieldDescriptorProto.Type` from
  `buffa-descriptor`, referenced by `buf/validate/validate.proto` — could
  not satisfy the bound and refused to compile under `json=true` codegen.
  The helpers now decode the buffered `serde_json::Value` directly via the
  `Enumeration` trait (`from_proto_name`, `from_i32`, default for `null`),
  which is the same dispatch the codegen-emitted `Deserialize` impl
  performs anyway. The `DeserializeOwned` bound is removed (a relaxation —
  non-breaking). Lenient mode (`ignore_unknown_enum_values`) is unchanged:
  any element that fails to decode — unknown variant, out-of-range
  integer, or wrong JSON type — is dropped from the container / leaves the
  optional unset, exactly as before. Additionally, that lenient filtering
  for closed-enum containers now works under `no_std`: the previous
  implementation needed the `std`-only scoped strict-mode override to
  surface a distinguishable error from the inner deserialize, but the new
  `Enumeration`-direct dispatch has no inner deserialize to override.

### Changed

- The workspace `[profile.release]` now sets `lto = true` and
  `codegen-units = 1`. This shrinks the prebuilt `protoc-gen-buffa` /
  `protoc-gen-buffa-packaging` release binaries by roughly 20% at the cost of
  ~2× clean release-build time. Cargo only honors profile sections from the
  top-level workspace, so library consumers of `buffa` / `buffa-build` do not
  inherit this — set `[profile.release]` in your own workspace (or
  `CARGO_PROFILE_RELEASE_LTO=true` for `cargo install`) to get the same
  benefit. ([#60](https://github.com/anthropics/buffa/issues/60))

- **`buffa-codegen`: empty ancillary content files and modules are no
  longer emitted.** A `.proto` with no oneofs / no extension declarations
  / `views=false` previously produced placeholder
  `<stem>.__oneof.rs` / `<stem>.__ext.rs` / `<stem>.__view.rs` /
  `<stem>.__view_oneof.rs` files containing only the `@generated` header,
  and the package stitcher unconditionally authored a
  `pub mod __buffa { pub mod oneof { ... } pub mod ext { ... } ... }`
  tree that `include!`d them. Codegen now omits an ancillary content file
  when it would be empty, the stitcher only `include!`s files that exist,
  and the `__buffa` wrapper (and each `view` / `oneof` / `ext` submodule
  inside it) is itself omitted when it would be empty — so a package with
  only owned messages emits no `__buffa` block at all. Eliminates pure
  noise in generated trees, editor file lists, search, and review diffs.
  **Consumers with checked-in generated code** will see file deletions
  and stitcher diffs on regeneration; remove orphaned empty files. The
  `__buffa::*` paths are an internal sentinel namespace (consumers reach
  for the natural-path re-exports added in 0.5.0), so no supported public
  surface changes — but a hand-written
  `use crate::pkg::__buffa::oneof::*;` for a package that has no oneofs
  would now fail to resolve (it was previously a no-op import of an
  empty module). ([#107](https://github.com/anthropics/buffa/pull/107))

## [0.5.2] - 2026-05-07

### Fixed

- **`buffa-codegen`: oneof `Serialize` match arms now use `Self::#variant`.**
  `generate_oneof_serialize` emits the manual JSON serde impl as
  `impl Serialize for #enum_ident { fn serialize(&self, …) { match self { … } } }`,
  where `Self` resolves to the oneof enum. The match arms used the
  fully-qualified `#enum_ident::#variant` form, which trips
  `clippy::use_self` in workspaces that opt it on — particularly visible
  under `connectrpc-build`, which doesn't carry an inner `#![allow(...)]`
  the way `protoc-gen-buffa-packaging` does, so the oneof companion file
  inherits the surrounding mod's lint set. The deserialize arms in
  `oneof_variant_deser_arm` remain qualified because they construct the
  oneof from inside the *message*'s `Deserialize` impl, where `Self` would
  be wrong. No behavioural change.
- **`buffa-codegen`: enum JSON deserialize errors use inlined format args.**
  The enum visitor's range-check and unknown-value error messages used
  positional `format!("enum value {} out of i32 range", v)` etc., which
  trip `clippy::uninlined_format_args` for the same reason as above (the
  enum impls live in the per-proto Owned content, outside the `__buffa`
  `#[allow(...)]` block). Now `format!("enum value {v} out of i32
  range")` etc. — semantically identical, lint-clean regardless of which
  module wrapper covers it.

## [0.5.1] - 2026-05-07

### Fixed

- **`buffa-codegen`: `ALLOW_LINTS` now includes `unused_qualifications`.**
  Cross-proto references within the same package are emitted through the
  canonical `super::super::__buffa::view::…` (and `…::oneof::…`) path even
  though the target lives in the same generated module. The bare name would
  resolve, but the canonical path is stable when a sibling proto defines a
  same-named natural-path re-export. Workspaces that opt
  `unused_qualifications = "warn"` and build with `-D warnings` were getting
  false positives from generated code; the lint is now in the package
  stitcher's `#[allow(...)]` block alongside `dead_code`, `unused_imports`,
  etc.

## [0.5.0] - 2026-05-05

This release is a minor bump under the
[Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html):
the only API break is `#[non_exhaustive]` on `buffa_codegen::GeneratedFileKind`
(see *Changed* below), which affects downstream code generators only — it does
not change the runtime API. Everything else is additive.

**Consumers with checked-in generated code must regenerate** with the 0.5.0
toolchain before depending on the 0.5.0 runtime crates: generated code from
0.5.0's `buffa-codegen` references `ViewReborrow`, `decode_bytes_to_bytes`,
and `__private::arbitrary_bytes`, none of which exist in `buffa` 0.4.0.

### Breaking changes

- **`buffa_codegen::GeneratedFileKind` is now `#[non_exhaustive]`.** Match it
  with a wildcard arm — future kinds can then be added without a major
  version bump. Build integrations that compare with `==` (the common case,
  including connect-rust) are unaffected.

### Added

- `buffa_codegen::GeneratedFileKind::Companion` and `apply_companions` let
  downstream code generators (e.g. connect-rust) supply extra per-proto
  files that buffa wires into the per-package stitcher, instead of having
  to mislabel them as `GeneratedFileKind::Owned` and rely on filename
  matching. Companion files are `include!`d at package root alongside
  owned message types. ([#81](https://github.com/anthropics/buffa/issues/81))

- `OwnedView<V>` gains a `reborrow<'b>(&'b self) -> &'b V::Reborrowed<'b>` method
  that makes the internal `'static` lifetime visible as `'b` (the lifetime of the
  borrow), so view fields can be passed into functions or return types bounded by
  the `OwnedView`'s lifetime. Requires `V: ViewReborrow`, a safe trait whose
  `reborrow` method body is a covariance-checked subtype coercion; codegen emits
  the `impl` automatically for every generated view type. Hand-written view types
  opt in with `impl ViewReborrow for MyView<'static> { type Reborrowed<'b> =
  MyView<'b>; fn reborrow<'b>(this: &'b Self) -> &'b Self::Reborrowed<'b> { this } }`
  — the body fails to compile for invariant view types, so no `unsafe` is needed.
  ([#82](https://github.com/anthropics/buffa/issues/82))

- Codegen now emits "natural-path" `pub use` re-exports for ancillary types
  (views, oneof enums, view-of-oneof enums, file-level extension consts,
  `register_types`) at the module path you'd write first — `pkg::FooView`,
  `pkg::foo::Kind`, `pkg::foo::KindView`, etc. The canonical `__buffa::`
  paths are unchanged and remain what generated code and downstream codegen
  always reference; the re-exports are purely an ergonomic convenience and
  are silently skipped when the natural name is already taken by a real
  proto item or by another candidate re-export. Because of that skip rule,
  adding a proto type whose name shadows a re-export (e.g. `message FooView`
  next to `message Foo`) can silently rebind a natural path between releases
  — the canonical `__buffa::` path is always stable; use it directly when a
  natural import stops resolving (see `examples/conflicts` for one alias
  convention). ([#80](https://github.com/anthropics/buffa/issues/80))

- Doc comments in generated Rust code now resolve AIP-192 proto type cross-references
  (`[Book][google.example.v1.Book]`, `[Book][]`) to rustdoc intra-doc links.
  Only type-level refs are resolved; member refs such as `[Genre.GENRE_SCI_FI][]`
  fall back to escaped literals. Unknown or cross-crate references also fall back
  silently. ([#26](https://github.com/anthropics/buffa/issues/26))

- `protoc-gen-buffa` and `protoc-gen-buffa-packaging` now respond to
  `--version` / `-V` and `--help` / `-h` instead of blocking on stdin.
  Any other command-line argument prints a "this is a protoc plugin" hint
  to stderr and exits non-zero.

- `buffa::types::decode_bytes_to_bytes` reads a length-delimited `bytes` field
  into a `bytes::Bytes` via `Buf::copy_to_bytes`. When decoding from a
  `Bytes`-backed buffer this is a zero-copy refcount bump. Generated
  `merge_field` arms for `bytes_fields`-tagged fields (singular, optional,
  repeated, and oneof) now use it instead of `Bytes::from(decode_bytes(..)?)`,
  eliminating one allocation + memcpy per field on the owned decode path. Note
  that in the zero-copy case the resulting field aliases the source
  allocation, so the source buffer is freed only once every aliased field is
  dropped. Consumers with checked-in generated code must regenerate to pick
  this up. ([#53](https://github.com/anthropics/buffa/issues/53))

### Fixed

- `buffa-types --features arbitrary` now compiles. `Any.value` is
  `bytes::Bytes` (since 0.4.0 / #51), which has no `Arbitrary` impl.
  Codegen now emits `#[arbitrary(with = ::buffa::__private::arbitrary_bytes*)]`
  on every `bytes_fields`-typed field — singular, optional, and repeated
  struct fields plus oneof variant inner fields — when
  `generate_arbitrary = true`, so the struct-level `derive(Arbitrary)`
  succeeds. Map values are unaffected (they are always `Vec<u8>` regardless
  of `bytes_fields`). The same fix covers any user crate that uses
  `bytes_fields` + `generate_arbitrary`. `cargo doc --workspace
  --all-features` and `cargo clippy --workspace --all-features` are also
  unblocked, and CI now runs `cargo check --workspace --all-features` to
  prevent recurrence.
  ([#88](https://github.com/anthropics/buffa/issues/88))

- `write_to` now emits fields in ascending field-number order regardless of
  cardinality (singular / repeated / map / oneof), matching prost,
  protoc-C++, and the spec's serialize-in-field-order recommendation.
  Previously fields were emitted grouped by kind, which broke
  byte-equivalence with other implementations for messages mixing a
  high-numbered singular field with a lower-numbered repeated/map/oneof.
  Decoders accept any order, so this is not a wire-compat break, but
  consumers content-addressing serialized bytes (e.g. `hash(encode(msg))`)
  will see different hashes for affected message shapes.
  ([#75](https://github.com/anthropics/buffa/issues/75))

## [0.4.0] - 2026-04-27

### Breaking changes

- **Ancillary generated types moved under `pkg::__buffa::`.** View structs,
  oneof enums, view-of-oneof enums, extension consts, and `register_types`
  no longer share the package-level Rust namespace with owned message
  structs. The new layout:

  | Item | Before | After |
  |---|---|---|
  | View struct | `pkg::FooView` | `pkg::__buffa::view::FooView` |
  | Nested view | `pkg::foo::BarView` | `pkg::__buffa::view::foo::BarView` |
  | Oneof enum | `pkg::foo::KindOneof` | `pkg::__buffa::oneof::foo::Kind` |
  | View-of-oneof | `pkg::foo::KindOneofView` | `pkg::__buffa::view::oneof::foo::Kind` |
  | Extension const | `pkg::FOO` | `pkg::__buffa::ext::FOO` |
  | Registration fn | `pkg::register_types` (per file) | `pkg::__buffa::register_types` (per package) |

  Owned message structs and nested-type modules are unchanged. Migration is
  a mechanical path rewrite per the table above. The `Oneof` / `OneofView`
  suffixes are dropped — the parallel module tree disambiguates.

  This makes name collisions between user proto types and codegen-derived
  ancillary names structurally impossible. `__buffa` is the **only** name
  codegen reserves in user namespace; it aligns with the existing `__buffa_`
  reserved field-name prefix. A proto message, file-level enum, or package
  segment that snake-cases to `__buffa` is rejected with
  `CodeGenError::ReservedModuleName`.

- **Consumer include pattern: use `buffa::include_proto!("dotted.pkg")`.**
  Codegen now emits a per-package `<dotted.pkg>.mod.rs` stitcher alongside
  the per-proto content files. Hand-authored
  `include!(concat!(env!("OUT_DIR"), "/my_file.rs"))` blocks no longer
  produce a complete module; replace with:

  ```rust
  pub mod my_pkg {
      buffa::include_proto!("my.pkg");
  }
  ```

  `buffa-build`'s `generate_include_file()` already emits the correct
  structure; consumers using that helper need no change.

- **`__buffa_cached_size` is removed from all generated structs (owned and
  view); `Message::compute_size` / `write_to` and `ViewEncode::compute_size` /
  `write_to` now take a `&mut SizeCache` parameter.** Sizes are recorded in an
  external pre-order `Vec<u32>` cache that the provided `encode*` methods
  construct internally, so generated types contain only their proto fields
  plus `__buffa_unknown_fields` — no interior mutability, structurally
  `Send + Sync`, and concurrent `encode()` of the same `&msg` from multiple
  threads is sound. `Message::cached_size()` and `ViewEncode::cached_size()`
  are removed; use the new provided `encoded_len()` to get the size alone.
  `__private::CachedSize` is removed. Hand-written `Message` / `ViewEncode`
  impls must add the `cache: &mut SizeCache` parameter, drop `cached_size()`,
  and (for nested message fields) wrap recursion in `cache.reserve()` /
  `cache.set()`; see the [custom-types section of the user
  guide](docs/guide.md#custom-type-implementations) for the pattern.
  ([#14](https://github.com/anthropics/buffa/issues/14),
  [#22](https://github.com/anthropics/buffa/pull/22))
- **`DefaultInstance` and `DefaultViewInstance` are no longer `unsafe` traits,
  and `HasDefaultViewInstance` is removed.** The liveness and immutability
  invariants are fully encoded by the return type and cannot be violated by
  a safe implementation. `DefaultViewInstance` is now implemented for
  `FooView<'v>` at every lifetime (not just `'static`), with
  `fn default_view_instance<'a>() -> &'a Self where Self: 'a`; the
  covariant lifetime coercion happens in the impl body where the compiler
  checks it via ordinary subtyping, eliminating the raw pointer cast in
  `Deref for MessageFieldView`. Hand-written impls must drop the `unsafe`
  keyword and adopt the new method signature; the separate
  `HasDefaultViewInstance` impl is no longer needed.
  ([#68](https://github.com/anthropics/buffa/issues/68),
  [#69](https://github.com/anthropics/buffa/issues/69))
- **`CodeGenError::OneofNameConflict` and `::ViewNameConflict` removed.**
  These collisions are now structurally impossible (the inputs that
  previously triggered them produce valid output).
- **`google.protobuf.Any.value` is now `::bytes::Bytes` instead of `Vec<u8>`.**
  Makes `Any::clone()` a cheap refcount bump (up to ~170x faster for large
  payloads) instead of a full memcpy. Call sites constructing an `Any` by hand
  need `.into()` on the payload (e.g. `value: my_vec.into()`, or pass `Bytes`
  directly). Reading `any.value` is unchanged — `Bytes` derefs to `&[u8]`.
  `buffa-types` now depends on `bytes` unconditionally.

### Deprecated

- **`buffa_build::proto_path_to_rust_module`** — consumers should use
  the per-package `<pkg>.mod.rs` stitcher path via `buffa::include_proto!`
  instead.

### Added

- **`ViewEncode<'a>` — serialization from borrowed view types.** Generated
  `*View<'a>` types implement `ViewEncode` (whenever views are generated,
  i.e. `generate_views(true)`, the default) with the same two-pass
  `compute_size`/`write_to` model as `Message`. Views can be constructed
  from borrowed `&'a str` / `&'a [u8]` and encoded without intermediate
  `String`/`Vec` allocation. Benchmarks: parity on serialize-only; ~6× on
  build+encode for a 15-label string-map message.
- **`buffa::include_proto!("dotted.pkg")` macro** — wraps the per-package
  `.mod.rs` stitcher; the canonical consumer integration point.
- **`MapView::new(Vec)` / `From<Vec>` / `FromIterator`** for constructing
  map views directly (for `ViewEncode`).
- **`SizeCache`** — external pre-order size table for the two-pass encode
  protocol (`[u32; 16]` inline + `Vec<u32>` spill, allocation-free for ≤16
  nested LEN sub-messages). The provided `encode*()` methods construct one
  internally; for hot loops, the new `encode_with_cache(&mut SizeCache, buf)`
  reuses a single cache across calls.
- **`Message::encoded_len()` / `ViewEncode::encoded_len()`** — provided
  method returning the serialized size without writing (replaces
  `compute_size()`-then-discard).
- **`Enumeration::values()`** — `&'static [Self]` slice of all variants for
  iteration.
- **`buffa-build` / `buffa-codegen`: `type_attribute`, `field_attribute`,
  `message_attribute`, `enum_attribute`** — attach Rust attributes (e.g.
  `#[derive(...)]`, `#[serde(...)]`) to specific generated types or fields
  by proto path.
- **`protoc-gen-buffa`: `text=true` and `allow_message_set=true` plugin
  parameters** — match the existing `buffa-build` config flags.
- **`#[must_use]` on `Message`/`ViewEncode` `compute_size`, `encoded_len`,
  `encode_to_vec`, `encode_to_bytes`.**

### Fixed

- A proto message named `Option` — anywhere in the proto package, including
  nested in a sibling message or in another file — no longer shadows
  `core::option::Option` in generated optional/oneof field types and the
  JSON deserialize path. Generated code now always emits the
  fully-qualified `::core::option::Option`.
  ([#36](https://github.com/anthropics/buffa/issues/36),
  [#64](https://github.com/anthropics/buffa/issues/64))
- Oneof variant names that PascalCase to a reserved Rust identifier (in
  practice, proto field `self` → variant `Self`) are now escaped.
  ([#47](https://github.com/anthropics/buffa/issues/47))
- Nested type and oneof sharing the same name (the gh#31 `RegionCodes`
  case) and `Foo` next to `FooView` (gh#32) — both now structurally
  resolved by the `__buffa::` namespacing above.

## [0.3.0] - 2026-04-01

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
- **`buffa-descriptor` crate** — `FileDescriptorProto` and friends are now in a
  standalone crate that depends only on `buffa`, so descriptor types are usable
  without pulling in `quote`/`syn`/`prettyplease`. `buffa-codegen` re-exports
  the module so existing `buffa_codegen::generated::*` paths still resolve.
  ([#8](https://github.com/anthropics/buffa/pull/8))
- **Proto source comments → rustdoc.** Comments from `.proto` files are now
  emitted as `///` doc comments on generated structs, fields, enums, variants,
  and view types. Requires `--include_source_info` (set automatically by
  `buffa-build` and the protoc plugins).
  ([#7](https://github.com/anthropics/buffa/pull/7))
- **`buffa::encoding::MAX_FIELD_NUMBER`** constant (`(1 << 29) - 1`), replacing
  the magic number at all call sites.
  ([#21](https://github.com/anthropics/buffa/pull/21))

### Changed

- **`buffa-build` skips writing unchanged outputs**, avoiding mtime bumps that
  trigger needless downstream recompilation.
  ([#17](https://github.com/anthropics/buffa/pull/17))
- **Generated code emits `Self`** in `impl` blocks instead of repeating the
  type name, so consumer crates that enable `clippy::use_self` get clean
  output. ([#15](https://github.com/anthropics/buffa/pull/15))

### Fixed

- **Codegen no longer reports a false name collision** between a nested type
  and a proto3 `optional` field whose synthetic oneof PascalCases to the same
  name. ([#20](https://github.com/anthropics/buffa/pull/20),
  fixes [#12](https://github.com/anthropics/buffa/issues/12))
- **Generated rustdoc no longer breaks on proto comments** containing
  `[foo][]` reference-style links or bare URLs — these are now escaped so
  rustdoc treats them as literal text.
  ([#25](https://github.com/anthropics/buffa/pull/25))

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

[Unreleased]: https://github.com/anthropics/buffa/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/anthropics/buffa/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/anthropics/buffa/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/anthropics/buffa/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/anthropics/buffa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/anthropics/buffa/releases/tag/v0.1.0

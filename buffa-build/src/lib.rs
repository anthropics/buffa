//! Build-time integration for buffa.
//!
//! Use this crate in your `build.rs` to compile `.proto` files into Rust code
//! at build time. Parses `.proto` files into a `FileDescriptorSet` (via
//! `protoc` or `buf`), then uses `buffa-codegen` to generate Rust source.
//!
//! # Example
//!
//! ```rust,ignore
//! // build.rs
//! fn main() {
//!     buffa_build::Config::new()
//!         .files(&["proto/my_service.proto"])
//!         .includes(&["proto/"])
//!         .compile()
//!         .unwrap();
//! }
//! ```
//!
//! # Requirements
//!
//! By default, requires `protoc` on the system PATH (or set via the `PROTOC`
//! environment variable) — the same as `prost-build` and `tonic-build`.
//!
//! If `protoc` is unavailable or outdated on your platform, `buf` can be
//! used instead — see [`Config::use_buf()`]. Alternatively, feed a
//! pre-compiled descriptor set via [`Config::descriptor_set()`].

use std::path::{Path, PathBuf};
use std::process::Command;

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;

#[doc(inline)]
pub use buffa_codegen::CodeGenConfig;
#[doc(inline)]
pub use buffa_codegen::FeatureGateNames;
#[doc(inline)]
pub use buffa_codegen::ReflectMode;
#[doc(inline)]
pub use buffa_codegen::{BytesRepr, MapRepr, PointerRepr, RepeatedRepr, StringRepr};

/// How to produce a `FileDescriptorSet` from `.proto` files.
#[derive(Debug, Clone, Default)]
enum DescriptorSource {
    /// Invoke `protoc` (default). Requires `protoc` on PATH or `PROTOC` env var.
    #[default]
    Protoc,
    /// Invoke `buf build --as-file-descriptor-set`. Requires `buf` on PATH.
    Buf,
    /// Read a pre-built `FileDescriptorSet` from a file.
    Precompiled(PathBuf),
}

/// Builder for configuring and running protobuf compilation.
pub struct Config {
    files: Vec<PathBuf>,
    includes: Vec<PathBuf>,
    out_dir: Option<PathBuf>,
    codegen_config: CodeGenConfig,
    descriptor_source: DescriptorSource,
    /// If set, generate a module-tree include file with this name in the
    /// output directory. Users can then `include!` this single file instead
    /// of manually setting up `pub mod` nesting.
    include_file: Option<String>,
}

impl Config {
    /// Create a new configuration with defaults.
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            includes: Vec::new(),
            out_dir: None,
            codegen_config: CodeGenConfig::default(),
            descriptor_source: DescriptorSource::default(),
            include_file: None,
        }
    }

    /// Add `.proto` files to compile.
    #[must_use]
    pub fn files(mut self, files: &[impl AsRef<Path>]) -> Self {
        self.files
            .extend(files.iter().map(|f| f.as_ref().to_path_buf()));
        self
    }

    /// Add include directories for protoc to search for imports.
    #[must_use]
    pub fn includes(mut self, includes: &[impl AsRef<Path>]) -> Self {
        self.includes
            .extend(includes.iter().map(|i| i.as_ref().to_path_buf()));
        self
    }

    /// Set the output directory for generated files.
    /// Defaults to `$OUT_DIR` if not set.
    #[must_use]
    pub fn out_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(dir.into());
        self
    }

    /// Enable or disable view type generation (default: true).
    #[must_use]
    pub fn generate_views(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_views = enabled;
        self
    }

    /// Additionally generate the lazy view family (`FooLazyView<'a>`)
    /// alongside the unchanged eager views (default: false).
    ///
    /// Lazy views decode in a single non-recursive pass, recording nested and
    /// repeated message fields as undecoded byte ranges that decode on access
    /// via fallible, by-value accessors (`.get()` / iteration) — untouched
    /// sub-trees cost nothing. Validation of deferred bytes happens on
    /// *access* (and in the fallible `to_owned_message`), not at decode.
    /// Groups, oneof message variants, and map message values stay eager;
    /// lazy views have no `ReflectMessage`/`OwnedView`/text surface. Eager
    /// codegen output is byte-identical with or without the flag. Requires
    /// [`generate_views`](Self::generate_views). See
    /// [`CodeGenConfig::lazy_views`] for full semantics.
    #[must_use]
    pub fn lazy_views(mut self, enabled: bool) -> Self {
        self.codegen_config.lazy_views = enabled;
        self
    }

    /// Enable or disable serde JSON generation (default: false).
    ///
    /// When enabled:
    /// - Generated message structs get `Serialize`/`Deserialize` derives.
    /// - Generated enum types get `Serialize`/`Deserialize` derives.
    /// - Generated view types (when `generate_views` is also enabled) get a
    ///   manual `impl Serialize` for zero-copy JSON serialization, so
    ///   `serde_json::to_string(&view)` works directly:
    ///
    ///   ```ignore
    ///   let view = MyMsgView::decode_view(&bytes)?;
    ///   let json = serde_json::to_string(&view)?;
    ///   ```
    ///
    /// The downstream crate must depend on `serde` and enable the `buffa/json`
    /// feature for the runtime helpers. When views are enabled, the crate must
    /// also enable `buffa-types/json` so the well-known type views implement
    /// `Serialize`; without it, references to e.g. `TimestampView<'_>` in the
    /// generated `Serialize` impl will fail with
    /// `the trait bound 'TimestampView<'_>: Serialize' is not satisfied`.
    ///
    /// **Limitations of the view `Serialize` impl:**
    /// - Extension fields are not included in view JSON output; serialize the
    ///   owned form (`view.to_owned_message()`) to include extensions.
    /// - The impl uses `serialize_map(None)` (unknown length) because the
    ///   number of emitted fields depends on default-omission rules. Most
    ///   self-describing serializers (notably `serde_json`) accept this, but
    ///   length-prefixed formats (e.g. `bincode`, `postcard`) will return a
    ///   runtime error. The owned types' derived `Serialize` does not have this
    ///   restriction.
    #[must_use]
    pub fn generate_json(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_json = enabled;
        self
    }

    /// Enable or disable `impl buffa::text::TextFormat` on generated message
    /// structs (default: false).
    ///
    /// When enabled, the downstream crate must enable the `buffa/text`
    /// feature for the runtime textproto encoder/decoder.
    #[must_use]
    pub fn generate_text(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_text = enabled;
        self
    }

    /// Enable or disable `#[derive(arbitrary::Arbitrary)]` on generated
    /// types (default: false).
    ///
    /// The derive is gated behind `#[cfg_attr(feature = "arbitrary", ...)]`
    /// so the downstream crate compiles with or without the feature enabled.
    ///
    /// Your crate's Cargo feature **must be named exactly `"arbitrary"`** —
    /// the generated `cfg_attr` uses that literal string and cannot be
    /// customised — and it must forward to `buffa/arbitrary`:
    ///
    /// ```toml
    /// [features]
    /// arbitrary = ["dep:arbitrary", "buffa/arbitrary"]
    /// ```
    ///
    /// Forgetting `"buffa/arbitrary"` produces a confusing
    /// `cannot find function 'arbitrary_bytes' in module '__private'` error
    /// in generated code when [`use_bytes_type`](Self::use_bytes_type) or
    /// [`use_bytes_type_in`](Self::use_bytes_type_in) is also enabled,
    /// because the helper that backs `#[arbitrary(with = ...)]` for
    /// `bytes::Bytes` fields lives in `buffa` under that feature gate.
    #[must_use]
    pub fn generate_arbitrary(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_arbitrary = enabled;
        self
    }

    /// Wrap generated `impl`s in `#[cfg(feature = "...")]` instead of
    /// emitting them unconditionally (default: false).
    ///
    /// When enabled, the impls controlled by [`generate_json`],
    /// [`generate_views`], and [`generate_text`] are wrapped in
    /// `#[cfg(feature = "json" | "views" | "text")]` (or
    /// `#[cfg_attr(feature = ..., ...)]` for derives and field attributes)
    /// rather than emitted unconditionally. The crate consuming the
    /// generated code must define matching Cargo features that enable the
    /// corresponding runtime support:
    ///
    /// ```toml
    /// [features]
    /// json  = ["buffa/json", "dep:serde", "dep:serde_json"]
    /// views = []
    /// text  = ["buffa/text"]
    /// ```
    ///
    /// The `generate_*` flags still control *whether* an impl kind is
    /// emitted at all — this flag only controls whether it is `cfg`-gated.
    /// `generate_arbitrary` is always `cfg_attr`-gated on
    /// `feature = "arbitrary"` regardless of this flag, because `arbitrary`
    /// is an optional dependency by design.
    ///
    /// Reach for this when generated code is the **public interface of a
    /// library crate** consumed by downstream projects with different
    /// feature needs — exactly the shape of `buffa-descriptor` and
    /// `buffa-types`, which ship every impl while letting the codegen
    /// toolchain (`buffa-codegen`/`buffa-build`/`protoc-gen-buffa`) depend
    /// on them with `default-features = false` and stay free of
    /// `serde`/`serde_json`/`base64`. Most consumers of `buffa-build` are
    /// **not** in this position: a `build.rs` that decides at build-script
    /// time whether to generate JSON wants `impl Serialize` to just exist.
    /// Default `false`.
    ///
    /// [`generate_json`]: Self::generate_json
    /// [`generate_views`]: Self::generate_views
    /// [`generate_text`]: Self::generate_text
    #[must_use]
    pub fn gate_impls_on_crate_features(mut self, enabled: bool) -> Self {
        self.codegen_config.gate_impls_on_crate_features = enabled;
        self
    }

    /// Gate only the reflection impls behind a `reflect` crate feature, without
    /// gating json/views/text (unlike
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features),
    /// which gates them together).
    ///
    /// For crates that ship views/text unconditionally but want the
    /// `buffa-descriptor`-dependent (and `std`-requiring) reflection surface to
    /// be opt-in. `buffa-types` is the motivating case.
    ///
    /// **Experimental and `#[doc(hidden)]`**, paired with
    /// [`generate_reflection_vtable`](Self::generate_reflection_vtable) until the
    /// public `ReflectMode` selector lands.
    #[doc(hidden)]
    #[must_use]
    pub fn gate_reflect_on_crate_feature(mut self, enabled: bool) -> Self {
        self.codegen_config.gate_reflect_on_crate_feature = enabled;
        self
    }

    /// Set the crate feature name the gated JSON impls are conditioned on
    /// (default: `"json"`).
    ///
    /// Only meaningful together with
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features);
    /// inert otherwise. Use when the consuming crate gates its JSON support
    /// behind a differently-named feature:
    ///
    /// ```toml
    /// [features]
    /// serde = ["buffa/json", "dep:serde", "dep:serde_json"]
    /// ```
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .generate_json(true)
    ///     .gate_impls_on_crate_features(true)
    ///     .json_feature_name("serde")
    /// # ;
    /// ```
    ///
    /// The name is emitted verbatim into `#[cfg(feature = "...")]`
    /// attributes and must be a valid Cargo feature name **declared in the
    /// consuming crate's `[features]` table**. A misspelled or undeclared
    /// name fails open: the `#[cfg]` is permanently false, so the gated
    /// impls silently compile away (on Rust ≥ 1.80 an undeclared name at
    /// least triggers the `unexpected_cfgs` warning). A name that is not a
    /// valid Cargo feature name at all (empty, or containing characters
    /// outside alphanumerics and `_`/`-`/`+`/`.`) makes [`compile`](Self::compile)
    /// fail with an error when the gate is active.
    #[must_use]
    pub fn json_feature_name(mut self, name: impl Into<String>) -> Self {
        self.codegen_config.feature_gate_names.json = name.into();
        self
    }

    /// Set the crate feature name the gated view impls are conditioned on
    /// (default: `"views"`).
    ///
    /// Only meaningful together with
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features);
    /// inert otherwise. See [`json_feature_name`](Self::json_feature_name).
    #[must_use]
    pub fn views_feature_name(mut self, name: impl Into<String>) -> Self {
        self.codegen_config.feature_gate_names.views = name.into();
        self
    }

    /// Set the crate feature name the gated textproto impls are conditioned
    /// on (default: `"text"`).
    ///
    /// Only meaningful together with
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features);
    /// inert otherwise. See [`json_feature_name`](Self::json_feature_name).
    #[must_use]
    pub fn text_feature_name(mut self, name: impl Into<String>) -> Self {
        self.codegen_config.feature_gate_names.text = name.into();
        self
    }

    /// Set the crate feature name the gated reflection impls are conditioned
    /// on (default: `"reflect"`).
    ///
    /// Only meaningful together with
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features)
    /// (or the experimental, hidden `gate_reflect_on_crate_feature`, which
    /// gates reflection alone); inert otherwise. See
    /// [`json_feature_name`](Self::json_feature_name).
    #[must_use]
    pub fn reflect_feature_name(mut self, name: impl Into<String>) -> Self {
        self.codegen_config.feature_gate_names.reflect = name.into();
        self
    }

    /// Prepend a prefix to every generated Rust type name (default: none).
    ///
    /// With prefix `"Rpc"`, `message User {}` generates `struct RpcUser`
    /// (and `RpcUserView` / `RpcUserOwnedView`); every cross-reference uses
    /// the prefixed name. Useful in multi-protocol systems where generated
    /// types from different domains would otherwise collide with each other
    /// or with a canonical hand-written model.
    ///
    /// Applies to message structs and enum types (top-level and nested).
    /// Module names, oneof enums, [`extern_path`](Self::extern_path)-mapped
    /// types (including well-known types), and the wire/JSON format are
    /// unaffected.
    ///
    /// When another crate references these prefixed types via its own
    /// [`extern_path`](Self::extern_path) mapping, the mapped Rust path must
    /// spell out the prefixed name (e.g. `::crate_a::RpcUser`) — the proto
    /// name carries no prefix, so the mapping is not derived automatically.
    ///
    /// The prefix must be PascalCase (`[A-Z][A-Za-z0-9]*`) — an ASCII
    /// uppercase letter followed by ASCII letters and digits — so the
    /// prefixed names stay conventionally cased; [`compile`](Self::compile)
    /// fails otherwise.
    #[must_use]
    pub fn type_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.codegen_config.type_name_prefix = prefix.into();
        self
    }

    /// Enable or disable `with_*` builder-style setter methods for
    /// explicit-presence fields (default: true).
    ///
    /// Each explicit-presence scalar, bytes, or enum field gets a
    /// `pub fn with_<name>(mut self, value: T) -> Self` method that wraps the
    /// value in `Some(...)` and returns `self`, enabling chained construction
    /// without the `Some(...)` boilerplate:
    ///
    /// ```ignore
    /// let req = MyRequest::default()
    ///     .with_name("alice")
    ///     .with_timeout_ms(30_000);
    /// ```
    ///
    /// String, bytes, and enum setters take `impl Into<T>` (so `&str`,
    /// `b"..."` literals, and bare enum variants work directly); other
    /// scalars take `T` to keep integer-literal inference unambiguous.
    ///
    /// Setters are pure inherent methods with no runtime dependency — they
    /// don't interact with the `json`/`views`/`text` feature gates. Disable
    /// only if you want to keep generated code minimal or have a competing
    /// `with_*` convention in your own crate.
    #[must_use]
    pub fn generate_with_setters(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_with_setters = enabled;
        self
    }

    /// Enable reflection on generated types (default: off).
    ///
    /// `generate_reflection(true)` selects [`ReflectMode::VTable`] — the fast
    /// path: `foo.reflect()` borrows `foo` directly (no encode/decode
    /// round-trip), and owned and view types implement `ReflectMessage`. For
    /// the smaller bridge implementation (`reflect()` round-trips through a
    /// [`DynamicMessage`]), use [`reflect_mode(ReflectMode::Bridge)`](Self::reflect_mode)
    /// instead. `generate_reflection(false)` is [`ReflectMode::Off`].
    ///
    /// Either mode embeds a lazily-built [`DescriptorPool`] (as
    /// `FileDescriptorSet` bytes) reachable as
    /// `your_crate::your_pkg::descriptor_pool()`.
    ///
    /// # Cargo.toml setup
    ///
    /// The consuming crate must depend on `buffa-descriptor` with the
    /// `reflect` feature and on `std`:
    ///
    /// ```toml
    /// [dependencies]
    /// buffa = { version = "0.7", features = ["std"] }
    /// buffa-descriptor = { version = "0.7", features = ["reflect", "std"] }
    /// ```
    ///
    /// When [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features)
    /// is also on, the impls are wrapped in `#[cfg(feature = "reflect")]`,
    /// so the consuming crate must declare a forwarding feature:
    ///
    /// ```toml
    /// [features]
    /// reflect = ["buffa-descriptor/reflect"]
    /// ```
    ///
    /// **Without the feature declared, the generated `Reflectable` impls
    /// silently disappear** — `cfg(feature = "reflect")` is permanently
    /// false in a crate that doesn't declare it. The first call to
    /// `.reflect()` fails to compile with "trait `Reflectable` not
    /// implemented", which is a misleading diagnostic. Most consumers
    /// should leave `gate_impls_on_crate_features` off.
    ///
    /// Reflecting message-typed fields also requires every crate that field
    /// types resolve to via an extern path — notably `buffa-types` for
    /// well-known types — to enable its own reflection feature; see
    /// [`reflect_mode`](Self::reflect_mode) ("Extern-path types") for the
    /// `Cargo.toml` requirement and mixed-mode behavior.
    ///
    /// # Performance
    ///
    /// In the default vtable mode, `reflect()` borrows `self` — no round-trip,
    /// no allocation; reflective accessors read fields in place. (Bridge mode
    /// instead pays one encode/decode round-trip plus a heap allocation per
    /// call.) Either way the first call pays a one-time pool build cost.
    ///
    /// # Build time and binary size
    ///
    /// Each generated package embeds its own copy of the full
    /// `FileDescriptorSet` (transitive closure). For a single-package
    /// crate this is one copy. For a multi-package codegen run the bytes
    /// duplicate per package — measurable for large proto trees. The
    /// serialization happens once per `compile()` call (not per package),
    /// so build-time CPU does not scale with package count. Vtable mode also
    /// emits an `impl ReflectMessage` per type, so it produces more code than
    /// bridge mode.
    ///
    /// [`ReflectCow`]: https://docs.rs/buffa-descriptor/latest/buffa_descriptor/reflect/enum.ReflectCow.html
    /// [`DynamicMessage`]: https://docs.rs/buffa-descriptor/latest/buffa_descriptor/reflect/struct.DynamicMessage.html
    /// [`DescriptorPool`]: https://docs.rs/buffa-descriptor/latest/buffa_descriptor/struct.DescriptorPool.html
    #[must_use]
    pub fn generate_reflection(mut self, enabled: bool) -> Self {
        // The simple on/off knob selects the fast vtable path; Bridge is opt-in
        // via `reflect_mode`.
        let mode = if enabled {
            ReflectMode::VTable
        } else {
            ReflectMode::Off
        };
        mode.apply(&mut self.codegen_config);
        self
    }

    /// Select the reflection mode (the fuller form of
    /// [`generate_reflection`](Self::generate_reflection)).
    ///
    /// - [`ReflectMode::Off`] — no reflection (the default); equivalent to
    ///   `generate_reflection(false)`.
    /// - [`ReflectMode::Bridge`] — `reflect()` round-trips through
    ///   `DynamicMessage`; smaller generated code, slower reflective access.
    /// - [`ReflectMode::VTable`] — `impl ReflectMessage` on owned and view
    ///   types, and `reflect()` borrows `self` with no round-trip; equivalent
    ///   to `generate_reflection(true)`. Does not require view generation —
    ///   with views off, only the owned impls are emitted.
    ///
    /// All non-`Off` modes require the consuming crate to depend on
    /// `buffa-descriptor` with its `reflect` feature and on `std`. The call
    /// site (`foo.reflect().get(fd)`) is identical across modes.
    ///
    /// # Extern-path types
    ///
    /// Reflection on a message reaches into its message-typed fields, so
    /// every crate that field types resolve to via an extern path must have
    /// its own reflection enabled. In particular, well-known types resolve
    /// to `buffa-types` by default, and its impls are behind a cargo
    /// feature: depend on `buffa-types = { ..., features = ["reflect"] }`
    /// or the build fails with unsatisfied `Reflectable` /
    /// `ReflectMessage` bounds on the WKT.
    ///
    /// # Mixed modes
    ///
    /// A vtable-mode message may embed owned message types generated in
    /// bridge mode (e.g. a dependency crate that chose the smaller output):
    /// reflective access degrades to an owned `DynamicMessage` snapshot at
    /// that boundary instead of failing. For a bridge-grade `repeated` or
    /// `map` field the snapshot is taken per element on every access, so
    /// reflecting a large mixed-mode collection scales the encode/decode
    /// cost by the element count. The *view* reflection surface cannot
    /// degrade — every view type embedded in a vtable-mode view must itself
    /// be vtable-grade, and a bridge-grade view field is a compile error.
    #[must_use]
    pub fn reflect_mode(mut self, mode: ReflectMode) -> Self {
        mode.apply(&mut self.codegen_config);
        self
    }

    /// Enable or disable idiomatic `UpperCamelCase` enum aliases (matches the
    /// [`CodeGenConfig`] default, currently on).
    ///
    /// Protobuf enum values are `SHOUTY_SNAKE_CASE` and stay the definitive Rust
    /// variants. When enabled, codegen additionally emits associated `const`s
    /// with the enum-name prefix stripped and the name converted to
    /// `UpperCamelCase` (`RULE_LEVEL_HIGH` → `RuleLevel::High`), purely
    /// additively — existing references and `Debug` output are unchanged.
    ///
    /// Aliases are suppressed per enum (with a build warning and a doc note) if
    /// any two values would collide after conversion, so a match is never forced
    /// to mix conventions. See [`CodeGenConfig::idiomatic_enum_aliases`].
    #[must_use]
    pub fn idiomatic_enum_aliases(mut self, enabled: bool) -> Self {
        self.codegen_config.idiomatic_enum_aliases = enabled;
        self
    }

    /// Emit one `<dotted.package>.rs` file per proto package instead of the
    /// per-proto-file content set plus `<pkg>.mod.rs` stitcher. Default:
    /// `false`.
    ///
    /// The single file inlines what the stitcher would otherwise `include!`,
    /// producing the same module structure. Required by
    /// [`idiomatic_imports`](Self::idiomatic_imports). See
    /// [`CodeGenConfig::file_per_package`] for caveats about packages that
    /// span multiple directories.
    #[must_use]
    pub fn file_per_package(mut self, enabled: bool) -> Self {
        self.codegen_config.file_per_package = enabled;
        self
    }

    /// **Experimental.** Emit `use`-backed short type names at the package
    /// root instead of fully-qualified paths, so struct fields read
    /// `MessageField<Timestamp>` instead of
    /// `::buffa::MessageField<::buffa_types::google::protobuf::Timestamp>`.
    /// Default: `false` (output is byte-for-byte identical to previous
    /// releases).
    ///
    /// Requires [`file_per_package`](Self::file_per_package) — the build
    /// fails otherwise. Short names that would collide with another item at
    /// the package root (or a name referenced bare by sibling emissions)
    /// fall back to parent-module qualification, then to the
    /// fully-qualified path.
    ///
    /// Only package-root type *declarations* are shortened; impl bodies,
    /// nested-message modules, and `__buffa` internals keep fully-qualified
    /// paths. "Experimental" means the output shape may change between
    /// releases and the option may be renamed or removed outside semver
    /// guarantees. See [`CodeGenConfig::idiomatic_imports`] for details.
    #[must_use]
    pub fn idiomatic_imports(mut self, enabled: bool) -> Self {
        self.codegen_config.idiomatic_imports = enabled;
        self
    }

    /// Enable or disable unknown field preservation (default: true).
    ///
    /// When enabled (the default), unrecognized fields encountered during
    /// decode are stored and re-emitted on encode — essential for proxy /
    /// middleware services and round-trip fidelity across schema versions.
    ///
    /// **Disabling is primarily a memory optimization** (24 bytes/message for
    /// the `UnknownFields` Vec header), not a throughput one. When no unknown
    /// fields appear on the wire — the common case for schema-aligned
    /// services — decode and encode costs are effectively identical in
    /// either mode. Consider disabling for embedded / `no_std` targets or
    /// large in-memory collections of small messages.
    #[must_use]
    pub fn preserve_unknown_fields(mut self, enabled: bool) -> Self {
        self.codegen_config.preserve_unknown_fields = enabled;
        self
    }

    /// Honor `features.utf8_validation = NONE` by emitting `Vec<u8>` / `&[u8]`
    /// for such string fields instead of `String` / `&str` (default: false).
    ///
    /// When disabled (the default), all string fields map to `String` and
    /// UTF-8 is validated on decode — stricter than proto2 requires, but
    /// ergonomic and safe.
    ///
    /// When enabled, string fields with `utf8_validation = NONE` become
    /// `Vec<u8>` / `&[u8]`. Decode skips validation; the caller chooses
    /// whether to `std::str::from_utf8` (checked) or `from_utf8_unchecked`
    /// (trusted-input fast path). This is the only sound Rust mapping when
    /// strings may actually contain non-UTF-8 bytes.
    ///
    /// **Note for proto2 users**: proto2's default is `utf8_validation = NONE`,
    /// so enabling this turns ALL proto2 string fields into `Vec<u8>`. Use
    /// only for new code or when profiling identifies UTF-8 validation as a
    /// bottleneck (it can be 10%+ of decode CPU for string-heavy messages).
    ///
    /// **JSON note**: fields normalized to bytes serialize as base64 in JSON
    /// (the proto3 JSON encoding for `bytes`). Keep strict mapping disabled
    /// for fields that need JSON string interop with other implementations.
    ///
    /// **Interaction with [`use_bytes_type`]**: when both are enabled,
    /// `map<bytes, bytes>` values stay `Vec<u8>` (the bytes-keyed JSON helper
    /// is concrete `HashMap<Vec<u8>, Vec<u8>>`). All other `bytes` shapes —
    /// singular / optional / repeated / oneof / `map<non-bytes, bytes>` —
    /// still become `bytes::Bytes`. The asymmetry is documented; if you hit
    /// it, see issue #76.
    ///
    /// [`use_bytes_type`]: Self::use_bytes_type
    #[must_use]
    pub fn strict_utf8_mapping(mut self, enabled: bool) -> Self {
        self.codegen_config.strict_utf8_mapping = enabled;
        self
    }

    /// Permit `option message_set_wire_format = true` on input messages.
    ///
    /// MessageSet is a legacy Google-internal wire format. Default: `false`
    /// (such messages produce a codegen error). Set to `true` only when
    /// compiling protos that interoperate with old Google-internal services.
    #[must_use]
    pub fn allow_message_set(mut self, enabled: bool) -> Self {
        self.codegen_config.allow_message_set = enabled;
        self
    }

    /// Declare an external type path mapping.
    ///
    /// The matched types reference the specified Rust path instead of being
    /// generated. This allows shared proto packages to be compiled once in a
    /// dedicated crate and referenced from others.
    ///
    /// `proto_path` is a fully-qualified protobuf path — either a **package**
    /// (`".my.common"`, mapping every type under it to a Rust module root) or a
    /// single **type FQN** (`".google.protobuf.Timestamp"`, mapping just that
    /// type, the prost/tonic idiom). The leading dot is optional and is added
    /// automatically. As in prost, the most specific entry wins: an exact type
    /// FQN beats a covering package prefix, which in turn beats a shorter
    /// prefix.
    ///
    /// `rust_path` is where the type(s) are accessible — a module root for a
    /// package mapping (e.g. `"::common_protos"`) or a full type path for a
    /// per-type mapping (e.g. `"::pbjson_types::Timestamp"`). It must be an
    /// absolute path (starting with `::` or `crate::`); any other value is
    /// emitted into the generated code verbatim and will fail to resolve there.
    ///
    /// **Nested types** inherit an enclosing message's per-type override:
    /// mapping `.my.pkg.Outer` to `::ext::Outer` resolves `.my.pkg.Outer.Inner`
    /// to `::ext::outer::Inner` — the override's parent module plus buffa's
    /// usual `snake_case(MessageName)` nested-types module (snake case of the
    /// *proto* message name, regardless of the override's final segment). This
    /// matches the layout of another buffa-generated crate; for a target crate
    /// laid out differently, add explicit per-type entries for the nested types
    /// as well.
    ///
    /// # Limitations
    ///
    /// An extern type that is referenced by a generated **view** must map to
    /// another buffa-generated crate — the view path is composed as
    /// `<rust_path_root>::__buffa::view::…`, which a non-buffa crate (e.g.
    /// `pbjson_types`) does not provide. Map per-type to a buffa crate, or
    /// disable views ([`generate_views(false)`](Self::generate_views)), for
    /// such types.
    ///
    /// A misconfigured mapping (a typo'd FQN target, a non-absolute
    /// `rust_path`, or a view-referenced type mapped to a non-buffa crate) is
    /// not diagnosed at generation time; it surfaces as an unresolved-path
    /// error when the generated code is compiled.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     // Whole-package mapping.
    ///     .extern_path(".my.common", "::common_protos")
    ///     // Per-type mapping (issue #111) — overrides the package prefix for
    ///     // just this type.
    ///     .extern_path(".google.protobuf.Timestamp", "::common_protos::well_known::Timestamp")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn extern_path(
        mut self,
        proto_path: impl Into<String>,
        rust_path: impl Into<String>,
    ) -> Self {
        let mut proto_path = proto_path.into();
        // Normalize: ensure the proto path is fully-qualified (leading dot).
        // Accept both ".my.package" and "my.package" for convenience.
        if !proto_path.starts_with('.') {
            proto_path.insert(0, '.');
        }
        self.codegen_config
            .extern_paths
            .push((proto_path, rust_path.into()));
        self
    }

    /// Configure `bytes` fields to use `bytes::Bytes` instead of `Vec<u8>`.
    ///
    /// Each path is a fully-qualified proto path prefix. Use `"."` to apply
    /// to all bytes fields, or specify individual field paths like
    /// `".my.pkg.MyMessage.data"`.
    ///
    /// Applies uniformly to singular, optional, repeated, oneof, **and
    /// `map<K, bytes>`** values — the map case lets `view → owned`
    /// conversion participate in the `to_owned_from_source` zero-copy
    /// `slice_ref` path. One carve-out: an effective `map<bytes, bytes>` keeps
    /// `Vec<u8>` values (the JSON helper for that combination is concrete
    /// `HashMap<Vec<u8>, Vec<u8>>`); every other shape becomes `Bytes`. A
    /// `bytes` map key is only reachable when [`strict_utf8_mapping`] is enabled
    /// *and* the `map<string, bytes>` field carries
    /// `[features.utf8_validation = NONE]` on its key, which normalizes the
    /// string key to `bytes` — `strict_utf8_mapping` alone does not trigger it.
    ///
    /// A **custom** `bytes` representation
    /// ([`bytes_type_custom`](Self::bytes_type_custom)) is honored for
    /// `map<K, bytes>` values too, the same as the built-in `Bytes` — but a
    /// custom map value (like a custom `repeated` element) must be a crate-local
    /// type, since codegen emits its `ReflectElement` / `ProtoElemJson` impls
    /// (the orphan rule forbids them for a foreign type).
    ///
    /// [`strict_utf8_mapping`]: Self::strict_utf8_mapping
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .use_bytes_type_in(&["."])  // all bytes fields use Bytes
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn use_bytes_type_in(self, paths: &[impl AsRef<str>]) -> Self {
        self.bytes_type_in(BytesRepr::Bytes, paths)
    }

    /// Use `bytes::Bytes` for all `bytes` fields in all messages.
    ///
    /// This is a convenience for `.use_bytes_type_in(&["."])`. Use
    /// [`use_bytes_type_in`] with specific proto paths if you only want `Bytes`
    /// for certain fields. See that method for the path-matching semantics, the
    /// `map<K, bytes>` rule, and the `map<bytes, bytes>` carve-out under
    /// [`strict_utf8_mapping`].
    ///
    /// [`use_bytes_type_in`]: Self::use_bytes_type_in
    /// [`strict_utf8_mapping`]: Self::strict_utf8_mapping
    #[must_use]
    pub fn use_bytes_type(self) -> Self {
        self.bytes_type(BytesRepr::Bytes)
    }

    /// Map `bytes` fields to a [`BytesRepr`] other than `Vec<u8>` for the given
    /// proto path prefixes. The bytes counterpart to
    /// [`string_type_in`](Self::string_type_in).
    ///
    /// Rules accumulate and the **last** matching rule wins, so call the broad
    /// [`bytes_type`](Self::bytes_type) *first*, then `bytes_type_in` for
    /// narrower overrides. For [`BytesRepr::Custom`], the downstream crate must
    /// depend on the crate providing the type (buffa does not re-export it).
    /// Only the owned Rust type changes — the wire format is unchanged and view
    /// types still borrow `&[u8]`.
    #[must_use]
    pub fn bytes_type_in(mut self, repr: BytesRepr, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .bytes_fields
            .extend(paths.iter().map(|p| (p.as_ref().to_string(), repr.clone())));
        self
    }

    /// Map every `bytes` field in all messages to the given [`BytesRepr`].
    /// Convenience for `.bytes_type_in(repr, &["."])`; call before any
    /// [`bytes_type_in`](Self::bytes_type_in) overrides (last matching rule
    /// wins).
    #[must_use]
    pub fn bytes_type(mut self, repr: BytesRepr) -> Self {
        self.codegen_config
            .bytes_fields
            .push((".".to_string(), repr));
        self
    }

    /// Map the matching `bytes` fields to a custom type named by its
    /// fully-qualified Rust path (e.g. `"::my_crate::MyBytes"`). The type must
    /// satisfy `buffa::ProtoBytes`, and the downstream crate must depend on the
    /// crate providing it. Shorthand for
    /// [`bytes_type_in`](Self::bytes_type_in)`(BytesRepr::Custom(path), paths)`.
    ///
    /// # Limitations
    ///
    /// - A **foreign** custom type used as a `repeated` element — or a
    ///   `map<K, bytes>` value — fails to compile: codegen emits
    ///   `ReflectElement` / `ProtoElemJson` impls for it, which the orphan rule
    ///   forbids for a foreign type. Wrap it in a crate-local newtype for those
    ///   cases; singular / optional / oneof uses work directly.
    /// - A `Custom` rule **does** apply to `map<K, bytes>` values (honored like
    ///   the built-in [`BytesRepr::Bytes`]); only the `map<bytes, bytes>`
    ///   carve-out keeps `Vec<u8>` values.
    /// - A `path` that does not parse as a Rust type is reported as a codegen
    ///   error from [`compile`](Self::compile).
    /// - A custom bytes type needs no native `arbitrary::Arbitrary` impl (a
    ///   generic builder handles it under `generate_arbitrary`).
    #[must_use]
    pub fn bytes_type_custom_in(self, path: &str, paths: &[impl AsRef<str>]) -> Self {
        self.bytes_type_in(BytesRepr::Custom(path.to_string()), paths)
    }

    /// Map every `bytes` field to the given custom type path. Convenience for
    /// `.bytes_type_custom_in(path, &["."])`; see it for the limitations
    /// (foreign `repeated` elements, `map` values, path parsing).
    #[must_use]
    pub fn bytes_type_custom(self, path: &str) -> Self {
        self.bytes_type(BytesRepr::Custom(path.to_string()))
    }

    /// Store the matching message-typed oneof variants inline instead of
    /// wrapping them in `Box<T>`.
    ///
    /// By default every message/group oneof variant is boxed so that recursive
    /// types compile. For non-recursive variants the `Box` is pure overhead (an
    /// allocation per construction); this opts the matching variants out.
    /// This affects the owned message enum only — view oneof variants remain
    /// boxed.
    ///
    /// Each path is a fully-qualified proto variant path prefix, e.g.
    /// `".my.pkg.MyMessage.body.small"` for one variant or `".my.pkg"` for a
    /// package (same matching as [`use_bytes_type_in`](Self::use_bytes_type_in)).
    /// A leading dot is added if missing, mirroring
    /// [`extern_path`](Self::extern_path).
    ///
    /// Recursive variants cannot be stored inline (the type would be
    /// unsized). A rule that names a recursive variant *exactly* is rejected
    /// at codegen time; a broader prefix rule silently keeps recursive
    /// variants boxed and inlines the rest. For example, with
    /// `unbox_oneof_in(&[".my.pkg.Node"])`, a self-referential
    /// `Node.kind.child` variant stays boxed while `Node`'s other message
    /// variants become inline.
    #[must_use]
    pub fn unbox_oneof_in(mut self, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .unboxed_oneof_fields
            .extend(paths.iter().map(|p| {
                let p = p.as_ref();
                // Normalize to the leading-dot form: matching and the
                // exact-path recursion error both depend on it.
                if p.starts_with('.') {
                    p.to_string()
                } else {
                    format!(".{p}")
                }
            }));
        self
    }

    /// Store every non-recursive message-typed oneof variant inline instead of
    /// boxing it. Convenience for `.unbox_oneof_in(&["."])`; recursive
    /// variants stay boxed.
    #[must_use]
    pub fn unbox_oneof(mut self) -> Self {
        self.codegen_config
            .unboxed_oneof_fields
            .push(".".to_string());
        self
    }

    /// Map `string` fields to a [`StringRepr`] other than `String` for the
    /// given proto path prefixes. The string counterpart to
    /// [`use_bytes_type_in`](Self::use_bytes_type_in).
    ///
    /// Each path is a fully-qualified proto path prefix (e.g.
    /// `".my.pkg.MyMessage.name"` for one field, `".my.pkg"` for a package).
    ///
    /// Rules accumulate and the **last** matching rule wins. Order therefore
    /// matters: call [`string_type`](Self::string_type) (the broad default)
    /// *first*, then `string_type_in` for narrower overrides — a broad rule
    /// added after a specific one will shadow it.
    ///
    /// For [`StringRepr::Custom`], the type must implement `buffa::ProtoString`,
    /// and the downstream crate must depend on the crate providing it (buffa does
    /// not re-export it). A foreign type cannot implement `ProtoString` directly
    /// (orphan rule) — point at a local newtype, or the `buffa-smolstr` crate for
    /// `smol_str::SmolStr`.
    ///
    /// Only the owned Rust type changes: the wire format is unchanged, view
    /// types still borrow `&str`, and `map<_, string>` keys and values stay
    /// `String`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .string_type_custom("::buffa_smolstr::SmolStr")  // broad default first
    ///     .string_type_custom_in("::my_crate::CompactStr", &[".my.pkg.Msg.body"]) // narrow override
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn string_type_in(mut self, repr: StringRepr, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .string_fields
            .extend(paths.iter().map(|p| (p.as_ref().to_string(), repr.clone())));
        self
    }

    /// Map every `string` field in all messages to the given [`StringRepr`].
    ///
    /// Convenience for `.string_type_in(repr, &["."])`. Call this *before* any
    /// [`string_type_in`](Self::string_type_in) overrides, since the last
    /// matching rule wins (a `"."` rule added later shadows earlier specific
    /// rules). `map<_, string>` keys and values stay `String`.
    #[must_use]
    pub fn string_type(mut self, repr: StringRepr) -> Self {
        self.codegen_config
            .string_fields
            .push((".".to_string(), repr));
        self
    }

    /// Map the matching `string` fields to a custom type that implements
    /// `buffa::ProtoString`, named by its fully-qualified Rust path (e.g.
    /// `"::buffa_smolstr::SmolStr"`, or a local newtype — a foreign type cannot
    /// implement the trait directly). The downstream crate must depend on the
    /// crate providing it. Shorthand for
    /// [`string_type_in`](Self::string_type_in)`(StringRepr::Custom(path), paths)`.
    ///
    /// # Limitations
    ///
    /// - A **foreign** custom type used as a `repeated` element fails to compile:
    ///   codegen emits a `ReflectElement` impl for it, which the orphan rule
    ///   forbids for a foreign type. Wrap it in a crate-local newtype for the
    ///   repeated case; singular / optional / oneof uses work directly.
    /// - **JSON of a `repeated` custom string** serializes elements through their
    ///   native `serde`, so such a type must derive `Serialize` / `Deserialize`
    ///   (and an external type must enable its `serde` feature). Singular /
    ///   optional / oneof custom strings use the `proto_string` with-module and
    ///   need no `serde` impl.
    /// - A `path` that does not parse as a Rust type is reported as a codegen
    ///   error from [`compile`](Self::compile).
    /// - A custom string type needs no native `arbitrary::Arbitrary` impl (a
    ///   generic builder handles it under `generate_arbitrary`).
    #[must_use]
    pub fn string_type_custom_in(self, path: &str, paths: &[impl AsRef<str>]) -> Self {
        self.string_type_in(StringRepr::Custom(path.to_string()), paths)
    }

    /// Map every `string` field to the given custom type path. Convenience for
    /// `.string_type_custom_in(path, &["."])`; see it for the limitations
    /// (foreign `repeated` elements, the `repeated` JSON `serde` requirement,
    /// path parsing).
    #[must_use]
    pub fn string_type_custom(self, path: &str) -> Self {
        self.string_type(StringRepr::Custom(path.to_string()))
    }

    /// Map the matching `map` fields to a [`MapRepr`] other than the default
    /// `HashMap`. Rules are matched with proto-segment-aware prefix logic; the
    /// **last** matching rule wins, so add a broad rule first and narrower
    /// overrides after.
    ///
    /// Use [`MapRepr::BTreeMap`] for the buffa-provided `BTreeMap` (deterministic
    /// key order, no extra dependency, no consumer code), or
    /// [`MapRepr::Custom`] for a crate-local newtype that implements
    /// `buffa::map_codec::MapStorage`.
    ///
    /// Only the owned collection changes: the wire format is unchanged and view
    /// types are unaffected.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .map_type(buffa_build::MapRepr::BTreeMap)                       // broad default
    ///     .map_type_in(buffa_build::MapRepr::HashMap, &[".my.pkg.Msg.cache"]) // narrow override
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn map_type_in(mut self, repr: MapRepr, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .map_fields
            .extend(paths.iter().map(|p| (p.as_ref().to_string(), repr.clone())));
        self
    }

    /// Map every `map` field in all messages to the given [`MapRepr`].
    /// Convenience for `.map_type_in(repr, &["."])`. Call this *before* any
    /// [`map_type_in`](Self::map_type_in) overrides, since the last matching
    /// rule wins.
    #[must_use]
    pub fn map_type(mut self, repr: MapRepr) -> Self {
        self.codegen_config.map_fields.push((".".to_string(), repr));
        self
    }

    /// Map the matching `map` fields to a custom collection implementing
    /// `buffa::map_codec::MapStorage`, named by its fully-qualified Rust path
    /// (e.g. `"::my_crate::OrderedMap"`). The path must **not** include the
    /// `<K, V>` parameters — they are applied positionally. Shorthand for
    /// [`map_type_in`](Self::map_type_in)`(MapRepr::Custom(path), paths)`.
    ///
    /// # Limitations
    ///
    /// - The path must name a **crate-local newtype** — a foreign map cannot
    ///   implement the buffa-owned reflection / serde traits (orphan rule).
    ///   Prefer the built-in [`MapRepr::BTreeMap`] unless you need a specific
    ///   foreign map.
    /// - The newtype must implement `buffa::MapStorage` plus the derive /
    ///   `FromIterator` / `ReflectMap` / serde / `arbitrary` bounds documented on
    ///   `buffa::map_codec::MapStorage` (the canonical list). JSON and
    ///   `arbitrary` work for every proto map key/value type regardless of the
    ///   container.
    /// - A path that does not parse as a Rust type is reported as a codegen
    ///   error from [`compile`](Self::compile).
    #[must_use]
    pub fn map_type_custom_in(self, path: &str, paths: &[impl AsRef<str>]) -> Self {
        self.map_type_in(MapRepr::Custom(path.to_string()), paths)
    }

    /// Map every `map` field to the given custom collection path. Convenience
    /// for `.map_type_custom_in(path, &["."])`; see it for the limitations (the
    /// crate-local newtype requirement, the trait bounds, path parsing).
    #[must_use]
    pub fn map_type_custom(self, path: &str) -> Self {
        self.map_type(MapRepr::Custom(path.to_string()))
    }

    /// Map the matching message fields to a [`PointerRepr`] other than the
    /// default `Inline`. Rules are matched with proto-segment-aware prefix
    /// logic; the **last** matching rule wins, so add a broad rule first and
    /// narrower overrides after. A leading dot is added to each path if
    /// missing.
    ///
    /// The default `Inline` is recursion-aware (recursive fields stay on
    /// `Box`), so this knob is for opting *out*: `PointerRepr::Box` for large
    /// or rarely-set submessages where reserving `size_of::<T>()` in the parent
    /// is wasteful, or `PointerRepr::Custom` for a third-party pointer.
    ///
    /// Applies to singular (and proto2 optional/required) message fields and to
    /// **boxed** oneof message/group variants (matched by the variant's path).
    /// A oneof variant opted into inline storage via [`unbox_oneof_in`](Self::unbox_oneof_in)
    /// takes precedence and gets no pointer; recursive variants stay boxed and so
    /// accept a custom pointer. Repeated message fields use a collection, not a
    /// pointer. For [`PointerRepr::Custom`], the pointer must implement
    /// `buffa::ProtoBox<T>` and be a crate-local newtype; the path is a
    /// **template** with a `*` placeholder for the message type (e.g.
    /// `"::my_crate::SmallBox<*>"`).
    ///
    /// Only the in-memory pointer changes: the wire format is unchanged and view
    /// types are unaffected.
    #[must_use]
    pub fn box_type_in(mut self, repr: PointerRepr, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .pointer_fields
            .extend(paths.iter().map(|p| {
                let p = p.as_ref();
                // Normalize to the leading-dot form: matching and the
                // exact-path Inline recursion error both depend on it.
                let p = if p.starts_with('.') {
                    p.to_string()
                } else {
                    format!(".{p}")
                };
                (p, repr.clone())
            }));
        self
    }

    /// Map every message field (and boxed oneof variant) to the given [`PointerRepr`].
    /// Convenience for `.box_type_in(repr, &["."])`. Call before any
    /// [`box_type_in`](Self::box_type_in) overrides, since the last matching
    /// rule wins. `box_type(PointerRepr::Box)` restores the pre-0.9 boxed
    /// default for every singular message field.
    #[must_use]
    pub fn box_type(mut self, repr: PointerRepr) -> Self {
        self.codegen_config
            .pointer_fields
            .push((".".to_string(), repr));
        self
    }

    /// Map the matching singular message fields to a custom pointer implementing
    /// `buffa::ProtoBox<T>`, named by a Rust type-path **template** with a `*`
    /// placeholder for the message type (e.g. `"::my_crate::SmallBox<*>"`).
    /// Shorthand for
    /// [`box_type_in`](Self::box_type_in)`(PointerRepr::Custom(template), paths)`.
    ///
    /// # Limitations
    ///
    /// - The template must contain at least one `*`; a template that omits it,
    ///   or whose substitution does not parse as a Rust type, is reported as a
    ///   codegen error from [`compile`](Self::compile).
    /// - The pointer must be exclusively owned (`Rc`/`Arc` are unusable — the
    ///   decoder needs `DerefMut`) and a crate-local newtype (a foreign pointer
    ///   cannot implement the buffa-owned `ProtoBox`).
    #[must_use]
    pub fn box_type_custom_in(self, template: &str, paths: &[impl AsRef<str>]) -> Self {
        self.box_type_in(PointerRepr::Custom(template.to_string()), paths)
    }

    /// Map every message field (and boxed oneof variant) to the given custom pointer template.
    /// Convenience for `.box_type_custom_in(template, &["."])`; see it for the
    /// limitations (the `*` placeholder, `Rc`/`Arc` exclusion, newtype rule).
    #[must_use]
    pub fn box_type_custom(self, template: &str) -> Self {
        self.box_type(PointerRepr::Custom(template.to_string()))
    }

    /// Map the matching `repeated` fields to a [`RepeatedRepr`] other than the
    /// default `Vec<T>`. Rules are matched with proto-segment-aware prefix
    /// logic; the **last** matching rule wins, so add a broad rule first and
    /// narrower overrides after. Applies only to `repeated` fields (not `map`).
    ///
    /// For [`RepeatedRepr::Custom`], the collection must implement
    /// `buffa::ProtoList<T>`. Unlike the scalar `string_type_custom` /
    /// `bytes_type_custom` knobs (which take a *complete* type path), this path
    /// is a **template** with a `*` placeholder for the element type, and it must
    /// name a **crate-local newtype** (a foreign collection cannot implement the
    /// buffa-owned `ProtoList`).
    ///
    /// Only the owned collection changes: the wire format is unchanged and view
    /// types still borrow `&[T]`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // `SmallList<T>` is a crate-local newtype over smallvec::SmallVec that
    /// // implements buffa::ProtoList (see the ProtoList docs for the template).
    /// buffa_build::Config::new()
    ///     .repeated_type_custom("::my_crate::SmallList<*>")               // broad default
    ///     .repeated_type_custom_in("::my_crate::SmallList8<*>", &[".my.pkg.Msg.tags"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn repeated_type_in(mut self, repr: RepeatedRepr, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .repeated_fields
            .extend(paths.iter().map(|p| (p.as_ref().to_string(), repr.clone())));
        self
    }

    /// Map every `repeated` field in all messages to the given
    /// [`RepeatedRepr`]. Convenience for `.repeated_type_in(repr, &["."])`.
    /// Call this *before* any [`repeated_type_in`](Self::repeated_type_in)
    /// overrides, since the last matching rule wins.
    #[must_use]
    pub fn repeated_type(mut self, repr: RepeatedRepr) -> Self {
        self.codegen_config
            .repeated_fields
            .push((".".to_string(), repr));
        self
    }

    /// Map the matching `repeated` fields to a custom collection implementing
    /// `buffa::ProtoList<T>`, named by a Rust type-path **template** with a `*`
    /// placeholder for the element type (e.g. `"::my_crate::SmallList<*>"`).
    /// Note the asymmetry with the scalar `string_type_custom` /
    /// `bytes_type_custom` knobs: those take a *complete* path, this takes a
    /// `*`-template that wraps the element. Shorthand for
    /// [`repeated_type_in`](Self::repeated_type_in)`(RepeatedRepr::Custom(template), paths)`.
    ///
    /// # Limitations
    ///
    /// - The template must contain at least one `*`; a template that omits it,
    ///   or whose substitution does not parse as a Rust type, is reported as a
    ///   codegen error from [`compile`](Self::compile).
    /// - The template must name a **crate-local newtype** — a foreign collection
    ///   cannot implement the buffa-owned `ProtoList` (orphan rule). This applies
    ///   to *every* build, not just reflection: the generated decode and clear
    ///   code require `Field: ProtoList`.
    /// - Under reflection / vtable the newtype must also implement
    ///   `buffa_descriptor`'s `ReflectList` (not derivable, but a `Vec`-backed
    ///   newtype can delegate to the inner `Vec<T>`). Under JSON it must
    ///   implement `serde::Serialize` / `Deserialize`; under `generate_arbitrary`,
    ///   `arbitrary::Arbitrary` (derivable on a newtype). See `buffa::ProtoList`
    ///   for a worked newtype example.
    #[must_use]
    pub fn repeated_type_custom_in(self, template: &str, paths: &[impl AsRef<str>]) -> Self {
        self.repeated_type_in(RepeatedRepr::Custom(template.to_string()), paths)
    }

    /// Map every `repeated` field to the given custom collection template.
    /// Convenience for `.repeated_type_custom_in(template, &["."])`; see it for
    /// the limitations (the `*` placeholder, foreign reflection, the JSON /
    /// `arbitrary` requirements).
    #[must_use]
    pub fn repeated_type_custom(self, template: &str) -> Self {
        self.repeated_type(RepeatedRepr::Custom(template.to_string()))
    }

    /// Add a custom attribute to generated types (messages and enums)
    /// matching a proto path prefix.
    ///
    /// `path` is a fully-qualified proto path prefix: `"."` applies to all
    /// types, `".my.pkg"` to types in that package, `".my.pkg.MyMessage"`
    /// to a specific type. A leading `.` is auto-prepended if omitted; a
    /// trailing `.` is trimmed. Prefix matching respects proto-segment
    /// boundaries, so `".my.pk"` does not match `".my.pkg.Msg"`.
    ///
    /// `attribute` is a raw Rust attribute string
    /// (e.g., `"#[derive(serde::Serialize)]"`). A malformed attribute
    /// produces [`CodeGenError::InvalidCustomAttribute`](buffa_codegen::CodeGenError)
    /// at compile time rather than being silently dropped.
    ///
    /// Multiple calls accumulate in insertion order — all matching attributes
    /// are emitted, and ordering is preserved in generated code.
    ///
    /// Also applies to generated oneof enums when `path` matches
    /// `".pkg.Msg.my_oneof"` (the oneof's fully-qualified path).
    ///
    /// # Pitfalls
    ///
    /// buffa already emits `#[derive(Clone, PartialEq)]` on messages and
    /// `#[derive(Clone, PartialEq, Debug)]` on oneofs (oneofs with a
    /// `[debug_redact = true]` variant get a generated `Debug` impl instead
    /// of the `Debug` derive); adding a duplicate derive via
    /// `type_attribute(".", "#[derive(Clone)]")` produces a compile error in
    /// the generated code.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .type_attribute(".", "#[derive(serde::Serialize)]")
    ///     .type_attribute(".my.pkg.MyEnum", "#[derive(strum::EnumIter)]")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn type_attribute(mut self, path: impl Into<String>, attribute: impl Into<String>) -> Self {
        self.codegen_config
            .type_attributes
            .push((normalize_attr_path(path.into()), attribute.into()));
        self
    }

    /// Add a custom attribute to generated struct fields matching a proto
    /// path prefix.
    ///
    /// `path` is a fully-qualified proto field path (e.g.,
    /// `".my.pkg.MyMessage.my_field"`). `"."` applies to all fields. A
    /// leading `.` is auto-prepended if omitted; a trailing `.` is trimmed.
    /// Prefix matching respects proto-segment boundaries.
    ///
    /// Also applies to oneof variants when `path` matches
    /// `".pkg.Msg.my_oneof.variant_name"`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .field_attribute(".my.pkg.MyMessage.secret_key", "#[serde(skip)]")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn field_attribute(
        mut self,
        path: impl Into<String>,
        attribute: impl Into<String>,
    ) -> Self {
        self.codegen_config
            .field_attributes
            .push((normalize_attr_path(path.into()), attribute.into()));
        self
    }

    /// Add a custom attribute to generated message structs only (not enums,
    /// not oneof enums — those are reached by
    /// [`enum_attribute`](Self::enum_attribute) and
    /// [`oneof_attribute`](Self::oneof_attribute) respectively) matching a
    /// proto path prefix.
    ///
    /// Same path-matching semantics as [`type_attribute`](Self::type_attribute) —
    /// leading `.` auto-prepended, trailing `.` trimmed, proto-segment-aware
    /// prefix matching, accumulation in insertion order. A malformed attribute
    /// produces a compile-time error. Useful for struct-only attributes like
    /// `#[serde(default)]`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .message_attribute(".", "#[serde(default)]")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn message_attribute(
        mut self,
        path: impl Into<String>,
        attribute: impl Into<String>,
    ) -> Self {
        self.codegen_config
            .message_attributes
            .push((normalize_attr_path(path.into()), attribute.into()));
        self
    }

    /// Add a custom attribute to generated enum types only (not message
    /// structs, not oneof enums — those are reached by
    /// [`type_attribute`](Self::type_attribute) on the oneof's path or by
    /// [`oneof_attribute`](Self::oneof_attribute)) matching a proto path
    /// prefix.
    ///
    /// Same path-matching semantics as [`type_attribute`](Self::type_attribute) —
    /// leading `.` auto-prepended, trailing `.` trimmed, proto-segment-aware
    /// prefix matching, accumulation in insertion order. A malformed attribute
    /// produces a compile-time error. Useful when you want to inject an
    /// attribute on every enum in a package without also matching the
    /// (often more numerous) messages that share the path prefix — e.g.
    /// `#[derive(strum::EnumIter)]`, which only makes sense on enums.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .enum_attribute(".my.pkg", "#[derive(strum::EnumIter)]")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn enum_attribute(mut self, path: impl Into<String>, attribute: impl Into<String>) -> Self {
        self.codegen_config
            .enum_attributes
            .push((normalize_attr_path(path.into()), attribute.into()));
        self
    }

    /// Add a custom attribute to generated oneof enums only (not message
    /// structs, not regular enums) matching a proto path prefix.
    ///
    /// Same path-matching semantics as [`type_attribute`](Self::type_attribute):
    /// a leading `.` is auto-prepended, a trailing `.` is trimmed, prefixes
    /// match on proto-path segments, and attributes accumulate in insertion
    /// order. The match key is the oneof's fully-qualified path
    /// (`.my.pkg.MyMessage.my_oneof`) — the whole-enum path has no variant
    /// segment; to target a single variant's field, append `.variant_name`
    /// and use [`field_attribute`](Self::field_attribute) instead. A
    /// malformed attribute produces a compile-time error in the generated
    /// code. Useful when a oneof needs a different attribute set than the
    /// surrounding types — for example to keep `#[derive(serde::Serialize)]`
    /// on messages and oneofs while
    /// [`enum_attribute`](Self::enum_attribute) gives the regular enums a
    /// different serde derive.
    ///
    /// Applies to the owned oneof enum only; the zero-copy view-of-oneof
    /// enum receives no custom attributes (true of the whole attribute
    /// family). For JSON serialization of both owned types and views, use
    /// [`generate_json(true)`](Self::generate_json), which emits canonical
    /// protobuf-JSON impls rather than derived ones.
    ///
    /// # Pitfalls
    ///
    /// Generated oneof enums already derive `Clone`, `PartialEq`, and
    /// `Debug` (oneofs containing `[debug_redact = true]` fields replace the
    /// `Debug` derive with a manual impl). Re-deriving any of these via
    /// `oneof_attribute` produces a conflicting-implementation compile error
    /// inside the generated code.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     // one specific oneof; ".my.pkg" would match every oneof in the package
    ///     .oneof_attribute(".my.pkg.MyMessage.my_oneof", "#[derive(serde::Serialize)]")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn oneof_attribute(
        mut self,
        path: impl Into<String>,
        attribute: impl Into<String>,
    ) -> Self {
        self.codegen_config
            .oneof_attributes
            .push((normalize_attr_path(path.into()), attribute.into()));
        self
    }

    /// Use `buf build` instead of `protoc` for descriptor generation.
    ///
    /// `buf` is often easier to install and keep current than `protoc`
    /// (which many distros pin to old versions). This mode is intended for
    /// the **single-crate case**: a `buf.yaml` at the crate root defining
    /// the module layout.
    ///
    /// Requires `buf` on PATH and a `buf.yaml` at the crate root. The
    /// [`includes()`](Self::includes) setting is ignored — buf resolves
    /// imports via its own module configuration.
    ///
    /// Each path given to [`files()`](Self::files) must be **relative to its
    /// owning module's directory** (the `path:` value inside `buf.yaml`), not
    /// the crate root where `buf.yaml` itself lives. buf strips the module
    /// path when producing `FileDescriptorProto.name`, so for
    /// `modules: [{path: proto}]` and a file on disk at
    /// `proto/api/v1/service.proto`, the descriptor name is
    /// `api/v1/service.proto` — that is what `.files()` must contain.
    /// Multiple modules in one `buf.yaml` work fine; buf enforces that
    /// module-relative names are unique across the workspace.
    ///
    /// # Monorepo / multi-module setups
    ///
    /// For a workspace-root `buf.yaml` with many modules, this mode is a
    /// poor fit. Prefer running `buf generate` with the `protoc-gen-buffa`
    /// plugin and checking in the generated code, or use
    /// [`descriptor_set()`](Self::descriptor_set) with the output of
    /// `buf build --as-file-descriptor-set -o fds.binpb <module-path>`
    /// run as a pre-build step.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // buf.yaml (at crate root):
    /// //   version: v2
    /// //   modules:
    /// //     - path: proto
    /// //
    /// // build.rs:
    /// buffa_build::Config::new()
    ///     .use_buf()
    ///     .files(&["api/v1/service.proto"])  // relative to module root
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn use_buf(mut self) -> Self {
        self.descriptor_source = DescriptorSource::Buf;
        self
    }

    /// Use a pre-compiled `FileDescriptorSet` binary file as input.
    ///
    /// Skips invoking `protoc` or `buf` entirely. The file must contain a
    /// serialized `google.protobuf.FileDescriptorSet` (as produced by
    /// `protoc --descriptor_set_out` or `buf build --as-file-descriptor-set`).
    ///
    /// When using this, `.files()` specifies which proto files in the
    /// descriptor set to generate code for (matching by proto file name).
    #[must_use]
    pub fn descriptor_set(mut self, path: impl Into<PathBuf>) -> Self {
        self.descriptor_source = DescriptorSource::Precompiled(path.into());
        self
    }

    /// Generate a module-tree include file alongside the per-package `.rs`
    /// files.
    ///
    /// The include file contains nested `pub mod` declarations with
    /// `include!()` directives that assemble the generated code into a
    /// module hierarchy matching the protobuf package structure. Users can
    /// then include this single file instead of manually creating the
    /// module tree.
    ///
    /// The form of the emitted `include!` directives depends on whether
    /// [`out_dir`](Self::out_dir) was set:
    ///
    /// - **Default (`$OUT_DIR`)**: emits
    ///   `include!(concat!(env!("OUT_DIR"), "/foo.rs"))`, for use from
    ///   `build.rs` via `include!(concat!(env!("OUT_DIR"), "/<name>"))`.
    /// - **Explicit `out_dir`**: emits sibling-relative `include!("foo.rs")`,
    ///   for checking the generated code into the source tree and referencing
    ///   it as a module (e.g. `mod gen;`).
    ///
    /// # Example — `build.rs` / `$OUT_DIR`
    ///
    /// ```rust,ignore
    /// // build.rs
    /// buffa_build::Config::new()
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .include_file("_include.rs")
    ///     .compile()
    ///     .unwrap();
    ///
    /// // lib.rs
    /// include!(concat!(env!("OUT_DIR"), "/_include.rs"));
    /// ```
    ///
    /// # Example — checked-in source
    ///
    /// ```rust,ignore
    /// // codegen.rs (run manually, not from build.rs)
    /// buffa_build::Config::new()
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .out_dir("src/gen")
    ///     .include_file("mod.rs")
    ///     .compile()
    ///     .unwrap();
    ///
    /// // lib.rs
    /// mod gen;
    /// ```
    #[must_use]
    pub fn include_file(mut self, name: impl Into<String>) -> Self {
        self.include_file = Some(name.into());
        self
    }

    /// Compile proto files and generate Rust source.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `OUT_DIR` is not set and no `out_dir` was configured
    /// - `protoc` or `buf` cannot be found on `PATH` (when using those sources)
    /// - the proto compiler exits with a non-zero status (syntax errors,
    ///   missing imports, etc.)
    /// - a precompiled descriptor set file cannot be read
    /// - the descriptor set bytes cannot be decoded as a `FileDescriptorSet`
    /// - code generation fails (e.g. unsupported proto feature)
    /// - the output directory cannot be created or written to
    pub fn compile(self) -> Result<(), Box<dyn std::error::Error>> {
        // When out_dir is explicitly set, the include file should use
        // relative `include!("foo.rs")` paths (the index is a sibling of the
        // generated files). When defaulted to $OUT_DIR, keep the
        // `concat!(env!("OUT_DIR"), ...)` form so that
        // `include!(concat!(env!("OUT_DIR"), "/_include.rs"))` from src/
        // still resolves to absolute paths.
        let relative_includes = self.out_dir.is_some();
        let out_dir = self
            .out_dir
            .or_else(|| std::env::var("OUT_DIR").ok().map(PathBuf::from))
            .ok_or("OUT_DIR not set and no out_dir configured")?;

        // Produce a FileDescriptorSet from the configured source.
        let descriptor_bytes = match &self.descriptor_source {
            DescriptorSource::Protoc => invoke_protoc(&self.files, &self.includes)?,
            DescriptorSource::Buf => invoke_buf()?,
            DescriptorSource::Precompiled(path) => std::fs::read(path).map_err(|e| {
                format!("failed to read descriptor set '{}': {}", path.display(), e)
            })?,
        };
        let fds = FileDescriptorSet::decode_from_slice(&descriptor_bytes)
            .map_err(|e| format!("failed to decode FileDescriptorSet: {}", e))?;

        // Determine which files were explicitly requested.
        //
        // `FileDescriptorProto.name` contains the path relative to the proto
        // source root (protoc: `--proto_path`; buf: the module root). For
        // Precompiled and Buf mode, `.files()` are expected to already be
        // proto-relative names. For Protoc mode, strip the longest matching
        // include prefix.
        let files_to_generate: Vec<String> = if matches!(
            self.descriptor_source,
            DescriptorSource::Precompiled(_) | DescriptorSource::Buf
        ) {
            self.files
                .iter()
                .filter_map(|f| f.to_str().map(str::to_string))
                .collect()
        } else {
            self.files
                .iter()
                .map(|f| proto_relative_name(f, &self.includes))
                .filter(|s| !s.is_empty())
                .collect()
        };

        // Generate Rust source. Per-proto content files plus a per-package
        // `.mod.rs` stitcher; only the stitchers need wiring into the
        // module tree (content files are reached via `include!` from
        // there).
        let (generated, warnings) = buffa_codegen::generate_with_diagnostics(
            &fds.file,
            &files_to_generate,
            &self.codegen_config,
        )?;

        // Surface non-fatal codegen diagnostics as Cargo build warnings. This
        // runs inside the consumer's `build.rs`, so `cargo:warning=` is shown in
        // their normal `cargo build` output.
        for warning in warnings {
            println!("cargo:warning=buffa: {warning}");
        }

        // Write output files; collect (name, package) for PackageMod entries.
        let mut output_entries: Vec<(String, String)> = Vec::new();
        for file in generated {
            let path = out_dir.join(&file.name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_if_changed(&path, file.content.as_bytes())?;
            if file.kind == buffa_codegen::GeneratedFileKind::PackageMod {
                output_entries.push((file.name, file.package));
            }
        }

        // Generate the include file if requested.
        if let Some(ref include_name) = self.include_file {
            let include_content = generate_include_file(&output_entries, relative_includes);
            let include_path = out_dir.join(include_name);
            write_if_changed(&include_path, include_content.as_bytes())?;
        }

        // Tell cargo to re-run if any proto file changes.
        //
        // For Buf mode, `self.files` are module-root-relative and cargo can't
        // stat them — use `buf ls-files` instead, which lists all workspace
        // protos with workspace-relative paths. This also catches changes to
        // transitively-imported protos (a gap in the Protoc mode, which only
        // watches explicitly-listed files).
        match self.descriptor_source {
            DescriptorSource::Buf => emit_buf_rerun_if_changed(),
            DescriptorSource::Protoc => {
                // Rerun if PROTOC changes (different binary may accept
                // protos the previous one rejected, e.g. newer editions).
                println!("cargo:rerun-if-env-changed=PROTOC");
                for proto_file in &self.files {
                    println!("cargo:rerun-if-changed={}", proto_file.display());
                }
            }
            DescriptorSource::Precompiled(ref path) => {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a user-supplied attribute-match path.
///
/// - Prepends `.` if absent so all stored paths are rooted.
/// - Trims trailing `.` so `".my.pkg."` and `".my.pkg"` behave identically
///   (trailing-dot patterns otherwise never match a real FQN).
/// - The bare catch-all `"."` is preserved as-is.
fn normalize_attr_path(mut path: String) -> String {
    if !path.starts_with('.') {
        path.insert(0, '.');
    }
    if path.len() > 1 {
        while path.ends_with('.') {
            path.pop();
        }
    }
    path
}

/// Write `content` to `path` only if the file doesn't already exist with
/// identical content. Avoids bumping timestamps on unchanged files, which
/// prevents unnecessary downstream recompilation.
fn write_if_changed(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Ok(existing) = std::fs::read(path) {
        if existing == content {
            return Ok(());
        }
    }
    std::fs::write(path, content)
}

/// Invoke `protoc` to produce a `FileDescriptorSet` (serialized bytes).
fn invoke_protoc(
    files: &[PathBuf],
    includes: &[PathBuf],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());

    let descriptor_file =
        tempfile::NamedTempFile::new().map_err(|e| format!("failed to create temp file: {}", e))?;
    let descriptor_path = descriptor_file.path().to_path_buf();

    let mut cmd = Command::new(&protoc);
    cmd.arg("--include_imports");
    cmd.arg("--include_source_info");
    cmd.arg(format!(
        "--descriptor_set_out={}",
        descriptor_path.display()
    ));

    for include in includes {
        cmd.arg(format!("--proto_path={}", include.display()));
    }

    for file in files {
        cmd.arg(file.as_os_str());
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run protoc ({}): {}", protoc, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("protoc failed: {}", stderr).into());
    }

    let bytes = std::fs::read(&descriptor_path)
        .map_err(|e| format!("failed to read descriptor set: {}", e))?;

    Ok(bytes)
}

/// Invoke `buf build` to produce a `FileDescriptorSet` (serialized bytes).
///
/// Requires a `buf.yaml` discoverable from the build script's cwd. Builds
/// the entire workspace — no `--path` filtering, because buf's `--path` flag
/// expects workspace-relative paths while `FileDescriptorProto.name` is
/// module-root-relative; passing user paths to both would be a contradiction.
/// Codegen filtering happens on our side via `files_to_generate` matching.
fn invoke_buf() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // buf build includes SourceCodeInfo by default (there's an
    // --exclude-source-info flag to disable it), so proto comments
    // propagate to generated code without an explicit opt-in here.
    let output = Command::new("buf")
        .arg("build")
        .arg("--as-file-descriptor-set")
        .arg("-o")
        .arg("-")
        .output()
        .map_err(|e| format!("failed to run buf (is it installed and on PATH?): {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(
            format!("buf build failed (is buf.yaml present at crate root?): {stderr}").into(),
        );
    }

    Ok(output.stdout)
}

/// Emit `cargo:rerun-if-changed` directives for a buf workspace.
///
/// Runs `buf ls-files` to discover all proto files with workspace-relative
/// paths (which cargo can stat). Also watches `buf.yaml` and `buf.lock`
/// (the latter only if it exists — cargo treats a missing rerun-if-changed
/// path as always-dirty). Failure is non-fatal: worst case cargo reruns
/// every build.
fn emit_buf_rerun_if_changed() {
    println!("cargo:rerun-if-changed=buf.yaml");
    if Path::new("buf.lock").exists() {
        println!("cargo:rerun-if-changed=buf.lock");
    }
    match Command::new("buf").arg("ls-files").output() {
        Ok(out) if out.status.success() => {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let path = line.trim();
                if !path.is_empty() {
                    println!("cargo:rerun-if-changed={path}");
                }
            }
        }
        _ => {
            // ls-files failed; cargo already knows about buf.yaml above.
            // If buf itself is missing, invoke_buf() will error clearly.
        }
    }
}

/// Convert a filesystem proto path to the name protoc uses in the descriptor.
///
/// `FileDescriptorProto.name` is relative to the `--proto_path` include
/// directory. This strips the longest matching include prefix; if no include
/// matches, returns the path as-is (not just file_name — that would break
/// nested proto directories).
fn proto_relative_name(file: &Path, includes: &[PathBuf]) -> String {
    // Longest prefix wins: a file under both "proto/" and "proto/vendor/"
    // should strip "proto/vendor/" for a correct relative name.
    let mut best: Option<&Path> = None;
    for include in includes {
        if let Ok(rel) = file.strip_prefix(include) {
            match best {
                Some(prev) if prev.as_os_str().len() <= rel.as_os_str().len() => {}
                _ => best = Some(rel),
            }
        }
    }
    best.unwrap_or(file).to_str().unwrap_or("").to_string()
}

/// Generate the content of an include file that assembles generated `.rs`
/// files into a nested module tree matching the protobuf package hierarchy.
///
/// Each generated file is named like `my.package.file_name.rs`. The package
/// segments become `pub mod` wrappers, and the file is `include!`d inside
/// the innermost module.
///
/// For example, files `["foo.bar.rs", "foo.baz.rs"]` produce:
/// ```text
/// pub mod foo {
///     #[allow(unused_imports)]
///     use super::*;
///     include!(concat!(env!("OUT_DIR"), "/foo.bar.rs"));
///     include!(concat!(env!("OUT_DIR"), "/foo.baz.rs"));
/// }
/// ```
///
/// When `relative` is true (the caller set [`Config::out_dir`] explicitly),
/// `include!` directives use bare sibling paths (`include!("foo.bar.rs")`)
/// instead of the `env!("OUT_DIR")` prefix, so the include file works when
/// checked into the source tree and referenced via `mod`.
fn generate_include_file(entries: &[(String, String)], relative: bool) -> String {
    let mode = if relative {
        buffa_codegen::IncludeMode::Relative("")
    } else {
        buffa_codegen::IncludeMode::OutDir
    };
    // Inner-allow off: this output is consumed via `include!` from
    // user-authored `lib.rs`, where `#![allow(...)]` is not valid.
    buffa_codegen::generate_module_tree(entries, mode, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_name_setters_reach_codegen_config() {
        let config = Config::new()
            .json_feature_name("serde")
            .views_feature_name("zero-copy")
            .text_feature_name(String::from("textproto"))
            .reflect_feature_name("reflection")
            .codegen_config;
        let names = &config.feature_gate_names;
        assert_eq!(names.json, "serde");
        assert_eq!(names.views, "zero-copy");
        assert_eq!(names.text, "textproto");
        assert_eq!(names.reflect, "reflection");
    }

    #[test]
    fn box_type_in_normalizes_leading_dot() {
        // Without normalization a dotless path would silently match nothing,
        // and the exact-path Inline recursion error would never fire for it.
        let config = Config::new()
            .box_type_in(PointerRepr::Box, &["my.pkg.Msg.inner", ".my.pkg.Other"])
            .codegen_config;
        assert_eq!(
            config.pointer_fields,
            vec![
                (".my.pkg.Msg.inner".to_string(), PointerRepr::Box),
                (".my.pkg.Other".to_string(), PointerRepr::Box),
            ]
        );
    }

    #[test]
    fn unbox_oneof_in_normalizes_leading_dot() {
        // Without normalization a dotless path would silently match nothing,
        // and the exact-path recursion error would never fire for it.
        let config = Config::new()
            .unbox_oneof_in(&["my.pkg.Msg.body.small", ".my.pkg.Other"])
            .codegen_config;
        assert_eq!(
            config.unboxed_oneof_fields,
            vec![
                ".my.pkg.Msg.body.small".to_string(),
                ".my.pkg.Other".to_string()
            ]
        );
    }

    #[test]
    fn proto_relative_name_strips_include() {
        let got = proto_relative_name(
            Path::new("proto/my/service.proto"),
            &[PathBuf::from("proto/")],
        );
        assert_eq!(got, "my/service.proto");
    }

    #[test]
    fn proto_relative_name_longest_prefix_wins() {
        // Overlapping includes: file under both proto/ and proto/vendor/.
        // Must strip the LONGER prefix for the correct relative name.
        let got = proto_relative_name(
            Path::new("proto/vendor/ext.proto"),
            &[PathBuf::from("proto/"), PathBuf::from("proto/vendor/")],
        );
        assert_eq!(got, "ext.proto");
        // Same with reversed include order.
        let got = proto_relative_name(
            Path::new("proto/vendor/ext.proto"),
            &[PathBuf::from("proto/vendor/"), PathBuf::from("proto/")],
        );
        assert_eq!(got, "ext.proto");
    }

    #[test]
    fn proto_relative_name_no_match_returns_full_path() {
        // Regression: previously fell back to file_name(), which stripped
        // directory components and broke descriptor_set() mode with nested
        // proto packages. Now returns the full path as-is.
        let got = proto_relative_name(Path::new("my/pkg/service.proto"), &[]);
        assert_eq!(got, "my/pkg/service.proto");
    }

    #[test]
    fn proto_relative_name_no_match_with_unrelated_includes() {
        let got = proto_relative_name(
            Path::new("src/my.proto"),
            &[PathBuf::from("other/"), PathBuf::from("third/")],
        );
        assert_eq!(got, "src/my.proto");
    }

    #[test]
    fn include_file_out_dir_mode_uses_env_var() {
        let entries = vec![
            ("foo.bar.rs".to_string(), "foo".to_string()),
            ("root.rs".to_string(), String::new()),
        ];
        let out = generate_include_file(&entries, false);
        assert!(
            out.contains(r#"include!(concat!(env!("OUT_DIR"), "/foo.bar.rs"));"#),
            "nested-package file should use env!(OUT_DIR): {out}"
        );
        assert!(
            out.contains(r#"include!(concat!(env!("OUT_DIR"), "/root.rs"));"#),
            "empty-package file should use env!(OUT_DIR): {out}"
        );
        assert!(!out.contains(r#"include!("foo.bar.rs")"#));
    }

    #[test]
    fn include_file_relative_mode_uses_sibling_paths() {
        let entries = vec![
            ("foo.bar.rs".to_string(), "foo".to_string()),
            ("root.rs".to_string(), String::new()),
        ];
        let out = generate_include_file(&entries, true);
        assert!(
            out.contains(r#"include!("foo.bar.rs");"#),
            "nested-package file should use relative path: {out}"
        );
        assert!(
            out.contains(r#"include!("root.rs");"#),
            "empty-package file should use relative path: {out}"
        );
        assert!(
            !out.contains("OUT_DIR"),
            "relative mode must not reference OUT_DIR: {out}"
        );
    }

    #[test]
    fn include_file_relative_mode_nested_packages() {
        // Two files in the same depth-2 package: verifies the relative flag
        // propagates through recursive emit() calls and both files land in
        // the same innermost mod.
        let entries = vec![
            ("a.b.one.rs".to_string(), "a.b".to_string()),
            ("a.b.two.rs".to_string(), "a.b".to_string()),
        ];
        let out = generate_include_file(&entries, true);
        // Both includes should appear once, at the same depth-2 indent,
        // inside a single `pub mod b { ... }`.
        let indent = "        "; // depth 2 = 8 spaces
        assert!(
            out.contains(&format!(r#"{indent}include!("a.b.one.rs");"#)),
            "first file at depth 2: {out}"
        );
        assert!(
            out.contains(&format!(r#"{indent}include!("a.b.two.rs");"#)),
            "second file at depth 2: {out}"
        );
        assert_eq!(
            out.matches("pub mod b {").count(),
            1,
            "both files share one `mod b`: {out}"
        );
        assert!(!out.contains("OUT_DIR"));
    }

    #[test]
    fn write_if_changed_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.rs");
        write_if_changed(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn write_if_changed_skips_identical_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.rs");
        std::fs::write(&path, b"content").unwrap();
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Sleep briefly so any write would produce a different mtime.
        std::thread::sleep(std::time::Duration::from_millis(50));

        write_if_changed(&path, b"content").unwrap();
        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn write_if_changed_overwrites_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.rs");
        std::fs::write(&path, b"old").unwrap();

        write_if_changed(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn normalize_attr_path_prepends_leading_dot() {
        assert_eq!(normalize_attr_path("my.pkg".into()), ".my.pkg");
    }

    #[test]
    fn normalize_attr_path_preserves_leading_dot() {
        assert_eq!(normalize_attr_path(".my.pkg".into()), ".my.pkg");
    }

    #[test]
    fn normalize_attr_path_trims_trailing_dot() {
        assert_eq!(normalize_attr_path("my.pkg.".into()), ".my.pkg");
        assert_eq!(normalize_attr_path(".my.pkg.".into()), ".my.pkg");
        assert_eq!(normalize_attr_path(".my.pkg...".into()), ".my.pkg");
    }

    #[test]
    fn normalize_attr_path_preserves_catchall() {
        assert_eq!(normalize_attr_path(".".into()), ".");
        assert_eq!(normalize_attr_path("".into()), ".");
    }

    #[test]
    fn type_attribute_forwards_normalized_path() {
        let cfg = Config::new().type_attribute("my.pkg.", "#[derive(Foo)]");
        assert_eq!(
            cfg.codegen_config.type_attributes,
            vec![(".my.pkg".to_string(), "#[derive(Foo)]".to_string())]
        );
    }

    #[test]
    fn field_attribute_forwards_normalized_path() {
        let cfg = Config::new().field_attribute("pkg.Msg.f", "#[serde(skip)]");
        assert_eq!(
            cfg.codegen_config.field_attributes,
            vec![(".pkg.Msg.f".to_string(), "#[serde(skip)]".to_string())]
        );
    }

    #[test]
    fn message_attribute_forwards_normalized_path() {
        let cfg = Config::new().message_attribute(".", "#[serde(default)]");
        assert_eq!(
            cfg.codegen_config.message_attributes,
            vec![(".".to_string(), "#[serde(default)]".to_string())]
        );
    }

    #[test]
    fn enum_attribute_forwards_normalized_path() {
        let cfg = Config::new().enum_attribute("my.pkg.", "#[derive(strum::EnumIter)]");
        assert_eq!(
            cfg.codegen_config.enum_attributes,
            vec![(
                ".my.pkg".to_string(),
                "#[derive(strum::EnumIter)]".to_string(),
            )]
        );
        // Other attribute lists must remain untouched.
        assert!(cfg.codegen_config.type_attributes.is_empty());
        assert!(cfg.codegen_config.message_attributes.is_empty());
        assert!(cfg.codegen_config.field_attributes.is_empty());
    }

    #[test]
    fn oneof_attribute_forwards_normalized_path() {
        let cfg = Config::new().oneof_attribute("my.pkg.Msg.payload.", "#[derive(Hash)]");
        assert_eq!(
            cfg.codegen_config.oneof_attributes,
            vec![(
                ".my.pkg.Msg.payload".to_string(),
                "#[derive(Hash)]".to_string()
            )]
        );
        // Other attribute lists must remain untouched.
        assert!(cfg.codegen_config.type_attributes.is_empty());
        assert!(cfg.codegen_config.enum_attributes.is_empty());
        assert!(cfg.codegen_config.message_attributes.is_empty());
        assert!(cfg.codegen_config.field_attributes.is_empty());
    }

    #[test]
    fn attribute_calls_accumulate_in_insertion_order() {
        let cfg = Config::new()
            .type_attribute(".", "#[derive(A)]")
            .type_attribute(".pkg.M", "#[derive(B)]")
            .type_attribute(".", "#[derive(C)]");
        let paths: Vec<_> = cfg
            .codegen_config
            .type_attributes
            .iter()
            .map(|(_, a)| a.as_str())
            .collect();
        assert_eq!(paths, vec!["#[derive(A)]", "#[derive(B)]", "#[derive(C)]"]);
    }
}

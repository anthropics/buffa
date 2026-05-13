//! Shared code generation logic for buffa.
//!
//! This crate takes protobuf descriptors (`google.protobuf.FileDescriptorProto`,
//! decoded from binary `FileDescriptorSet` data) and emits Rust source code
//! that uses the `buffa` runtime.
//!
//! It is used by:
//! - `protoc-gen-buffa` (protoc plugin)
//! - `buffa-build` (build.rs integration)
//!
//! # Architecture
//!
//! The code generator is intentionally decoupled from how descriptors are
//! obtained. It receives fully-resolved `FileDescriptorProto`s and produces
//! Rust source strings. This means:
//!
//! - It doesn't parse `.proto` files.
//! - It doesn't invoke `protoc`.
//! - It doesn't do import resolution or name linking.
//!
//! All of that is handled upstream (by protoc, buf, or a future parser).

pub(crate) mod comments;
pub mod context;
pub(crate) mod defaults;
pub(crate) mod enumeration;
pub(crate) mod extension;
pub(crate) mod features;
#[doc(hidden)]
pub use buffa_descriptor::generated;
pub mod idents;
pub(crate) mod impl_message;
pub(crate) mod impl_text;
pub(crate) mod imports;
pub(crate) mod message;
pub(crate) mod oneof;
pub(crate) mod view;

use crate::generated::descriptor::FileDescriptorProto;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Lints suppressed on generated code at module boundaries.
///
/// Consumed by [`generate_module_tree`], the per-package `.mod.rs`
/// stitcher, and `buffa-build`'s `_include.rs` writer. One list keeps
/// them in sync.
pub const ALLOW_LINTS: &[&str] = &[
    "non_camel_case_types",
    "dead_code",
    "unused_imports",
    // Cross-proto refs within the same package are emitted through the
    // canonical `super::super::__buffa::view::…` path even though the
    // target lives in the same generated module — using the bare name
    // would resolve, but the canonical path is stable when a sibling
    // proto defines a same-named natural-path re-export.
    "unused_qualifications",
    "clippy::derivable_impls",
    "clippy::match_single_binding",
    "clippy::uninlined_format_args",
    "clippy::doc_lazy_continuation",
    // A user `message View { message Inner }` produces
    // `__buffa::view::view::InnerView`; harmless but trips this lint.
    "clippy::module_inception",
];

/// Render [`ALLOW_LINTS`] as a `#[allow(…)]` attribute token stream.
pub fn allow_lints_attr() -> TokenStream {
    let lints: Vec<TokenStream> = ALLOW_LINTS
        .iter()
        .map(|l| syn::parse_str(l).expect("lint name parses as path"))
        .collect();
    quote! { #[allow( #(#lints),* )] }
}

/// One generated output file.
///
/// Each `.proto` produces up to five **content files** (`<stem>.rs`,
/// `<stem>.__view.rs`, `<stem>.__oneof.rs`, `<stem>.__view_oneof.rs`,
/// `<stem>.__ext.rs`) and each proto package produces one
/// `<dotted.pkg>.mod.rs` **stitcher** that `include!`s the content files
/// and authors the `pub mod __buffa { … }` ancillary tree.
/// Ancillary kinds with no content for that input file (e.g. a message
/// with no oneofs and no extensions) are omitted, and the stitcher's
/// `include!` set is filtered to match. The `__buffa` wrapper (and each
/// `view` / `oneof` / `ext` submodule inside it) is itself omitted when
/// it would be empty, so packages with only owned messages emit no
/// `__buffa` block at all.
/// See `DESIGN.md` → "Generated code layout".
///
/// Consumers normally only need to wire up the
/// [`GeneratedFileKind::PackageMod`] entries (one per package); the
/// per-proto content kinds are reached transitively via `include!` from
/// the stitcher. Write all files to disk; build a module tree from only
/// the `PackageMod` ones.
///
/// With [`CodeGenConfig::file_per_package`] set, the per-proto content
/// kinds are not emitted at all — the single `<dotted.pkg>.rs` (still
/// kind `PackageMod`) inlines what the stitcher would `include!`.
#[derive(Debug)]
pub struct GeneratedFile {
    /// The output file path (e.g., `"my.pkg.foo.rs"` or `"my.pkg.mod.rs"`).
    pub name: String,
    /// The proto package this file belongs to.
    pub package: String,
    /// What this file contains. Build integrations only need to wire up
    /// [`GeneratedFileKind::PackageMod`] files; everything else is reached
    /// via `include!` from there.
    pub kind: GeneratedFileKind,
    /// The generated Rust source code.
    pub content: String,
}

/// Kind of [`GeneratedFile`].
///
/// [`generate`] produces up to five per-proto content kinds — one each
/// of [`Owned`](Self::Owned), [`View`](Self::View), [`Oneof`](Self::Oneof),
/// [`ViewOneof`](Self::ViewOneof), and [`Ext`](Self::Ext) per input
/// `.proto` file — plus one [`PackageMod`](Self::PackageMod) stitcher per
/// package. Kinds with no content for the input (a proto with no oneofs
/// emits no [`Oneof`](Self::Oneof) / [`ViewOneof`](Self::ViewOneof);
/// no extensions, no [`Ext`](Self::Ext); etc.) are omitted. Build
/// integrations only need to wire up `PackageMod` entries; the per-proto
/// content kinds are reached via `include!` from the stitcher and need
/// only be written to disk alongside it. Under
/// [`CodeGenConfig::file_per_package`] only `PackageMod` is emitted.
///
/// [`Companion`](Self::Companion) is the one kind *not* produced by
/// [`generate`]: downstream code generators construct `Companion` files
/// themselves and merge them into buffa's output via
/// [`apply_companions`].
///
/// This enum is `#[non_exhaustive]` — match with a wildcard arm so new
/// kinds can be added without a major version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneratedFileKind {
    /// Owned message structs and enums (`<stem>.rs`).
    Owned,
    /// View structs (`<stem>.__view.rs`).
    View,
    /// Owned oneof enums (`<stem>.__oneof.rs`).
    Oneof,
    /// View oneof enums (`<stem>.__view_oneof.rs`).
    ViewOneof,
    /// File-level proto-extension consts (`<stem>.__ext.rs`) — the
    /// `pub const` `ExtensionDescriptor` items generated from `extend`
    /// blocks. Not to be confused with [`Companion`](Self::Companion),
    /// which is unrelated downstream-supplied content.
    Ext,
    /// Per-package stitcher (`<dotted.pkg>.mod.rs`). The only file build
    /// systems need to wire up directly.
    PackageMod,
    /// Extra per-proto content from a downstream code generator (service
    /// stubs, extra trait impls, etc.) that travels with buffa's output.
    ///
    /// Not produced by [`generate`]. Construct these in your own generator
    /// and pass them to [`apply_companions`], which appends an `include!`
    /// for each one at file scope in the matching package's
    /// [`PackageMod`](Self::PackageMod) — after buffa's own output, at
    /// package root alongside the owned message types (**not** under the
    /// `__buffa::` sentinel module). Items declared `pub` in a companion
    /// file are visible at `crate::<pkg>::*`.
    ///
    /// Not to be confused with [`Ext`](Self::Ext), which is the buffa-
    /// generated file holding protobuf `extend` consts.
    Companion,
}

/// Configuration for code generation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CodeGenConfig {
    /// Whether to generate borrowed view types (`MyMessageView<'a>`) in
    /// addition to owned types.
    pub generate_views: bool,
    /// Whether to preserve unknown fields (default: true).
    pub preserve_unknown_fields: bool,
    /// Whether to derive `serde::Serialize` / `serde::Deserialize` on
    /// generated message structs and enum types, and emit `#[serde(with = "...")]`
    /// attributes for proto3 JSON's special scalar encodings (int64 as quoted
    /// string, bytes as base64, etc.).
    ///
    /// When this is `true`, the downstream crate must depend on `serde` and
    /// must enable the `buffa/json` feature for the runtime helpers.
    ///
    /// Oneof fields use `#[serde(flatten)]` with custom `Serialize` /
    /// `Deserialize` impls so that each variant appears as a top-level
    /// JSON field (proto3 JSON inline oneof encoding).
    pub generate_json: bool,
    /// Whether to emit `#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]`
    /// on generated message structs and enum types.
    ///
    /// When this is `true`, the downstream crate must add `arbitrary` as an
    /// optional dependency and enable the `buffa/arbitrary` feature. The
    /// downstream crate's Cargo feature that gates `arbitrary` must be named
    /// exactly `"arbitrary"` — the generated `cfg_attr` uses that literal
    /// string and cannot be customized. This applies to both the struct-level
    /// `derive(Arbitrary)` and the per-field `#[arbitrary(with = ...)]`
    /// attributes emitted for `bytes_fields`-typed fields.
    ///
    /// For `bytes_fields`-typed fields, codegen emits `#[arbitrary(with = ...)]`
    /// using helpers in `::buffa::__private` since `bytes::Bytes` has no
    /// `Arbitrary` impl. Singular, optional, and repeated bytes fields are all
    /// covered. Map values are always `Vec<u8>` regardless of `bytes_fields`
    /// and require no special handling.
    pub generate_arbitrary: bool,
    /// External type path mappings.
    ///
    /// Each entry maps a fully-qualified protobuf path prefix (e.g.,
    /// `".my.common"`) to a Rust module path (e.g., `"::common_protos"`).
    /// Types under the proto prefix will reference the extern Rust path
    /// instead of being generated, allowing shared proto packages to be
    /// compiled once in a dedicated crate and referenced from others.
    ///
    /// Well-known types (`google.protobuf.*`) are automatically mapped to
    /// `::buffa_types::google::protobuf::*` without needing an explicit
    /// entry here. To override with a custom implementation, add an
    /// `extern_path` for `.google.protobuf` pointing to your crate.
    pub extern_paths: Vec<(String, String)>,
    /// Fully-qualified proto field paths whose `bytes` fields should use
    /// `bytes::Bytes` instead of `Vec<u8>`.
    ///
    /// Each entry is a proto path prefix (e.g., `".my.pkg.MyMessage.data"` for
    /// a specific field, or `"."` for all bytes fields). The path is matched
    /// as a prefix, so `"."` applies to every bytes field in every message.
    pub bytes_fields: Vec<String>,
    /// Honor `features.utf8_validation = NONE` by emitting `Vec<u8>` / `&[u8]`
    /// for such string fields instead of `String` / `&str`.
    ///
    /// When `false` (the default), buffa emits `String` for all string fields
    /// and **validates UTF-8 on decode** — stricter than proto2 requires, but
    /// ergonomic and safe.
    ///
    /// When `true`, string fields with `utf8_validation = NONE` (all proto2
    /// strings by default, and editions fields that opt into `NONE`) become
    /// `Vec<u8>` / `&[u8]`. Decode skips validation; the caller decides at the
    /// call site whether to `std::str::from_utf8` (checked) or
    /// `from_utf8_unchecked` (trusted-input fast path). This is the only
    /// sound Rust mapping when strings may actually contain non-UTF-8 bytes.
    ///
    /// **This is a breaking change for proto2** — enable only for new code or
    /// when profiling identifies UTF-8 validation as a bottleneck.
    pub strict_utf8_mapping: bool,
    /// Permit `option message_set_wire_format = true` on input messages.
    ///
    /// MessageSet is a legacy Google-internal wire format that wraps each
    /// extension in a group structure instead of using regular field tags.
    /// When `false` (the default), encountering such a message is a codegen
    /// error — the flag exists to make MessageSet use explicit, since the
    /// format is obsolete outside of interop with very old Google protos.
    pub allow_message_set: bool,
    /// Whether to emit `impl buffa::text::TextFormat` on generated message
    /// structs for textproto (human-readable text format) encoding/decoding.
    ///
    /// When this is `true`, the downstream crate must enable the `buffa/text`
    /// feature for the runtime encoder/decoder.
    pub generate_text: bool,
    /// Whether the per-package `.mod.rs` stitcher emits
    /// `__buffa::register_types(&mut TypeRegistry)`.
    ///
    /// Default `true`. The fn aggregates `Any` type entries and extension
    /// entries for every message in the package. Set to `false` for
    /// crates that don't use extensions/`Any`, or that hand-roll
    /// registration (e.g. `buffa-types`' `register_wkt_types`, which
    /// knows the JSON-Any `is_wkt` special-casing the generic fn does
    /// not). The per-message `__*_JSON_ANY` / `__*_TEXT_ANY` consts are
    /// still emitted; only the aggregating fn is suppressed.
    pub emit_register_fn: bool,
    /// Emit one `<dotted.package>.rs` per proto package instead of the
    /// per-proto-file content set plus `<pkg>.mod.rs` stitcher.
    ///
    /// The single file inlines what the stitcher would otherwise `include!`,
    /// producing the same `__buffa::{view,oneof,ext,...}` module structure.
    /// Intended for Buf Schema Registry generated SDKs, whose `lib.rs`
    /// synthesis builds the module tree from `<dotted.package>.rs` filenames.
    ///
    /// Under `strategy: directory` this only sees one directory's files per
    /// invocation, so the input module must be `PACKAGE_DIRECTORY_MATCH`-clean
    /// (one package per directory) for the output to be complete. BSR-hosted
    /// modules satisfy this by lint default. If a package spans multiple
    /// directories, separate invocations each emit their own `<pkg>.rs` and
    /// the last write wins — silent partial output, not a codegen error.
    pub file_per_package: bool,
    /// Custom attributes to inject on generated types (messages and enums).
    ///
    /// Each entry is `(proto_path, attribute)`. The `proto_path` is matched
    /// as a prefix against the fully-qualified proto name: `"."` applies to
    /// all types, `".my.pkg"` to types in that package, `".my.pkg.MyMessage"`
    /// to a specific type. The `attribute` is a raw Rust attribute string
    /// (e.g., `"#[derive(serde::Serialize)]"`).
    pub type_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated struct fields.
    ///
    /// Each entry is `(proto_path, attribute)`. The `proto_path` is matched
    /// as a prefix against the fully-qualified field path (e.g.,
    /// `".my.pkg.MyMessage.my_field"`). `"."` applies to all fields.
    pub field_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated message structs only (not enums).
    ///
    /// Same path-matching semantics as `type_attributes`, but only applied to
    /// message structs, not enum types. Useful for struct-only attributes like
    /// `#[serde(default)]`.
    pub message_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated enum types only (not messages).
    ///
    /// Same path-matching semantics as `type_attributes`, but only applied to
    /// enum types. Useful for enum-only attributes like
    /// `#[derive(strum::EnumIter)]` when the user does not want to apply the
    /// same attribute to every message in the matched scope.
    pub enum_attributes: Vec<(String, String)>,
}

impl Default for CodeGenConfig {
    fn default() -> Self {
        Self {
            generate_views: true,
            preserve_unknown_fields: true,
            generate_json: false,
            generate_arbitrary: false,
            extern_paths: Vec::new(),
            bytes_fields: Vec::new(),
            strict_utf8_mapping: false,
            allow_message_set: false,
            generate_text: false,
            emit_register_fn: true,
            file_per_package: false,
            type_attributes: Vec::new(),
            field_attributes: Vec::new(),
            message_attributes: Vec::new(),
            enum_attributes: Vec::new(),
        }
    }
}

/// Compute the effective extern path list by starting with user-provided
/// mappings and adding the default WKT mapping if appropriate.
///
/// The default mapping `".google.protobuf" → "::buffa_types::google::protobuf"`
/// is added unless:
/// - The user already provided an extern_path covering `.google.protobuf`
/// - Any of the files being generated are in the `google.protobuf` package
///   (i.e., we're building `buffa-types` itself)
pub(crate) fn effective_extern_paths(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Vec<(String, String)> {
    let mut paths = config.extern_paths.clone();

    // Only an EXACT .google.protobuf mapping suppresses auto-injection.
    // A sub-package mapping like .google.protobuf.compiler does NOT cover
    // WKTs like Timestamp — resolve_extern_prefix's longest-prefix matching
    // lets both coexist, so we still inject the parent mapping.
    let has_wkt_mapping = paths.iter().any(|(proto, _)| proto == ".google.protobuf");

    if !has_wkt_mapping {
        // Check if we're generating google.protobuf files ourselves
        // (e.g., building buffa-types). If so, don't auto-map.
        let generating_wkts = file_descriptors
            .iter()
            .filter(|fd| {
                fd.name
                    .as_deref()
                    .is_some_and(|n| files_to_generate.iter().any(|f| f == n))
            })
            .any(|fd| fd.package.as_deref() == Some("google.protobuf"));

        if !generating_wkts {
            paths.push((
                ".google.protobuf".to_string(),
                "::buffa_types::google::protobuf".to_string(),
            ));
        }
    }

    paths
}

/// Compute the effective file-level extern path list.
///
/// File-level mappings route a specific `.proto` file to a Rust module root,
/// taking priority over the package-level mappings from
/// [`effective_extern_paths`]. They exist to resolve a structural problem:
/// `descriptor.proto` is in the same `google.protobuf` package as the
/// JSON-mappable WKTs (`Timestamp`, `Any`, …), but its types live in
/// `buffa-descriptor`, not `buffa-types`. A single package-keyed
/// `.google.protobuf` extern_path can route the package to one crate or the
/// other; it can't split it. The file-level mapping splits it.
///
/// Auto-injected mappings (when not suppressed):
///
/// | Proto file | Rust module |
/// |---|---|
/// | `google/protobuf/descriptor.proto` | `::buffa_descriptor::generated::descriptor` |
/// | `google/protobuf/compiler/plugin.proto` | `::buffa_descriptor::generated::compiler` |
///
/// Suppression conditions, evaluated **per file**:
///
/// - **A user-provided `extern_path` covers the file's package.** That
///   override has covered the file's types since the package mapping was
///   introduced; auto-injecting a higher-priority file-level mapping would
///   silently redirect them away from the user's crate. Matching is via
///   the same longest-prefix logic the package resolver uses, so both an
///   exact `.google.protobuf` mapping and a sub-package
///   `.google.protobuf.compiler` mapping suppress the entries they cover —
///   `.google.protobuf` suppresses both, `.google.protobuf.compiler`
///   suppresses only `plugin.proto`.
/// - **The proto file itself is in `files_to_generate`.** When building
///   `buffa-descriptor` (or any local copy of `descriptor.proto`), its types
///   must resolve to the local module, not externally.
///
/// Currently internal-only — there is no `CodeGenConfig` field for
/// user-provided file-level mappings. The user-facing `extern_path` API
/// remains package-prefix keyed; per-file or per-type overrides may be added
/// later as a public feature if a concrete need arises.
pub(crate) fn effective_file_extern_paths(
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Vec<(String, String)> {
    // (proto file path, proto package, Rust module root). The package is
    // recorded alongside the file so the user-override suppression check
    // is per-file: a `.google.protobuf.compiler` extern_path covers only
    // `plugin.proto`, while `.google.protobuf` covers both.
    const DESCRIPTOR_FILES: [(&str, &str, &str); 2] = [
        (
            "google/protobuf/descriptor.proto",
            "google.protobuf",
            "::buffa_descriptor::generated::descriptor",
        ),
        (
            "google/protobuf/compiler/plugin.proto",
            "google.protobuf.compiler",
            "::buffa_descriptor::generated::compiler",
        ),
    ];

    DESCRIPTOR_FILES
        .into_iter()
        .filter(|(proto_file, package, _)| {
            // Yield to a user package-level extern_path that already covers
            // this file's package: anyone who wrote
            // `extern_path(".google.protobuf", "::my_crate")` (or a
            // sub-package mapping) today routes these types to their crate;
            // the auto-injected file-level mapping must not silently
            // outrank it.
            if context::resolve_extern_prefix(package, &config.extern_paths).is_some() {
                return false;
            }
            // Don't externalize a file we're generating locally.
            !files_to_generate.iter().any(|f| f == proto_file)
        })
        .map(|(proto_file, _, rust_module)| (proto_file.to_string(), rust_module.to_string()))
        .collect()
}

/// Generate Rust source files from a set of file descriptors.
///
/// `files_to_generate` is the set of file names that were explicitly requested
/// (matching `CodeGeneratorRequest.file_to_generate`). Descriptors for
/// dependencies may be present in `file_descriptors` but won't produce output
/// files unless they appear in `files_to_generate`.
///
/// Each `.proto` emits up to five content files (kinds with no content
/// are omitted); each distinct package emits one `<pkg>.mod.rs`
/// stitcher. Packages are processed in sorted order for deterministic
/// output.
pub fn generate(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Result<Vec<GeneratedFile>, CodeGenError> {
    let ctx = context::CodeGenContext::for_generate(file_descriptors, files_to_generate, config);

    // Group requested files by package. BTreeMap → deterministic output order.
    let mut by_package: std::collections::BTreeMap<String, Vec<&FileDescriptorProto>> =
        std::collections::BTreeMap::new();
    for file_name in files_to_generate {
        let file_desc = file_descriptors
            .iter()
            .find(|f| f.name.as_deref() == Some(file_name.as_str()))
            .ok_or_else(|| CodeGenError::FileNotFound(file_name.clone()))?;
        let pkg = file_desc.package.as_deref().unwrap_or("").to_string();
        by_package.entry(pkg).or_default().push(file_desc);
    }

    let mut output = Vec::new();
    for (package, files) in by_package {
        generate_package(&ctx, &package, &files, &mut output)?;
    }

    Ok(output)
}

/// Generate a module tree that assembles per-package `.mod.rs` files into
/// nested `pub mod` blocks matching the protobuf package hierarchy.
///
/// Each entry is a `(mod_file_name, package)` pair where `package` is the
/// dot-separated protobuf package name (e.g., `"google.api"`) and
/// `mod_file_name` is the corresponding `<pkg>.mod.rs` (only
/// [`GeneratedFileKind::PackageMod`] outputs need wiring; per-proto
/// content files are reached via `include!` from the stitcher).
///
/// `include_mode` controls how `include!` paths are emitted.
///
/// `emit_inner_allow` adds a `#![allow(...)]` inner attribute at the top —
/// valid when the output is used directly as a module file (`mod.rs`),
/// invalid when consumed via `include!`.
pub fn generate_module_tree<F: AsRef<str>, P: AsRef<str>>(
    entries: &[(F, P)],
    include_mode: IncludeMode<'_>,
    emit_inner_allow: bool,
) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;

    use crate::idents::escape_mod_ident;

    #[derive(Default)]
    struct ModNode {
        files: Vec<String>,
        children: BTreeMap<String, Self>,
    }

    let mut root = ModNode::default();

    for (file_name, package) in entries {
        let package = package.as_ref();
        let pkg_parts: Vec<&str> = if package.is_empty() {
            vec![]
        } else {
            package.split('.').collect()
        };

        let mut node = &mut root;
        for seg in &pkg_parts {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node.files.push(file_name.as_ref().to_string());
    }

    let lints = ALLOW_LINTS.join(", ");
    let mut out = String::new();
    let _ = writeln!(out, "// @generated by buffa-codegen. DO NOT EDIT.");
    if emit_inner_allow {
        let _ = writeln!(out, "#![allow({lints})]");
    }
    let _ = writeln!(out);

    fn emit(out: &mut String, node: &ModNode, depth: usize, mode: IncludeMode<'_>, lints: &str) {
        let indent = "    ".repeat(depth);

        for file in &node.files {
            match mode {
                IncludeMode::Relative(prefix) => {
                    let _ = writeln!(out, r#"{indent}include!("{prefix}{file}");"#);
                }
                IncludeMode::OutDir => {
                    let _ = writeln!(
                        out,
                        r#"{indent}include!(concat!(env!("OUT_DIR"), "/{file}"));"#
                    );
                }
            }
        }

        for (name, child) in &node.children {
            let escaped = escape_mod_ident(name);
            let _ = writeln!(out, "{indent}#[allow({lints})]");
            let _ = writeln!(out, "{indent}pub mod {escaped} {{");
            let _ = writeln!(out, "{indent}    use super::*;");
            emit(out, child, depth + 1, mode, lints);
            let _ = writeln!(out, "{indent}}}");
        }
    }

    emit(&mut out, &root, 0, include_mode, &lints);
    out
}

/// How [`generate_module_tree`] emits `include!` paths.
#[derive(Debug, Clone, Copy)]
pub enum IncludeMode<'a> {
    /// `include!("<prefix><file>")` — relative to the including file.
    /// Prefix is typically `""` or `"gen/"`.
    Relative(&'a str),
    /// `include!(concat!(env!("OUT_DIR"), "/<file>"))` — for build.rs output.
    OutDir,
}

/// Validate one input descriptor before generating code for it.
///
/// Checks, in one walk of the message tree:
///
/// - **Reserved field names**: no field starts with `__buffa_` (would clash
///   with generated `__buffa_unknown_fields` / `__buffa_cached_size`).
/// - **Module-name conflicts**: no two sibling messages snake_case to the
///   same module name (e.g. `HTTPRequest` vs `HttpRequest`).
/// - **Reserved sentinel**: no package segment, message-module name, or
///   file-level enum name equals [`SENTINEL_MOD`](context::SENTINEL_MOD).
///   Ancillary types live under `pkg::__buffa::…`; a proto element
///   emitting an item named `__buffa` at package root would produce
///   E0428 against `pub mod __buffa`. This is the only name buffa
///   reserves in user namespace.
fn validate_file(file: &FileDescriptorProto) -> Result<(), CodeGenError> {
    use std::collections::HashMap;

    let sentinel = context::SENTINEL_MOD;
    let package = file.package.as_deref().unwrap_or("");
    if package.split('.').any(|seg| seg == sentinel) {
        return Err(CodeGenError::ReservedModuleName {
            name: sentinel.to_string(),
            location: format!("package '{package}'"),
        });
    }
    // File-level enums emit `pub enum <name>` at package root with the
    // proto name preserved verbatim (no PascalCase normalization), so a
    // proto `enum __buffa` would land beside `pub mod __buffa`. Nested
    // enums live inside their owner message's module and cannot collide
    // with the package-root sentinel, so only file-level is checked.
    for enum_type in &file.enum_type {
        let name = enum_type.name.as_deref().unwrap_or("");
        if name == sentinel {
            return Err(CodeGenError::ReservedModuleName {
                name: sentinel.to_string(),
                location: format!("enum '{package}.{name}'"),
            });
        }
    }

    fn walk(
        messages: &[crate::generated::descriptor::DescriptorProto],
        scope: &str,
        sentinel: &str,
    ) -> Result<(), CodeGenError> {
        // snake_case module name → original proto name (for conflict diag).
        let mut seen: HashMap<String, &str> = HashMap::new();

        for msg in messages {
            let name = msg.name.as_deref().unwrap_or("");
            let fqn = if scope.is_empty() {
                name.to_string()
            } else {
                format!("{scope}.{name}")
            };

            for field in &msg.field {
                if let Some(fname) = &field.name {
                    if fname.starts_with("__buffa_") {
                        return Err(CodeGenError::ReservedFieldName {
                            message_name: fqn,
                            field_name: fname.clone(),
                        });
                    }
                }
            }

            let module_name = crate::oneof::to_snake_case(name);
            if module_name == sentinel {
                return Err(CodeGenError::ReservedModuleName {
                    name: sentinel.to_string(),
                    location: format!("message '{fqn}'"),
                });
            }
            if let Some(existing) = seen.get(&module_name) {
                return Err(CodeGenError::ModuleNameConflict {
                    scope: scope.to_string(),
                    name_a: existing.to_string(),
                    name_b: name.to_string(),
                    module_name,
                });
            }
            seen.insert(module_name, name);

            walk(&msg.nested_type, &fqn, sentinel)?;
        }
        Ok(())
    }

    walk(&file.message_type, package, sentinel)
}

/// Per-proto content streams plus the file stem, ready to be formatted.
struct ProtoContent {
    stem: String,
    owned: TokenStream,
    view: TokenStream,
    oneof: TokenStream,
    view_oneof: TokenStream,
    ext: TokenStream,
    /// Candidate `pub use` re-exports targeting the package root (top-level
    /// view structs, file-level extension consts). Filtered against the
    /// package-wide root namespace in [`generate_package_mod`] — the package
    /// can span multiple `.proto` files, so collisions are only knowable at
    /// the stitcher level.
    root_reexports: Vec<message::ReexportCandidate>,
}

/// Generate the per-`.proto` content token streams for one input file.
/// Each ancillary kind that has no content yields an empty stream and
/// is dropped at the file-emission stage.
fn generate_proto_content(
    ctx: &context::CodeGenContext,
    current_package: &str,
    file: &FileDescriptorProto,
    reg: &mut message::RegistryPaths,
) -> Result<ProtoContent, CodeGenError> {
    use crate::idents::make_field_ident;
    use crate::message::MessageOutput;

    validate_file(file)?;

    let resolver = imports::ImportResolver::new();
    let features = crate::features::for_file(file);

    let mut owned = TokenStream::new();
    let mut view = TokenStream::new();
    let mut oneof = TokenStream::new();
    let mut view_oneof = TokenStream::new();
    let mut ext = TokenStream::new();
    let mut root_reexports: Vec<message::ReexportCandidate> = Vec::new();
    let sentinel = make_field_ident(context::SENTINEL_MOD);

    for enum_type in &file.enum_type {
        let enum_rust_name = enum_type.name.as_deref().unwrap_or("");
        let enum_fqn = if current_package.is_empty() {
            enum_rust_name.to_string()
        } else {
            format!("{}.{}", current_package, enum_rust_name)
        };
        owned.extend(enumeration::generate_enum(
            ctx,
            enum_type,
            enum_rust_name,
            &enum_fqn,
            &features,
            &resolver,
        )?);
    }

    for message_type in &file.message_type {
        let top_level_name = message_type.name.as_deref().unwrap_or("");
        let proto_fqn = if current_package.is_empty() {
            top_level_name.to_string()
        } else {
            format!("{}.{}", current_package, top_level_name)
        };
        let MessageOutput {
            owned_top,
            owned_mod,
            oneof_tree: msg_oneof,
            view_tree: msg_view,
            view_oneof_tree: msg_view_oneof,
            reg: msg_reg,
        } = message::generate_message(
            ctx,
            message_type,
            current_package,
            top_level_name,
            &proto_fqn,
            &features,
            &resolver,
        )?;
        owned.extend(owned_top);
        let mod_ident = make_field_ident(&crate::oneof::to_snake_case(top_level_name));
        for p in msg_reg.json_ext {
            reg.json_ext.push(quote! { #mod_ident :: #p });
        }
        for p in msg_reg.text_ext {
            reg.text_ext.push(quote! { #mod_ident :: #p });
        }
        reg.json_any.extend(msg_reg.json_any);
        reg.text_any.extend(msg_reg.text_any);

        if !owned_mod.is_empty() {
            owned.extend(quote! {
                pub mod #mod_ident {
                    #[allow(unused_imports)]
                    use super::*;
                    #owned_mod
                }
            });
        }
        oneof.extend(msg_oneof);
        view.extend(msg_view);
        view_oneof.extend(msg_view_oneof);

        // Top-level message view → re-export at package root. The leading
        // `self::` is load-bearing: when consumers nest packages with
        // `pub mod a { use super::*; pub mod a_b { use super::*; … } }`
        // (`buffa-build`'s `_include.rs` does this), a parent package's
        // `__buffa` is in scope via the glob, and Rust's import-resolution
        // pass treats a glob-imported name as ambiguous against a
        // **macro-expanded** local one (the `pub mod __buffa` block arrives
        // via `include!()`), even though a non-macro local definition would
        // shadow the glob — see rustc E0659. `self::` resolves it
        // deterministically. `#[doc(inline)]` makes rustdoc render the type's
        // full page at the natural path instead of a "Re-export of …" stub.
        if ctx.config.generate_views {
            let view_ident = format_ident!("{top_level_name}View");
            root_reexports.push(message::ReexportCandidate {
                name: view_ident.to_string(),
                tokens: quote! {
                    #[doc(inline)]
                    pub use self :: #sentinel :: view :: #view_ident;
                },
            });
        }
    }

    // File-level `extend` declarations → `__buffa::ext::` (depth 2).
    let (file_ext_tokens, file_ext_json, file_ext_text) = extension::generate_extensions(
        ctx,
        &file.extension,
        current_package,
        2,
        &features,
        current_package,
    )?;
    ext.extend(file_ext_tokens);
    for id in file_ext_json {
        reg.json_ext.push(quote! { #sentinel :: ext :: #id });
    }
    for id in file_ext_text {
        reg.text_ext.push(quote! { #sentinel :: ext :: #id });
    }
    // File-level extension consts → re-export at package root. `self::` and
    // `#[doc(inline)]` for the same reasons as the view re-exports above.
    for ext_field in &file.extension {
        let const_ident = extension::extension_const_ident(ext_field.name.as_deref().unwrap_or(""));
        root_reexports.push(message::ReexportCandidate {
            name: const_ident.to_string(),
            tokens: quote! {
                #[doc(inline)]
                pub use self :: #sentinel :: ext :: #const_ident;
            },
        });
    }

    Ok(ProtoContent {
        stem: proto_path_to_stem(file.name.as_deref().unwrap_or("")),
        owned,
        view,
        oneof,
        view_oneof,
        ext,
        root_reexports,
    })
}

/// Per-section token streams for one package, ready for the stitcher.
///
/// In per-file mode each section holds `include!("<stem>...rs")` calls; in
/// `file_per_package` mode each holds the actual generated items.
#[derive(Default)]
struct PackageSections {
    owned: Vec<TokenStream>,
    view: Vec<TokenStream>,
    oneof: Vec<TokenStream>,
    view_oneof: Vec<TokenStream>,
    ext: Vec<TokenStream>,
}

impl PackageSections {
    /// Append one proto file's generated items in-line.
    ///
    /// Empty streams are skipped so each section's emptiness reflects
    /// "the package has no content of this kind" — symmetric with the
    /// per-file branch that filters at file-emission time.
    fn push_inline(&mut self, pc: ProtoContent) {
        let push_if_nonempty = |dst: &mut Vec<TokenStream>, ts: TokenStream| {
            if !ts.is_empty() {
                dst.push(ts);
            }
        };
        push_if_nonempty(&mut self.owned, pc.owned);
        push_if_nonempty(&mut self.view, pc.view);
        push_if_nonempty(&mut self.oneof, pc.oneof);
        push_if_nonempty(&mut self.view_oneof, pc.view_oneof);
        push_if_nonempty(&mut self.ext, pc.ext);
    }
}

/// Generate all output files for one proto package: up to five content
/// files per `.proto` (empty ancillary kinds are skipped) plus one
/// `<pkg>.mod.rs` stitcher, or a single `<pkg>.rs` when
/// [`CodeGenConfig::file_per_package`] is set.
fn generate_package(
    ctx: &context::CodeGenContext,
    current_package: &str,
    files: &[&FileDescriptorProto],
    out: &mut Vec<GeneratedFile>,
) -> Result<(), CodeGenError> {
    // Registry paths are package-root-relative; `register_types` lives at
    // `__buffa::register_types` (one level deep), so each path gets a
    // single `super::` prefix when emitted into the fn body.
    let mut reg = message::RegistryPaths::default();
    let mut root_reexports: Vec<message::ReexportCandidate> = Vec::new();

    let sections = if ctx.config.file_per_package {
        let mut sections = PackageSections::default();
        for file in files {
            let mut pc = generate_proto_content(ctx, current_package, file, &mut reg)?;
            root_reexports.append(&mut pc.root_reexports);
            sections.push_inline(pc);
        }
        sections
    } else {
        let mut sections = PackageSections::default();
        for file in files {
            let mut pc = generate_proto_content(ctx, current_package, file, &mut reg)?;
            root_reexports.append(&mut pc.root_reexports);
            let source = file.name.as_deref().unwrap_or("");
            let stem = pc.stem;

            // Empty ancillary token streams are skipped — neither the
            // content file nor the stitcher's `include!` is emitted.
            let emit = |suffix: &str,
                        kind: GeneratedFileKind,
                        tokens: TokenStream,
                        section: &mut Vec<TokenStream>,
                        out: &mut Vec<GeneratedFile>|
             -> Result<(), CodeGenError> {
                if tokens.is_empty() {
                    return Ok(());
                }
                let name = format!("{stem}{suffix}.rs");
                section.push(quote! { include!(#name); });
                out.push(GeneratedFile {
                    name,
                    package: current_package.to_string(),
                    kind,
                    content: format_tokens(tokens, source)?,
                });
                Ok(())
            };
            emit(
                "",
                GeneratedFileKind::Owned,
                pc.owned,
                &mut sections.owned,
                out,
            )?;
            emit(
                ".__view",
                GeneratedFileKind::View,
                pc.view,
                &mut sections.view,
                out,
            )?;
            emit(
                ".__oneof",
                GeneratedFileKind::Oneof,
                pc.oneof,
                &mut sections.oneof,
                out,
            )?;
            emit(
                ".__view_oneof",
                GeneratedFileKind::ViewOneof,
                pc.view_oneof,
                &mut sections.view_oneof,
                out,
            )?;
            emit(
                ".__ext",
                GeneratedFileKind::Ext,
                pc.ext,
                &mut sections.ext,
                out,
            )?;
        }
        sections
    };

    let reexport_block = surviving_root_reexports(ctx, files, &reg, root_reexports);

    out.push(GeneratedFile {
        name: if ctx.config.file_per_package {
            package_to_filename(current_package)
        } else {
            package_to_mod_filename(current_package)
        },
        package: current_package.to_string(),
        kind: GeneratedFileKind::PackageMod,
        content: generate_package_mod(ctx, &sections, &reg, &reexport_block)?,
    });

    Ok(())
}

/// Filter the candidate package-root re-exports against the package's
/// existing root namespace and against each other, returning the surviving
/// `pub use` lines.
///
/// The package root is shared across every `.proto` file in the package, so
/// the occupied-name set must be built from *all* of them — a top-level
/// message named `FooView` declared in `a.proto` would shadow `Foo`'s view
/// re-export from `b.proto`.
fn surviving_root_reexports(
    ctx: &context::CodeGenContext,
    files: &[&FileDescriptorProto],
    reg: &message::RegistryPaths,
    mut candidates: Vec<message::ReexportCandidate>,
) -> TokenStream {
    use crate::idents::make_field_ident;
    use std::collections::BTreeSet;

    // Names already occupied at package root by real items: top-level
    // messages, enums, message snake_case modules, and the `__buffa`
    // sentinel itself. File-level extension consts live in
    // `__buffa::ext::`, not at the root, so they are *candidates* (added
    // by `generate_proto_content`) rather than occupants.
    let mut occupied: BTreeSet<String> = BTreeSet::new();
    occupied.insert(context::SENTINEL_MOD.to_string());
    for file in files {
        for m in &file.message_type {
            let name = m.name.as_deref().unwrap_or("");
            occupied.insert(name.to_string());
            occupied.insert(crate::oneof::to_snake_case(name));
        }
        for e in &file.enum_type {
            occupied.insert(e.name.as_deref().unwrap_or("").to_string());
        }
    }

    // `register_types`, when emitted, lives at `__buffa::register_types`.
    // `self::` and `#[doc(inline)]` for the same reasons as the view
    // re-exports above.
    if ctx.config.emit_register_fn && !reg.is_empty() {
        let sentinel = make_field_ident(context::SENTINEL_MOD);
        candidates.push(message::ReexportCandidate {
            name: "register_types".to_string(),
            tokens: quote! {
                #[doc(inline)]
                pub use self :: #sentinel :: register_types;
            },
        });
    }

    message::emit_surviving_reexports(candidates, &occupied)
}

/// Render the per-package stitcher: owned items at root plus the
/// `__buffa::{view,oneof,ext,...}` module wrappers, followed by the
/// surviving package-root `pub use` re-exports.
fn generate_package_mod(
    ctx: &context::CodeGenContext,
    sections: &PackageSections,
    reg: &message::RegistryPaths,
    root_reexports: &TokenStream,
) -> Result<String, CodeGenError> {
    use crate::idents::make_field_ident;

    let owned = &sections.owned;
    let view = &sections.view;
    let view_oneof = &sections.view_oneof;
    let oneof = &sections.oneof;
    let ext = &sections.ext;

    // Each ancillary module is emitted only when its section has
    // content. The natural-path re-exports outside `__buffa` target
    // these modules — they are emitted only when their target items
    // exist, so the conditions align and re-exports never reference
    // a missing module.
    let view_oneof_mod = if !view_oneof.is_empty() {
        quote! {
            pub mod oneof {
                #[allow(unused_imports)]
                use super::*;
                #(#view_oneof)*
            }
        }
    } else {
        TokenStream::new()
    };

    // `view_oneof` is only populated for messages that have oneofs, and
    // every message also contributes to `view`, so `!view.is_empty()` is
    // sufficient — `view_oneof` non-empty implies `view` non-empty.
    debug_assert!(view_oneof.is_empty() || !view.is_empty());
    let view_mod = if ctx.config.generate_views && !view.is_empty() {
        quote! {
            pub mod view {
                #[allow(unused_imports)]
                use super::*;
                #(#view)*
                #view_oneof_mod
            }
        }
    } else {
        TokenStream::new()
    };

    let oneof_mod = if !oneof.is_empty() {
        quote! {
            pub mod oneof {
                #[allow(unused_imports)]
                use super::*;
                #(#oneof)*
            }
        }
    } else {
        TokenStream::new()
    };

    let ext_mod = if !ext.is_empty() {
        quote! {
            pub mod ext {
                #[allow(unused_imports)]
                use super::*;
                #(#ext)*
            }
        }
    } else {
        TokenStream::new()
    };

    let register_fn = if ctx.config.emit_register_fn && !reg.is_empty() {
        let json_any = &reg.json_any;
        let json_ext = &reg.json_ext;
        let text_any = &reg.text_any;
        let text_ext = &reg.text_ext;
        quote! {
            /// Register this package's `Any` type entries and extension entries.
            pub fn register_types(reg: &mut ::buffa::type_registry::TypeRegistry) {
                #( reg.register_json_any(super::#json_any); )*
                #( reg.register_json_ext(super::#json_ext); )*
                #( reg.register_text_any(super::#text_any); )*
                #( reg.register_text_ext(super::#text_ext); )*
            }
        }
    } else {
        TokenStream::new()
    };

    let sentinel = make_field_ident(context::SENTINEL_MOD);
    // The whole `pub mod __buffa { ... }` wrapper is itself omitted
    // when none of its inner modules or `register_types` exist.
    let buffa_mod = if view_mod.is_empty()
        && oneof_mod.is_empty()
        && ext_mod.is_empty()
        && register_fn.is_empty()
    {
        TokenStream::new()
    } else {
        let allow = allow_lints_attr();
        quote! {
            #allow
            pub mod #sentinel {
                #[allow(unused_imports)]
                use super::*;
                #view_mod
                #oneof_mod
                #ext_mod
                #register_fn
            }
        }
    };

    let tokens = quote! {
        #(#owned)*
        #buffa_mod
        #root_reexports
    };

    format_tokens(tokens, "")
}

/// Format a token stream into a generated-file string with the standard
/// header comment.
fn format_tokens(tokens: TokenStream, source: &str) -> Result<String, CodeGenError> {
    let syntax_tree =
        syn::parse2::<syn::File>(tokens).map_err(|e| CodeGenError::InvalidSyntax(e.to_string()))?;
    let formatted = prettyplease::unparse(&syntax_tree);
    let source_line = if source.is_empty() {
        String::new()
    } else {
        format!("// source: {source}\n")
    };
    Ok(format!(
        "// @generated by buffa-codegen. DO NOT EDIT.\n{source_line}\n{formatted}"
    ))
}

/// Convert a proto package name to its `.mod.rs` stitcher filename.
///
/// e.g., `"google.protobuf"` → `"google.protobuf.mod.rs"`. The unnamed
/// package uses the [`SENTINEL_MOD`](context::SENTINEL_MOD) name as its
/// filename stem — `package __buffa;` is already rejected by
/// `validate_file`, so the unnamed-package stitcher cannot
/// collide with any real package's.
pub fn package_to_mod_filename(package: &str) -> String {
    if package.is_empty() {
        format!("{}.mod.rs", context::SENTINEL_MOD)
    } else {
        format!("{package}.mod.rs")
    }
}

/// Convert a proto package name to its [`file_per_package`] output filename.
///
/// e.g., `"google.protobuf"` → `"google.protobuf.rs"`. The unnamed
/// package uses [`SENTINEL_MOD`](context::SENTINEL_MOD) — same
/// collision-avoidance as [`package_to_mod_filename`].
///
/// [`file_per_package`]: CodeGenConfig::file_per_package
pub fn package_to_filename(package: &str) -> String {
    if package.is_empty() {
        format!("{}.rs", context::SENTINEL_MOD)
    } else {
        format!("{package}.rs")
    }
}

/// Convert a `.proto` file path to its content-file stem.
///
/// e.g., `"google/protobuf/timestamp.proto"` → `"google.protobuf.timestamp"`.
/// Content files append `""`, `".__view"`, `".__oneof"`,
/// `".__view_oneof"`, or `".__ext"` plus `".rs"` — emitted only for
/// kinds with non-empty content.
pub fn proto_path_to_stem(proto_path: &str) -> String {
    let without_ext = proto_path.strip_suffix(".proto").unwrap_or(proto_path);
    without_ext.replace('/', ".")
}

/// Merge downstream [`Companion`](GeneratedFileKind::Companion) files into
/// the per-package stitcher produced by [`generate`].
///
/// For each companion file this function locates the
/// [`PackageMod`](GeneratedFileKind::PackageMod) entry in `files` with a
/// matching package and appends `include!("<name>");` at file scope after
/// buffa's own output — at package root, alongside the owned message types,
/// not under `__buffa::`. The companion files themselves are appended to
/// `files` so that build integrations can write everything to disk in one
/// pass.
///
/// **Call this once per build**; it does not deduplicate, so a second call
/// with the same companions emits a second `include!` for each, which fails
/// to compile downstream with a duplicate-definition error.
///
/// `name` must be a bare-sibling filename — the same convention buffa uses
/// for its own `include!` calls, so it resolves relative to the stitcher
/// without any `OUT_DIR` prefix. Names must not contain `"`, `\`, `/`, or
/// newlines (the function `debug_assert!`s this in debug builds), and must
/// not collide with any of buffa's own generated filenames for the same
/// package (`<stem>.rs`, `<stem>.__view.rs`, etc.) — pick an unused suffix
/// such as `<stem>.__myplugin.rs`.
///
/// Companion files with no matching `PackageMod` (e.g. for a package buffa
/// did not generate any output for) are still appended to `files` but no
/// `include!` is emitted; the caller is responsible for wiring them up. If
/// you don't expect orphans, check that every companion's `package` appears
/// in `files` as a `PackageMod` after calling.
pub fn apply_companions(files: &mut Vec<GeneratedFile>, companions: Vec<GeneratedFile>) {
    for comp in &companions {
        debug_assert!(
            !comp.name.contains(['"', '\\', '/', '\n']),
            "companion file name {:?} contains a character that would break \
             the generated include!() literal or its bare-sibling resolution",
            comp.name
        );
        if let Some(pkg_mod) = files
            .iter_mut()
            .find(|f| f.kind == GeneratedFileKind::PackageMod && f.package == comp.package)
        {
            pkg_mod
                .content
                .push_str(&format!("include!(\"{}\");\n", comp.name));
        }
    }
    files.extend(companions);
}

/// Code generation error.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CodeGenError {
    /// A required field was absent in a descriptor.
    ///
    /// The `&'static str` names the missing field for diagnostics.
    #[error("missing required descriptor field: {0}")]
    MissingField(&'static str),
    /// A resolved type path string could not be parsed as a Rust type.
    #[error("invalid Rust type path: '{0}'")]
    InvalidTypePath(String),
    /// The accumulated `TokenStream` failed to parse as valid Rust syntax.
    #[error("generated code failed to parse as Rust: {0}")]
    InvalidSyntax(String),
    /// A requested file was not present in the descriptor set.
    #[error("file_to_generate '{0}' not found in descriptor set")]
    FileNotFound(String),
    /// Unexpected descriptor state (e.g. a map entry or oneof that cannot be
    /// resolved to a known descriptor field).
    #[error("codegen error: {0}")]
    Other(String),
    /// A proto field name uses the `__buffa_` reserved prefix, which would
    /// conflict with buffa's internal generated fields.
    #[error(
        "reserved field name '{field_name}' in message '{message_name}': \
             proto field names starting with '__buffa_' conflict with buffa's \
             internal fields"
    )]
    ReservedFieldName {
        message_name: String,
        field_name: String,
    },
    /// Two sibling messages produce the same Rust module name after
    /// snake_case conversion (e.g., `HTTPRequest` and `HttpRequest` both
    /// become `pub mod http_request`).
    #[error(
        "module name conflict in '{scope}': messages '{name_a}' and '{name_b}' \
         both produce module '{module_name}'"
    )]
    ModuleNameConflict {
        scope: String,
        name_a: String,
        name_b: String,
        module_name: String,
    },
    /// A proto package segment, message name, or file-level enum name
    /// would emit a Rust item matching the reserved sentinel `__buffa`.
    ///
    /// This is the only name buffa reserves in user namespace. Resolve by
    /// renaming the proto element.
    #[error(
        "reserved name '{name}' at {location}: this name is reserved for \
         buffa's generated ancillary types (views, oneof enums, \
         extensions). Rename the proto element."
    )]
    ReservedModuleName { name: String, location: String },
    /// The input contains a message with `option message_set_wire_format = true`
    /// but [`CodeGenConfig::allow_message_set`] was not set.
    #[error(
        "message '{message_name}' uses `option message_set_wire_format = true` \
         but CodeGenConfig::allow_message_set is false; MessageSet is a legacy \
         wire format — set allow_message_set(true) if this is intentional"
    )]
    MessageSetNotSupported { message_name: String },
    /// A custom attribute string configured via [`CodeGenConfig::type_attributes`],
    /// [`CodeGenConfig::field_attributes`], or [`CodeGenConfig::message_attributes`]
    /// could not be parsed as a Rust attribute.
    #[error(
        "invalid custom attribute for path '{path}': '{attribute}' is not a valid \
         Rust attribute ({detail})"
    )]
    InvalidCustomAttribute {
        path: String,
        attribute: String,
        detail: String,
    },
}

#[cfg(test)]
mod tests;

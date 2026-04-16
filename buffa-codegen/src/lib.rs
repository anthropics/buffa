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
use quote::quote;

/// Result of generating Rust code for a single `.proto` file.
#[derive(Debug)]
pub struct GeneratedFile {
    /// The output file path (e.g., "my_package.rs").
    pub name: String,
    /// The generated Rust source code.
    pub content: String,
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
    /// optional dependency and enable the `buffa/arbitrary` feature.
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
    /// Whether to emit the file-level `register_types(&mut TypeRegistry)` fn.
    ///
    /// Default `true`. Set to `false` when multiple generated files are
    /// `include!`d into the same namespace (the identically-named fns would
    /// collide) — e.g. `buffa-types`' WKTs, which hand-roll
    /// `register_wkt_types` instead. The per-message `__*_JSON_ANY` /
    /// `__*_TEXT_ANY` consts are still emitted; only the aggregating fn
    /// is suppressed.
    pub emit_register_fn: bool,
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

/// Generate Rust source files from a set of file descriptors.
///
/// `files_to_generate` is the set of file names that were explicitly requested
/// (matching `CodeGeneratorRequest.file_to_generate`). Descriptors for
/// dependencies may be present in `file_descriptors` but won't produce output
/// files unless they appear in `files_to_generate`.
pub fn generate(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Result<Vec<GeneratedFile>, CodeGenError> {
    let ctx = context::CodeGenContext::for_generate(file_descriptors, files_to_generate, config);

    let mut output = Vec::new();
    for file_name in files_to_generate {
        let file_desc = file_descriptors
            .iter()
            .find(|f| f.name.as_deref() == Some(file_name.as_str()))
            .ok_or_else(|| CodeGenError::FileNotFound(file_name.clone()))?;

        let content = generate_file(&ctx, file_desc)?;
        let rust_filename = proto_path_to_rust_module(file_name);
        output.push(GeneratedFile {
            name: rust_filename,
            content,
        });
    }

    Ok(output)
}

/// Generate a module tree that assembles generated `.rs` files into
/// nested `pub mod` blocks matching the protobuf package hierarchy.
///
/// Each entry is a `(file_name, package)` pair where `package` is the
/// dot-separated protobuf package name (e.g., `"google.api"`). The module
/// tree is built from the **package** hierarchy so that `super::`-based
/// cross-package references resolve correctly.
///
/// `include_prefix` is prepended to file names in `include!` directives.
/// Use `""` for relative paths or `concat!(env!("OUT_DIR"), "/")` style
/// for build.rs output.
///
/// When `emit_inner_allow` is true, a `#![allow(...)]` inner attribute is
/// emitted at the top of the file. This is appropriate when the output is
/// used directly as a module file (e.g., `mod.rs`) but NOT when the output
/// is consumed via `include!` (inner attributes are not valid in that
/// context).
pub fn generate_module_tree(
    entries: &[(&str, &str)],
    include_prefix: &str,
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
        let pkg_parts: Vec<&str> = if package.is_empty() {
            vec![]
        } else {
            package.split('.').collect()
        };

        let mut node = &mut root;
        for seg in &pkg_parts {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node.files.push(file_name.to_string());
    }

    let mut out = String::new();
    writeln!(out, "// @generated by buffa. DO NOT EDIT.").unwrap();
    const ALLOW_LINTS: &str = "non_camel_case_types, dead_code, unused_imports, \
        clippy::derivable_impls, clippy::match_single_binding, \
        clippy::uninlined_format_args, clippy::doc_lazy_continuation";

    if emit_inner_allow {
        writeln!(out, "#![allow({ALLOW_LINTS})]").unwrap();
    }
    writeln!(out).unwrap();

    fn emit(out: &mut String, node: &ModNode, depth: usize, prefix: &str, lints: &str) {
        let indent = "    ".repeat(depth);

        for file in &node.files {
            writeln!(out, r#"{indent}include!("{prefix}{file}");"#).unwrap();
        }

        for (name, child) in &node.children {
            let escaped = escape_mod_ident(name);
            writeln!(out, "{indent}#[allow({lints})]").unwrap();
            writeln!(out, "{indent}pub mod {escaped} {{").unwrap();
            writeln!(out, "{indent}    use super::*;").unwrap();
            emit(out, child, depth + 1, prefix, lints);
            writeln!(out, "{indent}}}").unwrap();
        }
    }

    emit(&mut out, &root, 0, include_prefix, ALLOW_LINTS);
    out
}

/// Check that no fields in the file use the `__buffa_` reserved prefix.
fn check_reserved_field_names(file: &FileDescriptorProto) -> Result<(), CodeGenError> {
    fn check_message(
        msg: &crate::generated::descriptor::DescriptorProto,
        parent_name: &str,
    ) -> Result<(), CodeGenError> {
        let msg_name = msg.name.as_deref().unwrap_or("");
        let fqn = if parent_name.is_empty() {
            msg_name.to_string()
        } else {
            format!("{}.{}", parent_name, msg_name)
        };

        for field in &msg.field {
            if let Some(name) = &field.name {
                if name.starts_with("__buffa_") {
                    return Err(CodeGenError::ReservedFieldName {
                        message_name: fqn,
                        field_name: name.clone(),
                    });
                }
            }
        }

        for nested in &msg.nested_type {
            check_message(nested, &fqn)?;
        }

        Ok(())
    }

    let package = file.package.as_deref().unwrap_or("");
    for msg in &file.message_type {
        check_message(msg, package)?;
    }
    Ok(())
}

/// Check that no sibling messages produce the same snake_case module name.
///
/// For example, `HTTPRequest` and `HttpRequest` both produce
/// `pub mod http_request`, which would be a compile error.
fn check_module_name_conflicts(file: &FileDescriptorProto) -> Result<(), CodeGenError> {
    use std::collections::HashMap;

    fn check_siblings(
        messages: &[crate::generated::descriptor::DescriptorProto],
        scope: &str,
    ) -> Result<(), CodeGenError> {
        // Map from snake_case module name → original proto name.
        let mut seen: HashMap<String, &str> = HashMap::new();

        for msg in messages {
            let name = msg.name.as_deref().unwrap_or("");
            let module_name = crate::oneof::to_snake_case(name);

            if let Some(existing) = seen.get(&module_name) {
                return Err(CodeGenError::ModuleNameConflict {
                    scope: scope.to_string(),
                    name_a: existing.to_string(),
                    name_b: name.to_string(),
                    module_name,
                });
            }
            seen.insert(module_name, name);

            // Recurse into nested messages.
            let child_scope = if scope.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", scope, name)
            };
            check_siblings(&msg.nested_type, &child_scope)?;
        }

        Ok(())
    }

    let package = file.package.as_deref().unwrap_or("");
    check_siblings(&file.message_type, package)
}

/// Check that no message named `FooView` collides with the generated view
/// type for a sibling message `Foo`.
fn check_view_name_conflicts(file: &FileDescriptorProto) -> Result<(), CodeGenError> {
    use std::collections::HashSet;

    fn check_siblings(
        messages: &[crate::generated::descriptor::DescriptorProto],
        scope: &str,
    ) -> Result<(), CodeGenError> {
        // Collect all message names at this level.
        let names: HashSet<&str> = messages.iter().filter_map(|m| m.name.as_deref()).collect();

        // For each message Foo, check if FooView also exists.
        for msg in messages {
            let name = msg.name.as_deref().unwrap_or("");
            let view_name = format!("{}View", name);
            if names.contains(view_name.as_str()) {
                return Err(CodeGenError::ViewNameConflict {
                    scope: scope.to_string(),
                    owned_msg: name.to_string(),
                    view_msg: view_name,
                });
            }
        }

        // Recurse into nested messages.
        for msg in messages {
            let name = msg.name.as_deref().unwrap_or("");
            let child_scope = if scope.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", scope, name)
            };
            check_siblings(&msg.nested_type, &child_scope)?;
        }

        Ok(())
    }

    let package = file.package.as_deref().unwrap_or("");
    check_siblings(&file.message_type, package)
}

/// Generate Rust source for a single `.proto` file.
fn generate_file(
    ctx: &context::CodeGenContext,
    file: &FileDescriptorProto,
) -> Result<String, CodeGenError> {
    // Validate descriptors before generating code.
    check_reserved_field_names(file)?;
    check_module_name_conflicts(file)?;
    if ctx.config.generate_views {
        check_view_name_conflicts(file)?;
    }

    let resolver = imports::ImportResolver::for_file(file);
    let mut tokens = resolver.generate_use_block();
    let current_package = file.package.as_deref().unwrap_or("");
    let features = crate::features::for_file(file);
    for enum_type in &file.enum_type {
        let enum_rust_name = enum_type.name.as_deref().unwrap_or("");
        let enum_fqn = if current_package.is_empty() {
            enum_rust_name.to_string()
        } else {
            format!("{}.{}", current_package, enum_rust_name)
        };
        tokens.extend(enumeration::generate_enum(
            ctx,
            enum_type,
            enum_rust_name,
            &enum_fqn,
            &features,
            &resolver,
        )?);
    }
    // Collect paths to registry consts (both file-level and nested-in-message)
    // for the optional `register_types` fn below. JSON/text are tracked
    // separately so each registration line is emitted only under its
    // corresponding `generate_*` flag.
    let mut reg = message::RegistryPaths::default();

    for message_type in &file.message_type {
        let top_level_name = message_type.name.as_deref().unwrap_or("");
        let proto_fqn = if current_package.is_empty() {
            top_level_name.to_string()
        } else {
            format!("{}.{}", current_package, top_level_name)
        };
        let (msg_top, msg_mod, msg_reg) = message::generate_message(
            ctx,
            message_type,
            current_package,
            top_level_name,
            &proto_fqn,
            &features,
            &resolver,
        )?;
        tokens.extend(msg_top);
        // Nested extension const paths are relative to the message's module
        // scope; prefix with `<mod_ident>::` for the package-level view.
        let mod_name = crate::oneof::to_snake_case(top_level_name);
        let mod_ident = crate::message::make_field_ident(&mod_name);
        for p in msg_reg.json_ext {
            reg.json_ext.push(quote! { #mod_ident :: #p });
        }
        for p in msg_reg.text_ext {
            reg.text_ext.push(quote! { #mod_ident :: #p });
        }
        // Any-entry paths are already relative to the struct's scope
        // (= file scope for top-level messages) — no prefix needed.
        reg.json_any.extend(msg_reg.json_any);
        reg.text_any.extend(msg_reg.text_any);

        let view_mod = if ctx.config.generate_views {
            let (view_top, view_mod) = view::generate_view(
                ctx,
                message_type,
                current_package,
                top_level_name,
                &proto_fqn,
                &features,
            )?;
            tokens.extend(view_top);
            view_mod
        } else {
            TokenStream::new()
        };

        // Combine message and view module items into a single `pub mod`.
        if !msg_mod.is_empty() || !view_mod.is_empty() {
            tokens.extend(quote! {
                pub mod #mod_ident {
                    #[allow(unused_imports)]
                    use super::*;
                    #msg_mod
                    #view_mod
                }
            });
        }
    }

    let (file_ext_tokens, file_ext_json, file_ext_text) = extension::generate_extensions(
        ctx,
        &file.extension,
        current_package,
        0,
        &features,
        current_package,
    )?;
    tokens.extend(file_ext_tokens);
    for id in file_ext_json {
        reg.json_ext.push(quote! { #id });
    }
    for id in file_ext_text {
        reg.text_ext.push(quote! { #id });
    }

    // `register_types(&mut TypeRegistry)` — one call per entry, split by
    // format. Only emitted when at least one entry exists. Lines are
    // gated at codegen time by `generate_json` / `generate_text`; the
    // corresponding `register_*` methods on `TypeRegistry` are feature-gated
    // in buffa, so a flag/feature mismatch surfaces as a compile error.
    if ctx.config.emit_register_fn && !reg.is_empty() {
        let json_any = &reg.json_any;
        let json_ext = &reg.json_ext;
        let text_any = &reg.text_any;
        let text_ext = &reg.text_ext;
        tokens.extend(quote! {
            /// Register this file's `Any` type entries and extension entries
            /// (JSON and/or text, per codegen config) with the given registry.
            pub fn register_types(reg: &mut ::buffa::type_registry::TypeRegistry) {
                #( reg.register_json_any(#json_any); )*
                #( reg.register_json_ext(#json_ext); )*
                #( reg.register_text_any(#text_any); )*
                #( reg.register_text_ext(#text_ext); )*
            }
        });
    }

    // Parse the token stream into a syn::File and format with prettyplease.
    // Regular `//` comments cannot appear inside quote! blocks, so the file
    // header is prepended as a raw string after formatting.
    let syntax_tree =
        syn::parse2::<syn::File>(tokens).map_err(|e| CodeGenError::InvalidSyntax(e.to_string()))?;
    let formatted = prettyplease::unparse(&syntax_tree);

    let source_line = file
        .name
        .as_ref()
        .map_or(String::new(), |n| format!("// source: {n}\n"));

    Ok(format!(
        "// @generated by protoc-gen-buffa. DO NOT EDIT.\n{source_line}\n{formatted}"
    ))
}

/// Convert a `.proto` file path to a Rust module file name.
///
/// e.g., "google/protobuf/timestamp.proto" → "google.protobuf.timestamp.rs"
/// Convert a proto file path to a generated Rust file name.
///
/// e.g., `"google/protobuf/timestamp.proto"` → `"google.protobuf.timestamp.rs"`
pub fn proto_path_to_rust_module(proto_path: &str) -> String {
    let without_ext = proto_path.strip_suffix(".proto").unwrap_or(proto_path);
    format!("{}.rs", without_ext.replace('/', "."))
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
    /// The `{Name}Oneof` identifier buffa would emit for a oneof collides
    /// with another name in the parent message's Rust module (a nested
    /// message, a nested enum, or — when view generation is enabled — a
    /// `{Name}OneofView`-equivalent sibling). Resolve by renaming the
    /// oneof or the colliding nested type in the `.proto`.
    #[error(
        "name conflict in '{scope}': oneof '{oneof_name}' would emit as \
         '{attempted}', but that name already names another item in the \
         enclosing scope"
    )]
    OneofNameConflict {
        scope: String,
        oneof_name: String,
        attempted: String,
    },
    /// A message named `FooView` collides with the generated view type for
    /// message `Foo`.
    #[error(
        "name conflict in '{scope}': message '{view_msg}' collides with \
         the generated view type for message '{owned_msg}'"
    )]
    ViewNameConflict {
        scope: String,
        owned_msg: String,
        view_msg: String,
    },
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

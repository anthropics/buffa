//! protoc-gen-buffa-packaging — emits a `mod.rs` module tree for buffa's
//! per-package `<pkg>.mod.rs` stitcher output.
//!
//! This plugin reads the proto package structure (not message/service
//! bodies) and writes a `mod.rs` that `include!`s each per-package
//! stitcher (see [`buffa_codegen::package_to_mod_filename`]) at the right
//! module nesting. The per-proto content files are reached transitively
//! via `include!` from the stitchers, so this plugin only wires up one
//! file per package. Requires `strategy: all` so the plugin sees the full
//! file set in a single invocation.
//!
//! # buf.gen.yaml
//!
//! ```yaml
//! plugins:
//!   - local: protoc-gen-buffa
//!     out: src/generated
//!   - local: protoc-gen-buffa-packaging
//!     out: src/generated
//!     strategy: all
//! ```
//!
//! ```rust,ignore
//! #[path = "generated/mod.rs"]
//! pub mod proto;
//! ```
//!
//! # Options
//!
//! - `filter=services` — only include packages where at least one
//!   `.proto` declares a `service`. Useful when packaging output from a
//!   service-stub generator that skips files without services.
//! - `exclude_package=<pkg>` — drop a proto package (and its subpackages)
//!   from the module tree. Repeatable; the leading dot is optional. Must
//!   match the `exclude_package` passed to `protoc-gen-buffa` so the
//!   `mod.rs` never `include!`s a stitcher the codegen plugin skipped.
//! - `shared_descriptor_pool=true` — emit the shared `__buffa_fds` descriptor
//!   module at the root of `mod.rs` (dedup mode). Same option name as on
//!   `protoc-gen-buffa`, and the two **must match**: the codegen plugin emits
//!   the per-package delegations, this plugin emits the root module they point
//!   at. The descriptor set is embedded inline (a byte-string literal):
//!   protoc's plugin protocol carries only UTF-8 text, so a binary
//!   `include_bytes!` sidecar isn't possible here — inline still collapses the
//!   per-package duplication to one copy. (`buffa-build`, which writes files
//!   directly, uses a sidecar instead.) Feature gating
//!   (`gate_reflect_on_crate_feature` / `gate_impls_on_crate_features`) is not
//!   supported on this path (protoc-gen-buffa rejects the combination).
//!
//! Invoke the plugin once per output tree — use multiple entries in
//! buf.gen.yaml with different `out:` directories and filters to package
//! several trees from one `buf generate` run.
//!
//! # Matching a codegen plugin's output set
//!
//! This plugin cannot see the filesystem — it derives the set of packages
//! to `include!` from `file_to_generate` and the chosen filter. The
//! filter must produce the same set the codegen plugin actually emitted,
//! or the `mod.rs` will reference nonexistent stitchers (or miss real
//! ones).
//!
//! `protoc-gen-buffa` emits a stitcher for every package unconditionally,
//! so no filter is needed. A service-stub generator that skips packages
//! without a `service` declaration needs `filter=services`. If a codegen
//! plugin's skip condition is not expressible as a predicate on
//! `FileDescriptorProto`, it is not packageable by this plugin.

use std::io::{self, Read, Write};

use buffa::Message;
use buffa_codegen::generated::compiler::code_generator_response::File as CodeGeneratorResponseFile;
use buffa_codegen::generated::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};
use buffa_codegen::generated::descriptor::{Edition, FileDescriptorProto};

/// File-inclusion filter. Extend with new variants as downstream packaging
/// needs emerge (e.g., `has_ext:<name>` for extension-gated output).
#[derive(Debug, Default)]
enum Filter {
    /// Include every file in `file_to_generate`.
    #[default]
    All,
    /// Include only files whose descriptor declares at least one `service`.
    Services,
}

impl Filter {
    fn include(&self, fd: &FileDescriptorProto) -> bool {
        match self {
            Self::All => true,
            Self::Services => !fd.service.is_empty(),
        }
    }
}

/// Package selection: an inclusion [`Filter`] plus a set of package
/// exclusions applied on top of it. A package is stitched into `mod.rs`
/// only when the filter includes at least one of its files *and* the package
/// is not excluded. Exclusions route through
/// [`buffa_codegen::package_is_excluded`] — the same predicate
/// `protoc-gen-buffa` uses to drop files from generation — so both plugins
/// skip exactly the same packages.
#[derive(Debug, Default)]
struct Selection {
    filter: Filter,
    exclude: Vec<String>,
    shared_pool: bool,
}

impl Selection {
    fn include(&self, fd: &FileDescriptorProto) -> bool {
        self.filter.include(fd)
            && !buffa_codegen::package_is_excluded(
                fd.package.as_deref().unwrap_or(""),
                &self.exclude,
            )
    }
}

const HELP: &str = "\
protoc-gen-buffa-packaging — emits a mod.rs module tree for buffa output.

This binary speaks the protoc plugin protocol: it reads a serialized
CodeGeneratorRequest from stdin and writes a CodeGeneratorResponse to
stdout. It is not intended to be invoked directly. Use it via buf or
protoc alongside protoc-gen-buffa:

  # buf.gen.yaml
  plugins:
    - local: protoc-gen-buffa
      out: src/gen
    - local: protoc-gen-buffa-packaging
      out: src/gen
      strategy: all

Options (default: include every package in file_to_generate):
  filter=services       only include packages declaring at least one service
  exclude_package=<pkg> drop a package (and its subpackages) from the tree;
                        repeatable, leading dot optional. Must match the
                        exclude_package passed to protoc-gen-buffa.
  shared_descriptor_pool=true
                        emit the shared __buffa_fds descriptor module at the
                        tree root. Must match the shared_descriptor_pool=true
                        passed to protoc-gen-buffa.";

fn main() {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("{HELP}");
                return;
            }
            other => {
                eprintln!(
                    "{}: unrecognized argument {other:?}. This is a protoc \
                     plugin; run with --help for usage.",
                    env!("CARGO_PKG_NAME")
                );
                std::process::exit(2);
            }
        }
    }
    match run() {
        Ok(()) => {}
        Err(e) => {
            let response = CodeGeneratorResponse {
                error: Some(e),
                supported_features: Some(feature_flags()),
                ..Default::default()
            };
            write_response(&response).unwrap_or_else(|io_err| {
                eprintln!(
                    "protoc-gen-buffa-packaging: failed to write error response: {}",
                    io_err
                );
                std::process::exit(1);
            });
        }
    }
}

fn run() -> Result<(), String> {
    let mut input = Vec::new();
    io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| format!("failed to read stdin: {e}"))?;

    // protoc produced this, so the element bound is far above buffa's
    // untrusted-input default; see `tooling_decode_options` for the override.
    let request = buffa_codegen::decode_request(&input)?;

    let response = generate(&request)?;
    write_response(&response).map_err(|e| format!("failed to write stdout: {e}"))
}

fn generate(request: &CodeGeneratorRequest) -> Result<CodeGeneratorResponse, String> {
    let selection = parse_options(request.parameter.as_deref().unwrap_or(""))?;

    // Module tree wires up one `<pkg>.mod.rs` per package; collect the
    // distinct packages of the requested files (filtered).
    let mut packages = std::collections::BTreeSet::new();
    for proto_name in &request.file_to_generate {
        let fd = find_descriptor(&request.proto_file, proto_name).ok_or_else(|| {
            format!("file_to_generate entry {proto_name:?} has no FileDescriptorProto")
        })?;
        if selection.include(fd) {
            packages.insert(fd.package.as_deref().unwrap_or("").to_string());
        }
    }
    let entries: Vec<(String, String)> = packages
        .into_iter()
        .map(|p| (buffa_codegen::package_to_mod_filename(&p), p))
        .collect();
    let mut content = buffa_codegen::generate_module_tree(
        &entries,
        buffa_codegen::IncludeMode::Relative(""),
        true,
    );

    // Shared-pool mode: embed the descriptor set once, at the tree root, as an
    // inline byte-string module. protoc's plugin protocol carries only UTF-8
    // string content, so a binary `include_bytes!` sidecar isn't possible
    // here — that path is reserved for `buffa-build`, which writes files
    // directly. Inline still collapses the O(packages) duplication to one
    // copy, which is the dominant cost. The bytes cover the full transitive
    // closure (every `proto_file`), matching what the per-package embedding
    // would have carried, so cross-package reflection resolves.
    if selection.shared_pool {
        // No feature overrides here: this plugin never receives them (they are
        // protoc-gen-buffa options), so it can't reproduce them. protoc-gen-buffa
        // rejects `shared_descriptor_pool` + overrides, so a build that reaches
        // this point has none — the empty slice matches the codegen side.
        let fds_bytes = buffa_codegen::encode_descriptor_set(&request.proto_file, &[]);
        // Gate is `None`: the plugin protocol gives no access to
        // protoc-gen-buffa's feature-gate config, so the packaging path does
        // not support `gate_reflect_on_crate_feature` (protoc-gen-buffa rejects
        // that combination). The root module is emitted unconditionally.
        let root = buffa_codegen::shared_descriptor_root_module(
            &fds_bytes,
            buffa_codegen::FdsEmbedding::Inline,
            None,
        );
        // The module tree opens with `#![allow(...)]`, which must stay the
        // first item in the file. Splice the root module in just after it.
        content = splice_after_inner_attrs(&content, &root);
    }

    Ok(CodeGeneratorResponse {
        supported_features: Some(feature_flags()),
        minimum_edition: Some(Edition::EDITION_PROTO2 as i32),
        maximum_edition: Some(Edition::EDITION_2024 as i32),
        file: vec![CodeGeneratorResponseFile {
            name: Some("mod.rs".to_string()),
            content: Some(content),
            ..Default::default()
        }],
        ..Default::default()
    })
}

/// Insert `item` into `file` just after the leading header comments and
/// inner attributes (`#![...]`), which must remain the first tokens in a Rust
/// file. Everything from the first real item onward follows `item`.
fn splice_after_inner_attrs(file: &str, item: &str) -> String {
    // Assumes `\n` line endings (the only input is `generate_module_tree`
    // output, which uses `writeln!`). Each split offset lands on a line
    // boundary, so it is always a valid UTF-8 char boundary. The `+ 1` below
    // would drift under `\r\n`, so enforce the assumption rather than only
    // documenting it.
    debug_assert!(
        !file.contains('\r'),
        "splice_after_inner_attrs assumes LF line endings"
    );
    let mut prefix_end = 0;
    for line in file.lines() {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with("//") || t.starts_with("#!") {
            // +1 for the '\n' that `lines()` stripped.
            prefix_end += line.len() + 1;
        } else {
            break;
        }
    }
    let (head, rest) = file.split_at(prefix_end.min(file.len()));
    format!("{head}{item}\n{rest}")
}

fn parse_options(params: &str) -> Result<Selection, String> {
    let mut selection = Selection::default();
    for opt in params.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(value) = opt.strip_prefix("filter=") {
            selection.filter = match value.trim() {
                "services" => Filter::Services,
                other => {
                    return Err(format!("unknown filter {other:?}. Supported: services"));
                }
            };
        } else if let Some(value) = opt.strip_prefix("shared_descriptor_pool=") {
            selection.shared_pool = match value.trim() {
                "true" => true,
                "false" => false,
                other => {
                    return Err(format!(
                        "invalid shared_descriptor_pool value {other:?}, expected true or false"
                    ));
                }
            };
        } else if let Some(value) = opt.strip_prefix("exclude_package=") {
            // Shares protoc-gen-buffa's normalization (one helper in
            // buffa-codegen), so both plugins drop the same packages and the
            // mod.rs never references a skipped stitcher. The option key
            // itself must also stay spelled `exclude_package` in both
            // plugins — renaming or aliasing it in one without the other
            // recreates the mismatch the shared helper exists to prevent.
            selection
                .exclude
                .push(buffa_codegen::normalize_exclude_package(value)?);
        } else if opt
            .split_once('=')
            .is_some_and(|(k, _)| k.trim() == buffa_codegen::ELEMENT_MEMORY_LIMIT_OPT)
        {
            // Consumed before the request was decoded (it governs that
            // decode); see `buffa_codegen::peek_request_parameter`.
        } else {
            return Err(format!(
                "unknown plugin option {opt:?}. \
                 Supported: filter=services, exclude_package=<pkg>, \
                 element_memory_limit=<bytes>, shared_descriptor_pool=<bool>"
            ));
        }
    }
    Ok(selection)
}

fn find_descriptor<'a>(
    proto_file: &'a [FileDescriptorProto],
    name: &str,
) -> Option<&'a FileDescriptorProto> {
    proto_file
        .iter()
        .find(|fd| fd.name.as_deref() == Some(name))
}

fn write_response(response: &CodeGeneratorResponse) -> io::Result<()> {
    let mut output = Vec::new();
    response.encode(&mut output);
    io::stdout().write_all(&output)?;
    io::stdout().flush()
}

fn feature_flags() -> u64 {
    const FEATURE_PROTO3_OPTIONAL: u64 = 1;
    const FEATURE_SUPPORTS_EDITIONS: u64 = 2;
    FEATURE_PROTO3_OPTIONAL | FEATURE_SUPPORTS_EDITIONS
}

#[cfg(test)]
mod tests {
    use super::*;
    use buffa_codegen::generated::descriptor::ServiceDescriptorProto;

    fn file(name: &str, package: &str, has_service: bool) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(name.into()),
            package: Some(package.into()),
            service: if has_service {
                vec![ServiceDescriptorProto {
                    name: Some("Svc".into()),
                    ..Default::default()
                }]
            } else {
                vec![]
            },
            ..Default::default()
        }
    }

    fn request(param: Option<&str>, files: Vec<FileDescriptorProto>) -> CodeGeneratorRequest {
        CodeGeneratorRequest {
            parameter: param.map(|s| s.into()),
            file_to_generate: files.iter().map(|f| f.name.clone().unwrap()).collect(),
            proto_file: files,
            ..Default::default()
        }
    }

    #[test]
    fn no_filter_includes_all() {
        let req = request(
            None,
            vec![
                file("foo/v1/svc.proto", "foo.v1", true),
                file("bar/v1/types.proto", "bar.v1", false),
            ],
        );
        let resp = generate(&req).unwrap();
        assert_eq!(resp.file.len(), 1);
        let content = resp.file[0].content.as_deref().unwrap();
        // Module tree wires up one `<pkg>.mod.rs` per package.
        assert!(content.contains("foo.v1.mod.rs"));
        assert!(content.contains("bar.v1.mod.rs"));
    }

    #[test]
    fn services_filter_excludes_non_service_files() {
        // Filter is per-file; a package is included if any file in it
        // declares a service. `bar.v1` has no service files → excluded.
        let req = request(
            Some("filter=services"),
            vec![
                file("foo/v1/svc.proto", "foo.v1", true),
                file("bar/v1/types.proto", "bar.v1", false),
            ],
        );
        let resp = generate(&req).unwrap();
        let content = resp.file[0].content.as_deref().unwrap();
        assert!(content.contains("foo.v1.mod.rs"));
        assert!(!content.contains("bar.v1.mod.rs"));
    }

    #[test]
    fn shared_pool_option_emits_shared_root_module() {
        let req = request(
            Some("shared_descriptor_pool=true"),
            vec![
                file("foo/v1/svc.proto", "foo.v1", true),
                file("bar/v1/types.proto", "bar.v1", false),
            ],
        );
        let resp = generate(&req).unwrap();
        let content = resp.file[0].content.as_deref().unwrap();
        assert!(
            content.contains("pub mod __buffa_fds"),
            "shared_descriptor_pool=true must emit the shared root module: {content}"
        );
        assert!(content.contains("FILE_DESCRIPTOR_SET_BYTES"));
        // Still wires the package tree.
        assert!(content.contains("foo.v1.mod.rs"));
        // Placement is load-bearing: `#![allow(...)]` must stay the first item
        // in the file (inner attributes are rejected after any item), and the
        // root module must precede the package tree that delegates to it.
        let allow = content
            .find("#![allow")
            .expect("mod.rs must keep its inner allow");
        let root = content.find("pub mod __buffa_fds").unwrap();
        let tree = content.find("foo.v1.mod.rs").unwrap();
        assert!(
            allow < root && root < tree,
            "shared root module must be spliced after the inner attrs and \
             before the package tree: {content}"
        );
    }

    #[test]
    fn splice_after_inner_attrs_handles_edge_inputs() {
        // Typical module-tree prefix: comment, inner attr, blank line.
        assert_eq!(
            splice_after_inner_attrs("// header\n#![allow(x)]\n\npub mod a;\n", "ITEM"),
            "// header\n#![allow(x)]\n\nITEM\npub mod a;\n"
        );
        // Empty input: the item is the whole file.
        assert_eq!(splice_after_inner_attrs("", "ITEM"), "ITEM\n");
        // No inner attrs or comments: the item leads.
        assert_eq!(
            splice_after_inner_attrs("pub mod a;\n", "ITEM"),
            "ITEM\npub mod a;\n"
        );
        // Comment-only file: everything is prefix, item lands at the end.
        assert_eq!(
            splice_after_inner_attrs("// only\n", "ITEM"),
            "// only\nITEM\n"
        );
        // Missing trailing newline on the last prefix line: the `+ 1` for the
        // stripped '\n' overshoots and must be clamped to the file length.
        assert_eq!(
            splice_after_inner_attrs("#![allow(x)]", "ITEM"),
            "#![allow(x)]ITEM\n"
        );
    }

    #[test]
    fn no_shared_pool_omits_shared_root_module() {
        let req = request(None, vec![file("foo/v1/svc.proto", "foo.v1", true)]);
        let resp = generate(&req).unwrap();
        let content = resp.file[0].content.as_deref().unwrap();
        assert!(
            !content.contains("__buffa_fds"),
            "without shared_descriptor_pool the root module must be absent: {content}"
        );
    }

    #[test]
    fn unknown_filter_errors() {
        let err = parse_options("filter=bogus").unwrap_err();
        assert!(err.contains("bogus"));
    }

    #[test]
    fn unknown_option_errors() {
        let err = parse_options("bogus_option").unwrap_err();
        assert!(err.contains("bogus_option"));
    }

    #[test]
    fn empty_filter_value_errors() {
        // `filter=` with no value hits the unknown-filter arm with `""`.
        let err = parse_options("filter=").unwrap_err();
        assert!(err.contains("unknown filter"));
    }

    #[test]
    fn exclude_package_drops_package_and_subpackages() {
        let req = request(
            Some("exclude_package=.buf.validate,exclude_package=gnostic"),
            vec![
                file("example/user/v1/user.proto", "example.user.v1", false),
                file("buf/validate/validate.proto", "buf.validate", false),
                file(
                    "gnostic/openapi/v3/openapiv3.proto",
                    "gnostic.openapi.v3",
                    false,
                ),
            ],
        );
        let resp = generate(&req).unwrap();
        let content = resp.file[0].content.as_deref().unwrap();
        assert!(content.contains("example.user.v1.mod.rs"));
        assert!(!content.contains("buf.validate.mod.rs"));
        assert!(!content.contains("gnostic.openapi.v3.mod.rs"));
    }

    #[test]
    fn exclude_package_composes_with_services_filter() {
        // A service package that is also excluded is still dropped.
        let req = request(
            Some("filter=services,exclude_package=.secret"),
            vec![
                file("foo/v1/svc.proto", "foo.v1", true),
                file("secret/v1/svc.proto", "secret.v1", true),
            ],
        );
        let resp = generate(&req).unwrap();
        let content = resp.file[0].content.as_deref().unwrap();
        assert!(content.contains("foo.v1.mod.rs"));
        assert!(!content.contains("secret.v1.mod.rs"));
    }

    #[test]
    fn empty_exclude_package_errors() {
        let err = parse_options("exclude_package=").unwrap_err();
        assert!(err.contains("exclude_package"));
    }

    #[test]
    fn missing_descriptor_errors() {
        // file_to_generate entry with no matching FileDescriptorProto.
        let req = CodeGeneratorRequest {
            parameter: None,
            file_to_generate: vec!["orphan.proto".into()],
            proto_file: vec![],
            ..Default::default()
        };
        let err = generate(&req).unwrap_err();
        assert!(err.contains("orphan.proto"));
    }
}

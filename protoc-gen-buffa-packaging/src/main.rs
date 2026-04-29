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
  filter=services   only include packages declaring at least one service";

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

    let request = CodeGeneratorRequest::decode_from_slice(&input)
        .map_err(|e| format!("failed to decode CodeGeneratorRequest: {e}"))?;

    let response = generate(&request)?;
    write_response(&response).map_err(|e| format!("failed to write stdout: {e}"))
}

fn generate(request: &CodeGeneratorRequest) -> Result<CodeGeneratorResponse, String> {
    let filter = parse_filter(request.parameter.as_deref().unwrap_or(""))?;

    // Module tree wires up one `<pkg>.mod.rs` per package; collect the
    // distinct packages of the requested files (filtered).
    let mut packages = std::collections::BTreeSet::new();
    for proto_name in &request.file_to_generate {
        let fd = find_descriptor(&request.proto_file, proto_name).ok_or_else(|| {
            format!("file_to_generate entry {proto_name:?} has no FileDescriptorProto")
        })?;
        if filter.include(fd) {
            packages.insert(fd.package.as_deref().unwrap_or("").to_string());
        }
    }
    let entries: Vec<(String, String)> = packages
        .into_iter()
        .map(|p| (buffa_codegen::package_to_mod_filename(&p), p))
        .collect();
    let content = buffa_codegen::generate_module_tree(
        &entries,
        buffa_codegen::IncludeMode::Relative(""),
        true,
    );

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

fn parse_filter(params: &str) -> Result<Filter, String> {
    let mut filter = Filter::default();
    for opt in params.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(value) = opt.strip_prefix("filter=") {
            filter = match value.trim() {
                "services" => Filter::Services,
                other => {
                    return Err(format!("unknown filter {other:?}. Supported: services"));
                }
            };
        } else {
            return Err(format!(
                "unknown plugin option {opt:?}. Supported: filter=services"
            ));
        }
    }
    Ok(filter)
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
    fn unknown_filter_errors() {
        let err = parse_filter("filter=bogus").unwrap_err();
        assert!(err.contains("bogus"));
    }

    #[test]
    fn unknown_option_errors() {
        let err = parse_filter("bogus_option").unwrap_err();
        assert!(err.contains("bogus_option"));
    }

    #[test]
    fn empty_filter_value_errors() {
        // `filter=` with no value hits the unknown-filter arm with `""`.
        let err = parse_filter("filter=").unwrap_err();
        assert!(err.contains("unknown filter"));
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

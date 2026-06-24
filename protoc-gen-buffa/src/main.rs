//! protoc plugin for generating Rust code with buffa.
//!
//! This binary follows the protoc plugin protocol:
//! 1. Read a serialized `CodeGeneratorRequest` from stdin.
//! 2. Pass the file descriptors to `buffa-codegen`.
//! 3. Write a serialized `CodeGeneratorResponse` to stdout.
//!
//! Usage:
//!   protoc --buffa_out=. my_service.proto
//!
//! Or with buf:
//!   # buf.gen.yaml
//!   plugins:
//!     - local: protoc-gen-buffa
//!       out: src/gen

use std::io::{self, Read, Write};

use buffa::Message;
use buffa_codegen::generated::compiler::code_generator_response::File as CodeGeneratorResponseFile;
use buffa_codegen::generated::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};
use buffa_codegen::generated::descriptor::Edition;

use buffa_codegen::CodeGenConfig;

const HELP: &str = "\
protoc-gen-buffa — protoc plugin for generating Rust code with buffa.

This binary speaks the protoc plugin protocol: it reads a serialized
CodeGeneratorRequest from stdin and writes a CodeGeneratorResponse to
stdout. It is not intended to be invoked directly. Use it via protoc
or buf (with this binary on PATH):

  protoc --buffa_out=. my_service.proto

  # buf.gen.yaml
  plugins:
    - local: protoc-gen-buffa
      out: src/gen

To point protoc at a binary not on PATH, use
  --plugin=protoc-gen-buffa=/abs/path/to/protoc-gen-buffa

For a generated mod.rs module tree, also configure
protoc-gen-buffa-packaging.

Options are passed as a comma-separated parameter string, e.g.
  --buffa_opt=views=true,json=true,extern_path=.my.pkg=::my_crate

See <https://github.com/anthropics/buffa/blob/main/docs/guide.md> for
the full option list.";

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
            // Protocol: write a response with an error string, don't just crash.
            let response = CodeGeneratorResponse {
                error: Some(format!("{}", e)),
                supported_features: Some(feature_flags()),
                ..Default::default()
            };
            write_response(&response).unwrap_or_else(|io_err| {
                eprintln!(
                    "protoc-gen-buffa: failed to write error response: {}",
                    io_err
                );
                std::process::exit(1);
            });
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Read the entire request from stdin.
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;

    // Decode the CodeGeneratorRequest.
    let request = CodeGeneratorRequest::decode_from_slice(&input)
        .map_err(|e| format!("failed to decode CodeGeneratorRequest: {}", e))?;

    // Parse plugin parameters (e.g., "views=true,unknown_fields=false").
    let config = parse_config(request.parameter.as_deref().unwrap_or(""))?;

    // Run code generation, forwarding non-fatal warnings to stderr (protoc
    // surfaces plugin stderr to the user).
    let (generated, warnings) = buffa_codegen::generate_with_diagnostics(
        &request.proto_file,
        &request.file_to_generate,
        &config.codegen,
    )?;
    for warning in &warnings {
        eprintln!("protoc-gen-buffa: warning: {warning}");
    }

    // Build the response. `generated` is consumed here so the names and
    // contents move directly into the response rather than being cloned.
    let files: Vec<CodeGeneratorResponseFile> = generated
        .into_iter()
        .map(|g| CodeGeneratorResponseFile {
            name: Some(g.name),
            content: Some(g.content),
            ..Default::default()
        })
        .collect();

    let response = CodeGeneratorResponse {
        supported_features: Some(feature_flags()),
        // Tell protoc which editions we support.
        minimum_edition: Some(Edition::EDITION_PROTO2 as i32),
        maximum_edition: Some(Edition::EDITION_2024 as i32),
        file: files,
        ..Default::default()
    };

    write_response(&response)?;
    Ok(())
}

/// Write the serialized CodeGeneratorResponse to stdout.
fn write_response(response: &CodeGeneratorResponse) -> io::Result<()> {
    let mut output = Vec::new();
    response.encode(&mut output);
    io::stdout().write_all(&output)?;
    io::stdout().flush()?;
    Ok(())
}

/// Feature flags we support (bitmask).
fn feature_flags() -> u64 {
    const FEATURE_PROTO3_OPTIONAL: u64 = 1;
    const FEATURE_SUPPORTS_EDITIONS: u64 = 2;
    FEATURE_PROTO3_OPTIONAL | FEATURE_SUPPORTS_EDITIONS
}

/// Plugin configuration parsed from the parameter string.
struct PluginConfig {
    /// Code generation options passed to buffa-codegen.
    codegen: CodeGenConfig,
}

/// Parse the plugin parameter string into a PluginConfig.
///
/// Parameters are comma-separated key=value pairs:
///   --buffa_opt=views=true,unknown_fields=false,json=true
///
/// Extern paths use the format `extern_path=<proto>=<rust>`, where `<proto>`
/// is either a package or a single type FQN:
///   --buffa_opt=extern_path=.my.common=::common_protos
///   --buffa_opt=extern_path=.my.common.Shared=::shared_types::Shared
fn parse_config(params: &str) -> Result<PluginConfig, String> {
    let mut codegen = CodeGenConfig::default();

    if params.is_empty() {
        return Ok(PluginConfig { codegen });
    }

    for param in params.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (key, value) = param
            .split_once('=')
            .ok_or_else(|| format!("plugin option '{param}' must use key=value syntax"))?;
        match key.trim() {
            "views" => codegen.generate_views = parse_bool("views", value)?,
            "lazy_views" => codegen.lazy_views = parse_bool("lazy_views", value)?,
            "unknown_fields" => {
                codegen.preserve_unknown_fields = parse_bool("unknown_fields", value)?
            }
            "json" => codegen.generate_json = parse_bool("json", value)?,
            "text" => codegen.generate_text = parse_bool("text", value)?,
            "arbitrary" => codegen.generate_arbitrary = parse_bool("arbitrary", value)?,
            // `gate_impls=true` wraps generated impls in `#[cfg(feature = ...)]`
            // instead of emitting them unconditionally. For library crates whose
            // generated code is itself a public dependency surface; most plugin
            // invocations want the default (off).
            "gate_impls" => codegen.gate_impls_on_crate_features = parse_bool("gate_impls", value)?,
            // `json_feature=serde` (etc.) renames the crate feature a
            // gated impl kind is conditioned on. Inert without
            // `gate_impls=true`. An empty value is a hard error —
            // `#[cfg(feature = "")]` is permanently false and would
            // silently drop the gated impls. (A non-empty value that is
            // not a valid Cargo feature name is rejected by `generate`
            // when the gate is active.)
            key @ ("json_feature" | "views_feature" | "text_feature" | "reflect_feature") => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(format!(
                        "'{key}' requires a non-empty feature name \
                             (an empty name would silently disable the gated impls)"
                    ));
                }
                let names = &mut codegen.feature_gate_names;
                let slot = match key {
                    "json_feature" => &mut names.json,
                    "views_feature" => &mut names.views,
                    "text_feature" => &mut names.text,
                    _ => &mut names.reflect,
                };
                *slot = value.to_string();
            }
            "allow_message_set" => {
                codegen.allow_message_set = parse_bool("allow_message_set", value)?
            }
            "strict_utf8" | "strict_utf8_mapping" => {
                codegen.strict_utf8_mapping = parse_bool(key.trim(), value)?
            }
            "register_types" => codegen.emit_register_fn = parse_bool("register_types", value)?,
            // `with_setters=false` opts out of builder-style setter
            // methods. Like `register_types`, the default is on, so the
            // accepted spelling is the negation.
            "with_setters" => codegen.generate_with_setters = parse_bool("with_setters", value)?,
            // `reflection=true` selects the fast vtable mode (same as
            // `reflect_mode=vtable`); `reflect_mode=bridge` opts into the
            // smaller round-trip implementation.
            "reflection" => {
                let mode = if parse_bool("reflection", value)? {
                    buffa_codegen::ReflectMode::VTable
                } else {
                    buffa_codegen::ReflectMode::Off
                };
                mode.apply(&mut codegen);
            }
            // `reflect_mode=off|bridge|vtable` is the fuller form of
            // `reflection=`. `vtable` additionally emits `impl ReflectMessage`
            // on owned + view types and makes `reflect()` borrow `self`.
            "reflect_mode" => match value.trim() {
                "off" => buffa_codegen::ReflectMode::Off.apply(&mut codegen),
                "bridge" => buffa_codegen::ReflectMode::Bridge.apply(&mut codegen),
                "vtable" => buffa_codegen::ReflectMode::VTable.apply(&mut codegen),
                other => {
                    return Err(format!(
                        "invalid reflect_mode '{other}', expected off, bridge, or vtable"
                    ));
                }
            },
            "file_per_package" => codegen.file_per_package = parse_bool("file_per_package", value)?,
            // Experimental: `use`-backed short type names at the package
            // root. Requires file_per_package=true (rejected by codegen
            // otherwise).
            "idiomatic_imports" => {
                codegen.idiomatic_imports = parse_bool("idiomatic_imports", value)?
            }
            // `type_name_prefix=Rpc` prepends a prefix to every generated
            // message/enum type name (and their view types). The value is
            // passed through verbatim; buffa-codegen rejects anything that
            // is not PascalCase at generation time (same rule as the
            // builder API).
            "type_name_prefix" => codegen.type_name_prefix = value.to_string(),
            "extern_path" => {
                // value is "<proto_path>=<rust_path>"
                if let Some((proto, rust)) = value.split_once('=') {
                    let proto = proto.trim();
                    let rust = rust.trim();
                    if proto.is_empty() || rust.is_empty() {
                        return Err(format!(
                            "invalid extern_path format '{value}', \
                                 expected 'extern_path=.proto.pkg=::rust::path' \
                                 (or a type FQN, 'extern_path=.proto.pkg.Type=::rust::path::Type')"
                        ));
                    }
                    let mut proto = proto.to_string();
                    // Normalize: accept both ".my.pkg" and "my.pkg".
                    if !proto.starts_with('.') {
                        proto.insert(0, '.');
                    }
                    codegen.extern_paths.push((proto, rust.to_string()));
                } else {
                    return Err(format!(
                        "invalid extern_path format '{}', \
                             expected 'extern_path=.proto.pkg=::rust::path' \
                             (or a type FQN, 'extern_path=.proto.pkg.Type=::rust::path::Type')",
                        value
                    ));
                }
            }
            "mod_file" => {
                return Err("the mod_file option was removed in 0.2; use \
                         protoc-gen-buffa-packaging instead. See CHANGELOG \
                         for migration."
                    .to_string());
            }
            other => {
                return Err(format!(
                    "unknown plugin option '{other}'; see \
                     <https://github.com/anthropics/buffa/blob/main/docs/guide.md#plugin-options> \
                     for the supported options"
                ))
            }
        }
    }

    Ok(PluginConfig { codegen })
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!(
            "invalid boolean value for '{key}': '{other}', expected true or false"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_err(params: &str) -> String {
        match parse_config(params) {
            Ok(_) => panic!("expected parse_config({params:?}) to fail"),
            Err(err) => err,
        }
    }

    #[test]
    fn empty_params_returns_defaults() {
        let config = parse_config("").unwrap();
        let defaults = CodeGenConfig::default();
        assert_eq!(config.codegen.generate_views, defaults.generate_views);
        assert_eq!(
            config.codegen.preserve_unknown_fields,
            defaults.preserve_unknown_fields
        );
        assert_eq!(config.codegen.generate_json, defaults.generate_json);
        assert!(config.codegen.extern_paths.is_empty());
    }

    #[test]
    fn views_true() {
        let config = parse_config("views=true").unwrap();
        assert!(config.codegen.generate_views);
    }

    #[test]
    fn views_false() {
        let config = parse_config("views=false").unwrap();
        assert!(!config.codegen.generate_views);
    }

    #[test]
    fn lazy_views_true() {
        let config = parse_config("lazy_views=true").unwrap();
        assert!(config.codegen.lazy_views);
        assert!(!parse_config("").unwrap().codegen.lazy_views);
    }

    #[test]
    fn json_true() {
        let config = parse_config("json=true").unwrap();
        assert!(config.codegen.generate_json);
    }

    #[test]
    fn unknown_fields_false() {
        let config = parse_config("unknown_fields=false").unwrap();
        assert!(!config.codegen.preserve_unknown_fields);
    }

    #[test]
    fn unknown_fields_true() {
        let config = parse_config("unknown_fields=true").unwrap();
        assert!(config.codegen.preserve_unknown_fields);
    }

    #[test]
    fn file_per_package_true() {
        let config = parse_config("file_per_package=true").unwrap();
        assert!(config.codegen.file_per_package);
    }

    #[test]
    fn file_per_package_default_is_false() {
        let config = parse_config("").unwrap();
        assert!(!config.codegen.file_per_package);
    }

    #[test]
    fn idiomatic_imports_true() {
        let config = parse_config("file_per_package=true,idiomatic_imports=true").unwrap();
        assert!(config.codegen.idiomatic_imports);
    }

    #[test]
    fn idiomatic_imports_defaults_off() {
        let config = parse_config("").unwrap();
        assert!(!config.codegen.idiomatic_imports);
    }

    #[test]
    fn extern_path_with_leading_dot() {
        let config = parse_config("extern_path=.my.common=::common_protos").unwrap();
        assert_eq!(config.codegen.extern_paths.len(), 1);
        assert_eq!(config.codegen.extern_paths[0].0, ".my.common");
        assert_eq!(config.codegen.extern_paths[0].1, "::common_protos");
    }

    #[test]
    fn extern_path_without_leading_dot_is_normalized() {
        let config = parse_config("extern_path=my.common=::common_protos").unwrap();
        assert_eq!(config.codegen.extern_paths[0].0, ".my.common");
    }

    #[test]
    fn multiple_params() {
        let config = parse_config("views=true,json=true").unwrap();
        assert!(config.codegen.generate_views);
        assert!(config.codegen.generate_json);
    }

    #[test]
    fn multiple_extern_paths() {
        let config =
            parse_config("extern_path=.my.a=::crate_a,extern_path=.my.b=::crate_b").unwrap();
        assert_eq!(config.codegen.extern_paths.len(), 2);
        assert_eq!(config.codegen.extern_paths[0].0, ".my.a");
        assert_eq!(config.codegen.extern_paths[1].0, ".my.b");
    }

    #[test]
    fn whitespace_is_trimmed() {
        let config = parse_config(" views = true , json = true ").unwrap();
        assert!(config.codegen.generate_views);
        assert!(config.codegen.generate_json);
    }

    #[test]
    fn unknown_param_errors() {
        let err = parse_err("unknown_key=value");
        assert!(err.contains("unknown_key"));
    }

    #[test]
    fn missing_equals_errors() {
        let err = parse_err("json");
        assert!(err.contains("key=value"));
    }

    #[test]
    fn invalid_bool_errors() {
        let err = parse_err("json=yes");
        assert!(err.contains("json"));
        assert!(err.contains("true or false"));
    }

    #[test]
    fn invalid_bool_for_default_on_option_errors() {
        let err = parse_err("unknown_fields=yes");
        assert!(err.contains("unknown_fields"));
        assert!(err.contains("true or false"));
    }

    #[test]
    fn invalid_reflect_mode_errors() {
        let err = parse_err("reflect_mode=fast");
        assert!(err.contains("reflect_mode"));
        assert!(err.contains("off, bridge, or vtable"));
    }

    #[test]
    fn invalid_extern_path_errors() {
        let err = parse_err("extern_path=no_equals_sign");
        assert!(err.contains("extern_path"));
    }

    #[test]
    fn empty_extern_path_side_errors() {
        let err = parse_err("extern_path=.my.common=");
        assert!(err.contains("extern_path"));
    }

    #[test]
    fn register_types_false() {
        let config = parse_config("register_types=false").unwrap();
        assert!(!config.codegen.emit_register_fn);
    }

    #[test]
    fn register_types_true() {
        let config = parse_config("register_types=true").unwrap();
        assert!(config.codegen.emit_register_fn);
    }

    #[test]
    fn register_types_default_is_true() {
        let config = parse_config("").unwrap();
        assert!(config.codegen.emit_register_fn);
    }

    #[test]
    fn gate_impls_true() {
        let config = parse_config("gate_impls=true").unwrap();
        assert!(config.codegen.gate_impls_on_crate_features);
    }

    #[test]
    fn gate_impls_default_is_false() {
        let config = parse_config("").unwrap();
        assert!(!config.codegen.gate_impls_on_crate_features);
    }

    #[test]
    fn feature_name_overrides() {
        let config =
            parse_config("json_feature=serde,views_feature=v,text_feature=t,reflect_feature=r")
                .unwrap();
        assert_eq!(config.codegen.feature_gate_names.json, "serde");
        assert_eq!(config.codegen.feature_gate_names.views, "v");
        assert_eq!(config.codegen.feature_gate_names.text, "t");
        assert_eq!(config.codegen.feature_gate_names.reflect, "r");
    }

    #[test]
    fn empty_feature_name_is_rejected() {
        let err = match parse_config("json_feature=") {
            Err(err) => err,
            Ok(_) => panic!("empty feature name must be a parse error"),
        };
        assert!(
            err.contains("json_feature"),
            "error names the option: {err}"
        );
    }

    #[test]
    fn feature_names_default() {
        let config = parse_config("").unwrap();
        assert_eq!(config.codegen.feature_gate_names.json, "json");
        assert_eq!(config.codegen.feature_gate_names.views, "views");
        assert_eq!(config.codegen.feature_gate_names.text, "text");
        assert_eq!(config.codegen.feature_gate_names.reflect, "reflect");
    }

    #[test]
    fn type_name_prefix_parsed() {
        let config = parse_config("type_name_prefix=Rpc").unwrap();
        assert_eq!(config.codegen.type_name_prefix, "Rpc");
    }

    #[test]
    fn type_name_prefix_default_is_empty() {
        let config = parse_config("").unwrap();
        assert!(config.codegen.type_name_prefix.is_empty());
    }

    #[test]
    fn type_name_prefix_not_trimmed() {
        // The value is passed through verbatim (no per-value trim) so the
        // plugin and the builder API accept/reject exactly the same strings —
        // codegen later rejects this one as not PascalCase.
        let config = parse_config("type_name_prefix= Rpc").unwrap();
        assert_eq!(config.codegen.type_name_prefix, " Rpc");
    }

    #[test]
    fn with_setters_false() {
        let config = parse_config("with_setters=false").unwrap();
        assert!(!config.codegen.generate_with_setters);
    }

    #[test]
    fn with_setters_default_is_true() {
        let config = parse_config("").unwrap();
        assert!(config.codegen.generate_with_setters);
    }

    #[test]
    fn mod_file_errors_with_migration_hint() {
        let err = parse_config("mod_file=mod.rs").err().unwrap();
        assert!(err.contains("protoc-gen-buffa-packaging"));
    }

    #[test]
    fn text_true() {
        let config = parse_config("text=true").unwrap();
        assert!(config.codegen.generate_text);
    }

    #[test]
    fn text_default_is_false() {
        let config = parse_config("").unwrap();
        assert!(!config.codegen.generate_text);
    }

    #[test]
    fn allow_message_set_true() {
        let config = parse_config("allow_message_set=true").unwrap();
        assert!(config.codegen.allow_message_set);
    }

    #[test]
    fn allow_message_set_default_is_false() {
        let config = parse_config("").unwrap();
        assert!(!config.codegen.allow_message_set);
    }
}

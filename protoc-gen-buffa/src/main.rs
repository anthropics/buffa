//! protoc plugin for generating Rust code with buffa.
//!
//! This binary follows the protoc plugin protocol:
//! 1. Read a serialized `CodeGeneratorRequest` from stdin.
//! 2. Pass the file descriptors to `buffa-codegen`.
//! 3. Write a serialized `CodeGeneratorResponse` to stdout.
//!
//! Usage:
//!   protoc --buffa_out=. --plugin=protoc-gen-buffa my_service.proto
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

fn main() {
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

    // Run code generation.
    let generated = buffa_codegen::generate(
        &request.proto_file,
        &request.file_to_generate,
        &config.codegen,
    )?;

    // Build the response.
    let files: Vec<CodeGeneratorResponseFile> = generated
        .iter()
        .map(|g| CodeGeneratorResponseFile {
            name: Some(g.name.clone()),
            content: Some(g.content.clone()),
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
/// Extern paths use the format `extern_path=<proto>=<rust>`:
///   --buffa_opt=extern_path=.my.common=::common_protos
fn parse_config(params: &str) -> Result<PluginConfig, String> {
    let mut codegen = CodeGenConfig::default();

    if params.is_empty() {
        return Ok(PluginConfig { codegen });
    }

    for param in params.split(',') {
        let param = param.trim();
        if let Some((key, value)) = param.split_once('=') {
            match key.trim() {
                "views" => codegen.generate_views = value.trim() == "true",
                "unknown_fields" => codegen.preserve_unknown_fields = value.trim() != "false",
                "json" => codegen.generate_json = value.trim() == "true",
                "text" => codegen.generate_text = value.trim() == "true",
                "arbitrary" => codegen.generate_arbitrary = value.trim() == "true",
                "allow_message_set" => codegen.allow_message_set = value.trim() == "true",
                "strict_utf8" | "strict_utf8_mapping" => {
                    codegen.strict_utf8_mapping = value.trim() == "true"
                }
                "register_types" => codegen.emit_register_fn = value.trim() != "false",
                "file_per_package" => codegen.file_per_package = value.trim() == "true",
                "extern_path" => {
                    // value is "<proto_path>=<rust_path>"
                    if let Some((proto, rust)) = value.split_once('=') {
                        let mut proto = proto.trim().to_string();
                        // Normalize: accept both ".my.pkg" and "my.pkg".
                        if !proto.starts_with('.') {
                            proto.insert(0, '.');
                        }
                        codegen.extern_paths.push((proto, rust.trim().to_string()));
                    } else {
                        eprintln!(
                            "protoc-gen-buffa: invalid extern_path format '{}', \
                             expected 'extern_path=.proto.pkg=::rust::path'",
                            value
                        );
                    }
                }
                "mod_file" => {
                    return Err("the mod_file option was removed in 0.2; use \
                         protoc-gen-buffa-packaging instead. See CHANGELOG \
                         for migration."
                        .to_string());
                }
                other => {
                    eprintln!("protoc-gen-buffa: unknown parameter '{}'", other);
                }
            }
        }
    }

    Ok(PluginConfig { codegen })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn unknown_param_is_ignored() {
        // Should not panic; unknown params produce an eprintln warning.
        let config = parse_config("unknown_key=value").unwrap();
        let defaults = CodeGenConfig::default();
        assert_eq!(config.codegen.generate_views, defaults.generate_views);
    }

    #[test]
    fn invalid_extern_path_is_ignored() {
        // Missing "=" in the value — should not panic.
        let config = parse_config("extern_path=no_equals_sign").unwrap();
        assert!(config.codegen.extern_paths.is_empty());
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

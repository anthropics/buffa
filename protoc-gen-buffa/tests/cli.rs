use std::io::Write;
use std::process::{Command, Stdio};

use buffa::Message;
use buffa_codegen::generated::compiler::code_generator_response::Feature;
use buffa_codegen::generated::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};
use buffa_codegen::generated::descriptor::Edition;

const BIN: &str = env!("CARGO_BIN_EXE_protoc-gen-buffa");

/// Run the plugin on `request` and decode its response. The plugin reports
/// errors in the response, so it must exit successfully either way.
fn run_plugin(request: &CodeGeneratorRequest) -> CodeGeneratorResponse {
    let mut input = Vec::new();
    request.encode(&mut input);

    let mut child = Command::new(BIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();

    assert!(out.status.success());
    CodeGeneratorResponse::decode_from_slice(&out.stdout).unwrap()
}

#[test]
fn version_prints_name_and_version() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("protoc-gen-buffa {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag() {
    let out = Command::new(BIN).arg("-V").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .starts_with("protoc-gen-buffa "));
}

#[test]
fn help_mentions_protoc_plugin_protocol() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("protoc plugin protocol"));
    assert!(stdout.contains("--buffa_opt"));
}

#[test]
fn unrecognized_arg_exits_2() {
    let out = Command::new(BIN).arg("bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("unrecognized argument"));
    assert!(stderr.contains("bogus"));
}

#[test]
fn invalid_plugin_option_returns_response_error() {
    let response = run_plugin(&CodeGeneratorRequest {
        parameter: Some("unknown_key=value".into()),
        ..Default::default()
    });
    let error = response.error.as_deref().unwrap_or_default();
    assert!(error.contains("unknown_key"), "error response: {error}");
}

#[test]
fn error_response_declares_the_edition_range() {
    // FEATURE_SUPPORTS_EDITIONS without the range is rejected by buf before
    // it reads `error`, which would hide the message.
    let response = run_plugin(&CodeGeneratorRequest {
        parameter: Some("unknown_key=value".into()),
        ..Default::default()
    });
    assert!(response.error.is_some());
    let features = response.supported_features.unwrap_or_default();
    assert_ne!(features & Feature::FEATURE_SUPPORTS_EDITIONS as u64, 0);
    assert_eq!(
        response.minimum_edition,
        Some(Edition::EDITION_PROTO2 as i32)
    );
    assert_eq!(response.maximum_edition, Some(Edition::EDITION_2024 as i32));
}

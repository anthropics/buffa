use std::io::Write;
use std::process::{Command, Stdio};

use buffa::Message;
use buffa_codegen::generated::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};
use buffa_codegen::generated::descriptor::Edition;

const BIN: &str = env!("CARGO_BIN_EXE_protoc-gen-buffa-packaging");

#[test]
fn version_prints_name_and_version() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("protoc-gen-buffa-packaging {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag() {
    let out = Command::new(BIN).arg("-V").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .starts_with("protoc-gen-buffa-packaging "));
}

#[test]
fn help_mentions_filter_option() {
    for flag in ["--help", "-h"] {
        let out = Command::new(BIN).arg(flag).output().unwrap();
        assert!(out.status.success());
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(stdout.contains("protoc plugin protocol"));
        assert!(stdout.contains("filter=services"));
    }
}

#[test]
fn unrecognized_arg_exits_2() {
    let out = Command::new(BIN).arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr)
        .unwrap()
        .contains("unrecognized argument"));
}

#[test]
fn invalid_plugin_option_returns_response_error() {
    let request = CodeGeneratorRequest {
        parameter: Some("filter=bogus".into()),
        ..Default::default()
    };
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
    let response = CodeGeneratorResponse::decode_from_slice(&out.stdout).unwrap();
    let error = response.error.as_deref().unwrap_or_default();
    assert!(error.contains("bogus"), "error response: {error}");
    // Buf validates the feature advertisement before surfacing this error.
    // An incomplete edition range would replace our diagnostic with a protocol error.
    assert_eq!(response.supported_features, Some(3));
    assert_eq!(
        response.minimum_edition,
        Some(Edition::EDITION_PROTO2 as i32)
    );
    assert_eq!(response.maximum_edition, Some(Edition::EDITION_2024 as i32));
}

use std::io::Write;
use std::process::{Command, Stdio};

use buffa::Message;
use buffa_codegen::generated::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};

const BIN: &str = env!("CARGO_BIN_EXE_protoc-gen-buffa");

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
    let request = CodeGeneratorRequest {
        parameter: Some("unknown_key=value".into()),
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
    assert!(error.contains("unknown_key"), "error response: {error}");
}

#[test]
fn custom_field_options_reach_generated_output_without_changing_other_fields() {
    use buffa_codegen::generated::descriptor::{
        field_descriptor_proto::{Label, Type},
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    };
    let field = |name: &str, number: i32, kind: Type| FieldDescriptorProto {
        name: Some(name.to_owned()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(kind),
        ..Default::default()
    };
    let request = CodeGeneratorRequest {
        parameter: Some(
            "views=false,unknown_fields=false,json=true,\
            bytes_type_custom_in=.demo.Request.key=crate::SecretBytes,\
            string_type_custom_in=.demo.Request.identity=crate::SecretString,\
            repeated_type_custom_in=.demo.Request.embedding=crate::SecretList<*>"
                .to_owned(),
        ),
        file_to_generate: vec!["request.proto".to_owned()],
        proto_file: vec![FileDescriptorProto {
            name: Some("request.proto".to_owned()),
            syntax: Some("proto3".to_owned()),
            package: Some("demo".to_owned()),
            message_type: vec![DescriptorProto {
                name: Some("Request".to_owned()),
                field: vec![
                    field("key", 1, Type::TYPE_BYTES),
                    field("identity", 2, Type::TYPE_STRING),
                    field("media_type", 3, Type::TYPE_STRING),
                    FieldDescriptorProto {
                        label: Some(Label::LABEL_REPEATED),
                        ..field("embedding", 4, Type::TYPE_FLOAT)
                    },
                    FieldDescriptorProto {
                        label: Some(Label::LABEL_REPEATED),
                        ..field("scores", 5, Type::TYPE_FLOAT)
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut child = Command::new(BIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&request.encode_to_vec())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response = CodeGeneratorResponse::decode_from_slice(&output.stdout).unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    let content = response
        .file
        .iter()
        .filter_map(|file| file.content.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(content.contains("pub key: crate::SecretBytes"));
    assert!(content.contains("pub identity: crate::SecretString"));
    assert!(content.contains("pub media_type: ::buffa::alloc::string::String"));
    assert!(content.contains("pub embedding: crate::SecretList<f32>"));
    assert!(content.contains("pub scores: ::buffa::alloc::vec::Vec<f32>"));
    assert!(!content.contains("pub __buffa_unknown_fields"));
}

#[test]
fn invalid_custom_rust_types_are_reported_by_codegen() {
    use buffa_codegen::generated::descriptor::{
        field_descriptor_proto::{Label, Type},
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    };
    // `crate::` does not parse; the others parse as Rust types but are not type
    // paths, so they could never implement `ProtoBytes` / `ProtoList`.
    let cases = [
        ("bytes_type_custom_in", "crate::"),
        ("bytes_type_custom_in", "_"),
        ("bytes_type_custom_in", "()"),
        ("bytes_type_custom_in", "[u8; 4]"),
        ("bytes_type_custom_in", "&'static [u8]"),
        ("string_type_custom_in", "impl AsRef<str>"),
        ("repeated_type_custom_in", "[*; 4]"),
    ];
    for (option, rust_type) in cases {
        let request = CodeGeneratorRequest {
            parameter: Some(format!("{option}=.demo.Request.key={rust_type}")),
            file_to_generate: vec!["request.proto".to_owned()],
            proto_file: vec![FileDescriptorProto {
                name: Some("request.proto".to_owned()),
                syntax: Some("proto3".to_owned()),
                package: Some("demo".to_owned()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_owned()),
                    field: vec![FieldDescriptorProto {
                        name: Some("key".to_owned()),
                        number: Some(1),
                        label: Some(if option == "repeated_type_custom_in" {
                            Label::LABEL_REPEATED
                        } else {
                            Label::LABEL_OPTIONAL
                        }),
                        r#type: Some(if option == "string_type_custom_in" {
                            Type::TYPE_STRING
                        } else {
                            Type::TYPE_BYTES
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (response, _) = run_plugin(&request);
        let error = response.error.as_deref().unwrap_or_default();
        assert!(
            error.contains("invalid Rust type path") && error.contains(rust_type),
            "{option}={rust_type}: error response: {error:?}"
        );
        assert!(response.file.is_empty(), "{option}={rust_type}");
    }
}

/// Runs the plugin on `request`, returning the decoded response and stderr.
fn run_plugin(request: &CodeGeneratorRequest) -> (CodeGeneratorResponse, String) {
    let mut child = Command::new(BIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&request.encode_to_vec())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    (
        CodeGeneratorResponse::decode_from_slice(&output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn a_custom_type_rule_on_an_extension_warns_in_plugin_terms() {
    use buffa_codegen::generated::descriptor::{
        descriptor_proto::ExtensionRange,
        field_descriptor_proto::{Label, Type},
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    };
    let bytes_field = |name: &str, number: i32| FieldDescriptorProto {
        name: Some(name.to_owned()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_BYTES),
        ..Default::default()
    };
    let request = CodeGeneratorRequest {
        parameter: Some("bytes_type_custom_in=.e.ext_bytes=crate::SecretBytes".to_owned()),
        file_to_generate: vec!["e.proto".to_owned()],
        proto_file: vec![FileDescriptorProto {
            name: Some("e.proto".to_owned()),
            syntax: Some("proto2".to_owned()),
            package: Some("e".to_owned()),
            message_type: vec![DescriptorProto {
                name: Some("Holder".to_owned()),
                field: vec![bytes_field("data", 1)],
                extension_range: vec![ExtensionRange {
                    start: Some(100),
                    end: Some(200),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            extension: vec![FieldDescriptorProto {
                extendee: Some(".e.Holder".to_owned()),
                ..bytes_field("ext_bytes", 100)
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let (response, stderr) = run_plugin(&request);
    assert!(response.error.is_none(), "{:?}", response.error);
    let warning = stderr
        .lines()
        .find(|line| line.contains("'.e.ext_bytes'"))
        .unwrap_or_else(|| panic!("no warning for the rule; stderr: {stderr}"));
    assert!(warning.contains("extension"), "{warning}");
    assert!(warning.contains("bytes_type_custom_in"), "{warning}");
    assert!(!warning.contains("buffa-build"), "{warning}");
    assert!(!warning.contains("bytes_fields"), "{warning}");
}

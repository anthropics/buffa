//! `Config::extern_path` proto names: `compile` normalizes a valid name and
//! rejects a malformed one, with the rules `protoc-gen-buffa` applies.

use buffa_build::Config;
use buffa_codegen::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_codegen::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};

/// `svc.proto` (package `my.svc`) with a field of `.my.common.Shared`, which
/// `common.proto` declares.
fn descriptor_set() -> Vec<u8> {
    let common = FileDescriptorProto {
        name: Some("common.proto".into()),
        package: Some("my.common".into()),
        syntax: Some("proto3".into()),
        message_type: vec![DescriptorProto {
            name: Some("Shared".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let svc = FileDescriptorProto {
        name: Some("svc.proto".into()),
        package: Some("my.svc".into()),
        syntax: Some("proto3".into()),
        dependency: vec!["common.proto".into()],
        message_type: vec![DescriptorProto {
            name: Some("Request".into()),
            field: vec![FieldDescriptorProto {
                name: Some("shared".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".my.common.Shared".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    buffa_codegen::encode_descriptor_set(&[common, svc], &[])
}

#[test]
fn every_accepted_spelling_maps_the_package() {
    for proto_path in [".my.common", "my.common", ".my.common.", "my.common."] {
        let dir = tempfile::tempdir().unwrap();
        Config::new()
            .descriptor_set_bytes(descriptor_set())
            .files(&["svc.proto"])
            .extern_path(proto_path, "::common_protos")
            .out_dir(dir.path())
            .compile()
            .unwrap_or_else(|e| panic!("{proto_path:?}: {e}"));

        let generated = std::fs::read_to_string(dir.path().join("svc.rs")).unwrap();
        assert!(
            generated.contains("::common_protos::Shared"),
            "{proto_path:?} should map my.common.Shared: {generated}"
        );
    }
}

#[test]
fn malformed_name_is_rejected_and_quoted_as_given() {
    for (proto_path, problem) in [
        ("my..common", "'my..common' has an empty segment"),
        (".my.common..", "'.my.common..' has an empty segment"),
        ("", "'' has an empty segment"),
        (
            "my.common-name",
            "'my.common-name': segment 'common-name' is not a protobuf identifier",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let err = Config::new()
            .descriptor_set_bytes(descriptor_set())
            .files(&["svc.proto"])
            .extern_path(proto_path, "::common_protos")
            .out_dir(dir.path())
            .compile()
            .expect_err("a malformed extern_path proto name must be rejected");
        let message = err.to_string();
        assert!(
            message.contains("extern_path") && message.contains(problem),
            "{proto_path:?}: {message}"
        );
    }
}

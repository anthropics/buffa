//! MessageSet extension IDs are int32 values, rather than ordinary wire tags.
use buffa_codegen::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_codegen::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_codegen::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, MessageOptions,
};
use buffa_codegen::{generate, generate_with_diagnostics, CodeGenConfig};

const MAX: i32 = buffa::encoding::MAX_FIELD_NUMBER as i32;

fn files(number: i32, message_set: bool, nested: bool) -> Vec<FileDescriptorProto> {
    let container = DescriptorProto {
        name: Some("Container".into()),
        extension_range: vec![ExtensionRange {
            start: Some(4),
            end: Some(if message_set { i32::MAX } else { MAX + 1 }),
            ..Default::default()
        }],
        options: MessageOptions {
            message_set_wire_format: Some(message_set),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let extendee = if nested {
        ".test.Scope.Container"
    } else {
        ".test.Container"
    };
    let extension = FieldDescriptorProto {
        name: Some("item".into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(".test.Payload".into()),
        extendee: Some(extendee.into()),
        ..Default::default()
    };
    vec![
        FileDescriptorProto {
            name: Some("container.proto".into()),
            package: Some("test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![if nested {
                DescriptorProto {
                    name: Some("Scope".into()),
                    nested_type: vec![container],
                    ..Default::default()
                }
            } else {
                container
            }],
            ..Default::default()
        },
        FileDescriptorProto {
            name: Some("extension.proto".into()),
            package: Some("test".into()),
            syntax: Some("proto2".into()),
            dependency: vec!["container.proto".into()],
            message_type: vec![DescriptorProto {
                name: Some("Payload".into()),
                extension: if nested {
                    vec![extension.clone()]
                } else {
                    vec![]
                },
                ..Default::default()
            }],
            extension: if nested { vec![] } else { vec![extension] },
            ..Default::default()
        },
    ]
}

fn config() -> CodeGenConfig {
    let mut config = CodeGenConfig::default();
    config.allow_message_set = true;
    config.generate_views = false;
    config
}

#[test]
fn message_set_extension_numbers_above_ordinary_limit_generate() {
    for nested in [false, true] {
        for number in [MAX, MAX + 1, 1_000_000_000, i32::MAX - 1, i32::MAX] {
            let files = files(number, true, nested);
            let selected = vec!["extension.proto".into()];
            // The scalar bound admits i32::MAX; a descriptor pool separately
            // enforces the exclusive extension range end.
            for output in [
                generate(&files, &selected, &config()).unwrap(),
                generate_with_diagnostics(&files, &selected, &config())
                    .unwrap()
                    .0,
            ] {
                assert!(output
                    .iter()
                    .any(|f| f.content.contains(&format!("{number}u32"))));
            }
        }
    }
}

#[test]
fn ordinary_extensions_and_nonpositive_message_set_ids_still_fail() {
    for nested in [false, true] {
        for (message_set, numbers) in [
            (false, vec![MAX + 1, i32::MAX]),
            (true, vec![0, -1, i32::MIN]),
        ] {
            for number in numbers {
                let files = files(number, message_set, nested);
                let selected = vec!["extension.proto".into()];
                assert!(generate(&files, &selected, &config())
                    .unwrap_err()
                    .to_string()
                    .contains(&format!("invalid field number: {number}")));
                assert!(generate_with_diagnostics(&files, &selected, &config()).is_err());
            }
        }
    }
}

#[test]
fn ordinary_fields_keep_the_ordinary_limit() {
    let mut files = files(1000, true, false);
    files[1].message_type[0].field.push(FieldDescriptorProto {
        name: Some("too_large".into()),
        number: Some(MAX + 1),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_INT32),
        ..Default::default()
    });
    assert!(generate(&files, &["extension.proto".into()], &config()).is_err());
}

#![cfg(feature = "reflect")]

use buffa::editions::FieldPresence;
use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::feature_set::FieldPresence as DescriptorPresence;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FeatureSet, FieldDescriptorProto, FieldOptions, FileDescriptorProto,
    FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn required_field() -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some("value".into()),
        number: Some(1),
        label: Some(Label::LABEL_REQUIRED),
        r#type: Some(Type::TYPE_INT32),
        ..Default::default()
    }
}

fn file(syntax: Option<&str>) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("required-label.proto".into()),
        package: Some("required.test".into()),
        syntax: syntax.map(str::to_owned),
        edition: (syntax == Some("editions")).then_some(Edition::EDITION_2023),
        message_type: vec![DescriptorProto {
            name: Some("Message".into()),
            field: vec![required_field()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn extension_host() -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("host.proto".into()),
        package: Some("required.test".into()),
        syntax: Some("proto2".into()),
        message_type: vec![DescriptorProto {
            name: Some("Host".into()),
            field: vec![required_field()],
            extension_range: vec![ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn invalid_syntax_files() -> [FileDescriptorProto; 3] {
    let mut edition_2024 = file(Some("editions"));
    edition_2024.edition = Some(Edition::EDITION_2024);
    [file(Some("proto3")), file(Some("editions")), edition_2024]
}

fn assert_rejected_transactionally(file: FileDescriptorProto, expected_field: &str) {
    assert_rejected_transactionally_with(file, expected_field, |err| {
        assert!(matches!(
            err,
            PoolError::RequiredFieldOutsideProto2 { field } if field == expected_field
        ));
        assert_eq!(
            err.to_string(),
            format!("field {expected_field} uses a required label outside a proto2 file")
        );
    });
}

/// Adds `file` to a pool that holds `host.proto`, passes the error to
/// `assert_error`, and checks that the pool is unchanged.
fn assert_rejected_transactionally_with(
    file: FileDescriptorProto,
    expected_field: &str,
    assert_error: impl FnOnce(&PoolError),
) {
    let file_name = file.name.clone().unwrap();
    let mut pool = DescriptorPool::new(FileDescriptorSet {
        file: vec![extension_host()],
        ..Default::default()
    })
    .unwrap();
    let err = pool
        .add_file_descriptor_set(FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        })
        .expect_err("the file must be rejected");
    assert_error(&err);
    assert_eq!(pool.files().len(), 1);
    assert_eq!(pool.messages().len(), 1);
    assert_eq!(pool.extensions().len(), 0);
    assert!(pool.file_by_name(&file_name).is_none());
    assert!(pool.file_containing_symbol(expected_field).is_none());
    assert!(pool.message_by_name("required.test.Message").is_none());
    assert_eq!(
        pool.message_by_name("required.test.Host")
            .unwrap()
            .field(1)
            .unwrap()
            .presence(),
        FieldPresence::LegacyRequired
    );
}

#[test]
fn required_field_labels_are_rejected_outside_proto2() {
    for file in invalid_syntax_files() {
        assert_rejected_transactionally(file, "required.test.Message.value");
    }
}

#[test]
fn encoded_required_field_labels_are_rejected_outside_proto2() {
    for file in invalid_syntax_files() {
        let bytes = FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        }
        .encode_to_vec();
        let err = DescriptorPool::decode(&bytes).unwrap_err();
        assert!(matches!(
            err,
            PoolError::RequiredFieldOutsideProto2 { field }
                if field == "required.test.Message.value"
        ));
    }
}

#[test]
fn nested_required_field_labels_are_rejected_outside_proto2() {
    for mut file in invalid_syntax_files() {
        let message = &mut file.message_type[0];
        message.field.clear();
        message.nested_type.push(DescriptorProto {
            name: Some("Nested".into()),
            field: vec![required_field()],
            ..Default::default()
        });
        assert_rejected_transactionally(file, "required.test.Message.Nested.value");
    }
}

#[test]
fn required_extension_labels_are_rejected_outside_proto2() {
    for file in invalid_syntax_files() {
        for nested in [false, true] {
            let mut file = file.clone();
            file.dependency.push("host.proto".into());
            file.message_type[0].field.clear();
            let extension = FieldDescriptorProto {
                name: Some("flag".into()),
                number: Some(100),
                extendee: Some(".required.test.Host".into()),
                ..required_field()
            };
            let expected_field = if nested {
                file.message_type[0].extension.push(extension);
                "required.test.Message.flag"
            } else {
                file.extension.push(extension);
                "required.test.flag"
            };
            assert_rejected_transactionally(file, expected_field);
        }
    }
}

/// `file` importing `host.proto`, with its message's field replaced by an
/// extension of `Host`, declared at file level or nested in the message.
/// Returns the file and the extension's full name.
fn with_extension(
    mut file: FileDescriptorProto,
    extension: FieldDescriptorProto,
    nested: bool,
) -> (FileDescriptorProto, &'static str) {
    file.dependency.push("host.proto".into());
    file.message_type[0].field.clear();
    if nested {
        file.message_type[0].extension.push(extension);
        (file, "required.test.Message.flag")
    } else {
        file.extension.push(extension);
        (file, "required.test.flag")
    }
}

fn assert_required_extension_rejected(file: FileDescriptorProto, expected_field: &str) {
    assert_rejected_transactionally_with(file, expected_field, |err| {
        assert!(matches!(
            err,
            PoolError::RequiredExtension { field } if field == expected_field
        ));
        assert_eq!(
            err.to_string(),
            format!("extension {expected_field} is required")
        );
    });
}

#[test]
fn required_extension_labels_are_rejected_in_proto2() {
    for syntax in [None, Some(""), Some("proto2")] {
        for nested in [false, true] {
            let extension = FieldDescriptorProto {
                name: Some("flag".into()),
                number: Some(100),
                extendee: Some(".required.test.Host".into()),
                ..required_field()
            };
            let (file, expected_field) = with_extension(file(syntax), extension, nested);
            assert_required_extension_rejected(file, expected_field);
        }
    }
}

#[test]
fn legacy_required_extensions_are_rejected_in_editions() {
    for edition in [Edition::EDITION_2023, Edition::EDITION_2024] {
        for nested in [false, true] {
            let mut file = file(Some("editions"));
            file.edition = Some(edition);
            let extension = FieldDescriptorProto {
                name: Some("flag".into()),
                number: Some(100),
                extendee: Some(".required.test.Host".into()),
                label: Some(Label::LABEL_OPTIONAL),
                options: buffa::MessageField::some(FieldOptions {
                    features: buffa::MessageField::some(FeatureSet {
                        field_presence: Some(DescriptorPresence::LEGACY_REQUIRED),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..required_field()
            };
            let (file, expected_field) = with_extension(file, extension, nested);
            assert_required_extension_rejected(file, expected_field);
        }
    }
}

#[test]
fn optional_and_repeated_extensions_link_in_every_syntax() {
    let mut edition_2023 = file(Some("editions"));
    edition_2023.edition = Some(Edition::EDITION_2023);
    for file in [file(Some("proto2")), file(Some("proto3")), edition_2023] {
        for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
            let extension = FieldDescriptorProto {
                name: Some("flag".into()),
                number: Some(100),
                extendee: Some(".required.test.Host".into()),
                label: Some(label),
                ..required_field()
            };
            let (file, name) = with_extension(file.clone(), extension, false);
            let pool = DescriptorPool::new(FileDescriptorSet {
                file: vec![extension_host(), file],
                ..Default::default()
            })
            .unwrap();
            assert!(pool.extension_by_name(name).is_some());
        }
    }
}

#[test]
fn required_field_labels_link_in_explicit_and_implicit_proto2_files() {
    for syntax in [None, Some(""), Some("proto2")] {
        let pool = DescriptorPool::new(FileDescriptorSet {
            file: vec![file(syntax)],
            ..Default::default()
        })
        .expect("proto2 allows required field labels");
        assert_eq!(
            pool.message_by_name("required.test.Message")
                .unwrap()
                .field(1)
                .unwrap()
                .presence(),
            FieldPresence::LegacyRequired
        );
    }
}

#[test]
fn optional_and_repeated_field_labels_link_in_proto3_and_editions() {
    for file in invalid_syntax_files() {
        for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
            let mut file = file.clone();
            file.message_type[0].field[0].label = Some(label);
            let expected_presence =
                if label == Label::LABEL_REPEATED || file.syntax.as_deref() == Some("proto3") {
                    FieldPresence::Implicit
                } else {
                    FieldPresence::Explicit
                };
            let pool = DescriptorPool::new(FileDescriptorSet {
                file: vec![file],
                ..Default::default()
            })
            .expect("only the required label is forbidden");
            assert_eq!(
                pool.message_by_name("required.test.Message")
                    .unwrap()
                    .field(1)
                    .unwrap()
                    .presence(),
                expected_presence
            );
        }
    }
}

#[test]
fn editions_legacy_required_features_link_without_required_labels() {
    for edition in [Edition::EDITION_2023, Edition::EDITION_2024] {
        let mut file = file(Some("editions"));
        file.edition = Some(edition);
        let field = &mut file.message_type[0].field[0];
        field.label = Some(Label::LABEL_OPTIONAL);
        field.options = buffa::MessageField::some(FieldOptions {
            features: buffa::MessageField::some(FeatureSet {
                field_presence: Some(DescriptorPresence::LEGACY_REQUIRED),
                ..Default::default()
            }),
            ..Default::default()
        });
        let pool = DescriptorPool::new(FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        })
        .expect("Editions use LEGACY_REQUIRED features instead of required labels");
        assert_eq!(
            pool.message_by_name("required.test.Message")
                .unwrap()
                .field(1)
                .unwrap()
                .presence(),
            FieldPresence::LegacyRequired
        );
    }
}

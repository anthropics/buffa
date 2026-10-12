#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::feature_set::MessageEncoding;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FeatureSet, FieldDescriptorProto, FieldOptions, FileDescriptorProto,
    FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn group_set(syntax: Option<&str>, nested: bool, label: Label) -> FileDescriptorSet {
    let host_name = if nested { "Outer.Host" } else { "Host" };
    let host = DescriptorProto {
        name: Some("Host".into()),
        field: vec![FieldDescriptorProto {
            name: Some("group".into()),
            number: Some(10),
            label: Some(label),
            r#type: Some(Type::TYPE_GROUP),
            type_name: Some(format!(".group.test.{host_name}.Group")),
            ..Default::default()
        }],
        nested_type: vec![DescriptorProto {
            name: Some("Group".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("groups.proto".into()),
            package: Some("group.test".into()),
            syntax: syntax.map(Into::into),
            edition: (syntax == Some("editions")).then_some(Edition::EDITION_2023),
            message_type: vec![if nested {
                DescriptorProto {
                    name: Some("Outer".into()),
                    nested_type: vec![host],
                    ..Default::default()
                }
            } else {
                host
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn assert_group_error(error: &PoolError, field_name: &str) {
    assert!(
        matches!(error, PoolError::GroupFieldInProto3 { field } if field == field_name),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!("field {field_name} uses a group type in proto3")
    );
}

#[test]
fn proto3_group_fields_are_rejected() {
    for nested in [false, true] {
        for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
            let error = DescriptorPool::new(group_set(Some("proto3"), nested, label)).unwrap_err();
            let field = if nested {
                "group.test.Outer.Host.group"
            } else {
                "group.test.Host.group"
            };
            assert_group_error(&error, field);
        }
    }
}

#[test]
fn encoded_proto3_group_fields_are_rejected() {
    for nested in [false, true] {
        let bytes = group_set(Some("proto3"), nested, Label::LABEL_OPTIONAL).encode_to_vec();
        let error = DescriptorPool::decode(&bytes).unwrap_err();
        let field = if nested {
            "group.test.Outer.Host.group"
        } else {
            "group.test.Host.group"
        };
        assert_group_error(&error, field);
    }
}

#[test]
fn proto2_group_fields_keep_delimited_encoding() {
    for syntax in [None, Some(""), Some("proto2")] {
        for nested in [false, true] {
            for label in [
                Label::LABEL_OPTIONAL,
                Label::LABEL_REQUIRED,
                Label::LABEL_REPEATED,
            ] {
                let pool = DescriptorPool::new(group_set(syntax, nested, label)).unwrap();
                let name = if nested {
                    "group.test.Outer.Host"
                } else {
                    "group.test.Host"
                };
                assert!(pool
                    .message_by_name(name)
                    .unwrap()
                    .field(10)
                    .unwrap()
                    .is_delimited());
            }
        }
    }
}

#[test]
fn proto3_message_fields_keep_length_prefixed_encoding() {
    let mut set = group_set(Some("proto3"), false, Label::LABEL_OPTIONAL);
    set.file[0].message_type[0].field[0].r#type = Some(Type::TYPE_MESSAGE);
    let pool = DescriptorPool::new(set).unwrap();
    assert!(!pool
        .message_by_name("group.test.Host")
        .unwrap()
        .field(10)
        .unwrap()
        .is_delimited());
}

#[test]
fn editions_delimited_message_fields_are_accepted() {
    for edition in [Edition::EDITION_2023, Edition::EDITION_2024] {
        let mut set = group_set(Some("editions"), false, Label::LABEL_OPTIONAL);
        set.file[0].edition = Some(edition);
        let field = &mut set.file[0].message_type[0].field[0];
        field.r#type = Some(Type::TYPE_MESSAGE);
        field.options = FieldOptions {
            features: FeatureSet {
                message_encoding: Some(MessageEncoding::DELIMITED),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
        .into();
        let pool = DescriptorPool::new(set).unwrap();
        assert!(pool
            .message_by_name("group.test.Host")
            .unwrap()
            .field(10)
            .unwrap()
            .is_delimited());
    }
}

#[test]
fn proto3_files_can_import_messages_with_proto2_groups() {
    let mut set = group_set(Some("proto2"), false, Label::LABEL_OPTIONAL);
    set.file.push(FileDescriptorProto {
        name: Some("importer.proto".into()),
        package: Some("import.test".into()),
        syntax: Some("proto3".into()),
        dependency: vec!["groups.proto".into()],
        message_type: vec![DescriptorProto {
            name: Some("Importer".into()),
            field: vec![FieldDescriptorProto {
                name: Some("host".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".group.test.Host".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    let pool = DescriptorPool::new(set).unwrap();
    assert!(!pool
        .message_by_name("import.test.Importer")
        .unwrap()
        .field(1)
        .unwrap()
        .is_delimited());
    assert!(pool
        .message_by_name("group.test.Host")
        .unwrap()
        .field(10)
        .unwrap()
        .is_delimited());
}

fn option_extension_set(syntax: &str, nested: bool, label: Label, ty: Type) -> FileDescriptorSet {
    let mut set = group_set(Some(syntax), false, label);
    let extension = FieldDescriptorProto {
        number: Some(50001),
        extendee: Some(".google.protobuf.MessageOptions".into()),
        r#type: Some(ty),
        ..set.file[0].message_type[0].field.remove(0)
    };
    set.file[0].dependency.push("options-host.proto".into());
    if nested {
        set.file[0].message_type[0].extension.push(extension);
    } else {
        set.file[0].extension.push(extension);
    }
    set.file.insert(
        0,
        FileDescriptorProto {
            name: Some("options-host.proto".into()),
            package: Some("google.protobuf".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("MessageOptions".into()),
                extension_range: vec![ExtensionRange {
                    start: Some(1000),
                    end: Some(536_870_912),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    set
}

#[test]
fn proto3_group_extensions_are_rejected() {
    for nested in [false, true] {
        for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
            let set = option_extension_set("proto3", nested, label, Type::TYPE_GROUP);
            let error = DescriptorPool::new(set).unwrap_err();
            assert_group_error(
                &error,
                if nested {
                    "group.test.Host.group"
                } else {
                    "group.test.group"
                },
            );
        }
    }
}

#[test]
fn valid_custom_option_extensions_are_accepted() {
    for nested in [false, true] {
        for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
            for (syntax, ty, delimited) in [
                ("proto2", Type::TYPE_GROUP, true),
                ("proto3", Type::TYPE_MESSAGE, false),
            ] {
                let pool =
                    DescriptorPool::new(option_extension_set(syntax, nested, label, ty)).unwrap();
                let name = if nested {
                    "group.test.Host.group"
                } else {
                    "group.test.group"
                };
                assert_eq!(
                    pool.extension_by_name(name).unwrap().field().is_delimited(),
                    delimited
                );
            }
        }
    }
}

#[test]
fn proto3_group_errors_name_fields_without_a_package() {
    let mut set = group_set(Some("proto3"), false, Label::LABEL_OPTIONAL);
    set.file[0].package = None;
    set.file[0].message_type[0].field[0].type_name = Some(".Host.Group".into());
    let error = DescriptorPool::new(set).unwrap_err();
    assert_group_error(&error, "Host.group");
}

#[test]
fn proto3_group_rejection_does_not_mutate_an_existing_pool() {
    let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
    let before = pool.clone();
    let mut set = group_set(Some("proto3"), false, Label::LABEL_OPTIONAL);
    set.file.insert(
        0,
        FileDescriptorProto {
            name: Some("valid.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Valid".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let error = pool.add_file_descriptor_set(set).unwrap_err();
    assert_group_error(&error, "group.test.Host.group");
    assert_eq!(pool.files(), before.files());
    assert_eq!(pool.messages().len(), before.messages().len());
    assert_eq!(pool.enums().len(), before.enums().len());
    assert_eq!(pool.extensions().len(), before.extensions().len());
    assert!(pool.message_by_name("group.test.Host").is_none());
    assert!(pool.message_by_name("valid.test.Valid").is_none());
    assert!(pool.file_containing_symbol("group.test.Host").is_none());
    assert!(pool.file_containing_symbol("valid.test.Valid").is_none());
}

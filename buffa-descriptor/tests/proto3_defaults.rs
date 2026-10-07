#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::feature_set::FieldPresence;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, EnumDescriptorProto, EnumValueDescriptorProto, FeatureSet,
    FieldDescriptorProto, FieldOptions, FileDescriptorProto, FileDescriptorSet, FileOptions,
    OneofDescriptorProto,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn field(ty: Type, default: Option<&str>) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some("value".into()),
        number: Some(1),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(ty),
        default_value: default.map(Into::into),
        ..Default::default()
    }
}

fn file(syntax: Option<&str>, field: FieldDescriptorProto) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("defaults.proto".into()),
        package: Some("defaults.test".into()),
        syntax: syntax.map(Into::into),
        message_type: vec![DescriptorProto {
            name: Some("Defaults".into()),
            field: vec![field],
            ..Default::default()
        }],
        enum_type: vec![EnumDescriptorProto {
            name: Some("State".into()),
            value: vec![EnumValueDescriptorProto {
                name: Some("ZERO".into()),
                number: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn set(file: FileDescriptorProto) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    }
}

fn assert_default_error(err: PoolError, expected_field: &str) {
    assert!(
        matches!(&err, PoolError::Proto3FieldWithDefault { field } if field == expected_field),
        "unexpected error: {err}"
    );
    assert_eq!(
        err.to_string(),
        format!("field {expected_field} declares a default value in a proto3 file")
    );
}

fn assert_rejected(set: FileDescriptorSet, expected_field: &str) {
    assert_default_error(DescriptorPool::new(set).unwrap_err(), expected_field);
}

#[test]
fn proto3_scalar_defaults_are_rejected_including_zero_and_empty_values() {
    for (ty, default) in [
        (Type::TYPE_INT32, "7"),
        (Type::TYPE_INT32, "0"),
        (Type::TYPE_INT64, "-1"),
        (Type::TYPE_UINT64, "0"),
        (Type::TYPE_FLOAT, "0"),
        (Type::TYPE_DOUBLE, "1.5"),
        (Type::TYPE_BOOL, "false"),
        (Type::TYPE_STRING, ""),
        (Type::TYPE_STRING, "text"),
        (Type::TYPE_BYTES, ""),
    ] {
        assert_rejected(
            set(file(Some("proto3"), field(ty, Some(default)))),
            "defaults.test.Defaults.value",
        );
    }
}

#[test]
fn proto3_enum_defaults_are_rejected_with_explicit_or_inferred_type() {
    for ty in [Some(Type::TYPE_ENUM), None] {
        let mut value = field(Type::TYPE_ENUM, Some("ZERO"));
        value.r#type = ty;
        value.type_name = Some(".defaults.test.State".into());
        assert_rejected(
            set(file(Some("proto3"), value)),
            "defaults.test.Defaults.value",
        );
    }
}

#[test]
fn proto3_oneof_and_optional_defaults_are_rejected() {
    for proto3_optional in [false, true] {
        let mut value = field(Type::TYPE_BOOL, Some("false"));
        value.oneof_index = Some(0);
        value.proto3_optional = Some(proto3_optional);
        let mut descriptor = file(Some("proto3"), value);
        descriptor.message_type[0].oneof_decl = vec![OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        }];
        assert_rejected(set(descriptor), "defaults.test.Defaults.value");
    }
}

#[test]
fn proto3_nested_field_defaults_are_rejected() {
    let mut descriptor = file(Some("proto3"), field(Type::TYPE_STRING, None));
    descriptor.message_type[0].nested_type = vec![DescriptorProto {
        name: Some("Nested".into()),
        field: vec![field(Type::TYPE_STRING, Some(""))],
        ..Default::default()
    }];
    assert_rejected(set(descriptor), "defaults.test.Defaults.Nested.value");
}

fn extension_set(nested: bool, default: Option<&str>) -> FileDescriptorSet {
    let extendee = FileDescriptorProto {
        name: Some("google/protobuf/descriptor.proto".into()),
        package: Some("google.protobuf".into()),
        syntax: Some("proto2".into()),
        message_type: vec![DescriptorProto {
            name: Some("FieldOptions".into()),
            extension_range: vec![ExtensionRange {
                start: Some(1000),
                end: Some(536_870_912),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let extension = FieldDescriptorProto {
        extendee: Some(".google.protobuf.FieldOptions".into()),
        number: Some(50001),
        proto3_optional: Some(true),
        ..field(Type::TYPE_BOOL, default)
    };
    let mut declaring_file = FileDescriptorProto {
        name: Some("extensions.proto".into()),
        package: Some("options.test".into()),
        syntax: Some("proto3".into()),
        dependency: vec!["google/protobuf/descriptor.proto".into()],
        ..Default::default()
    };
    if nested {
        declaring_file.message_type = vec![DescriptorProto {
            name: Some("Scope".into()),
            extension: vec![extension],
            ..Default::default()
        }];
    } else {
        declaring_file.extension = vec![extension];
    }
    FileDescriptorSet {
        file: vec![extendee, declaring_file],
        ..Default::default()
    }
}

#[test]
fn proto3_extension_defaults_are_rejected_using_the_declaring_file_syntax() {
    for nested in [false, true] {
        let expected_field = if nested {
            "options.test.Scope.value"
        } else {
            "options.test.value"
        };
        assert_rejected(extension_set(nested, Some("false")), expected_field);
        DescriptorPool::new(extension_set(nested, None))
            .expect("proto3 option extensions without defaults remain valid");
    }
}

#[test]
fn defaults_link_in_proto2_and_in_editions_with_explicit_presence() {
    for (syntax, edition) in [
        (None, None),
        (Some("proto2"), None),
        (Some("editions"), Some(Edition::EDITION_2023)),
        (Some("editions"), Some(Edition::EDITION_2024)),
    ] {
        for (ty, default) in [
            (Type::TYPE_INT32, "7"),
            (Type::TYPE_BOOL, "false"),
            (Type::TYPE_STRING, ""),
            (Type::TYPE_BYTES, ""),
        ] {
            let mut descriptor = file(syntax, field(ty, Some(default)));
            descriptor.edition = edition;
            DescriptorPool::new(set(descriptor))
                .unwrap_or_else(|err| panic!("{syntax:?} {edition:?} {ty:?} {default:?}: {err}"));
        }
    }
}

fn presence_features(presence: FieldPresence) -> FeatureSet {
    FeatureSet {
        field_presence: Some(presence),
        ..Default::default()
    }
}

fn editions_file(field: FieldDescriptorProto, edition: Edition) -> FileDescriptorProto {
    let mut descriptor = file(Some("editions"), field);
    descriptor.edition = Some(edition);
    descriptor
}

#[test]
fn editions_implicit_presence_fields_reject_defaults() {
    for edition in [Edition::EDITION_2023, Edition::EDITION_2024] {
        for (ty, default) in [
            (Type::TYPE_INT32, "7"),
            (Type::TYPE_INT32, "0"),
            (Type::TYPE_STRING, ""),
        ] {
            // The feature set on the field itself.
            let mut on_field = field(ty, Some(default));
            on_field.options = FieldOptions {
                features: presence_features(FieldPresence::IMPLICIT).into(),
                ..Default::default()
            }
            .into();
            // The feature inherited from the file.
            let mut inherited = editions_file(field(ty, Some(default)), edition);
            inherited.options = FileOptions {
                features: presence_features(FieldPresence::IMPLICIT).into(),
                ..Default::default()
            }
            .into();

            for descriptor in [editions_file(on_field, edition), inherited] {
                let err = DescriptorPool::new(set(descriptor)).unwrap_err();
                assert!(
                    matches!(
                        &err,
                        PoolError::ImplicitPresenceFieldWithDefault { field }
                            if field == "defaults.test.Defaults.value"
                    ),
                    "{edition:?} {ty:?} {default:?}: {err}"
                );
                assert_eq!(
                    err.to_string(),
                    "field defaults.test.Defaults.value has implicit presence and declares a \
                     default value"
                );
            }
        }
    }
}

#[test]
fn editions_extensions_keep_their_defaults_under_implicit_presence() {
    // An extension has presence even when the file's `field_presence` is
    // `IMPLICIT`; protoc builds this file.
    let descriptor = FileDescriptorProto {
        name: Some("foo.proto".into()),
        syntax: Some("editions".into()),
        edition: Some(Edition::EDITION_2023),
        options: FileOptions {
            features: presence_features(FieldPresence::IMPLICIT).into(),
            ..Default::default()
        }
        .into(),
        message_type: vec![DescriptorProto {
            name: Some("Foo".into()),
            extension_range: vec![ExtensionRange {
                start: Some(1),
                end: Some(100),
                ..Default::default()
            }],
            ..Default::default()
        }],
        extension: vec![FieldDescriptorProto {
            name: Some("bar".into()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_STRING),
            default_value: Some("Hello world".into()),
            extendee: Some("Foo".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let pool = DescriptorPool::new(set(descriptor)).unwrap();
    assert!(pool.extension_by_name("bar").is_some());
}

#[test]
fn editions_fields_with_presence_keep_their_defaults() {
    for presence in [FieldPresence::EXPLICIT, FieldPresence::LEGACY_REQUIRED] {
        let mut with_presence = field(Type::TYPE_INT32, Some("7"));
        with_presence.options = FieldOptions {
            features: presence_features(presence).into(),
            ..Default::default()
        }
        .into();
        DescriptorPool::new(set(editions_file(with_presence, Edition::EDITION_2023)))
            .unwrap_or_else(|err| panic!("{presence:?}: {err}"));
    }

    // A oneof member has presence under a file-level `IMPLICIT`.
    let mut member = field(Type::TYPE_INT32, Some("7"));
    member.oneof_index = Some(0);
    let mut descriptor = editions_file(member, Edition::EDITION_2023);
    descriptor.message_type[0]
        .oneof_decl
        .push(OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        });
    descriptor.options = FileOptions {
        features: presence_features(FieldPresence::IMPLICIT).into(),
        ..Default::default()
    }
    .into();
    DescriptorPool::new(set(descriptor)).unwrap();
}

#[test]
fn proto3_fields_without_explicit_defaults_remain_valid() {
    for ty in [
        Type::TYPE_INT32,
        Type::TYPE_BOOL,
        Type::TYPE_STRING,
        Type::TYPE_BYTES,
    ] {
        DescriptorPool::new(set(file(Some("proto3"), field(ty, None))))
            .expect("an unset default is valid in proto3");
    }
}

#[test]
fn decoded_proto3_descriptors_preserve_and_reject_explicit_defaults() {
    for (ty, default) in [(Type::TYPE_INT32, "0"), (Type::TYPE_STRING, "")] {
        let bytes = set(file(Some("proto3"), field(ty, Some(default)))).encode_to_vec();
        assert_default_error(
            DescriptorPool::decode(&bytes).unwrap_err(),
            "defaults.test.Defaults.value",
        );
        let valid_bytes = set(file(Some("proto3"), field(ty, None))).encode_to_vec();
        DescriptorPool::decode(&valid_bytes)
            .expect("an absent default remains valid after decoding");
    }
}

#[test]
fn rejected_proto3_defaults_leave_the_pool_unchanged_and_allow_retry() {
    let mut pool = DescriptorPool::new(set(FileDescriptorProto {
        name: Some("baseline.proto".into()),
        syntax: Some("proto3".into()),
        message_type: vec![DescriptorProto {
            name: Some("Existing".into()),
            ..Default::default()
        }],
        ..Default::default()
    }))
    .unwrap();
    let baseline = (
        pool.files().len(),
        pool.messages().len(),
        pool.enums().len(),
        pool.extensions().len(),
    );
    let invalid = file(Some("proto3"), field(Type::TYPE_STRING, Some("")));
    assert_default_error(
        pool.add_file_descriptor_set(set(invalid)).unwrap_err(),
        "defaults.test.Defaults.value",
    );
    assert_eq!(
        (
            pool.files().len(),
            pool.messages().len(),
            pool.enums().len(),
            pool.extensions().len(),
        ),
        baseline
    );
    assert!(pool.message_by_name("Existing").is_some());
    assert!(pool.message_by_name("defaults.test.Defaults").is_none());
    assert!(pool.enum_by_name("defaults.test.State").is_none());
    assert!(pool.file_by_name("defaults.proto").is_none());

    pool.add_file_descriptor_set(set(file(Some("proto3"), field(Type::TYPE_STRING, None))))
        .expect("the corrected descriptor can be added with the same file and symbol names");
    assert!(pool.message_by_name("defaults.test.Defaults").is_some());
}

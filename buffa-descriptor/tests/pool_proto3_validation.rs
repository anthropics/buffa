#![cfg(feature = "reflect")]

use buffa_descriptor::generated::descriptor::feature_set::EnumType;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::FeatureSet;
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto,
    FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet, OneofDescriptorProto,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn enum_with_type(enum_type: EnumType) -> EnumDescriptorProto {
    EnumDescriptorProto {
        name: Some("Kind".into()),
        value: vec![EnumValueDescriptorProto {
            name: Some("UNKNOWN".into()),
            number: Some(0),
            ..Default::default()
        }],
        options: EnumOptions {
            features: FeatureSet {
                enum_type: Some(enum_type),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

fn enum_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_ENUM),
        type_name: Some(type_name.into()),
        ..Default::default()
    }
}

fn proto3_file(
    package: &str,
    messages: Vec<DescriptorProto>,
    enums: Vec<EnumDescriptorProto>,
    extensions: Vec<FieldDescriptorProto>,
) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some(format!("{package}.proto")),
            package: Some(package.into()),
            syntax: Some("proto3".into()),
            message_type: messages,
            enum_type: enums,
            extension: extensions,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn ext_targeting_non_options_file() -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![
            FileDescriptorProto {
                name: Some("legacy.proto".into()),
                package: Some("ext.test".into()),
                syntax: Some("proto2".into()),
                message_type: vec![DescriptorProto {
                    name: Some("RegularMessage".into()),
                    extension_range: vec![
                        buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange {
                            start: Some(100),
                            end: Some(200),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
            FileDescriptorProto {
                name: Some("ext.test.proto".into()),
                package: Some("ext.test".into()),
                syntax: Some("proto3".into()),
                dependency: vec!["legacy.proto".into()],
                extension: vec![FieldDescriptorProto {
                    name: Some("my_ext".into()),
                    number: Some(100),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    extendee: Some(".ext.test.RegularMessage".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn ext_targeting_options_file() -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![
            FileDescriptorProto {
                name: Some("opts.proto".into()),
                package: Some("google.protobuf".into()),
                syntax: Some("proto2".into()),
                message_type: vec![DescriptorProto {
                    name: Some("MethodOptions".into()),
                    extension_range: vec![
                        buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange {
                            start: Some(100),
                            end: Some(200),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
            FileDescriptorProto {
                name: Some("ext.test.proto".into()),
                package: Some("ext.test".into()),
                syntax: Some("proto3".into()),
                dependency: vec!["opts.proto".into()],
                extension: vec![FieldDescriptorProto {
                    name: Some("my_ext".into()),
                    number: Some(100),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    extendee: Some(".google.protobuf.MethodOptions".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn proto2_file_with_extension() -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("legacy.proto".into()),
            package: Some("legacy.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("RegularMessage".into()),
                extension_range: vec![
                    buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange {
                        start: Some(100),
                        end: Some(200),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            extension: vec![FieldDescriptorProto {
                name: Some("my_ext".into()),
                number: Some(100),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                extendee: Some(".legacy.test.RegularMessage".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn proto3_extension_targeting_non_options_message_is_rejected() {
    let error = DescriptorPool::new(ext_targeting_non_options_file())
        .expect_err("proto3 extension to non-Options must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field } if field == "ext.test.my_ext"),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "field ext.test.my_ext extends a non-Options message in proto3"
    );
}

#[test]
fn proto3_extension_targeting_options_message_is_accepted() {
    DescriptorPool::new(ext_targeting_options_file()).unwrap();
}

fn ext_targeting_namesake(
    package: &str,
    message_name: &str,
    extendee: &str,
    file_name: &str,
) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![
            FileDescriptorProto {
                name: Some(format!("{file_name}.proto")),
                package: Some(package.into()),
                syntax: Some("proto2".into()),
                message_type: vec![DescriptorProto {
                    name: Some(message_name.into()),
                    extension_range: vec![
                        buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange {
                            start: Some(100),
                            end: Some(200),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
            FileDescriptorProto {
                name: Some("ext.test.proto".into()),
                package: Some("ext.test".into()),
                syntax: Some("proto3".into()),
                dependency: vec![format!("{file_name}.proto")],
                extension: vec![FieldDescriptorProto {
                    name: Some("my_ext".into()),
                    number: Some(100),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    extendee: Some(extendee.into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn proto3_extension_targeting_non_google_options_namesake_is_rejected() {
    // Ends in `Options` but lives outside `google.protobuf`: not one of the
    // nine extendable messages.
    let error = DescriptorPool::new(ext_targeting_namesake(
        "other.pkg",
        "MyOptions",
        ".other.pkg.MyOptions",
        "namesake",
    ))
    .expect_err("proto3 extension to non-google Options namesake must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field } if field == "ext.test.my_ext"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_extension_targeting_fake_google_options_is_rejected() {
    // Inside `google.protobuf` and ends in `Options`, but not one of the
    // nine real options messages.
    let error = DescriptorPool::new(ext_targeting_namesake(
        "google.protobuf",
        "FakeOptions",
        ".google.protobuf.FakeOptions",
        "fakeopts",
    ))
    .expect_err("proto3 extension to fake Options must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field } if field == "ext.test.my_ext"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto2_extension_targeting_non_options_message_is_accepted() {
    DescriptorPool::new(proto2_file_with_extension()).unwrap();
}

#[test]
fn proto3_singular_implicit_closed_enum_is_deferred_to_separate_check() {
    // Singular implicit-presence closed enums are open PR #609's scope, so
    // this check deliberately leaves them alone to avoid double-reporting.
    let fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![enum_field("value", 1, ".enum.test.Kind")],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::CLOSED)],
        vec![],
    );
    DescriptorPool::new(fds).unwrap();
}

#[test]
fn proto3_optional_closed_enum_field_is_rejected() {
    let mut field = enum_field("value", 1, ".enum.test.Kind");
    field.proto3_optional = Some(true);
    field.oneof_index = Some(0);
    let fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![field],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("_value".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::CLOSED)],
        vec![],
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 optional closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field } if field == "enum.test.Sample.value"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_map_value_closed_enum_field_is_rejected() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    let entry = DescriptorProto {
        name: Some("ValuesEntry".into()),
        field: vec![
            FieldDescriptorProto {
                name: Some("key".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("value".into()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".enum.test.Kind".into()),
                ..Default::default()
            },
        ],
        options: buffa_descriptor::generated::descriptor::MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let mut field = enum_field("values", 1, ".enum.test.Sample.ValuesEntry");
    field.label = Some(Label::LABEL_REPEATED);
    field.r#type = Some(Type::TYPE_MESSAGE);
    let fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![field],
            nested_type: vec![entry],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::CLOSED)],
        vec![],
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 map-value closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field } if field == "enum.test.Sample.values"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_repeated_closed_enum_field_is_rejected() {
    let mut field = enum_field("values", 1, ".enum.test.Kind");
    field.label = Some(Label::LABEL_REPEATED);
    let fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![field],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::CLOSED)],
        vec![],
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 repeated closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field } if field == "enum.test.Sample.values"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_oneof_closed_enum_field_is_rejected() {
    let mut field = enum_field("choice", 1, ".enum.test.Kind");
    field.oneof_index = Some(0);
    let fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![field],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("choice_group".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::CLOSED)],
        vec![],
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 oneof closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field } if field == "enum.test.Sample.choice"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_open_enum_field_is_accepted() {
    let fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![enum_field("value", 1, ".enum.test.Kind")],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::OPEN)],
        vec![],
    );
    DescriptorPool::new(fds).unwrap();
}

#[test]
fn proto2_closed_enum_field_is_accepted() {
    let fds = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("legacy.proto".into()),
            package: Some("legacy.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("Sample".into()),
                field: vec![enum_field("value", 1, ".legacy.test.Kind")],
                ..Default::default()
            }],
            enum_type: vec![enum_with_type(EnumType::CLOSED)],
            ..Default::default()
        }],
        ..Default::default()
    };
    DescriptorPool::new(fds).unwrap();
}

#![cfg(feature = "reflect")]

use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::feature_set::EnumType;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto, FeatureSet,
    FieldDescriptorProto, FieldOptions, FileDescriptorProto, FileDescriptorSet,
    OneofDescriptorProto,
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
) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some(format!("{package}.proto")),
            package: Some(package.into()),
            syntax: Some("proto3".into()),
            message_type: messages,
            enum_type: enums,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A two-file set. The proto2 file `extendee.proto` declares the extendable
/// message `<package>.<message>`, and `ext.test.proto`, in the given
/// `syntax`, declares the string extension `ext.test.my_ext` of it.
fn extension_set(package: &str, message: &str, syntax: &str) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![
            FileDescriptorProto {
                name: Some("extendee.proto".into()),
                package: Some(package.into()),
                syntax: Some("proto2".into()),
                message_type: vec![DescriptorProto {
                    name: Some(message.into()),
                    extension_range: vec![ExtensionRange {
                        start: Some(100),
                        end: Some(200),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            FileDescriptorProto {
                name: Some("ext.test.proto".into()),
                package: Some("ext.test".into()),
                syntax: Some(syntax.into()),
                dependency: vec!["extendee.proto".into()],
                extension: vec![FieldDescriptorProto {
                    name: Some("my_ext".into()),
                    number: Some(100),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    extendee: Some(format!(".{package}.{message}")),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// Moves the extension of `extension_set`'s second file into a message
/// `Scope` of that file, so its full name is `ext.test.Scope.my_ext`.
fn nest_extension(fds: &mut FileDescriptorSet) {
    let file = &mut fds.file[1];
    let extension = file.extension.remove(0);
    file.message_type.push(DescriptorProto {
        name: Some("Scope".into()),
        extension: vec![extension],
        ..Default::default()
    });
}

#[test]
fn proto3_extension_targeting_non_options_message_is_rejected() {
    let error = DescriptorPool::new(extension_set("legacy.test", "RegularMessage", "proto3"))
        .expect_err("proto3 extension to non-Options must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field, extendee }
        if field == "ext.test.my_ext" && extendee == "legacy.test.RegularMessage"),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "extension ext.test.my_ext is declared in a proto3 file and extends \
         legacy.test.RegularMessage, which is not an options message"
    );
}

#[test]
fn proto3_nested_extension_targeting_non_options_message_is_rejected() {
    let mut fds = extension_set("legacy.test", "RegularMessage", "proto3");
    nest_extension(&mut fds);
    let error = DescriptorPool::new(fds)
        .expect_err("a nested proto3 extension of a non-options message must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field, .. }
        if field == "ext.test.Scope.my_ext"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_extension_of_a_message_already_in_the_pool_is_rejected_transactionally() {
    let mut fds = extension_set("legacy.test", "RegularMessage", "proto3");
    let extension_file = fds.file.remove(1);
    let mut pool = DescriptorPool::new(fds).unwrap();
    let error = pool
        .add_file_descriptor_set(FileDescriptorSet {
            file: vec![extension_file],
            ..Default::default()
        })
        .expect_err("proto3 extension to non-Options must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field, .. }
        if field == "ext.test.my_ext"),
        "unexpected error: {error:?}"
    );
    assert_eq!(pool.files().len(), 1);
    assert_eq!(pool.extensions().len(), 0);
    assert!(pool.file_by_name("ext.test.proto").is_none());
}

/// protoc accepts the nine options messages under `google.protobuf` and under
/// `proto2`. Each name is declared here as a stand-in so the set does not
/// need `descriptor.proto`.
#[test]
fn proto3_extension_targeting_each_options_message_is_accepted() {
    for package in ["google.protobuf", "proto2"] {
        for message in [
            "FileOptions",
            "MessageOptions",
            "FieldOptions",
            "OneofOptions",
            "EnumOptions",
            "EnumValueOptions",
            "ServiceOptions",
            "MethodOptions",
            "ExtensionRangeOptions",
        ] {
            if let Err(error) = DescriptorPool::new(extension_set(package, message, "proto3")) {
                panic!("{package}.{message}: {error}");
            }
        }
    }
}

#[test]
fn proto3_extension_targeting_non_google_options_namesake_is_rejected() {
    // Ends in `Options` but lives outside `google.protobuf`: not one of the
    // nine extendable messages.
    let error = DescriptorPool::new(extension_set("other.pkg", "MyOptions", "proto3"))
        .expect_err("proto3 extension to non-google Options namesake must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field, extendee }
        if field == "ext.test.my_ext" && extendee == "other.pkg.MyOptions"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto3_extension_targeting_fake_google_options_is_rejected() {
    // Inside `google.protobuf` and ends in `Options`, but not one of the
    // nine real options messages.
    let error = DescriptorPool::new(extension_set("google.protobuf", "FakeOptions", "proto3"))
        .expect_err("proto3 extension to fake Options must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ExtensionTargetNotOptions { field, extendee }
        if field == "ext.test.my_ext" && extendee == "google.protobuf.FakeOptions"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn proto2_extension_targeting_non_options_message_is_accepted() {
    DescriptorPool::new(extension_set("legacy.test", "RegularMessage", "proto2")).unwrap();
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
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 optional closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field, .. } if field == "enum.test.Sample.value"),
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
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 repeated closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field, .. } if field == "enum.test.Sample.values"),
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
    );
    let error =
        DescriptorPool::new(fds).expect_err("proto3 oneof closed enum field must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field, .. } if field == "enum.test.Sample.choice"),
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

/// `kind.proto`, declaring the enum `legacy.test.Kind` in the given syntax.
/// The enum sets no features: in a proto2 file it is closed because of the
/// file's syntax, which is the closed enum that protoc output can carry.
fn enum_file(syntax: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("kind.proto".into()),
        package: Some("legacy.test".into()),
        syntax: Some(syntax.into()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("Kind".into()),
            value: vec![EnumValueDescriptorProto {
                name: Some("UNKNOWN".into()),
                number: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The proto3 file `enum.test.proto`, importing `kind.proto`, whose message
/// `Sample` holds `field`.
fn proto3_file_naming_kind(field: FieldDescriptorProto) -> FileDescriptorProto {
    let oneof_decl = field.oneof_index.map(|_| OneofDescriptorProto {
        name: Some("choice".into()),
        ..Default::default()
    });
    let mut fds = proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![field],
            oneof_decl: oneof_decl.into_iter().collect(),
            ..Default::default()
        }],
        vec![],
    );
    let mut file = fds.file.remove(0);
    file.dependency.push("kind.proto".into());
    file
}

fn repeated_kind_field() -> FieldDescriptorProto {
    FieldDescriptorProto {
        label: Some(Label::LABEL_REPEATED),
        ..enum_field("value", 1, ".legacy.test.Kind")
    }
}

#[test]
fn proto3_fields_naming_a_proto2_enum_are_rejected() {
    let field = || enum_field("value", 1, ".legacy.test.Kind");
    let optional = FieldDescriptorProto {
        proto3_optional: Some(true),
        oneof_index: Some(0),
        ..field()
    };
    let oneof_member = FieldDescriptorProto {
        oneof_index: Some(0),
        ..field()
    };
    for (shape, field) in [
        ("singular", field()),
        ("repeated", repeated_kind_field()),
        ("optional", optional),
        ("oneof member", oneof_member),
    ] {
        // The enum's file follows the file that names it, so the enum links
        // after the field does.
        let fds = FileDescriptorSet {
            file: vec![proto3_file_naming_kind(field), enum_file("proto2")],
            ..Default::default()
        };
        let error = DescriptorPool::new(fds).expect_err(shape);
        assert!(
            matches!(&error, PoolError::Proto3ClosedEnumField { field, enum_name }
            if field == "enum.test.Sample.value" && enum_name == "legacy.test.Kind"),
            "{shape}: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            "field enum.test.Sample.value is declared in a proto3 file and uses the closed enum \
             legacy.test.Kind"
        );
    }
}

/// The pool already holds a proto2 file with a message, so the new proto3
/// message is not the pool's first and its file is not the pool's first.
#[test]
fn proto3_field_naming_an_enum_already_in_the_pool_is_rejected_transactionally() {
    let mut fds = extension_set("legacy.test", "RegularMessage", "proto2");
    fds.file.push(enum_file("proto2"));
    let mut pool = DescriptorPool::new(fds).unwrap();
    let error = pool
        .add_file_descriptor_set(FileDescriptorSet {
            file: vec![proto3_file_naming_kind(repeated_kind_field())],
            ..Default::default()
        })
        .expect_err("a proto3 field naming a closed enum must be rejected");
    assert!(
        matches!(&error, PoolError::Proto3ClosedEnumField { field, .. } if field == "enum.test.Sample.value"),
        "unexpected error: {error:?}"
    );
    assert_eq!(pool.files().len(), 3);
    assert_eq!(pool.messages().len(), 1);
    assert!(pool.file_by_name("enum.test.proto").is_none());
    assert!(pool.message_by_name("enum.test.Sample").is_none());
}

/// The pool's first message is in a proto3 file. A proto2 file added later
/// is held to its own syntax, under which a closed enum field is valid.
#[test]
fn proto2_closed_enum_field_added_after_a_proto3_file_is_accepted() {
    let mut pool = DescriptorPool::new(proto3_file(
        "enum.test",
        vec![DescriptorProto {
            name: Some("Sample".into()),
            field: vec![enum_field("value", 1, ".enum.test.Kind")],
            ..Default::default()
        }],
        vec![enum_with_type(EnumType::OPEN)],
    ))
    .unwrap();
    let mut legacy = enum_file("proto2");
    legacy.message_type.push(DescriptorProto {
        name: Some("Sample".into()),
        field: vec![repeated_kind_field()],
        ..Default::default()
    });
    pool.add_file_descriptor_set(FileDescriptorSet {
        file: vec![legacy],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(pool.files().len(), 2);
}

/// `extension_set` for `google.protobuf.MethodOptions` in a proto3 file, with
/// the extension typed as `legacy.test.Kind`, which `kind.proto` declares in
/// `enum_syntax`. `nested` declares the extension inside the message `Scope`.
fn enum_extension_set(label: Label, nested: bool, enum_syntax: &str) -> FileDescriptorSet {
    let mut fds = extension_set("google.protobuf", "MethodOptions", "proto3");
    fds.file.push(enum_file(enum_syntax));
    let file = &mut fds.file[1];
    file.dependency.push("kind.proto".into());
    let extension = &mut file.extension[0];
    extension.label = Some(label);
    extension.r#type = Some(Type::TYPE_ENUM);
    extension.type_name = Some(".legacy.test.Kind".into());
    if nested {
        nest_extension(&mut fds);
    }
    fds
}

#[test]
fn proto3_extensions_naming_a_proto2_enum_are_rejected() {
    for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
        for (nested, expected) in [(false, "ext.test.my_ext"), (true, "ext.test.Scope.my_ext")] {
            let error = DescriptorPool::new(enum_extension_set(label, nested, "proto2"))
                .expect_err("a proto3 extension naming a closed enum must be rejected");
            assert!(
                matches!(&error, PoolError::Proto3ClosedEnumField { field, enum_name }
                if field == expected && enum_name == "legacy.test.Kind"),
                "{label:?} {expected}: {error:?}"
            );
        }
    }
}

#[test]
fn proto3_extensions_naming_a_proto3_enum_are_accepted() {
    for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
        for nested in [false, true] {
            DescriptorPool::new(enum_extension_set(label, nested, "proto3")).unwrap();
        }
    }
}

/// `features.enum_type = OPEN` on a field or extension makes it open whatever
/// its enum declares, in a proto3 file as elsewhere.
#[test]
fn proto3_field_level_open_enum_override_is_accepted() {
    let open = FieldOptions {
        features: FeatureSet {
            enum_type: Some(EnumType::OPEN),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };

    let field = FieldDescriptorProto {
        options: open.clone().into(),
        ..repeated_kind_field()
    };
    DescriptorPool::new(FileDescriptorSet {
        file: vec![proto3_file_naming_kind(field), enum_file("proto2")],
        ..Default::default()
    })
    .unwrap();

    let mut fds = enum_extension_set(Label::LABEL_OPTIONAL, false, "proto2");
    fds.file[1].extension[0].options = open.into();
    DescriptorPool::new(fds).unwrap();
}

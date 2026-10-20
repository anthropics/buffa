#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet, OneofDescriptorProto,
};
use buffa_descriptor::reflect::{DynamicMessage, ReflectMessage, ValueRef};
use buffa_descriptor::DescriptorPool;

fn field(
    name: &str,
    number: i32,
    ty: Type,
    default: Option<&str>,
    type_name: Option<&str>,
) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(ty),
        default_value: default.map(Into::into),
        type_name: type_name.map(Into::into),
        ..Default::default()
    }
}

fn defaults_pool() -> Arc<DescriptorPool> {
    let nested = DescriptorProto {
        name: Some("Nested".into()),
        field: vec![field("value", 1, Type::TYPE_INT32, Some("23"), None)],
        ..Default::default()
    };
    let message = DescriptorProto {
        name: Some("Defaults".into()),
        field: vec![
            field("double_value", 1, Type::TYPE_DOUBLE, Some("1.25"), None),
            field("float_value", 2, Type::TYPE_FLOAT, Some("inf"), None),
            field(
                "int64_value",
                3,
                Type::TYPE_INT64,
                Some("-9223372036854775808"),
                None,
            ),
            field(
                "uint64_value",
                4,
                Type::TYPE_UINT64,
                Some("18446744073709551615"),
                None,
            ),
            field("int32_value", 5, Type::TYPE_INT32, Some("2147483647"), None),
            field(
                "uint32_value",
                6,
                Type::TYPE_UINT32,
                Some("4294967295"),
                None,
            ),
            field("bool_value", 7, Type::TYPE_BOOL, Some("true"), None),
            field("string_value", 8, Type::TYPE_STRING, Some("declared"), None),
            field(
                "bytes_value",
                9,
                Type::TYPE_BYTES,
                Some(r"\x41\000\377\400\777\x100\x1ff"),
                None,
            ),
            field(
                "enum_value",
                10,
                Type::TYPE_ENUM,
                Some("HIGH"),
                Some(".defaults.test.State"),
            ),
            field(
                "enum_without_default",
                11,
                Type::TYPE_ENUM,
                None,
                Some(".defaults.test.State"),
            ),
            field("empty_string", 12, Type::TYPE_STRING, Some(""), None),
            field("empty_bytes", 13, Type::TYPE_BYTES, Some(""), None),
            field(
                "nested",
                14,
                Type::TYPE_MESSAGE,
                None,
                Some(".defaults.test.Defaults.Nested"),
            ),
            field(
                "negative_infinity",
                15,
                Type::TYPE_DOUBLE,
                Some("-inf"),
                None,
            ),
            field("not_a_number", 16, Type::TYPE_FLOAT, Some("nan"), None),
            field("hex_int32", 17, Type::TYPE_INT32, Some("0x10"), None),
            field("octal_int32", 18, Type::TYPE_INT32, Some("010"), None),
            field(
                "negative_hex_int32",
                19,
                Type::TYPE_INT32,
                Some("-0X10"),
                None,
            ),
            field(
                "hex_uint32",
                20,
                Type::TYPE_UINT32,
                Some("0xffffffff"),
                None,
            ),
            field("octal_uint64", 21, Type::TYPE_UINT64, Some("017"), None),
            field("hex_float", 22, Type::TYPE_FLOAT, Some("0x10"), None),
            field("octal_double", 23, Type::TYPE_DOUBLE, Some("010"), None),
            field(
                "large_hex_double",
                24,
                Type::TYPE_DOUBLE,
                Some("0xffffffffffffffff"),
                None,
            ),
            field("negative_nan", 25, Type::TYPE_FLOAT, Some("-nan"), None),
        ],
        nested_type: vec![nested],
        ..Default::default()
    };
    let file = FileDescriptorProto {
        name: Some("defaults.proto".into()),
        package: Some("defaults.test".into()),
        syntax: Some("proto2".into()),
        message_type: vec![message],
        enum_type: vec![EnumDescriptorProto {
            name: Some("State".into()),
            value: vec![
                EnumValueDescriptorProto {
                    name: Some("LOW".into()),
                    number: Some(0),
                    ..Default::default()
                },
                EnumValueDescriptorProto {
                    name: Some("HIGH".into()),
                    number: Some(7),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    Arc::new(
        DescriptorPool::new(FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        })
        .expect("valid proto2 defaults link"),
    )
}

#[test]
fn dynamic_get_uses_declared_defaults_without_setting_fields() {
    let pool = defaults_pool();
    let message_index = pool.message_index("defaults.test.Defaults").unwrap();
    let descriptor = pool.message_by_name("defaults.test.Defaults").unwrap();
    let message = DynamicMessage::new(Arc::clone(&pool), message_index);

    for number in 1..=25 {
        let field = descriptor.field(number).unwrap();
        assert!(
            !message.has(field),
            "field {} must remain absent",
            field.name()
        );
    }
    assert!(message.encode_to_vec().is_empty());

    assert!(matches!(
        message.get(descriptor.field(1).unwrap()),
        ValueRef::F64(1.25)
    ));
    assert!(
        matches!(message.get(descriptor.field(2).unwrap()), ValueRef::F32(value) if value.is_infinite() && value.is_sign_positive())
    );
    assert!(matches!(
        message.get(descriptor.field(3).unwrap()),
        ValueRef::I64(i64::MIN)
    ));
    assert!(matches!(
        message.get(descriptor.field(4).unwrap()),
        ValueRef::U64(u64::MAX)
    ));
    assert!(matches!(
        message.get(descriptor.field(5).unwrap()),
        ValueRef::I32(i32::MAX)
    ));
    assert!(matches!(
        message.get(descriptor.field(6).unwrap()),
        ValueRef::U32(u32::MAX)
    ));
    assert!(matches!(
        message.get(descriptor.field(7).unwrap()),
        ValueRef::Bool(true)
    ));
    assert!(matches!(
        message.get(descriptor.field(8).unwrap()),
        ValueRef::String("declared")
    ));
    assert!(
        matches!(message.get(descriptor.field(9).unwrap()), ValueRef::Bytes(value) if value == [b'A', 0, 0xff, 0, 0xff, 0x10, b'0', 0x1f, b'f'])
    );
    assert!(matches!(
        message.get(descriptor.field(10).unwrap()),
        ValueRef::EnumNumber(7)
    ));
    assert!(matches!(
        message.get(descriptor.field(11).unwrap()),
        ValueRef::EnumNumber(0)
    ));
    assert!(matches!(
        message.get(descriptor.field(12).unwrap()),
        ValueRef::String("")
    ));
    assert!(
        matches!(message.get(descriptor.field(13).unwrap()), ValueRef::Bytes(value) if value.is_empty())
    );
    assert!(
        matches!(message.get(descriptor.field(15).unwrap()), ValueRef::F64(value) if value.is_infinite() && value.is_sign_negative())
    );
    assert!(
        matches!(message.get(descriptor.field(16).unwrap()), ValueRef::F32(value) if value.is_nan())
    );
    assert!(matches!(
        message.get(descriptor.field(17).unwrap()),
        ValueRef::I32(16)
    ));
    assert!(matches!(
        message.get(descriptor.field(18).unwrap()),
        ValueRef::I32(8)
    ));
    assert!(matches!(
        message.get(descriptor.field(19).unwrap()),
        ValueRef::I32(-16)
    ));
    assert!(matches!(
        message.get(descriptor.field(20).unwrap()),
        ValueRef::U32(u32::MAX)
    ));
    assert!(matches!(
        message.get(descriptor.field(21).unwrap()),
        ValueRef::U64(15)
    ));
    assert!(matches!(
        message.get(descriptor.field(22).unwrap()),
        ValueRef::F32(16.0)
    ));
    assert!(matches!(
        message.get(descriptor.field(23).unwrap()),
        ValueRef::F64(8.0)
    ));
    assert!(matches!(
        message.get(descriptor.field(24).unwrap()),
        ValueRef::F64(18446744073709551616.0)
    ));
    assert!(
        matches!(message.get(descriptor.field(25).unwrap()), ValueRef::F32(value) if value.is_nan())
    );

    // The default is also visible through an absent nested-message view.
    let ValueRef::Message(nested) = message.get(descriptor.field(14).unwrap()) else {
        panic!("nested message default missing");
    };
    let nested_descriptor = pool
        .message_by_name("defaults.test.Defaults.Nested")
        .unwrap();
    assert!(matches!(
        nested.get(nested_descriptor.field(1).unwrap()),
        ValueRef::I32(23)
    ));
}

#[test]
fn dynamic_get_uses_declared_defaults_for_extensions() {
    let file = FileDescriptorProto {
        name: Some("extensions.proto".into()),
        package: Some("defaults.test".into()),
        syntax: Some("proto2".into()),
        message_type: vec![DescriptorProto {
            name: Some("Host".into()),
            extension_range: vec![ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            }],
            ..Default::default()
        }],
        extension: vec![FieldDescriptorProto {
            extendee: Some(".defaults.test.Host".into()),
            ..field(
                "label",
                100,
                Type::TYPE_STRING,
                Some("extension default"),
                None,
            )
        }],
        ..Default::default()
    };
    let pool = Arc::new(
        DescriptorPool::new(FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        })
        .expect("valid proto2 extension default links"),
    );
    let message_index = pool.message_index("defaults.test.Host").unwrap();
    let extension = pool.extension_by_name("defaults.test.label").unwrap();
    let message = DynamicMessage::new(Arc::clone(&pool), message_index);

    assert!(matches!(
        message.get(extension.field()),
        ValueRef::String("extension default")
    ));
    assert!(!message.has(extension.field()));
    assert!(message.encode_to_vec().is_empty());
}

#[test]
fn pool_rejects_unparseable_explicit_defaults() {
    for (ty, default, type_name) in [
        (Type::TYPE_INT32, "not an integer", None),
        (Type::TYPE_INT32, "+10", None),
        (Type::TYPE_INT32, "2147483648", None),
        (Type::TYPE_INT32, "-0x80000001", None),
        (Type::TYPE_INT32, "08", None),
        (Type::TYPE_INT32, "0x", None),
        (Type::TYPE_UINT32, "-1", None),
        (Type::TYPE_UINT32, "0x100000000", None),
        (Type::TYPE_UINT64, "18446744073709551616", None),
        (Type::TYPE_UINT64, "-0", None),
        (Type::TYPE_BOOL, "yes", None),
        (Type::TYPE_DOUBLE, "+010", None),
        (Type::TYPE_DOUBLE, "+1.5", None),
        (Type::TYPE_DOUBLE, "infinity", None),
        (Type::TYPE_DOUBLE, "-infinity", None),
        (Type::TYPE_DOUBLE, "INF", None),
        (Type::TYPE_DOUBLE, "NaN", None),
        (Type::TYPE_BYTES, r"\xgg", None),
        (Type::TYPE_BYTES, "\\", None),
        (Type::TYPE_ENUM, "MISSING", Some(".defaults.test.State")),
    ] {
        let descriptor = FileDescriptorProto {
            name: Some("invalid-default.proto".into()),
            package: Some("defaults.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("Message".into()),
                field: vec![field("value", 1, ty, Some(default), type_name)],
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("State".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("KNOWN".into()),
                    number: Some(0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let error = DescriptorPool::new(FileDescriptorSet {
            file: vec![descriptor],
            ..Default::default()
        })
        .unwrap_err();
        assert!(
            matches!(
                error,
                buffa_descriptor::PoolError::InvalidFieldDefault { .. }
            ),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn dynamic_get_uses_declared_default_for_an_unset_oneof_member() {
    let mut value = field("value", 1, Type::TYPE_INT32, Some("23"), None);
    value.oneof_index = Some(0);
    let descriptor = FileDescriptorProto {
        name: Some("oneof-default.proto".into()),
        package: Some("defaults.test".into()),
        syntax: Some("proto2".into()),
        message_type: vec![DescriptorProto {
            name: Some("Message".into()),
            field: vec![value],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("choice".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let pool = Arc::new(
        DescriptorPool::new(FileDescriptorSet {
            file: vec![descriptor],
            ..Default::default()
        })
        .expect("proto2 oneof defaults are supported"),
    );
    let message_index = pool.message_index("defaults.test.Message").unwrap();
    let field = pool.message(message_index).field(1).unwrap();
    let message = DynamicMessage::new(Arc::clone(&pool), message_index);

    assert!(matches!(message.get(field), ValueRef::I32(23)));
    assert!(!message.has(field));
    assert!(message.encode_to_vec().is_empty());
}

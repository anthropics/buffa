//! Declared defaults are parsed and validated when the pool links, and the
//! typed value is exposed on `FieldDescriptor::default_value`.
//!
//! protoc rejects a default that does not parse for the field's type
//! ("Couldn't parse default value"), and protobuf-go does too ("has invalid
//! default"); before this change the pool only checked whether a default
//! was present, never what it said.

#![cfg(feature = "reflect")]

use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::feature_set::FieldPresence;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, EnumDescriptorProto, EnumValueDescriptorProto, FeatureSet,
    FieldDescriptorProto, FieldOptions, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::{DefaultValue, DescriptorPool, PoolError, Value};

fn field(
    name: &str,
    number: i32,
    ty: Type,
    type_name: Option<&str>,
    default: Option<&str>,
) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(ty),
        type_name: type_name.map(Into::into),
        default_value: default.map(Into::into),
        ..Default::default()
    }
}

fn color_enum() -> EnumDescriptorProto {
    EnumDescriptorProto {
        name: Some("Color".into()),
        value: vec![
            EnumValueDescriptorProto {
                name: Some("RED".into()),
                number: Some(1),
                ..Default::default()
            },
            EnumValueDescriptorProto {
                name: Some("BLUE".into()),
                number: Some(2),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn link(file: FileDescriptorProto) -> DescriptorPool {
    DescriptorPool::new(FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    })
    .expect("descriptor set links")
}

fn proto2_file(fields: Vec<FieldDescriptorProto>) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("t.proto".into()),
        package: Some("t".into()),
        syntax: Some("proto2".into()),
        message_type: vec![DescriptorProto {
            name: Some("M".into()),
            field: fields,
            ..Default::default()
        }],
        enum_type: vec![color_enum()],
        ..Default::default()
    }
}

fn default_of(pool: &DescriptorPool, number: u32) -> Option<DefaultValue> {
    pool.message_by_name("t.M")
        .expect("message links")
        .field(number)
        .expect("field exists")
        .default_value()
        .cloned()
}

#[test]
fn valid_defaults_are_parsed_and_exposed() {
    let pool = link(proto2_file(vec![
        field("n", 1, Type::TYPE_INT32, None, Some("7")),
        field("big", 2, Type::TYPE_INT64, None, Some("-9000000000")),
        field("u", 3, Type::TYPE_UINT32, None, Some("42")),
        field(
            "uu",
            4,
            Type::TYPE_UINT64,
            None,
            Some("18446744073709551615"),
        ),
        field("s", 5, Type::TYPE_SINT32, None, Some("-3")),
        field("sf", 6, Type::TYPE_SFIXED64, None, Some("1234567890123")),
        field("fx", 7, Type::TYPE_FIXED32, None, Some("9")),
        field("fl", 8, Type::TYPE_FLOAT, None, Some("1.5")),
        field("d", 9, Type::TYPE_DOUBLE, None, Some("3.25")),
        field("neg_inf", 10, Type::TYPE_DOUBLE, None, Some("-inf")),
        field("b", 11, Type::TYPE_BOOL, None, Some("true")),
        field("st", 12, Type::TYPE_STRING, None, Some("hello")),
        field("empty_st", 13, Type::TYPE_STRING, None, Some("")),
        field("by", 14, Type::TYPE_BYTES, None, Some("\\x01\\002\\nA")),
        field("empty_by", 15, Type::TYPE_BYTES, None, Some("")),
        field("color", 16, Type::TYPE_ENUM, Some(".t.Color"), Some("BLUE")),
        field("plain", 17, Type::TYPE_INT32, None, None),
    ]));
    assert_eq!(default_of(&pool, 1), Some(DefaultValue::I32(7)));
    assert_eq!(
        default_of(&pool, 2),
        Some(DefaultValue::I64(-9_000_000_000))
    );
    assert_eq!(default_of(&pool, 3), Some(DefaultValue::U32(42)));
    assert_eq!(default_of(&pool, 4), Some(DefaultValue::U64(u64::MAX)));
    assert_eq!(default_of(&pool, 5), Some(DefaultValue::I32(-3)));
    assert_eq!(
        default_of(&pool, 6),
        Some(DefaultValue::I64(1_234_567_890_123))
    );
    assert_eq!(default_of(&pool, 7), Some(DefaultValue::U32(9)));
    assert_eq!(default_of(&pool, 8), Some(DefaultValue::F32(1.5)));
    assert_eq!(default_of(&pool, 9), Some(DefaultValue::F64(3.25)));
    assert_eq!(
        default_of(&pool, 10),
        Some(DefaultValue::F64(f64::NEG_INFINITY))
    );
    assert_eq!(default_of(&pool, 11), Some(DefaultValue::Bool(true)));
    assert_eq!(
        default_of(&pool, 12),
        Some(DefaultValue::String("hello".into()))
    );
    assert_eq!(
        default_of(&pool, 13),
        Some(DefaultValue::String(String::new()))
    );
    assert_eq!(
        default_of(&pool, 14),
        Some(DefaultValue::Bytes(vec![0x01, 0x02, 0x0A, b'A']))
    );
    assert_eq!(default_of(&pool, 15), Some(DefaultValue::Bytes(Vec::new())));
    assert_eq!(default_of(&pool, 16), Some(DefaultValue::EnumNumber(2)));
    assert_eq!(default_of(&pool, 17), None);
}

#[test]
fn nan_and_required_defaults_are_exposed() {
    let mut required = field("req", 2, Type::TYPE_INT32, None, Some("3"));
    required.label = Some(Label::LABEL_REQUIRED);
    let pool = link(proto2_file(vec![
        field("nan_f", 1, Type::TYPE_FLOAT, None, Some("nan")),
        required,
    ]));
    match default_of(&pool, 1) {
        Some(DefaultValue::F32(v)) => assert!(v.is_nan()),
        other => panic!("expected an f32 NaN default, got {other:?}"),
    }
    assert_eq!(default_of(&pool, 2), Some(DefaultValue::I32(3)));
}

#[test]
fn extension_defaults_are_parsed_and_exposed() {
    let mut file = proto2_file(vec![]);
    file.message_type[0].extension_range = vec![ExtensionRange {
        start: Some(100),
        end: Some(200),
        ..Default::default()
    }];
    let mut ext = field("ext_n", 100, Type::TYPE_INT32, None, Some("11"));
    ext.extendee = Some(".t.M".into());
    file.extension = vec![ext];
    let pool = link(file);
    let extension = pool.extension_by_name("t.ext_n").expect("extension links");
    assert_eq!(
        extension.field().default_value(),
        Some(&DefaultValue::I32(11))
    );
}

#[test]
fn editions_explicit_presence_defaults_are_parsed() {
    let mut with_presence = field("value", 1, Type::TYPE_INT32, None, Some("7"));
    with_presence.options = FieldOptions {
        features: FeatureSet {
            field_presence: Some(FieldPresence::EXPLICIT),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();
    let mut file = proto2_file(vec![with_presence]);
    file.enum_type = vec![];
    file.syntax = Some("editions".into());
    file.edition = Some(Edition::EDITION_2023);
    let pool = link(file);
    assert_eq!(default_of(&pool, 1), Some(DefaultValue::I32(7)));
}

#[test]
fn invalid_defaults_are_rejected_at_link() {
    for (ty, type_name, raw) in [
        (Type::TYPE_INT32, None, "seven"),
        (Type::TYPE_INT32, None, "2147483648"),
        (Type::TYPE_INT32, None, ""),
        (Type::TYPE_INT64, None, "9223372036854775808"),
        (Type::TYPE_UINT32, None, "-1"),
        (Type::TYPE_UINT64, None, "1e3"),
        (Type::TYPE_BOOL, None, "yes"),
        (Type::TYPE_BOOL, None, "1"),
        (Type::TYPE_DOUBLE, None, "abc"),
        (Type::TYPE_FLOAT, None, "1.2.3"),
        (Type::TYPE_BYTES, None, "\\x"),
        (Type::TYPE_BYTES, None, "\\q"),
        (Type::TYPE_BYTES, None, "\\777"),
        (Type::TYPE_BYTES, None, "\\x100"),
        (Type::TYPE_ENUM, Some(".t.Color"), "GREEN"),
    ] {
        let set = FileDescriptorSet {
            file: vec![proto2_file(vec![field(
                "value",
                1,
                ty,
                type_name,
                Some(raw),
            )])],
            ..Default::default()
        };
        let err = DescriptorPool::new(set).unwrap_err();
        match &err {
            PoolError::InvalidDefaultValue {
                field,
                value,
                reason,
            } => {
                assert_eq!(field, "t.M.value", "{ty:?} {raw:?}");
                assert_eq!(value, raw, "{ty:?}");
                assert!(!reason.is_empty(), "{ty:?} {raw:?}");
            }
            other => panic!("{ty:?} {raw:?}: expected InvalidDefaultValue, got {other:?}"),
        }
        assert!(
            err.to_string().contains("does not parse"),
            "{ty:?} {raw:?}: {err}"
        );
    }
}

#[test]
fn presence_errors_still_win_over_parse_errors() {
    // A proto3 field with an unparseable default reports the proto3 rule,
    // not the parse failure: the parse runs only once a default is allowed.
    let mut proto3 = proto2_file(vec![field(
        "value",
        1,
        Type::TYPE_INT32,
        None,
        Some("seven"),
    )]);
    proto3.syntax = Some("proto3".into());
    let err = DescriptorPool::new(FileDescriptorSet {
        file: vec![proto3],
        ..Default::default()
    })
    .unwrap_err();
    assert!(
        matches!(&err, PoolError::Proto3FieldWithDefault { .. }),
        "got {err:?}"
    );

    let mut repeated = field("value", 1, Type::TYPE_INT32, None, Some("seven"));
    repeated.label = Some(Label::LABEL_REPEATED);
    let err = DescriptorPool::new(FileDescriptorSet {
        file: vec![proto2_file(vec![repeated])],
        ..Default::default()
    })
    .unwrap_err();
    assert!(
        matches!(&err, PoolError::RepeatedFieldWithDefault { .. }),
        "got {err:?}"
    );
}

#[test]
fn declared_defaults_convert_to_reflect_values() {
    let pool = link(proto2_file(vec![
        field("n", 1, Type::TYPE_INT32, None, Some("7")),
        field("by", 2, Type::TYPE_BYTES, None, Some("\\x01")),
        field("color", 3, Type::TYPE_ENUM, Some(".t.Color"), Some("BLUE")),
    ]));
    let message = pool.message_by_name("t.M").expect("message links");
    let convert = |number: u32| {
        Value::from(
            message
                .field(number)
                .expect("field exists")
                .default_value()
                .expect("default declared"),
        )
    };
    assert_eq!(convert(1), Value::I32(7));
    assert_eq!(convert(2), Value::Bytes(vec![0x01]));
    assert_eq!(convert(3), Value::EnumNumber(2));
}

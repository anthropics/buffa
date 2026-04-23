//! JSON/serde codegen: custom derive, field attrs, enum proto-name handling,
//! oneof flatten + null/duplicate handling.

use super::*;

// ── JSON codegen tests ─────────────────────────────────────────────────

fn json_config() -> CodeGenConfig {
    CodeGenConfig {
        generate_json: true,
        generate_views: false,
        ..CodeGenConfig::default()
    }
}

#[test]
fn test_json_enum_has_custom_impls_and_from_proto_name() {
    let mut file = proto3_file("color.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![
            enum_value("RED", 0),
            enum_value("GREEN", 1),
            enum_value("BLUE", 2),
        ],
        ..Default::default()
    });
    let files =
        generate(&[file], &["color.proto".to_string()], &json_config()).expect("should generate");
    let content = &joined(&files);
    // Custom Serialize impl uses proto_name
    assert!(
        content.contains("impl ::serde::Serialize for Color"),
        "missing custom Serialize impl on enum: {content}"
    );
    // Custom Deserialize impl
    assert!(
        content.contains("impl<'de> ::serde::Deserialize<'de> for Color"),
        "missing custom Deserialize impl on enum: {content}"
    );
    assert!(
        content.contains("fn from_proto_name"),
        "missing from_proto_name impl: {content}"
    );
    assert!(
        content.contains(r#""RED" => ::core::option::Option::Some(Self::RED)"#),
        "missing RED arm in from_proto_name: {content}"
    );
}

#[test]
fn test_json_enum_alias_in_from_proto_name() {
    let mut file = proto3_file("status.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![
            enum_value("UNKNOWN", 0),
            enum_value("STARTED", 1),
            enum_value("RUNNING", 1), // alias for STARTED
        ],
        options: (crate::generated::descriptor::EnumOptions {
            allow_alias: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });
    let files =
        generate(&[file], &["status.proto".to_string()], &json_config()).expect("should generate");
    let content = &joined(&files);
    // Primary name must be in from_proto_name
    assert!(
        content.contains(r#""STARTED" => ::core::option::Option::Some(Self::STARTED)"#),
        "missing STARTED arm: {content}"
    );
    // Alias name must also map to the primary variant
    assert!(
        content.contains(r#""RUNNING" => ::core::option::Option::Some(Self::STARTED)"#),
        "missing RUNNING alias arm: {content}"
    );
}

#[test]
fn test_json_message_has_derive_and_field_attrs() {
    let mut file = proto3_file("scalars_json.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("count".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                json_name: Some("count".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("big_num".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT64),
                json_name: Some("bigNum".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("data".to_string()),
                number: Some(3),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_BYTES),
                json_name: Some("data".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("ratio".to_string()),
                number: Some(4),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_FLOAT),
                json_name: Some("ratio".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    });

    let files = generate(&[file], &["scalars_json.proto".to_string()], &json_config())
        .expect("should generate");
    let content = &joined(&files);
    // Struct gets serde derive and default
    assert!(
        content.contains("derive(::serde::Serialize, ::serde::Deserialize)"),
        "missing serde derive on struct: {content}"
    );
    assert!(
        content.contains("serde(default)"),
        "missing #[serde(default)] on struct: {content}"
    );
    // i32 field: rename + skip_serializing_if
    assert!(
        content.contains(r#"rename = "count""#),
        "missing rename for count: {content}"
    );
    assert!(
        content.contains("is_zero_i32"),
        "missing skip_serializing_if for count: {content}"
    );
    // i64 field: rename + with + skip_serializing_if
    assert!(
        content.contains(r#"with = "::buffa::json_helpers::int64""#),
        "missing int64 with attr: {content}"
    );
    assert!(
        content.contains("is_zero_i64"),
        "missing skip_serializing_if for bigNum: {content}"
    );
    // bytes field: rename + with + skip_serializing_if
    assert!(
        content.contains(r#"with = "::buffa::json_helpers::bytes""#),
        "missing bytes with attr: {content}"
    );
    assert!(
        content.contains("is_empty_bytes"),
        "missing skip_serializing_if for data: {content}"
    );
    // float field: rename + with + skip_serializing_if
    assert!(
        content.contains(r#"with = "::buffa::json_helpers::float""#),
        "missing float with attr: {content}"
    );
    assert!(
        content.contains("is_zero_f32"),
        "missing skip_serializing_if for ratio: {content}"
    );
    // cached_size gets skip
    assert!(
        content.contains("serde(skip)"),
        "missing serde(skip) for cached_size: {content}"
    );
}

#[test]
fn test_json_oneof_field_is_flattened() {
    let mut file = proto3_file("oneof_json.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOneof".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("count".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_INT32),
            oneof_index: Some(0),
            json_name: Some("count".to_string()),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(&[file], &["oneof_json.proto".to_string()], &json_config())
        .expect("should generate");
    let content = &joined(&files);
    // The oneof field uses flatten so its variants appear as top-level JSON fields.
    assert!(
        content.contains("serde(flatten)"),
        "oneof field must have serde(flatten): {content}"
    );
    // The oneof enum must have a custom Serialize impl.
    assert!(
        content.contains("impl serde::Serialize for Kind"),
        "oneof enum must have Serialize impl: {content}"
    );
}

#[test]
fn test_json_oneof_deserialize_null_and_duplicate_handling() {
    let mut file = proto3_file("oneof_deser.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOneof".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("count".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                json_name: Some("count".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("name".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                json_name: Some("name".to_string()),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(&[file], &["oneof_deser.proto".to_string()], &json_config())
        .expect("should generate");
    let content = &joined(&files);

    // Null handling: each arm wraps with NullableDeserializeSeed
    assert!(
        content.contains("NullableDeserializeSeed"),
        "oneof deserialize must use NullableDeserializeSeed: {content}"
    );

    // Duplicate detection: check for is_some() guard on the oneof variable
    assert!(
        content.contains("__oneof_kind.is_some()"),
        "oneof deserialize must check for duplicate fields: {content}"
    );

    // Helper-using type (int32) should define _DeserSeed struct
    assert!(
        content.contains("struct _DeserSeed"),
        "helper-using variant must define _DeserSeed: {content}"
    );

    // Custom Deserialize is on the message, not the oneof enum
    assert!(
        content.contains("Deserialize<'de> for WithOneof"),
        "message must have custom Deserialize impl: {content}"
    );
    assert!(
        !content.contains("Deserialize<'de> for Kind"),
        "oneof enum must NOT have Deserialize impl: {content}"
    );

    // Default-serde type (string) should use DefaultDeserializeSeed
    assert!(
        content.contains("DefaultDeserializeSeed"),
        "default-serde variant must use DefaultDeserializeSeed: {content}"
    );
}

#[test]
fn test_json_oneof_value_variant_forwards_null() {
    // Regression: google.protobuf.Value-typed oneof variants must NOT use
    // NullableDeserializeSeed (which treats JSON null as "variant absent").
    // JSON `null` is a VALID value for Value (it means Kind::NullValue),
    // so null must be forwarded to Value::deserialize.
    let mut file = proto3_file("value_oneof.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Wrapper".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("text".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("meta".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".google.protobuf.Value".to_string()),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("payload".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    // Need a dummy Value message in the descriptor set so type_name resolves.
    let mut value_file = proto3_file("google/protobuf/struct.proto");
    value_file.package = Some("google.protobuf".to_string());
    value_file.message_type.push(DescriptorProto {
        name: Some("Value".to_string()),
        ..Default::default()
    });

    let files = generate(
        &[value_file, file],
        &["value_oneof.proto".to_string()],
        &json_config(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // The `meta` (Value) arm should use DefaultDeserializeSeed directly
    // (forward null), not NullableDeserializeSeed (intercept null).
    // Look for the meta match arm pattern: it should NOT be nullable.
    // The `text` arm is a string type — it WILL use NullableDeserializeSeed.
    // So: count NullableDeserializeSeed uses. Should be 1 (text), not 2.
    let nullable_count = content.matches("NullableDeserializeSeed").count();
    assert_eq!(
        nullable_count, 1,
        "Value variant must NOT use NullableDeserializeSeed (only text should): {content}"
    );
}

#[test]
fn test_no_serde_attrs_without_generate_json_flag() {
    let mut file = proto3_file("plain.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "big_num",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT64,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["plain.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        !content.contains("serde"),
        "serde attrs must be absent without generate_json: {content}"
    );
}

// ── register_types / Any entry emission ──────────────────────────────────

#[test]
fn test_json_any_const_emitted_per_message() {
    let mut file = proto3_file("any_entry.proto");
    file.package = Some("acme".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Widget".to_string()),
        ..Default::default()
    });
    let files = generate(&[file], &["any_entry.proto".to_string()], &json_config())
        .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub const __WIDGET_JSON_ANY: ::buffa::type_registry::JsonAnyEntry"),
        "missing JSON Any const: {content}"
    );
    assert!(
        content.contains(r#"type_url: "type.googleapis.com/acme.Widget""#),
        "wrong type_url: {content}"
    );
    assert!(
        content.contains("::buffa::type_registry::any_to_json::<Widget>"),
        "missing any_to_json fn pointer: {content}"
    );
    assert!(
        content.contains("::buffa::type_registry::any_from_json::<Widget>"),
        "missing any_from_json fn pointer: {content}"
    );
    assert!(
        content.contains("is_wkt: false"),
        "user messages must emit is_wkt: false: {content}"
    );
    // json_config has generate_text off — no TEXT_ANY const.
    assert!(
        !content.contains("__WIDGET_TEXT_ANY"),
        "TEXT_ANY must be absent with generate_text off: {content}"
    );
}

#[test]
fn test_register_types_emitted_with_json_any_only() {
    // A file with messages but no extensions still emits register_types
    // (Any-only). Proto3 has no extension syntax, so this is the common case.
    let mut file = proto3_file("reg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Foo".to_string()),
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Bar".to_string()),
        ..Default::default()
    });
    let files =
        generate(&[file], &["reg.proto".to_string()], &json_config()).expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub fn register_types(reg: &mut ::buffa::type_registry::TypeRegistry)"),
        "missing register_types fn: {content}"
    );
    assert!(
        content.contains("reg.register_json_any(super::__FOO_JSON_ANY)"),
        "missing Foo JSON Any registration: {content}"
    );
    assert!(
        content.contains("reg.register_json_any(super::__BAR_JSON_ANY)"),
        "missing Bar JSON Any registration: {content}"
    );
    // No generate_text → no register_text_* calls in the body.
    assert!(
        !content.contains("register_text_any"),
        "register_text_any must be absent without generate_text: {content}"
    );
}

#[test]
fn test_register_types_includes_nested_message_any_entries() {
    // Nested message Any consts live inside `pub mod outer`; register_types
    // must qualify them as `outer::__INNER_JSON_ANY`.
    let mut file = proto3_file("nested_any.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(&[file], &["nested_any.proto".to_string()], &json_config())
        .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("reg.register_json_any(super::__OUTER_JSON_ANY)"),
        "missing top-level Outer: {content}"
    );
    assert!(
        content.contains("reg.register_json_any(super::outer::__INNER_JSON_ANY)"),
        "missing nested Inner path: {content}"
    );
}

#[test]
fn test_any_entry_not_emitted_without_generate_json_or_text() {
    let mut file = proto3_file("noany.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["noany.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        !content.contains("_JSON_ANY") && !content.contains("_TEXT_ANY"),
        "Any consts must be absent: {content}"
    );
    assert!(
        !content.contains("register_types"),
        "register_types must be absent: {content}"
    );
}

#[test]
fn test_text_any_emitted_independent_of_json() {
    // generate_text on, generate_json OFF. This is the decoupling point:
    // __MSG_TEXT_ANY is emitted, __MSG_JSON_ANY is not, and register_types
    // calls only register_text_any.
    let mut file = proto3_file("textonly.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        ..Default::default()
    });
    let cfg = CodeGenConfig {
        generate_text: true,
        ..Default::default()
    };
    let files = generate(&[file], &["textonly.proto".to_string()], &cfg).expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub const __MSG_TEXT_ANY: ::buffa::type_registry::TextAnyEntry"),
        "missing TEXT_ANY const: {content}"
    );
    assert!(
        content.contains("::buffa::type_registry::any_encode_text::<Msg>"),
        "missing any_encode_text fn pointer: {content}"
    );
    assert!(
        !content.contains("__MSG_JSON_ANY"),
        "JSON_ANY must be absent with generate_json off: {content}"
    );
    assert!(
        content.contains("reg.register_text_any(super::__MSG_TEXT_ANY)"),
        "missing register_text_any call: {content}"
    );
    assert!(
        !content.contains("register_json_any"),
        "register_json_any must be absent: {content}"
    );
}

#[test]
fn message_named_result_does_not_shadow_std_result_in_serde() {
    // A proto message named "Result" inside another message creates a
    // `pub mod parent { pub struct Result { ... } }` which shadows
    // `std::result::Result`. The generated serde Deserialize impl must use
    // `::core::result::Result` to avoid the conflict.
    let result_msg = DescriptorProto {
        name: Some("Result".into()),
        field: vec![make_field(
            "value",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT64,
        )],
        ..Default::default()
    };
    let parent = DescriptorProto {
        name: Some("ParseJob".into()),
        nested_type: vec![result_msg],
        ..Default::default()
    };
    let mut file = proto3_file("job.proto");
    file.package = Some("pkg".into());
    file.message_type.push(parent);

    let files =
        generate(&[file], &["job.proto".to_string()], &json_config()).expect("should generate");

    let content = &joined(&files);

    // The custom Deserialize impl for Result must use ::core::result::Result,
    // not bare `Result` which would resolve to the proto message type.
    assert!(
        !content.contains("fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self"),
        "serde Deserialize must not use bare `Result<Self, ...>` — it shadows \
         the proto message named Result.\nGenerated code:\n{content}"
    );
    assert!(
        content.contains("::core::result::Result<Self"),
        "serde Deserialize should use ::core::result::Result.\nGenerated code:\n{content}"
    );
}

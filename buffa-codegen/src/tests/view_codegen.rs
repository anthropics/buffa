//! View type codegen: struct fields, repeated views, oneof views.

use super::*;

// -----------------------------------------------------------------------
// View codegen tests
// -----------------------------------------------------------------------

#[test]
fn test_view_explicit_presence_scalar_is_option() {
    // proto3 optional: synthetic oneof wrapping a single field.
    let mut file = proto3_file("opt_scalar.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("value".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_INT32),
            proto3_optional: Some(true),
            oneof_index: Some(0),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_value".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["opt_scalar.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // View struct field should be Option<i32>.
    assert!(
        content.contains("pub value: ::core::option::Option<i32>"),
        "view field for proto3 optional i32 must be ::core::option::Option<i32>: {content}"
    );
    // The synthetic oneof must not produce a view enum (it only wraps one field).
    // No `_ValueView` enum should appear.
    assert!(
        !content.contains("pub enum ValueView"),
        "synthetic oneof must not produce a view enum: {content}"
    );
}

#[test]
fn test_view_repeated_message_field() {
    let mut file = proto3_file("rep_msg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Item".to_string()),
        field: vec![make_field(
            "val",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Container".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("items".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".Item".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["rep_msg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // Both Item and Container views should be generated.
    assert!(
        content.contains("pub struct ItemView"),
        "missing ItemView: {content}"
    );
    assert!(
        content.contains("pub struct ContainerView"),
        "missing ContainerView: {content}"
    );
    // The items field on ContainerView must be RepeatedView<'_, ItemView<'_>>.
    assert!(
        content.contains("RepeatedView") && content.contains("ItemView"),
        "ContainerView.items must be RepeatedView<ItemView>: {content}"
    );
    // _decode_ctx must be generated for both view types.
    assert!(
        content.contains("fn _decode_ctx"),
        "missing _decode_ctx impl: {content}"
    );
}

#[test]
fn test_view_packed_scalar_reserves_capacity() {
    let mut file = proto3_file("packed_view.proto");
    file.message_type.push(DescriptorProto {
        name: Some("PackedView".to_string()),
        field: vec![
            // varint kinds: divisor = 1 (payload.len() is an upper bound)
            make_field("ids", 1, Label::LABEL_REPEATED, Type::TYPE_UINT32),
            make_field("flags", 2, Label::LABEL_REPEATED, Type::TYPE_BOOL),
            // 4-byte fixed kinds: divisor = 4
            make_field("ratios", 3, Label::LABEL_REPEATED, Type::TYPE_FLOAT),
            make_field("hashes", 4, Label::LABEL_REPEATED, Type::TYPE_FIXED32),
            // 8-byte fixed kinds: divisor = 8
            make_field("scores", 5, Label::LABEL_REPEATED, Type::TYPE_DOUBLE),
            make_field("offsets", 6, Label::LABEL_REPEATED, Type::TYPE_SFIXED64),
            // Non-packable repeated: must NOT emit a packed reserve(...) call.
            make_field("names", 7, Label::LABEL_REPEATED, Type::TYPE_STRING),
        ],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["packed_view.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // Varint kinds reserve payload.len() (upper bound: ≥1 byte/element).
    assert!(
        content.contains("view.ids.reserve(payload.len());"),
        "varint packed view must reserve using the payload length: {content}"
    );
    assert!(
        content.contains("view.flags.reserve(payload.len());"),
        "bool packed view must reserve using the payload length: {content}"
    );
    // 4-byte fixed kinds reserve payload.len() / 4.
    assert!(
        content.contains("view.ratios.reserve(payload.len() / 4usize);"),
        "float packed view must reserve the exact element count: {content}"
    );
    assert!(
        content.contains("view.hashes.reserve(payload.len() / 4usize);"),
        "fixed32 packed view must reserve the exact element count: {content}"
    );
    // 8-byte fixed kinds reserve payload.len() / 8.
    assert!(
        content.contains("view.scores.reserve(payload.len() / 8usize);"),
        "double packed view must reserve the exact element count: {content}"
    );
    assert!(
        content.contains("view.offsets.reserve(payload.len() / 8usize);"),
        "sfixed64 packed view must reserve the exact element count: {content}"
    );
    // Non-packable repeated types (string/bytes/message) must not emit
    // a packed-reserve call — there is no packed wire payload for them.
    assert!(
        !content.contains("view.names.reserve("),
        "string repeated view must not emit a packed-reserve call: {content}"
    );
}

#[test]
fn test_view_oneof_with_message_variant() {
    let mut file = proto3_file("oneof_msg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Body".to_string()),
        field: vec![make_field(
            "data",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("count".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("body".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".Body".to_string()),
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
    let files = generate(
        &[file],
        &["oneof_msg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // View struct must reference its view-oneof enum at the sentinel path.
    // (Prettyplease may wrap the path across lines, so check the
    // tail segment.)
    assert!(
        content.contains("__buffa::view::oneof::request::Payload"),
        "RequestView must reference __buffa::view::oneof::request::Payload: {content}"
    );
    // The oneof view enum must have both variants.
    assert!(
        content.contains("Count(i32)"),
        "Payload view must have Count(i32): {content}"
    );
    assert!(
        content.contains("BodyView") && content.contains("::buffa::alloc::boxed::Box<"),
        "Payload view must have boxed BodyView variant: {content}"
    );
    // Decode arm for the message variant must consume one recursion level.
    assert!(
        content.contains("ctx.descend()?"),
        "message-type oneof variant must check recursion depth: {content}"
    );
}

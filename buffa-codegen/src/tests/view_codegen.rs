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
    let content = crate::tests::all_content(&files);
    // View struct field should be Option<i32>.
    assert!(
        content.contains("pub value: Option<i32>"),
        "view field for proto3 optional i32 must be Option<i32>: {content}"
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
    let content = crate::tests::all_content(&files);
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
    // _decode_depth must be generated for both view types.
    assert!(
        content.contains("fn _decode_depth"),
        "missing _decode_depth impl: {content}"
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
    let content = crate::tests::all_content(&files);
    // View struct `RequestView` (inside `view::`) has `payload` field
    // pointing at its view-of-oneof enum `Payload` (no View suffix —
    // the `view::oneofs::` tree prefix disambiguates). From inside
    // `view::RequestView` (depth 1) the enum lives at
    // `view::oneofs::request::Payload`, reached as `oneofs::request::Payload`.
    assert!(
        content.contains("pub payload: ::core::option::Option<oneofs::request::Payload"),
        "RequestView must have payload: ::core::option::Option<oneofs::request::Payload>: {content}"
    );
    // The view-of-oneof enum (inside `view::oneofs::request`) has both
    // variants. Its message-typed `Body` variant references sibling
    // view `BodyView<'a>` in the sibling `view::` tree — from inside
    // `view::oneofs::request` (depth 3), the path is
    // `super::super::super::view::BodyView` via the standard view-path
    // resolver.
    assert!(
        content.contains("Count(i32)"),
        "Payload enum must have Count(i32): {content}"
    );
    assert!(
        content.contains("Body(::buffa::alloc::boxed::Box<") && content.contains("BodyView"),
        "Payload enum must have Body boxed referencing BodyView: {content}"
    );
    // Decode arm for the message variant must check recursion depth.
    assert!(
        content.contains("RecursionLimitExceeded"),
        "message-type oneof variant must check recursion depth: {content}"
    );
}

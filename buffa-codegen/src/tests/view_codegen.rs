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
    let content = &files[0].content;
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
    let content = &files[0].content;
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
    let content = &files[0].content;
    // View struct must have an optional PayloadView field.
    assert!(
        content.contains("pub payload: ::core::option::Option<request::PayloadView"),
        "RequestView must have payload: ::core::option::Option<request::PayloadView>: {content}"
    );
    // The oneof view enum must have both variants.
    assert!(
        content.contains("Count(i32)"),
        "PayloadView must have Count(i32): {content}"
    );
    assert!(
        content.contains("Body(::buffa::alloc::boxed::Box<super::BodyView"),
        "PayloadView must have Body boxed: {content}"
    );
    // Decode arm for the message variant must check recursion depth.
    assert!(
        content.contains("RecursionLimitExceeded"),
        "message-type oneof variant must check recursion depth: {content}"
    );
}

/// When `Foo` and `FooView` are both messages in the same package, `Foo`'s zero-copy
/// view is skipped. Parent views that reference `Foo` must use owned `MessageField<Foo>`
/// (not `MessageFieldView<FooView<'a>>`, which would name the wrong type).
#[test]
fn test_view_field_uses_owned_when_message_view_skipped() {
    let mut file = proto3_file("view_skip_field.proto");
    file.package = Some("test.view_skip_field".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Foo".to_string()),
        field: vec![make_field(
            "x",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("FooView".to_string()),
        field: vec![make_field(
            "y",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Bar".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("foo".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".test.view_skip_field.Foo".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["view_skip_field.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &files[0].content;
    assert!(
        !content.contains("pub struct FooView<'a>"),
        "Foo's borrowed view must be skipped when FooView message exists: {content}"
    );
    assert!(
        content.contains("pub struct BarView<'a>"),
        "BarView should still be generated: {content}"
    );
    assert!(
        content.contains("pub struct BarView<'a> {\n    /// Field 1: `foo`\n    pub foo: ::buffa::MessageField<Foo>"),
        "BarView.foo must use owned MessageField<Foo>, not MessageFieldView<FooView<'a>>: {content}"
    );
    assert!(
        !content.contains("MessageFieldView<super::foo::FooView"),
        "BarView must not reference a synthetic FooView borrow type for nested Foo: {content}"
    );
    assert!(
        content.contains("view.foo.get_or_insert_default()"),
        "skipped nested message decode must merge into MessageField via get_or_insert_default: {content}"
    );
}

//! Owned-view wrapper codegen: `FooOwnedView` structs and field accessors.

use super::*;

/// A simple message with an implicit-presence string and an int32.
fn simple_msg_file(proto_name: &str) -> FileDescriptorProto {
    let mut file = proto3_file(proto_name);
    file.message_type.push(DescriptorProto {
        name: Some("Item".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("count", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        ],
        ..Default::default()
    });
    file
}

#[test]
fn test_owned_view_wrapper_struct_and_value_accessors() {
    let files = generate(
        &[simple_msg_file("item.proto")],
        &["item.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct ItemOwnedView(::buffa::OwnedView<ItemView<'static>>)"),
        "missing ItemOwnedView wrapper struct: {content}"
    );
    // Copy field types are returned by value, tied to `&self` via elision.
    assert!(
        content.contains("pub fn name(&self) -> &'_ str"),
        "implicit-presence string accessor must return &str by value: {content}"
    );
    assert!(
        content.contains("pub fn count(&self) -> i32"),
        "scalar accessor must return i32 by value: {content}"
    );
    // Infrastructure surface.
    for needle in [
        "pub fn decode(",
        "pub fn decode_with_options(",
        "pub fn from_owned(",
        "pub fn view(&self) -> &ItemView<'_>",
        "pub fn to_owned_message(",
        "pub fn into_bytes(",
    ] {
        assert!(content.contains(needle), "missing `{needle}`: {content}");
    }
    // Conversions to/from the raw OwnedView.
    assert!(
        content.contains("From<::buffa::OwnedView<ItemView<'static>>> for ItemOwnedView"),
        "missing From<OwnedView> conversion: {content}"
    );
    assert!(
        content.contains("From<ItemOwnedView> for ::buffa::OwnedView<ItemView<'static>>"),
        "missing Into-OwnedView conversion: {content}"
    );
    // Natural-path re-export at the package root, same treatment as the view.
    assert!(
        content.contains("pub use self::__buffa::view::ItemOwnedView"),
        "missing natural-path re-export of the wrapper: {content}"
    );
    // The view-family trait impl and the AsRef escape hatch ride along with
    // the wrapper.
    assert!(
        content.contains("impl ::buffa::HasMessageView for"),
        "missing HasMessageView impl: {content}"
    );
    assert!(
        content.contains("type ViewHandle = ItemOwnedView"),
        "HasMessageView::ViewHandle must name the wrapper: {content}"
    );
    assert!(
        content.contains("AsRef<::buffa::OwnedView<ItemView<'static>>> for ItemOwnedView"),
        "missing AsRef impl on the wrapper: {content}"
    );
}

#[test]
fn test_owned_view_optional_string_accessor_returns_option() {
    let mut file = proto3_file("opt.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("label".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_STRING),
            proto3_optional: Some(true),
            oneof_index: Some(0),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_label".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["opt.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub fn label(&self) -> ::core::option::Option<&'_ str>"),
        "explicit-presence string accessor must return Option<&str>: {content}"
    );
}

#[test]
fn test_owned_view_message_repeated_and_map_accessors_return_refs() {
    let mut file = proto3_file("containers.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("v", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let mut msg_field = make_field("inner", 1, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    msg_field.type_name = Some(".Inner".to_string());
    let mut map_field = make_field("labels", 3, Label::LABEL_REPEATED, Type::TYPE_MESSAGE);
    map_field.type_name = Some(".Holder.LabelsEntry".to_string());
    let map_entry = DescriptorProto {
        name: Some("LabelsEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    file.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![
            msg_field,
            make_field("tags", 2, Label::LABEL_REPEATED, Type::TYPE_STRING),
            map_field,
        ],
        nested_type: vec![map_entry],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["containers.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // Non-Copy container fields are handed out by reference.
    assert!(
        content.contains("pub fn inner(&self) -> &::buffa::MessageFieldView<")
            || content
                .contains("pub fn inner(\n        &self,\n    ) -> &::buffa::MessageFieldView<"),
        "message-field accessor must return &MessageFieldView: {content}"
    );
    assert!(
        content.contains("&::buffa::RepeatedView<'_, &'_ str>"),
        "repeated-string accessor must return &RepeatedView<'_, &str>: {content}"
    );
    assert!(
        content.contains("&::buffa::MapView<'_, &'_ str, &'_ str>"),
        "map accessor must return &MapView<'_, &str, &str>: {content}"
    );
    // Map-entry synthetic messages must not get a wrapper.
    assert!(
        !content.contains("LabelsEntryOwnedView"),
        "map-entry synthetic message must not get an owned-view wrapper: {content}"
    );
}

#[test]
fn test_owned_view_oneof_accessor_returns_option_ref() {
    let mut file = proto3_file("oneof.proto");
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
                name: Some("text".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
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
        &["oneof.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub fn payload("),
        "missing oneof accessor: {content}"
    );
    assert!(
        content.contains(".payload.as_ref()"),
        "oneof accessor must return Option<&Kind> via as_ref(): {content}"
    );
}

#[test]
fn test_owned_view_reserved_field_name_suppressed_with_warning() {
    let mut file = proto3_file("reserved.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Blob".to_string()),
        field: vec![
            // Collides with the wrapper's inherent `bytes()` method.
            make_field("bytes", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("ok", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    });
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["reserved.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    // The buffer accessor keeps its meaning; no string accessor is emitted
    // for the colliding field, but the unaffected sibling still gets one.
    assert!(
        content.contains("pub fn bytes(&self) -> &::buffa::bytes::Bytes"),
        "inherent bytes() must survive: {content}"
    );
    assert!(
        !content.contains("pub fn bytes(&self) -> &'_ str"),
        "colliding field accessor must be suppressed: {content}"
    );
    assert!(
        content.contains("pub fn ok(&self) -> &'_ str"),
        "non-colliding accessor must still be generated: {content}"
    );
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::OwnedViewAccessorSuppressed { wrapper_name, field_name }
                if wrapper_name == "BlobOwnedView" && field_name == "bytes"
        )),
        "expected an OwnedViewAccessorSuppressed warning: {warnings:?}"
    );
}

#[test]
fn test_owned_view_not_generated_without_views() {
    let files = generate(
        &[simple_msg_file("noview.proto")],
        &["noview.proto".to_string()],
        &CodeGenConfig {
            generate_views: false,
            ..CodeGenConfig::default()
        },
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        !content.contains("OwnedView"),
        "no owned-view wrapper without generate_views: {content}"
    );
}

#[test]
fn test_owned_view_serialize_impl_gated_on_json() {
    let with_json = generate(
        &[simple_msg_file("json_on.proto")],
        &["json_on.proto".to_string()],
        &CodeGenConfig {
            generate_json: true,
            ..CodeGenConfig::default()
        },
    )
    .expect("should generate");
    let content = &joined(&with_json);
    assert!(
        content.contains("impl ::serde::Serialize for ItemOwnedView"),
        "json config must emit a Serialize forwarding impl for the wrapper: {content}"
    );

    let without_json = generate(
        &[simple_msg_file("json_off.proto")],
        &["json_off.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&without_json);
    assert!(
        !content.contains("impl ::serde::Serialize for ItemOwnedView"),
        "no wrapper Serialize impl without generate_json: {content}"
    );
}

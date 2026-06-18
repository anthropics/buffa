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

// -----------------------------------------------------------------------
// Required-field presence on views (#170)
// -----------------------------------------------------------------------

fn proto2_file(name: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.to_string()),
        syntax: Some("proto2".to_string()),
        ..Default::default()
    }
}

#[test]
fn test_view_required_fields_get_presence_tracking() {
    let mut file = proto2_file("req.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Layer".to_string()),
        field: vec![
            make_field("version", 15, Label::LABEL_REQUIRED, Type::TYPE_UINT32),
            make_field("name", 1, Label::LABEL_REQUIRED, Type::TYPE_STRING),
            make_field("extent", 5, Label::LABEL_OPTIONAL, Type::TYPE_UINT32),
        ],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["req.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);

    // A hidden seen-bit word on the view struct...
    assert!(
        content.contains("pub __buffa_required_seen_0: u64"),
        "view must carry a seen-bit word for required fields: {content}"
    );
    // ...has_* accessors for the required fields only...
    assert!(
        content.contains("pub const fn has_version(&self) -> bool"),
        "required field must get a has_* accessor: {content}"
    );
    assert!(
        content.contains("pub const fn has_name(&self) -> bool"),
        "required field must get a has_* accessor: {content}"
    );
    assert!(
        !content.contains("fn has_extent"),
        "proto2 optional must not get a has_* accessor (it is Option<T>): {content}"
    );
    // ...and the decode arms set the bits.
    assert!(
        content.contains("view.__buffa_required_seen_0 |= 1u64"),
        "decode arm must set the seen bit: {content}"
    );
    assert!(
        content.contains("view.__buffa_required_seen_0 |= 2u64"),
        "second required field must use the next bit: {content}"
    );
}

#[test]
fn test_view_required_message_field_uses_is_set() {
    let mut file = proto2_file("req_msg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("x", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("inner".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REQUIRED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["req_msg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);

    // Presence delegates to MessageFieldView::is_set — no bit word needed.
    assert!(
        content.contains("pub const fn has_inner(&self) -> bool"),
        "required message field must get a has_* accessor: {content}"
    );
    assert!(
        content.contains("self.inner.is_set()"),
        "message-field has_* must delegate to is_set: {content}"
    );
    let outer_view = files
        .iter()
        .filter(|f| f.content.contains("pub struct OuterView"))
        .map(|f| f.content.as_str())
        .collect::<String>();
    assert!(
        !outer_view.contains("__buffa_required_seen"),
        "no seen-bit word when all required fields are message-typed: {outer_view}"
    );
}

#[test]
fn test_view_without_required_fields_is_unchanged() {
    let mut file = proto3_file("plain.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "value",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
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
        !content.contains("__buffa_required_seen"),
        "proto3 views must not grow presence words: {content}"
    );
    assert!(
        !content.contains("fn has_"),
        "proto3 views must not grow has_* accessors: {content}"
    );
}

#[test]
fn test_view_more_than_64_required_fields_use_two_words() {
    let mut file = proto2_file("req_many.proto");
    let fields: Vec<FieldDescriptorProto> = (1..=65)
        .map(|i| make_field(&format!("f{i}"), i, Label::LABEL_REQUIRED, Type::TYPE_INT32))
        .collect();
    file.message_type.push(DescriptorProto {
        name: Some("Wide".to_string()),
        field: fields,
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["req_many.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub __buffa_required_seen_0: u64"),
        "first word must exist: {content}"
    );
    assert!(
        content.contains("pub __buffa_required_seen_1: u64"),
        "65th required field must spill into a second word: {content}"
    );
    assert!(
        content.contains("view.__buffa_required_seen_1 |= 1u64"),
        "bit 64 must target the second word: {content}"
    );
    for f in &files {
        syn::parse_file(&f.content)
            .unwrap_or_else(|e| panic!("generated file {} must parse: {e}", f.name));
    }
}

#[test]
fn test_view_required_editions_legacy_required_tracked() {
    // Editions 2023: presence comes from features, not the proto2 label —
    // `field_presence = LEGACY_REQUIRED` must get the same tracking.
    use crate::generated::descriptor::{
        feature_set::FieldPresence, Edition, FeatureSet, FieldOptions,
    };

    let mut token = make_field("token", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    token.options = FieldOptions {
        features: FeatureSet {
            field_presence: Some(FieldPresence::LEGACY_REQUIRED),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();
    let note = make_field("note", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING);

    let file = FileDescriptorProto {
        name: Some("ed_req.proto".to_string()),
        edition: Some(Edition::EDITION_2023),
        message_type: vec![DescriptorProto {
            name: Some("Rec".to_string()),
            field: vec![token, note],
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = generate(
        &[file],
        &["ed_req.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub const fn has_token(&self) -> bool"),
        "editions LEGACY_REQUIRED field must get a has_* accessor: {content}"
    );
    assert!(
        content.contains("pub __buffa_required_seen_0: u64"),
        "editions LEGACY_REQUIRED field must get a seen-bit word: {content}"
    );
    assert!(
        !content.contains("fn has_note"),
        "editions explicit-presence field must not get a has_* accessor: {content}"
    );
}

#[test]
fn test_view_required_interspersed_scalar_and_message_bits() {
    // Bit positions are assigned to scalar-like required fields only — a
    // message-typed required field between two scalars must not consume a
    // bit, so the second scalar lands on bit 1 (mask 2), not bit 2 (mask 4).
    let mut file = proto2_file("req_mix.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("x", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let mut middle = make_field("m", 2, Label::LABEL_REQUIRED, Type::TYPE_MESSAGE);
    middle.type_name = Some(".Inner".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Mixed".to_string()),
        field: vec![
            make_field("a", 1, Label::LABEL_REQUIRED, Type::TYPE_INT32),
            middle,
            make_field("b", 3, Label::LABEL_REQUIRED, Type::TYPE_INT32),
        ],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["req_mix.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);

    assert!(
        content.contains("view.__buffa_required_seen_0 |= 1u64"),
        "first scalar must take bit 0: {content}"
    );
    assert!(
        content.contains("view.__buffa_required_seen_0 |= 2u64"),
        "scalar after a message-typed required field must take bit 1: {content}"
    );
    assert!(
        !content.contains("|= 4u64"),
        "message-typed required field must not consume a bit: {content}"
    );
    assert!(
        content.contains("self.m.is_set()"),
        "message-typed required field must delegate to is_set: {content}"
    );
}

#[test]
fn test_view_required_group_uses_is_set() {
    // Required group fields delegate presence to `MessageFieldView::is_set`,
    // exactly like message fields — no seen-bit word.
    let mut file = proto2_file("req_group.proto");
    let mut group_field = make_field("item", 1, Label::LABEL_REQUIRED, Type::TYPE_GROUP);
    group_field.type_name = Some(".Wrap.Item".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Wrap".to_string()),
        field: vec![group_field],
        nested_type: vec![DescriptorProto {
            name: Some("Item".to_string()),
            field: vec![make_field("x", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["req_group.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub const fn has_item(&self) -> bool"),
        "required group field must get a has_* accessor: {content}"
    );
    assert!(
        content.contains("self.item.is_set()"),
        "required group has_* must delegate to is_set: {content}"
    );
    let wrap_view = files
        .iter()
        .filter(|f| f.content.contains("pub struct WrapView"))
        .map(|f| f.content.as_str())
        .collect::<String>();
    assert!(
        !wrap_view.contains("__buffa_required_seen"),
        "no seen-bit word when all required fields are group-typed: {wrap_view}"
    );
}

#[test]
fn test_lazy_view_required_presence_parity() {
    // With `lazy_views` enabled, the lazy struct carries the same seen-bit
    // word and has_* accessors as the eager view, so the two families answer
    // presence identically.
    let mut file = proto2_file("req_lazy.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Layer".to_string()),
        field: vec![
            make_field("version", 15, Label::LABEL_REQUIRED, Type::TYPE_UINT32),
            make_field("name", 1, Label::LABEL_REQUIRED, Type::TYPE_STRING),
        ],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["req_lazy.proto".to_string()],
        &CodeGenConfig {
            lazy_views: true,
            ..Default::default()
        },
    )
    .expect("should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub struct LayerLazyView"),
        "lazy_views must emit the lazy struct: {content}"
    );
    assert_eq!(
        content.matches("pub __buffa_required_seen_0: u64").count(),
        2,
        "both the eager and lazy structs must carry the seen-bit word: {content}"
    );
    assert_eq!(
        content
            .matches("pub const fn has_version(&self) -> bool")
            .count(),
        2,
        "both the eager and lazy views must expose has_*: {content}"
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
    // The per-field decode method must be generated for both view types
    // (the tag loop itself is provided by the MessageView trait).
    assert!(
        content.contains("fn merge_view_field"),
        "missing merge_view_field impl: {content}"
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

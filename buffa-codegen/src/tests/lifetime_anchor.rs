//! The lifetime anchor for view structs under `preserve_unknown_fields(false)`.
//!
//! Compiling the generated code (`buffa-test/protos/self_recursive_lean.proto`)
//! proves the anchor is enough; these assertions prove it is applied to the
//! right structs, in both directions, and that the eager and lazy families are
//! decided differently. Without the "must not carry it" cases, returning `false`
//! from the predicate unconditionally would still compile.

use super::*;

fn message_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(format!(".pkg.{type_name}")),
        ..Default::default()
    }
}

fn oneof_field(
    name: &str,
    number: i32,
    ty: Type,
    type_name: Option<&str>,
    index: i32,
) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(ty),
        type_name: type_name.map(|t| format!(".pkg.{t}")),
        oneof_index: Some(index),
        ..Default::default()
    }
}

fn oneof_decl(name: &str) -> OneofDescriptorProto {
    OneofDescriptorProto {
        name: Some(name.to_string()),
        ..Default::default()
    }
}

fn generate_lean(messages: Vec<DescriptorProto>, lazy: bool) -> String {
    let mut file = proto3_file("lean.proto");
    file.package = Some("pkg".to_string());
    file.message_type = messages;
    let config = CodeGenConfig {
        generate_views: true,
        lazy_views: lazy,
        preserve_unknown_fields: false,
        ..Default::default()
    };
    let files = generate(&[file], &["lean.proto".to_string()], &config).expect("should generate");
    joined(&files)
}

/// The body of one generated struct, so an assertion about the marker says
/// something about that struct rather than about the file it shares with others.
fn struct_body<'a>(content: &'a str, decl: &str) -> &'a str {
    let start = content
        .find(decl)
        .unwrap_or_else(|| panic!("`{decl}` missing from generated output:\n{content}"));
    let rest = &content[start..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("`{decl}` body not terminated:\n{rest}"));
    &rest[..end]
}

#[test]
fn test_lean_eager_view_markers_follow_the_anchor_rule() {
    // Singular self-reference: `MessageFieldView<Self>` carries no lifetime, so
    // the struct needs the marker (#449).
    let content = generate_lean(
        vec![DescriptorProto {
            name: Some("Node".to_string()),
            field: vec![message_field("next", 1, "Node")],
            ..Default::default()
        }],
        false,
    );
    assert!(
        struct_body(&content, "pub struct NodeView<'a>").contains("__buffa_phantom"),
        "a singular self-reference must anchor 'a with the marker"
    );

    // Repeated self-reference: `RepeatedView<'a, Self>` anchors it.
    let content = generate_lean(
        vec![DescriptorProto {
            name: Some("Chain".to_string()),
            field: vec![FieldDescriptorProto {
                label: Some(Label::LABEL_REPEATED),
                ..message_field("links", 1, "Chain")
            }],
            ..Default::default()
        }],
        false,
    );
    assert!(
        !struct_body(&content, "pub struct ChainView<'a>").contains("__buffa_phantom"),
        "a repeated field already carries 'a; the marker would be noise"
    );

    // Map whose value is the message itself: the synthetic entry is repeated, and
    // `MapView<'a, K, V>` carries the lifetime.
    let content = generate_lean(
        vec![DescriptorProto {
            name: Some("SelfMap".to_string()),
            field: vec![FieldDescriptorProto {
                label: Some(Label::LABEL_REPEATED),
                r#type: Some(Type::TYPE_MESSAGE),
                name: Some("kids".to_string()),
                number: Some(1),
                type_name: Some(".pkg.SelfMap.KidsEntry".to_string()),
                ..Default::default()
            }],
            nested_type: vec![DescriptorProto {
                name: Some("KidsEntry".to_string()),
                field: vec![
                    make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
                    message_field("value", 2, "SelfMap"),
                ],
                options: MessageOptions {
                    map_entry: Some(true),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        false,
    );
    assert!(
        !struct_body(&content, "pub struct SelfMapView<'a>").contains("__buffa_phantom"),
        "a map field already carries 'a"
    );

    // A `&'a str` beside the self-reference anchors the struct.
    let content = generate_lean(
        vec![DescriptorProto {
            name: Some("NamedNode".to_string()),
            field: vec![
                make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
                message_field("next", 2, "NamedNode"),
            ],
            ..Default::default()
        }],
        false,
    );
    assert!(
        !struct_body(&content, "pub struct NamedNodeView<'a>").contains("__buffa_phantom"),
        "a borrowed scalar already anchors 'a"
    );

    // Self-reference behind a oneof whose variants are all messages: the oneof
    // enum reaches 'a through this same struct, so the struct needs the marker.
    let content = generate_lean(
        vec![DescriptorProto {
            name: Some("Tree".to_string()),
            field: vec![
                oneof_field("left", 1, Type::TYPE_MESSAGE, Some("Tree"), 0),
                oneof_field("leaf", 2, Type::TYPE_INT32, None, 0),
            ],
            oneof_decl: vec![oneof_decl("shape")],
            ..Default::default()
        }],
        false,
    );
    assert!(
        struct_body(&content, "pub struct TreeView<'a>").contains("__buffa_phantom"),
        "a oneof of messages does not anchor 'a"
    );

    // The same oneof with a `string` variant does anchor it, in both spellings.
    for (name, ty, label) in [
        ("MixedString", Type::TYPE_STRING, "string"),
        ("MixedBytes", Type::TYPE_BYTES, "bytes"),
    ] {
        let content = generate_lean(
            vec![DescriptorProto {
                name: Some(name.to_string()),
                field: vec![
                    oneof_field("tag", 1, ty, None, 0),
                    oneof_field("node", 2, Type::TYPE_MESSAGE, Some(name), 0),
                ],
                oneof_decl: vec![oneof_decl("payload")],
                ..Default::default()
            }],
            false,
        );
        assert!(
            !struct_body(&content, &format!("pub struct {name}View<'a>"))
                .contains("__buffa_phantom"),
            "a oneof with a {label} variant anchors 'a itself"
        );
    }

    // Mutual recursion: neither side anchors the other, so both need the marker.
    let content = generate_lean(
        vec![
            DescriptorProto {
                name: Some("IndirectA".to_string()),
                field: vec![message_field("other", 1, "IndirectB")],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("IndirectB".to_string()),
                field: vec![message_field("other", 1, "IndirectA")],
                ..Default::default()
            },
        ],
        false,
    );
    for name in ["IndirectAView", "IndirectBView"] {
        assert!(
            struct_body(&content, &format!("pub struct {name}<'a>")).contains("__buffa_phantom"),
            "{name} sits in a reference cycle and must anchor 'a itself"
        );
    }

    // Self-reference through a nested message: Outer -> Inner -> Outer.
    let content = generate_lean(
        vec![DescriptorProto {
            name: Some("Outer".to_string()),
            field: vec![message_field("child", 1, "Outer.Inner")],
            nested_type: vec![DescriptorProto {
                name: Some("Inner".to_string()),
                field: vec![message_field("back", 1, "Outer")],
                ..Default::default()
            }],
            ..Default::default()
        }],
        false,
    );
    assert!(
        struct_body(&content, "pub struct OuterView<'a>").contains("__buffa_phantom"),
        "a cycle through a nested message still leaves Outer unanchored"
    );
}

#[test]
fn test_lean_lazy_views_need_no_marker_for_message_fields() {
    // The lazy family emits `LazyMessageFieldView<'a, V>`, which borrows the
    // decode buffer itself, so a message-typed field — singular or a oneof
    // variant — anchors 'a there. #449 never broke a lazy struct for want of a
    // marker, so the marker must not spread across that family.
    let content = generate_lean(
        vec![
            DescriptorProto {
                name: Some("Node".to_string()),
                field: vec![message_field("next", 1, "Node")],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Tree".to_string()),
                field: vec![
                    oneof_field("left", 1, Type::TYPE_MESSAGE, Some("Tree"), 0),
                    oneof_field("leaf", 2, Type::TYPE_INT32, None, 0),
                ],
                oneof_decl: vec![oneof_decl("shape")],
                ..Default::default()
            },
        ],
        true,
    );
    for name in ["NodeView", "TreeView"] {
        assert!(
            struct_body(&content, &format!("pub struct {name}<'a>")).contains("__buffa_phantom"),
            "the eager view is still generated under lazy_views(true) and still needs the marker"
        );
    }
    for name in ["NodeLazyView", "TreeLazyView"] {
        assert!(
            !struct_body(&content, &format!("pub struct {name}<'a>")).contains("__buffa_phantom"),
            "{name} anchors 'a through LazyMessageFieldView, so it must not gain the marker"
        );
    }
}

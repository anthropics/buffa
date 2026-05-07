//! Tests for "natural-path" `pub use` re-exports of `__buffa::` ancillary
//! types.
//!
//! Each ancillary item (view struct, oneof enum, view-of-oneof enum,
//! file-level extension const, `register_types`) is re-exported at the
//! module path a user would write first — `pkg::FooView`,
//! `pkg::foo::Kind`, `pkg::foo::KindView`, etc. — *unless* that name is
//! already occupied by a real proto item or by another candidate re-export.
//! These tests cover the surviving cases and every collision rule.

use super::*;

/// Build a one-message file: `Event` with oneof `payload` and a nested
/// message `Detail`. The minimal fixture exercising oneofs + nested views.
fn event_file() -> FileDescriptorProto {
    let msg = DescriptorProto {
        name: Some("Event".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Detail".to_string()),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("payload".to_string()),
            ..Default::default()
        }],
        field: vec![{
            let mut f = make_field("data", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.oneof_index = Some(0);
            f
        }],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];
    file
}

/// Generate with views enabled and return concatenated content.
fn gen_str(files: Vec<FileDescriptorProto>) -> String {
    let config = CodeGenConfig::default();
    let names: Vec<String> = files
        .iter()
        .map(|f| f.name.clone().unwrap_or_default())
        .collect();
    let out = generate(&files, &names, &config).expect("codegen should succeed");
    joined(&out)
}

#[test]
fn natural_reexports_for_oneof_view_and_nested() {
    let content = gen_str(vec![event_file()]);
    // Top-level message view at package root.
    assert!(
        content.contains("pub use self::__buffa::view::EventView;"),
        "missing top-level view re-export: {content}"
    );
    // Owned oneof enum inside `pub mod event { … }`.
    assert!(
        content.contains("pub use super::__buffa::oneof::event::Payload;"),
        "missing oneof re-export: {content}"
    );
    // View-of-oneof enum, renamed via `as`.
    assert!(
        content.contains("pub use super::__buffa::view::oneof::event::Payload as PayloadView;"),
        "missing view-of-oneof re-export: {content}"
    );
    // Nested message view inside `pub mod event { … }`.
    assert!(
        content.contains("pub use super::__buffa::view::event::DetailView;"),
        "missing nested view re-export: {content}"
    );
}

#[test]
fn nested_reexport_path_depth_matches_nesting() {
    // Deeply nested `Outer.Middle.Inner` with oneof on `Inner` — verify the
    // `super::` chain reaches the package root from inside
    // `pkg::outer::middle::inner`.
    let inner = DescriptorProto {
        name: Some("Inner".to_string()),
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        field: vec![{
            let mut f = make_field("a", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
            f.oneof_index = Some(0);
            f
        }],
        ..Default::default()
    };
    let middle = DescriptorProto {
        name: Some("Middle".to_string()),
        nested_type: vec![inner],
        ..Default::default()
    };
    let outer = DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![middle],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![outer];
    let content = gen_str(vec![file]);
    // Inner is at nesting 2; its `pub mod inner` contents are at 3 supers.
    assert!(
        content
            .contains("pub use super::super::super::__buffa::oneof::outer::middle::inner::Kind;"),
        "deeply nested oneof re-export missing or wrong depth: {content}"
    );
}

#[test]
fn oneof_reexport_skipped_when_nested_message_collides() {
    // Nested message `Payload` next to oneof `payload` — both want
    // `pkg::event::Payload`, so the oneof re-export is dropped; the nested
    // message view re-export and the canonical `__buffa::` enum survive.
    let msg = DescriptorProto {
        name: Some("Event".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Payload".to_string()),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("payload".to_string()),
            ..Default::default()
        }],
        field: vec![{
            let mut f = make_field("a", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.oneof_index = Some(0);
            f
        }],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];
    let content = gen_str(vec![file]);
    assert!(
        !content.contains("pub use super::__buffa::oneof::event::Payload;"),
        "oneof re-export should be skipped when nested message collides: {content}"
    );
    // The view-of-oneof and the nested-message view *also* both want
    // `PayloadView`; both are dropped.
    assert!(
        !content.contains("PayloadView;"),
        "PayloadView re-export should be skipped (mutual collision): {content}"
    );
    // The canonical `__buffa::` form is unchanged.
    assert!(
        content.contains("pub enum Payload {"),
        "canonical oneof enum should still exist: {content}"
    );
}

#[test]
fn oneof_reexport_skipped_when_nested_enum_collides() {
    // Nested enum `RegionCodes` next to oneof `region_codes` — same shape
    // as the nested-message case but the occupied name comes from
    // `msg.enum_type` rather than `msg.nested_type`.
    let msg = DescriptorProto {
        name: Some("PerkRestrictions".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("RegionCodes".to_string()),
            value: vec![enum_value("REGION_CODES_UNKNOWN", 0), enum_value("US", 1)],
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("region_codes".to_string()),
            ..Default::default()
        }],
        field: vec![{
            let mut f = make_field("code", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.oneof_index = Some(0);
            f
        }],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];
    let content = gen_str(vec![file]);
    assert!(
        !content.contains("pub use super::__buffa::oneof::perk_restrictions::RegionCodes;"),
        "oneof re-export should be skipped when nested enum collides: {content}"
    );
    // The view-of-oneof's `RegionCodesView` has no competitor → it survives.
    assert!(
        content.contains("RegionCodes as RegionCodesView;"),
        "view-of-oneof re-export should survive (no collision): {content}"
    );
}

#[test]
fn root_view_reexport_skipped_when_top_level_message_collides() {
    // A top-level message named `EventView` shadows `Event`'s view re-export.
    let event = DescriptorProto {
        name: Some("Event".to_string()),
        ..Default::default()
    };
    let event_view = DescriptorProto {
        name: Some("EventView".to_string()),
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![event, event_view];
    let content = gen_str(vec![file]);
    assert!(
        !content.contains("pub use self::__buffa::view::EventView;"),
        "view re-export should be skipped when message collides: {content}"
    );
    // `EventView`'s own view (`EventViewView`) still re-exports.
    assert!(
        content.contains("pub use self::__buffa::view::EventViewView;"),
        "EventView's own view re-export should survive: {content}"
    );
}

#[test]
fn root_view_reexport_skipped_when_collides_across_files() {
    // `Event` declared in one file, `EventView` in another file of the same
    // package. The collision must be detected at the package root.
    let mut a = proto3_file("a.proto");
    a.package = Some("pkg".to_string());
    a.message_type = vec![DescriptorProto {
        name: Some("Event".to_string()),
        ..Default::default()
    }];
    let mut b = proto3_file("b.proto");
    b.package = Some("pkg".to_string());
    b.message_type = vec![DescriptorProto {
        name: Some("EventView".to_string()),
        ..Default::default()
    }];
    let content = gen_str(vec![a, b]);
    assert!(
        !content.contains("pub use self::__buffa::view::EventView;"),
        "cross-file view collision should drop the re-export: {content}"
    );
}

#[test]
fn no_reexports_when_views_disabled_and_no_oneofs() {
    let msg = DescriptorProto {
        name: Some("Plain".to_string()),
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];
    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let out = generate(&[file], &["test.proto".to_string()], &config).unwrap();
    let content = joined(&out);
    assert!(
        !content.contains("pub use"),
        "no re-exports expected without views or oneofs: {content}"
    );
}

#[test]
fn oneof_reexport_present_when_views_disabled() {
    // Oneofs always live in `__buffa::oneof`; their re-export should still
    // fire when `generate_views = false`.
    let mut file = event_file();
    // Drop the nested message so there's nothing view-related at all.
    file.message_type[0].nested_type.clear();
    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let out = generate(&[file], &["test.proto".to_string()], &config).unwrap();
    let content = joined(&out);
    assert!(
        content.contains("pub use super::__buffa::oneof::event::Payload;"),
        "oneof re-export should fire without views: {content}"
    );
    assert!(
        !content.contains("PayloadView"),
        "no view-of-oneof re-export without views: {content}"
    );
}

#[test]
fn file_level_extension_reexported_at_root() {
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    // Need a message to extend — a simple synthetic one is enough.
    file.message_type = vec![DescriptorProto {
        name: Some("Target".to_string()),
        extension_range: vec![
            crate::generated::descriptor::descriptor_proto::ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    file.extension = vec![{
        let mut f = make_field("my_opt", 100, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
        f.extendee = Some(".pkg.Target".to_string());
        f
    }];
    let content = gen_str(vec![file]);
    assert!(
        content.contains("pub use self::__buffa::ext::MY_OPT;"),
        "file-level extension re-export missing: {content}"
    );
}

#[test]
fn register_types_reexported_at_root() {
    // `register_types` is only emitted when there's something to register —
    // an extension is enough.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![DescriptorProto {
        name: Some("Target".to_string()),
        extension_range: vec![
            crate::generated::descriptor::descriptor_proto::ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    file.extension = vec![{
        let mut f = make_field("my_opt", 100, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
        f.extendee = Some(".pkg.Target".to_string());
        f
    }];
    // `register_types` requires at least one registry entry; JSON
    // extensions produce `__*_JSON_EXT` consts that populate it.
    let config = CodeGenConfig {
        generate_json: true,
        ..Default::default()
    };
    let out = generate(&[file], &["test.proto".to_string()], &config).unwrap();
    let content = joined(&out);
    assert!(
        content.contains("pub use self::__buffa::register_types;"),
        "register_types re-export missing: {content}"
    );
}

#[test]
fn extension_const_collision_with_message_drops_reexport() {
    // A top-level message named `MY_OPT` (legal proto, even if unusual)
    // shadows the file-level extension's natural re-export.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![
        DescriptorProto {
            name: Some("Target".to_string()),
            extension_range: vec![
                crate::generated::descriptor::descriptor_proto::ExtensionRange {
                    start: Some(100),
                    end: Some(200),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        DescriptorProto {
            name: Some("MY_OPT".to_string()),
            ..Default::default()
        },
    ];
    file.extension = vec![{
        let mut f = make_field("my_opt", 100, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
        f.extendee = Some(".pkg.Target".to_string());
        f
    }];
    let content = gen_str(vec![file]);
    assert!(
        !content.contains("pub use self::__buffa::ext::MY_OPT;"),
        "extension re-export should be dropped on collision: {content}"
    );
}

#[test]
fn message_owned_mod_emitted_for_reexports_only() {
    // `Event` has a oneof but no nested types: pre-re-export this produced
    // no `pub mod event { … }` in the owned content. With re-exports it must
    // still appear so that `pkg::event::Payload` resolves.
    let mut file = event_file();
    file.message_type[0].nested_type.clear();
    let content = gen_str(vec![file]);
    assert!(
        content.contains("pub mod event {"),
        "owned-mod block must be emitted to host the re-exports: {content}"
    );
}

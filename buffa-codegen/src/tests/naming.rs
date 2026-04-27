//! Naming validation: reserved __buffa_ prefix rejection, module/type name
//! conflict detection (snake_case collisions, Type vs TypeView).

use super::*;

#[test]
fn test_reserved_field_name_rejected() {
    let field = make_field(
        "__buffa_cached_size",
        1,
        Label::LABEL_OPTIONAL,
        Type::TYPE_INT32,
    );
    let msg = DescriptorProto {
        name: Some("BadMessage".to_string()),
        field: vec![field],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("my.pkg".to_string());
    file.message_type = vec![msg];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("__buffa_cached_size"),
        "error should mention the field name: {err}"
    );
    assert!(
        err.to_string().contains("my.pkg.BadMessage"),
        "error should mention the message name: {err}"
    );
}

#[test]
fn test_non_reserved_field_name_accepted() {
    let field = make_field("cached_size", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
    let msg = DescriptorProto {
        name: Some("OkMessage".to_string()),
        field: vec![field],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("my.pkg".to_string());
    file.message_type = vec![msg];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(
        result.is_ok(),
        "cached_size should be allowed as a field name"
    );
}

#[test]
fn test_module_name_conflict_detected() {
    // HTTPRequest and HttpRequest both produce module http_request.
    let mut file = proto3_file("test.proto");
    file.package = Some("my.pkg".to_string());
    file.message_type = vec![
        DescriptorProto {
            name: Some("HTTPRequest".to_string()),
            ..Default::default()
        },
        DescriptorProto {
            name: Some("HttpRequest".to_string()),
            ..Default::default()
        },
    ];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("http_request"),
        "should mention module name: {err}"
    );
    assert!(
        err.contains("HTTPRequest"),
        "should mention first message: {err}"
    );
    assert!(
        err.contains("HttpRequest"),
        "should mention second message: {err}"
    );
}

#[test]
fn test_nested_module_name_conflict_detected() {
    // Two nested messages with colliding snake_case inside the same parent.
    let parent = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![
            DescriptorProto {
                name: Some("FOO".to_string()),
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Foo".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![parent];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("foo"), "should mention module name: {err}");
}

#[test]
fn test_different_snake_case_names_no_conflict() {
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![
        DescriptorProto {
            name: Some("FooBar".to_string()),
            ..Default::default()
        },
        DescriptorProto {
            name: Some("FooBaz".to_string()),
            ..Default::default()
        },
    ];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(
        result.is_ok(),
        "distinct snake_case names should not conflict"
    );
}

#[test]
fn test_nested_type_oneof_coexist_in_separate_trees() {
    // Nested message "MyField" and oneof "my_field" coexist structurally:
    // the oneof enum lives at `__buffa::oneof::parent::MyField`, the nested
    // struct at `parent::MyField`.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("MyField".to_string()),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("my_field".to_string()),
            ..Default::default()
        }],
        // A real field referencing the oneof so it's not synthetic.
        field: vec![{
            let mut f = make_field("val", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.oneof_index = Some(0);
            f
        }],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    let files = result.expect("nested type + oneof same-name should coexist");
    let content = &joined(&files);
    assert!(
        content.contains("pub enum MyField {"),
        "oneof enum should drop Oneof suffix: {content}"
    );
    assert!(
        content.contains("pub struct MyField"),
        "nested message struct should keep its original name: {content}"
    );
}

#[test]
fn test_nested_type_oneof_no_conflict() {
    // Nested message "Inner" and oneof "my_field" — the oneof enum is
    // "MyFieldOneof" so neither side collides regardless.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("my_field".to_string()),
            ..Default::default()
        }],
        field: vec![{
            let mut f = make_field("val", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.oneof_index = Some(0);
            f
        }],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(result.is_ok(), "Inner and MyField should not conflict");
}

#[test]
fn test_nested_enum_oneof_coexists_with_suffix() {
    // Nested enum "RegionCodes" and oneof "region_codes": the oneof enum
    // is always "RegionCodesOneof" under the uniform rule; this is the
    // gh#31 motivating example.
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

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    let files = result.expect("nested enum + oneof same-name should coexist");
    let content = &joined(&files);
    // Both `pub enum RegionCodes` declarations exist — one in the owned
    // tree (the proto enum), one in `__buffa::oneof::` (the oneof enum).
    assert_eq!(
        content.matches("pub enum RegionCodes {").count(),
        2,
        "expected two RegionCodes enums (proto + oneof): {content}"
    );
}

#[test]
fn test_sibling_oneof_view_names_do_not_collide() {
    // Two sibling oneofs `my_field` and `my_field_view`. Both live in
    // `__buffa::oneof::parent::{MyField, MyFieldView}` — distinct because
    // protoc rejects duplicate oneof names.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("my_field".to_string()),
                ..Default::default()
            },
            OneofDescriptorProto {
                name: Some("my_field_view".to_string()),
                ..Default::default()
            },
        ],
        field: vec![
            {
                let mut f = make_field("a", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
                f.oneof_index = Some(0);
                f
            },
            {
                let mut f = make_field("b", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
                f.oneof_index = Some(1);
                f
            },
        ],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![msg];

    let files = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("sibling oneof names must resolve without collision");
    let content = &joined(&files);
    assert!(
        content.contains("pub enum MyField {"),
        "first oneof owned enum: {content}"
    );
    assert!(
        content.contains("pub enum MyFieldView {"),
        "second oneof owned enum: {content}"
    );
}

#[test]
fn test_foo_and_fooview_siblings_coexist() {
    // Messages "Foo" and "FooView" — Foo's view (`__buffa::view::FooView`)
    // and the owned `FooView` struct (`pkg::FooView`) live in different
    // trees. Previously rejected via ViewNameConflict.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![
        DescriptorProto {
            name: Some("Foo".to_string()),
            ..Default::default()
        },
        DescriptorProto {
            name: Some("FooView".to_string()),
            ..Default::default()
        },
    ];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    let files = result.expect("Foo + FooView siblings should coexist");
    let content = &joined(&files);
    assert!(content.contains("pub struct Foo {"));
    // pkg::FooView (owned) and __buffa::view::FooView<'a> (Foo's view).
    assert!(content.contains("pub struct FooView {"));
    assert!(content.contains("pub struct FooView<'a>"));
}

#[test]
fn test_top_level_message_named_view_compiles() {
    // Regression vs PR #54's bug: a top-level `message View { Inner }`
    // emits `pub mod view {}` (its nested-type module). Ancillary types
    // live under `__buffa::view::`, NOT a bare `pub mod view {}`, so no
    // E0428.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![DescriptorProto {
        name: Some("View".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let result = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    );
    let files = result.expect("top-level `message View` must compile");
    // Each content file must parse individually.
    for f in files
        .iter()
        .filter(|f| f.kind != GeneratedFileKind::PackageMod)
    {
        syn::parse_str::<syn::File>(&f.content).expect("generated content must parse");
    }
    let content = joined(&files);
    // The sentinel wraps the ancillary tree; the message's own
    // `pub mod view` (its nested-type module) is in the owned content.
    assert!(content.contains("pub mod __buffa {"));
}

#[test]
fn test_reserved_sentinel_package_segment_rejected() {
    let mut file = proto3_file("test.proto");
    file.package = Some("foo.__buffa".to_string());
    file.message_type = vec![DescriptorProto {
        name: Some("X".to_string()),
        ..Default::default()
    }];
    let err = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect_err("__buffa package segment must be rejected");
    assert!(
        matches!(err, CodeGenError::ReservedModuleName { .. }),
        "expected ReservedModuleName, got {err:?}"
    );
}

#[test]
fn test_reserved_sentinel_message_name_rejected() {
    // Message name `__Buffa` snake_cases to `__buffa`.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![DescriptorProto {
        name: Some("__Buffa".to_string()),
        ..Default::default()
    }];
    let err = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect_err("__Buffa message name must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("__buffa"), "error should name sentinel: {msg}");
    assert!(msg.contains("pkg.__Buffa"), "error should locate it: {msg}");
}

#[test]
fn test_reserved_sentinel_file_level_enum_rejected() {
    // File-level `enum __buffa` emits `pub enum __buffa` at package root —
    // E0428 against `pub mod __buffa`. Nested enums (inside a message)
    // live in the owner's module and cannot collide, so are not checked.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.enum_type = vec![EnumDescriptorProto {
        name: Some("__buffa".to_string()),
        value: vec![enum_value("V", 0)],
        ..Default::default()
    }];
    let err = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect_err("file-level enum __buffa must be rejected");
    assert!(
        matches!(err, CodeGenError::ReservedModuleName { .. }),
        "expected ReservedModuleName, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("enum 'pkg.__buffa'"),
        "should locate it: {msg}"
    );
}

#[test]
fn test_nested_enum_named_buffa_allowed() {
    // Nested enums emit inside the owner message's module, not at
    // package root, so `Foo { enum __buffa }` is fine.
    let mut file = proto3_file("test.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![DescriptorProto {
        name: Some("Foo".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("__buffa".to_string()),
            value: vec![enum_value("V", 0)],
            ..Default::default()
        }],
        ..Default::default()
    }];
    generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("nested enum __buffa should be allowed");
}

#[test]
fn test_proto3_optional_field_name_matches_nested_enum_no_conflict() {
    // Proto3 `optional MatchOperator match_operator = 4;` creates a synthetic
    // oneof named `_match_operator`.  `to_pascal_case("_match_operator")` yields
    // `MatchOperator`, which collides with the nested enum.  But synthetic oneofs
    // never generate a Rust enum, so this must be accepted.
    let msg = DescriptorProto {
        name: Some("StringFieldMatcher".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("MatchOperator".to_string()),
            value: vec![
                enum_value("MATCH_OPERATOR_UNKNOWN", 0),
                enum_value("MATCH_OPERATOR_EXACT_MATCH", 1),
            ],
            ..Default::default()
        }],
        // protoc wraps proto3 optional in a synthetic oneof named `_match_operator`.
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_match_operator".to_string()),
            ..Default::default()
        }],
        field: vec![{
            let mut f = make_field("match_operator", 4, Label::LABEL_OPTIONAL, Type::TYPE_ENUM);
            f.type_name = Some(".minimal.StringFieldMatcher.MatchOperator".to_string());
            f.oneof_index = Some(0);
            f.proto3_optional = Some(true);
            f
        }],
        ..Default::default()
    };
    let mut file = proto3_file("test.proto");
    file.package = Some("minimal".to_string());
    file.message_type = vec![msg];

    let config = CodeGenConfig::default();
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(
        result.is_ok(),
        "synthetic oneof should not conflict with nested enum: {}",
        result.unwrap_err()
    );
}

#[test]
fn test_nested_message_named_option_does_not_shadow_prelude() {
    // Reproduces gh#36: a nested message named `Option` shadows
    // `core::option::Option`, causing `pub value: Option<option::Value>` to
    // resolve to the proto struct instead of the standard library type.
    // The codegen must emit `::core::option::Option<...>` in this scope.
    let option_msg = DescriptorProto {
        name: Some("Option".to_string()),
        field: vec![
            make_field("title", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            {
                let mut f = make_field("int_value", 2, Label::LABEL_OPTIONAL, Type::TYPE_UINT64);
                f.oneof_index = Some(0);
                f
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("value".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let picker_msg = DescriptorProto {
        name: Some("Picker".to_string()),
        field: vec![{
            let mut f = make_field("options", 1, Label::LABEL_REPEATED, Type::TYPE_MESSAGE);
            f.type_name = Some(".test.option_shadow.Picker.Option".to_string());
            f
        }],
        nested_type: vec![option_msg],
        ..Default::default()
    };
    let mut file = proto3_file("option_shadow.proto");
    file.package = Some("test.option_shadow".to_string());
    file.message_type = vec![picker_msg];

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["option_shadow.proto".to_string()], &config);
    let files = result.expect("nested Option message should not break codegen");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Option"),
        "nested Option struct must exist: {content}"
    );
    // The oneof field on Option must use the fully-qualified
    // `::core::option::Option` to avoid resolving to the proto struct.
    assert!(
        !content.contains("pub value: Option<"),
        "bare Option<> in struct field would shadow core::option::Option: {content}"
    );
    assert!(
        content.contains("::core::option::Option<"),
        "must use fully-qualified ::core::option::Option: {content}"
    );
}

#[test]
fn test_top_level_message_named_option_qualifies_option() {
    // A top-level message named `Option` shadows the prelude. All Option type
    // references must be `::core::option::Option` (unconditionally qualified).
    let mut file = proto3_file("option_top.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![
        DescriptorProto {
            name: Some("Option".to_string()),
            ..Default::default()
        },
        DescriptorProto {
            name: Some("Wrapper".to_string()),
            field: vec![{
                let mut f = make_field("tag", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
                f.proto3_optional = Some(true);
                f.oneof_index = Some(0);
                f
            }],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("_tag".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["option_top.proto".to_string()], &config);
    let files = result.expect("top-level Option should not break codegen");
    let content = &joined(&files);
    // The Wrapper struct must use qualified Option for its optional field.
    assert!(
        content.contains("::core::option::Option<"),
        "must use fully-qualified ::core::option::Option for optional field: {content}"
    );
    assert!(
        !content.contains("pub tag: Option<"),
        "bare Option<> on Wrapper field would shadow core::option::Option: {content}"
    );
}

#[test]
fn test_nested_option_blocked_propagates_through_sibling_subtree() {
    // `Outer { nested Option; nested Middle { nested Inner } }` — `Option`
    // is declared in `mod outer`, so it shadows the prelude there AND in
    // `mod outer::middle` via `use super::*`. `Inner.x` (emitted inside
    // `mod outer::middle`) must use the fully-qualified path.
    let inner_msg = DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![{
            let mut f = make_field("x", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
            f.proto3_optional = Some(true);
            f.oneof_index = Some(0);
            f
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_x".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let middle_msg = DescriptorProto {
        name: Some("Middle".to_string()),
        field: vec![{
            let mut f = make_field("note", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.proto3_optional = Some(true);
            f.oneof_index = Some(0);
            f
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_note".to_string()),
            ..Default::default()
        }],
        nested_type: vec![inner_msg],
        ..Default::default()
    };
    let outer_msg = DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![
            DescriptorProto {
                name: Some("Option".to_string()),
                ..Default::default()
            },
            middle_msg,
        ],
        ..Default::default()
    };
    let mut file = proto3_file("option_deep.proto");
    file.package = Some("pkg".to_string());
    file.message_type = vec![outer_msg];

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let files = generate(&[file], &["option_deep.proto".to_string()], &config)
        .expect("nested Option sibling should not break codegen");
    let content = &joined(&files);
    // `Middle.note` lives in `mod outer` (Option in scope); `Inner.x` lives
    // in `mod outer::middle` (Option in scope via `use super::*`). Both must
    // be qualified.
    assert!(
        !content.contains("pub note: Option<"),
        "Middle.note must qualify Option (sibling collision): {content}"
    );
    assert!(
        !content.contains("pub x: Option<"),
        "Inner.x must qualify Option (inherited via use super::*): {content}"
    );
}

#[test]
fn test_cross_file_message_named_option_shadows_prelude() {
    // Reproduces gh#64: two files share a package. File A defines a top-level
    // `message Option`. File B has a oneof (which needs `Option<OneofEnum>`).
    // Because ImportResolver only checks names within a single file, file B's
    // resolver doesn't see file A's `Option` and emits bare `Option<...>`.
    // The stitcher combines both files into one module scope via `include!`,
    // so file A's `pub struct Option` shadows the prelude in file B's code.
    let mut file_a = proto3_file("message.proto");
    file_a.package = Some("pkg".to_string());
    file_a.message_type = vec![DescriptorProto {
        name: Some("Option".to_string()),
        field: vec![make_field(
            "text",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    }];

    let mut file_b = proto3_file("session.proto");
    file_b.package = Some("pkg".to_string());
    file_b.message_type = vec![DescriptorProto {
        name: Some("Wrapper".to_string()),
        field: vec![{
            let mut f = make_field("a", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.oneof_index = Some(0);
            f
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let files = generate(
        &[file_a, file_b],
        &["message.proto".to_string(), "session.proto".to_string()],
        &config,
    )
    .expect("cross-file Option should not break codegen");

    // file B's owned output (session.rs) must use ::core::option::Option
    // for the oneof field, not bare Option which would resolve to file A's struct.
    let session_owned = files
        .iter()
        .find(|f| f.name.starts_with("session") && f.kind == GeneratedFileKind::Owned)
        .expect("session owned file must exist");
    assert!(
        !session_owned.content.contains("pub kind: Option<"),
        "bare Option<> in session.rs would shadow file A's struct: {}",
        session_owned.content,
    );
    assert!(
        session_owned.content.contains("::core::option::Option<"),
        "session.rs must use fully-qualified ::core::option::Option: {}",
        session_owned.content,
    );
}

#[test]
fn test_message_named_type_with_nested() {
    // Proto message named "Type" (a Rust keyword) with a nested message.
    // This must produce valid Rust: `pub mod r#type { ... }`.
    let mut file = proto3_file("type_test.proto");
    file.package = Some("google.api.expr.v1alpha1".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Type".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("primitive".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".google.api.expr.v1alpha1.Type.PrimitiveType".to_string()),
            ..Default::default()
        }],
        nested_type: vec![],
        enum_type: vec![EnumDescriptorProto {
            name: Some("PrimitiveType".to_string()),
            value: vec![
                enum_value("PRIMITIVE_TYPE_UNSPECIFIED", 0),
                enum_value("BOOL", 1),
            ],
            ..Default::default()
        }],
        ..Default::default()
    });

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["type_test.proto".to_string()], &config);
    let files = result.expect("message named Type should generate valid code");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Type"),
        "missing struct Type: {content}"
    );
    assert!(
        content.contains("pub mod r#type"),
        "missing r#type module: {content}"
    );
}

#[test]
fn test_message_with_oneof_field_named_type() {
    // Reproduces the CEL checked.proto Type message which has:
    // - A oneof named `type_kind` with a field `Type type = 11`
    //   (field named "type" with self-referential type)
    let mut file = proto3_file("checked.proto");
    file.package = Some("google.api.expr.v1alpha1".to_string());

    // The Type message with a self-referential oneof field named "type"
    file.message_type.push(DescriptorProto {
        name: Some("Type".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("message_type".to_string()),
                number: Some(9),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("type".to_string()),
                number: Some(11),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".google.api.expr.v1alpha1.Type".to_string()),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("type_kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["checked.proto".to_string()], &config);
    let files = result.expect("Type message with oneof 'type' field should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Type"),
        "missing struct Type: {content}"
    );
}

#[test]
fn test_oneof_variant_named_self_escapes_to_self_underscore() {
    // Regression for #47. A oneof variant whose proto name PascalCases to
    // a reserved Rust identifier (only `Self` is reachable: no other
    // lowercase Rust keyword PascalCases to another reserved ident) must
    // be sanitized; otherwise codegen emits `pub enum X { Self(...) }`,
    // which is a parse error.
    let mut file = proto3_file("self_variant.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Identity".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("self".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_BOOL),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("manager".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("identity".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["self_variant.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("oneof with `self` variant must compile");
    let content = &joined(&files);
    // The reserved `Self` is suffixed to `Self_` by `make_field_ident`;
    // the bare `Manager` variant is unaffected.
    assert!(
        content.contains("Self_(bool)"),
        "expected `Self_(bool)` variant; got:\n{content}"
    );
    assert!(
        content.contains("Manager(::buffa::alloc::string::String)"),
        "non-keyword variant must remain unrenamed; got:\n{content}"
    );
    // Defense in depth: no bare `Self(` (which would not parse).
    assert!(
        !content.contains(" Self("),
        "raw `Self(` survived in generated code:\n{content}"
    );
}

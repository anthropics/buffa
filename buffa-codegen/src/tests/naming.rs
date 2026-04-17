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
fn test_nested_type_and_oneof_with_same_name_coexist() {
    // Under the generated-code layout rule (see DESIGN.md), nested
    // messages live in the owner's sub-module (`parent::MyField`) while
    // oneof enums live in the parallel `oneofs::` tree
    // (`parent::oneofs::parent::MyField`). Different modules — no Rust
    // name collision — so a proto with nested `MyField` + `oneof
    // my_field` compiles cleanly.
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

    let files = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("nested struct + same-name oneof live in different trees");
    let oneofs = files
        .iter()
        .find(|f| f.kind == crate::GeneratedFileKind::Oneofs)
        .expect("oneofs stream emitted");
    assert!(
        oneofs.content.contains("pub enum MyField"),
        "oneof enum MyField should land in oneofs tree: {}",
        oneofs.content
    );
    let owned = files
        .iter()
        .find(|f| f.kind == crate::GeneratedFileKind::Owned)
        .expect("owned stream emitted");
    assert!(
        owned.content.contains("pub struct MyField"),
        "nested struct MyField stays in owner sub-module: {}",
        owned.content
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
fn test_nested_enum_and_oneof_with_same_name_coexist() {
    // Nested enum `RegionCodes` in `parent::RegionCodes` + oneof
    // `region_codes` in `parent::oneofs::parent::RegionCodes` — two
    // different modules under the generated-code layout rule. This
    // used to conflict (gh#31 motivating example); the parallel-tree
    // design makes it structurally impossible.
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

    let files = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("nested enum + same-name oneof live in different trees");
    let oneofs = files
        .iter()
        .find(|f| f.kind == crate::GeneratedFileKind::Oneofs)
        .expect("oneofs stream emitted");
    assert!(
        oneofs.content.contains("pub enum RegionCodes"),
        "oneof enum goes in oneofs tree: {}",
        oneofs.content
    );
    let owned = files
        .iter()
        .find(|f| f.kind == crate::GeneratedFileKind::Owned)
        .expect("owned stream emitted");
    assert!(
        owned.content.contains("pub enum RegionCodes"),
        "nested enum stays in owner sub-module (owned tree): {}",
        owned.content
    );
}

#[test]
fn test_oneof_and_oneof_view_drop_suffix_in_parallel_trees() {
    // Owned oneof `Kind` lands in `pkg::oneofs::parent::Kind`; its view
    // counterpart lands in `pkg::view::oneofs::parent::Kind` — same
    // PascalCase ident in both trees. The `View` suffix is dropped on
    // the view-of-oneof enum because the path prefix already
    // disambiguates (the documented exception — suffix retention — is
    // only for message view STRUCTS, not oneof view enums).
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
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

    let config = CodeGenConfig::default(); // views enabled by default
    let files = generate(&[file], &["test.proto".to_string()], &config)
        .expect("owned + view oneof enums should coexist");
    let oneofs = files
        .iter()
        .find(|f| f.kind == crate::GeneratedFileKind::Oneofs)
        .expect("oneofs stream emitted");
    assert!(
        oneofs.content.contains("pub enum Kind {"),
        "owned oneof enum in oneofs:: tree (no suffix): {}",
        oneofs.content
    );
    let view_oneofs = files
        .iter()
        .find(|f| f.kind == crate::GeneratedFileKind::ViewOneofs)
        .expect("view_oneofs stream emitted");
    assert!(
        view_oneofs.content.contains("pub enum Kind<"),
        "view-of-oneof enum in view::oneofs:: tree (no View suffix): {}",
        view_oneofs.content
    );
}

#[test]
fn test_oneof_view_does_not_collide_with_nested_view_struct_name() {
    // Nested message `MyFieldView` alongside `oneof my_field`. Names
    // produced under the generated-code layout rule:
    // - nested msg  `MyFieldView`   → `parent::MyFieldView`
    // - nested view `MyFieldViewView` → `view::parent::MyFieldViewView`
    // - oneof owned `MyField`       → `oneofs::parent::MyField`
    // - oneof view  `MyField`       → `view::oneofs::parent::MyField`  (no View suffix)
    //
    // All four paths are distinct. The parallel-tree layout makes this
    // pattern trivially legal.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("MyFieldView".to_string()),
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

    generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("parallel-tree namespacing makes this pattern legal");
}

#[test]
fn test_sibling_oneof_with_view_like_name_is_legal() {
    // Two sibling oneofs `my_field` and `my_field_view`. Under the
    // parallel-tree layout:
    // - `my_field`      → owned `oneofs::parent::MyField`,    view `view::oneofs::parent::MyField`
    // - `my_field_view` → owned `oneofs::parent::MyFieldView`, view `view::oneofs::parent::MyFieldView`
    //
    // (Oneof view enums drop the `View` suffix — the tree prefix is
    // the disambiguator.) All four names are distinct across the two
    // trees, so this is legal.
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

    generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("sibling oneofs with View-suffixed names should be legal");
}

#[test]
fn test_sibling_oneofs_get_distinct_names() {
    // Two oneofs with distinct PascalCase names — no collision at all
    // under PR 1's no-suffix scheme.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("my_field".to_string()),
                ..Default::default()
            },
            OneofDescriptorProto {
                name: Some("other_field".to_string()),
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

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["test.proto".to_string()], &config);
    let files = result.expect("sibling oneofs should get distinct names");
    let content = all_content(&files);
    assert!(
        content.contains("pub enum MyField"),
        "first oneof should emit as MyField: {content}"
    );
    assert!(
        content.contains("pub enum OtherField"),
        "second oneof should emit as OtherField: {content}"
    );
}

#[test]
fn test_top_level_view_no_longer_collides_with_named_view_message() {
    // Under PR 1's `view::` namespace the top-level view for message `Foo`
    // lives at `pkg::view::FooView`, so a sibling message literally named
    // `FooView` at `pkg::FooView` doesn't collide with anything. Its own
    // view lands at `pkg::view::FooViewView` — ugly but legal.
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

    let config = CodeGenConfig::default(); // views enabled by default
    let files = generate(&[file], &["test.proto".to_string()], &config)
        .expect("namespace split resolves the old collision");
    let content = all_content(&files);
    assert!(
        content.contains("pub struct Foo "),
        "owned Foo should still be generated: {content}"
    );
    assert!(
        content.contains("pub struct FooView "),
        "owned FooView message should still be generated at package scope: {content}"
    );
    // Under PR 1 the view tree is stitched by `generate_module_tree`,
    // not wrapped per-file. Per-proto outputs are three siblings: an
    // owned `.rs`, a `.__view.rs`, and a `.__ext.rs` — verify the
    // view-tree sibling exists and carries the view structs.
    let view_file = files
        .iter()
        .find(|f| f.name == "test.__view.rs")
        .expect("expected __view.rs output");
    assert!(
        view_file.content.contains("pub struct FooView"),
        "view struct FooView<'a> should land in the view-tree file: {}",
        view_file.content
    );
    assert!(
        view_file.content.contains("pub struct FooViewView"),
        "FooView (owned) also gets a view: `FooViewView<'a>`: {}",
        view_file.content
    );
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
    let content = &files[0].content;
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
    // A top-level message named `Option` — file-level ImportResolver should
    // detect this and qualify all Option type references in the file.
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
    let content = &files[0].content;
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
    // `mod outer::middle` via `use super::*`. The child resolver for
    // `Middle` must inherit the parent's blocked set so that `Inner`
    // (emitted inside `mod outer::middle`) qualifies its optional field.
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
    let content = &files[0].content;
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
    let content = &files[0].content;
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
    let content = &files[0].content;
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
    let content = all_content(&files);
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

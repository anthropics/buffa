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
fn test_nested_type_oneof_coexists_with_suffix() {
    // Nested message "MyField" and oneof "my_field" coexist: the oneof
    // enum is always named "MyFieldOneof" under the uniform-suffix rule,
    // which happens to be collision-free against the nested struct.
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
    let files = result.expect("nested type + oneof name collision should resolve, not error");
    let content = &files[0].content;
    assert!(
        content.contains("MyFieldOneof"),
        "oneof enum should be suffixed with Oneof: {content}"
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
    let files = result.expect("nested enum + oneof name collision should resolve, not error");
    let content = &files[0].content;
    assert!(
        content.contains("RegionCodesOneof"),
        "oneof enum should be suffixed with Oneof: {content}"
    );
    assert!(
        content.contains("pub enum RegionCodes"),
        "nested enum should keep its original name: {content}"
    );
}

#[test]
fn test_nested_type_oneof_view_uses_suffix() {
    // When view generation is on, the view enum uses the uniform
    // suffixed name (MyFieldOneofView) alongside its owned counterpart.
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

    let config = CodeGenConfig::default(); // views enabled by default
    let result = generate(&[file], &["test.proto".to_string()], &config);
    let files = result.expect("view codegen should handle oneof rename");
    let content = &files[0].content;
    assert!(
        content.contains("MyFieldOneofView"),
        "view enum should use suffixed name: {content}"
    );
}

#[test]
fn test_oneof_coexists_with_nested_view_struct_name() {
    // Nested message `MyFieldView` + oneof `my_field` with views enabled:
    // under the uniform-suffix rule the oneof enum is `MyFieldOneof` and
    // its view is `MyFieldOneofView`; neither name collides with the
    // nested message's view struct (also `MyFieldView`).
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

    let config = CodeGenConfig::default();
    let files = generate(&[file], &["test.proto".to_string()], &config)
        .expect("uniform suffix keeps the oneof clear of nested view name");
    let content = &files[0].content;
    assert!(
        content.contains("pub enum MyFieldOneof {"),
        "owned oneof enum should be MyFieldOneof: {content}"
    );
    assert!(
        content.contains("pub enum MyFieldOneofView<"),
        "view oneof enum should be MyFieldOneofView<'a>: {content}"
    );
    // The nested message's own view struct remains unchanged.
    assert!(
        content.contains("pub struct MyFieldView"),
        "nested message view struct must be preserved: {content}"
    );
}

#[test]
fn test_sibling_oneof_view_names_do_not_collide() {
    // Two sibling oneofs `my_field` and `my_field_view`. Under the
    // uniform-suffix rule they become `MyFieldOneof`/`MyFieldOneofView`
    // and `MyFieldViewOneof`/`MyFieldViewOneofView` — never collide with
    // each other or with their own view counterparts.
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
    let content = &files[0].content;
    assert!(
        content.contains("pub enum MyFieldOneof {"),
        "first oneof owned enum should be MyFieldOneof: {content}"
    );
    assert!(
        content.contains("pub enum MyFieldOneofView<"),
        "first oneof view enum should be MyFieldOneofView<'a>: {content}"
    );
    assert!(
        content.contains("pub enum MyFieldViewOneof {"),
        "second oneof owned enum should be MyFieldViewOneof: {content}"
    );
    assert!(
        content.contains("pub enum MyFieldViewOneofView<"),
        "second oneof view enum should be MyFieldViewOneofView<'a>: {content}"
    );
}

#[test]
fn test_oneof_suffix_conflict_error_includes_scope() {
    // Verify the diagnostic carries the parent message's FQN so users can
    // locate which message triggered the error in a large descriptor set.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![
            DescriptorProto {
                name: Some("MyField".to_string()),
                ..Default::default()
            },
            DescriptorProto {
                name: Some("MyFieldOneof".to_string()),
                ..Default::default()
            },
        ],
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

    let err = generate(
        &[file],
        &["test.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect_err("double collision must error");
    match err {
        CodeGenError::OneofNameConflict {
            scope,
            oneof_name,
            attempted,
        } => {
            assert_eq!(scope, "pkg.Parent");
            assert_eq!(oneof_name, "my_field");
            assert_eq!(attempted, "MyFieldOneof");
        }
        other => panic!("expected OneofNameConflict, got {other:?}"),
    }
}

#[test]
fn test_oneof_name_conflict_errors() {
    // A nested type literally named "MyFieldOneof" alongside oneof
    // "my_field" leaves the uniform-suffix name with nowhere to go —
    // users must rename one side in the `.proto`.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("MyFieldOneof".to_string()),
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
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("MyFieldOneof"),
        "error should mention the attempted name: {err}"
    );
}

#[test]
fn test_sibling_oneofs_get_distinct_names() {
    // Two oneofs `my_field` and `my_field_oneof` — both want
    // `MyFieldOneof` as their Rust name. Sequential allocation must
    // assign distinct names, e.g. `MyFieldOneof` and `MyFieldOneofOneof`.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("my_field".to_string()),
                ..Default::default()
            },
            OneofDescriptorProto {
                name: Some("my_field_oneof".to_string()),
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
    let content = &files[0].content;
    assert!(
        content.contains("MyFieldOneof"),
        "first oneof should be suffixed: {content}"
    );
    assert!(
        content.contains("MyFieldOneofOneof"),
        "second oneof should be double-suffixed: {content}"
    );
}

#[test]
fn test_view_name_conflict_detected() {
    // Messages "Foo" and "FooView" — Foo's view type collides with FooView struct.
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
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Foo"), "should mention owned message: {err}");
    assert!(
        err.contains("FooView"),
        "should mention view collision: {err}"
    );
}

#[test]
fn test_view_name_conflict_not_checked_when_views_disabled() {
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

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["test.proto".to_string()], &config);
    assert!(result.is_ok(), "no conflict when views are disabled");
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
    let content = &files[0].content;
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

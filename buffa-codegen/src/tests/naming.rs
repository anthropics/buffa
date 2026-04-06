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
fn test_nested_type_oneof_conflict_resolved_with_suffix() {
    // Nested message "MyField" and oneof "my_field" both produce MyField in
    // PascalCase.  The oneof enum should be renamed to MyFieldOneof.
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
    // Nested message "Inner" and oneof "my_field" produce different names.
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
fn test_nested_enum_oneof_conflict_resolved_with_suffix() {
    // Nested enum "RegionCodes" and oneof "region_codes" both produce
    // RegionCodes in PascalCase.  The oneof enum should become
    // RegionCodesOneof (the original gh#31 example).
    let msg = DescriptorProto {
        name: Some("PerkRestrictions".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("RegionCodes".to_string()),
            value: vec![
                enum_value("REGION_CODES_UNKNOWN", 0),
                enum_value("US", 1),
            ],
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
fn test_oneof_shadows_parent_message_name() {
    // message DataType { oneof data_type { ... } } — the oneof enum would
    // shadow the parent struct imported via `use super::*`.  The oneof enum
    // should become DataTypeOneof.
    let msg = DescriptorProto {
        name: Some("DataType".to_string()),
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("data_type".to_string()),
            ..Default::default()
        }],
        field: vec![
            {
                let mut f = make_field("variant", 1, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
                f.type_name = Some(".pkg.DataType.Variant".to_string());
                f.oneof_index = Some(0);
                f
            },
        ],
        nested_type: vec![
            DescriptorProto {
                name: Some("Variant".to_string()),
                field: vec![make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING)],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Ref".to_string()),
                field: vec![{
                    let mut f = make_field("target", 1, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
                    f.type_name = Some(".pkg.DataType".to_string());
                    f
                }],
                ..Default::default()
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
    let files = result.expect("oneof shadowing parent name should resolve, not error");
    let content = &files[0].content;
    assert!(
        content.contains("DataTypeOneof"),
        "oneof enum should be suffixed to avoid shadowing parent: {content}"
    );
    assert!(
        content.contains("pub struct DataType"),
        "parent message struct should keep its name: {content}"
    );
}

#[test]
fn test_nested_type_oneof_conflict_view_uses_suffix() {
    // When view generation is on, the view enum should also use the
    // Oneof-suffixed name (MyFieldOneofView).
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
fn test_oneof_suffix_double_collision_errors() {
    // Nested types "MyField" and "MyFieldOneof" plus oneof "my_field" —
    // the suffix fallback also collides, so this must error.
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
    // nested message "MyField", oneof "my_field" → MyFieldOneof,
    // oneof "my_field_oneof" → would naturally be MyFieldOneof too.
    // Sequential allocation must assign distinct names.
    let msg = DescriptorProto {
        name: Some("Parent".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("MyField".to_string()),
            ..Default::default()
        }],
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
fn test_view_name_collision_skips_view_generation() {
    // Messages "Foo" and "FooView" — Foo's view type would collide with the
    // FooView struct, so Foo's view is skipped rather than erroring.
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
    let files = result.expect("view collision should be handled gracefully");
    let content = &files[0].content;
    assert!(
        content.contains("pub struct Foo"),
        "owned Foo struct must exist: {content}"
    );
    assert!(
        content.contains("pub struct FooView"),
        "FooView message struct must exist: {content}"
    );
    // The generated view for Foo would be `pub struct FooView<'a>` with a
    // `MessageView` impl — but that collides with the user-defined FooView
    // message, so the view must be skipped.  The user-defined FooView
    // message's *own* view (`FooViewView<'a>`) should still exist.
    assert!(
        content.contains("FooViewView"),
        "FooView message's own view (FooViewView) should still be generated: {content}"
    );
    assert!(
        !content.contains("impl<'a> ::buffa::MessageView<'a> for FooView<'a>"),
        "Foo's view impl should be skipped to avoid collision: {content}"
    );
}

#[test]
fn test_view_name_collision_skips_view_across_files_same_package() {
    // Same as test_view_name_collision_skips_view_generation, but `Foo` and
    // `FooView` live in different FileDescriptorProtos. They still share one
    // generated Rust module per package, so Foo's view must be skipped.
    let mut place = proto3_file("place.proto");
    place.package = Some("pkg".to_string());
    place.message_type = vec![DescriptorProto {
        name: Some("Foo".to_string()),
        ..Default::default()
    }];

    let mut view = proto3_file("view.proto");
    view.package = Some("pkg".to_string());
    view.message_type = vec![DescriptorProto {
        name: Some("FooView".to_string()),
        ..Default::default()
    }];

    let files = [place, view];
    let config = CodeGenConfig::default();
    let result = generate(
        &files,
        &["place.proto".to_string(), "view.proto".to_string()],
        &config,
    );
    let generated = result.expect("cross-file view collision should be handled");
    let place_rs = generated
        .iter()
        .find(|f| f.name.contains("place"))
        .expect("place output");
    assert!(
        !place_rs
            .content
            .contains("impl<'a> ::buffa::MessageView<'a> for FooView<'a>"),
        "Foo's view impl should be skipped (collision with FooView from view.proto): {}",
        place_rs.content
    );
    let view_rs = generated
        .iter()
        .find(|f| f.name.contains("view"))
        .expect("view output");
    assert!(
        view_rs.content.contains("FooViewView"),
        "FooView message's own view should still be generated: {}",
        view_rs.content
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
fn test_sibling_proto_option_shadows_prelude_across_generated_files() {
    // Two `.proto` files in the same package, `include!`d into one Rust module:
    // a top-level `message Option` in one file emits `pub struct Option` at
    // the shared module root, shadowing `core::option::Option` for siblings.
    let mut defs = proto3_file("defs.proto");
    defs.package = Some("pkg".to_string());
    defs.message_type = vec![DescriptorProto {
        name: Some("Option".to_string()),
        ..Default::default()
    }];

    let mut user = proto3_file("user.proto");
    user.package = Some("pkg".to_string());
    user.message_type = vec![DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![{
            let mut f = make_field("value", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.proto3_optional = Some(true);
            f.oneof_index = Some(0);
            f
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_value".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let fds = vec![defs, user];
    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(
        &fds,
        &["defs.proto".to_string(), "user.proto".to_string()],
        &config,
    );
    let files = result.expect("cross-file Option shadow should resolve");
    let user_rs = files
        .iter()
        .find(|f| f.name == "user.rs")
        .expect("user.rs output");
    assert!(
        user_rs.content.contains("::core::option::Option<"),
        "Holder must use qualified Option when sibling defs.proto defines message Option: {}",
        user_rs.content
    );
    assert!(
        !user_rs.content.contains("pub value: Option<"),
        "bare Option<> on Holder would resolve to the proto struct: {}",
        user_rs.content
    );
}

#[test]
fn test_option_message_in_other_package_does_not_qualify_prelude() {
    // Same `generate()` batch, different protobuf packages → different Rust
    // module roots; `message Option` in pkg.a must not force `::core::option::Option`
    // in pkg.b.
    let mut defs = proto3_file("defs.proto");
    defs.package = Some("pkg.a".to_string());
    defs.message_type = vec![DescriptorProto {
        name: Some("Option".to_string()),
        ..Default::default()
    }];

    let mut user = proto3_file("user.proto");
    user.package = Some("pkg.b".to_string());
    user.message_type = vec![DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![{
            let mut f = make_field("value", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
            f.proto3_optional = Some(true);
            f.oneof_index = Some(0);
            f
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_value".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let fds = vec![defs, user];
    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(
        &fds,
        &["defs.proto".to_string(), "user.proto".to_string()],
        &config,
    );
    let files = result.expect("cross-package batch should generate");
    let user_rs = files
        .iter()
        .find(|f| f.name == "user.rs")
        .expect("user.rs output");
    assert!(
        !user_rs.content.contains("::core::option::Option<"),
        "pkg.b should keep bare prelude Option when pkg.a defines message Option: {}",
        user_rs.content
    );
    assert!(
        user_rs.content.contains("Option<"),
        "expected standard optional wrapper in Holder: {}",
        user_rs.content
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
fn test_nested_message_shadowing_sibling_module_path() {
    // Reproducer for gh#38: nested message Picker.Validation shadows the
    // top-level Validation module, causing oneof variant types like
    // `super::validation::Boolean` to resolve to the wrong module.
    //
    // With the fix, nested messages get deeper nesting so the generated
    // super:: chain escapes to the correct scope.

    // Top-level Validation with nested Boolean.
    let validation_msg = DescriptorProto {
        name: Some("Validation".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Boolean".to_string()),
            field: vec![make_field("value", 1, Label::LABEL_OPTIONAL, Type::TYPE_BOOL)],
            ..Default::default()
        }],
        ..Default::default()
    };

    // Top-level Picker with nested Validation that has a oneof referencing
    // the top-level Validation.Boolean.
    let picker_msg = DescriptorProto {
        name: Some("Picker".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Validation".to_string()),
            field: vec![FieldDescriptorProto {
                name: Some("date_selected".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".test.oneof_sibling.Validation.Boolean".to_string()),
                oneof_index: Some(0),
                ..Default::default()
            }],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("rule".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut file = proto3_file("picker.proto");
    file.package = Some("test.oneof_sibling".to_string());
    file.message_type = vec![validation_msg, picker_msg];

    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let result = generate(&[file], &["picker.proto".to_string()], &config);
    let files = result.expect("nested message shadowing sibling should not break codegen");
    let content = &files[0].content;

    // The oneof variant must NOT use a single `super::validation::Boolean`
    // which would resolve to the nested Picker.Validation module.
    assert!(
        !content.contains("DateSelected(super::validation::Boolean)"),
        "single super:: resolves to nested module, not sibling: {content}"
    );
    // It should use two super:: hops to escape the nested module.
    assert!(
        content.contains("super::super::validation::Boolean"),
        "must use super::super:: to reach sibling top-level module: {content}"
    );
}

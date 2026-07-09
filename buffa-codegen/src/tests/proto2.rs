//! Proto2 codegen: optional → Option<T>, required always-encoded,
//! unpacked repeated, closed enums.

use super::*;

// ── Proto2 tests ─────────────────────────────────────────────────────

fn proto2_file(name: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.to_string()),
        syntax: Some("proto2".to_string()),
        ..Default::default()
    }
}

#[test]
fn test_proto2_optional_scalar_is_option() {
    let mut file = proto2_file("p2opt.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "count",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2opt.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 optional scalar should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub count: ::core::option::Option<i32>"),
        "proto2 optional int32 must be ::core::option::Option<i32>: {content}"
    );
}

#[test]
fn test_proto2_required_scalar_is_bare_type_and_always_encoded() {
    let mut file = proto2_file("p2req.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "count",
            1,
            Label::LABEL_REQUIRED,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2req.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 required scalar should generate");
    let content = &joined(&files);
    // Required fields use the bare type, not Option<T>.
    assert!(
        content.contains("pub count: i32"),
        "proto2 required int32 must be bare i32: {content}"
    );
    // Required fields must always be encoded; zero-default suppression must
    // not appear for this field.
    assert!(
        !content.contains("self.count != 0"),
        "proto2 required field must not have zero-default guard: {content}"
    );
}

#[test]
fn test_proto2_repeated_scalar_is_unpacked_by_default() {
    let mut file = proto2_file("p2rep.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "ids",
            1,
            Label::LABEL_REPEATED,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2rep.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 repeated scalar should generate");
    let content = &joined(&files);
    // Unpacked write_to: no packed-payload size accumulation.
    // (The view decode arm uses `let payload = borrow_bytes(...)` for its
    // lenient packed-accept path, so we look for the typed accumulator
    // `let payload: u32` that appears only in the packed write_to path.)
    assert!(
        !content.contains("let payload: u32"),
        "proto2 repeated scalar must be unpacked by default: {content}"
    );
    // Each element gets its own tag in write_to.
    assert!(
        content.contains("put_int32_field"),
        "missing put_int32_field in unpacked write_to: {content}"
    );
}

#[test]
fn test_proto2_optional_enum_is_option_enum_value() {
    let mut file = proto2_file("p2enum.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("color".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Color".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2enum.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 optional enum should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub color: ::core::option::Option<Color>"),
        "proto2 optional enum must be ::core::option::Option<Color> (closed enum): {content}"
    );
}

#[test]
fn test_proto2_enum_default_is_first_declared_variant() {
    // Enum with a zero-valued variant that is NOT listed first.  Proto2 default
    // is the first declared value regardless of its number; proto3 would prefer
    // the zero-valued one.
    let mut file = proto2_file("p2enumdef.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Priority".to_string()),
        value: vec![
            enum_value("HIGH", 1), // first declared, non-zero
            enum_value("NONE", 0), // zero-valued but listed second
            enum_value("LOW", 3),
        ],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2enumdef.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 enum default should generate");
    let content = &joined(&files);
    // Default impl must use HIGH (first declared), not NONE (zero-valued).
    assert!(
        content.contains("impl ::core::default::Default for Priority"),
        "missing Default impl: {content}"
    );
    // The default() body must reference HIGH, not NONE.
    let default_pos = content.find("fn default()").expect("missing fn default()");
    let after_default = &content[default_pos..default_pos + 80];
    assert!(
        after_default.contains("HIGH"),
        "proto2 enum Default must be first variant (HIGH), got: {after_default}"
    );
    assert!(
        !after_default.contains("NONE"),
        "proto2 enum Default must not be zero variant (NONE): {after_default}"
    );
}

// ── enum_type feature-override tests ─────────────────────────────────

fn proto2_open_enum_override_file(name: &str) -> FileDescriptorProto {
    let mut file = proto2_file(name);
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("color".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".pkg.Color".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("other".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".pkg.Color".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    file
}

#[test]
fn test_proto2_open_enum_override_rules_open_matching_fields() {
    for (case, rule) in [
        ("enum_type", ".pkg.Color"),
        ("message_prefix", ".pkg.Msg"),
        ("package_prefix", ".pkg"),
        ("catchall", "."),
    ] {
        let file_name = format!("p2_open_{case}.proto");
        let file = proto2_open_enum_override_file(&file_name);
        let config = CodeGenConfig {
            feature_overrides: open_enum_overrides(&[rule]),
            ..Default::default()
        };

        let files = generate(&[file], &[file_name], &config)
            .unwrap_or_else(|err| panic!("proto2 {case} override should generate: {err}"));
        let content = &joined(&files);

        assert!(
            content.contains("pub color: ::core::option::Option<::buffa::EnumValue<Color>>"),
            "{case} enum override must open color: {content}"
        );
        assert!(
            content.contains("pub other: ::core::option::Option<::buffa::EnumValue<Color>>"),
            "{case} enum override must open other: {content}"
        );
    }
}

#[test]
fn test_proto2_open_enum_override_field_path_only_opens_matching_field() {
    let file = proto2_open_enum_override_file("p2_open_field.proto");
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&[".pkg.Msg.color"]),
        ..Default::default()
    };

    let files = generate(&[file], &["p2_open_field.proto".to_string()], &config)
        .expect("proto2 field override should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub color: ::core::option::Option<::buffa::EnumValue<Color>>"),
        "field-specific enum override must open color: {content}"
    );
    assert!(
        content.contains("pub other: ::core::option::Option<Color>"),
        "field-specific override must not open sibling fields: {content}"
    );
}

#[test]
fn test_proto2_open_enum_override_direct_config_normalizes_paths_at_match_time() {
    let file = proto2_open_enum_override_file("p2_open_direct_normalized.proto");
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&["pkg.Msg.color.", "", "..."]),
        ..Default::default()
    };

    let files = generate(
        &[file],
        &["p2_open_direct_normalized.proto".to_string()],
        &config,
    )
    .expect("proto2 dotless/trailing-dot direct config should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub color: ::core::option::Option<::buffa::EnumValue<Color>>"),
        "dotless/trailing-dot override paths must match color: {content}"
    );
    assert!(
        content.contains("pub other: ::core::option::Option<Color>"),
        "empty/all-dot override rules must not become catch-all: {content}"
    );
}

#[test]
fn test_proto2_open_enum_override_inert_rule_warns() {
    let file = proto2_open_enum_override_file("p2_open_inert.proto");
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&[".pkg.Msg.color", ".pkg.Nope"]),
        ..Default::default()
    };

    let (_, warnings) =
        generate_with_diagnostics(&[file], &["p2_open_inert.proto".to_string()], &config)
            .expect("proto2 inert-rule config should generate");

    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::FeatureOverrideMatchedNothing { rule, .. } if rule == ".pkg.Nope"
        )),
        "an inert override rule must produce a warning: {warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::FeatureOverrideMatchedNothing { rule, .. } if rule == ".pkg.Msg.color"
        )),
        "a matched rule must not warn: {warnings:?}"
    );
}

#[test]
fn test_proto2_open_enum_override_repeated_and_packed_repeated_fields() {
    let mut file = proto2_file("p2_open_repeated.proto");
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    let mut packed = FieldDescriptorProto {
        name: Some("packed_colors".to_string()),
        number: Some(2),
        label: Some(Label::LABEL_REPEATED),
        r#type: Some(Type::TYPE_ENUM),
        type_name: Some(".pkg.Color".to_string()),
        ..Default::default()
    };
    packed.options = crate::generated::descriptor::FieldOptions {
        packed: Some(true),
        ..Default::default()
    }
    .into();
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("colors".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_REPEATED),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".pkg.Color".to_string()),
                ..Default::default()
            },
            packed,
        ],
        ..Default::default()
    });
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&[".pkg.Color"]),
        ..Default::default()
    };

    let files = generate(&[file], &["p2_open_repeated.proto".to_string()], &config)
        .expect("proto2 repeated enum override should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub colors: ::buffa::alloc::vec::Vec<::buffa::EnumValue<Color>>"),
        "repeated closed enum override must use EnumValue: {content}"
    );
    assert!(
        content.contains("pub packed_colors: ::buffa::alloc::vec::Vec<::buffa::EnumValue<Color>>"),
        "packed repeated closed enum override must use EnumValue: {content}"
    );
}

#[test]
fn test_proto2_open_enum_override_oneof_variant() {
    let mut file = proto2_file("p2_open_oneof.proto");
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("color".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".pkg.Color".to_string()),
            oneof_index: Some(0),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("choice".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&[".pkg.Msg.color"]),
        ..Default::default()
    };

    let files = generate(&[file], &["p2_open_oneof.proto".to_string()], &config)
        .expect("proto2 oneof enum override should generate");
    let content = &joined(&files);

    assert!(
        content.contains("Color(::buffa::EnumValue<"),
        "oneof enum override must use EnumValue payload: {content}"
    );
}

#[test]
fn test_proto2_open_enum_override_map_value_uses_outer_field_path() {
    let mut file = proto2_file("p2_open_map.proto");
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });

    let map_entry = DescriptorProto {
        name: Some("ColorsEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            FieldDescriptorProto {
                name: Some("value".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".pkg.Color".to_string()),
                ..Default::default()
            },
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("colors".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".pkg.Msg.ColorsEntry".to_string()),
            ..Default::default()
        }],
        nested_type: vec![map_entry],
        ..Default::default()
    });
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&[".pkg.Msg.colors"]),
        ..Default::default()
    };

    let files = generate(&[file], &["p2_open_map.proto".to_string()], &config)
        .expect("proto2 map enum override should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub colors: ::buffa::__private::HashMap<"),
        "map field should still use the default map collection: {content}"
    );
    assert!(
        content.contains("::buffa::EnumValue<Color>"),
        "map value override must use EnumValue by matching the outer field path: {content}"
    );
}

#[test]
fn test_proto2_open_enum_override_required_enum_default_uses_enum_value() {
    let mut file = proto2_file("p2_open_required_default.proto");
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Priority".to_string()),
        value: vec![enum_value("LOW", 0), enum_value("HIGH", 2)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("level".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REQUIRED),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".pkg.Priority".to_string()),
            default_value: Some("HIGH".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&["."]),
        ..Default::default()
    };

    let files = generate(
        &[file],
        &["p2_open_required_default.proto".to_string()],
        &config,
    )
    .expect("proto2 required enum default override should generate");
    let content = &joined(&files);

    assert!(
        content.contains("pub level: ::buffa::EnumValue<Priority>"),
        "required closed enum override must use EnumValue: {content}"
    );
    assert!(
        content.contains("level: ::buffa::EnumValue::Known(Priority::HIGH)"),
        "required enum default must wrap known values as EnumValue: {content}"
    );
}

#[test]
fn test_proto2_open_enum_override_required_enum_implicit_default_uses_enum_default() {
    let mut file = proto2_file("p2_open_required_implicit_default.proto");
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Priority".to_string()),
        value: vec![enum_value("HIGH", 2), enum_value("LOW", 0)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("level".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REQUIRED),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".pkg.Priority".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let config = CodeGenConfig {
        feature_overrides: open_enum_overrides(&["."]),
        ..Default::default()
    };

    let files = generate(
        &[file],
        &["p2_open_required_implicit_default.proto".to_string()],
        &config,
    )
    .expect("proto2 required enum implicit default override should generate");
    let content = &joined(&files);
    let compact = content.split_whitespace().collect::<String>();

    assert!(
        content.contains("pub level: ::buffa::EnumValue<Priority>"),
        "required closed enum override must use EnumValue: {content}"
    );
    assert!(
        compact.contains("level:::buffa::EnumValue::Known(")
            && compact.contains("<Priorityas::core::default::Default>::default()"),
        "required enum implicit default must wrap the enum's declared default: {content}"
    );
    assert!(
        compact.contains("self.level=::buffa::EnumValue::Known(")
            && compact.contains("<Priorityas::core::default::Default>::default()"),
        "clear() must restore the enum's declared default: {content}"
    );
}

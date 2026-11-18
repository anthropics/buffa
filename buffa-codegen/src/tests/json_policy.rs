//! `json_policy_types`: custom `bytes` and `string` representations whose
//! JSON goes through `ProtoBytesJson` / `ProtoStringJson`.

use super::*;

/// Package `policy.test`:
///
/// ```proto
/// message Holder {
///   bytes key = 1;
///   string name = 2;
///   map<string, Mode> modes = 3;
///   bytes plain = 4;
/// }
/// enum Mode { MODE_UNSPECIFIED = 0; }
/// ```
fn policy_file() -> FileDescriptorProto {
    let modes_entry = DescriptorProto {
        name: Some("ModesEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            FieldDescriptorProto {
                type_name: Some(".policy.test.Mode".to_string()),
                ..make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_ENUM)
            },
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let holder = DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
            make_field("name", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            FieldDescriptorProto {
                type_name: Some(".policy.test.Holder.ModesEntry".to_string()),
                ..make_field("modes", 3, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
            make_field("plain", 4, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
            make_field("embedding", 5, Label::LABEL_REPEATED, Type::TYPE_FLOAT),
            make_field("ordinary", 6, Label::LABEL_REPEATED, Type::TYPE_FLOAT),
        ],
        nested_type: vec![modes_entry],
        ..Default::default()
    };
    let mut file = proto3_file("policy.proto");
    file.package = Some("policy.test".to_string());
    file.message_type = vec![holder];
    file.enum_type = vec![EnumDescriptorProto {
        name: Some("Mode".to_string()),
        value: vec![enum_value("MODE_UNSPECIFIED", 0)],
        ..Default::default()
    }];
    file
}

fn policy_config() -> CodeGenConfig {
    let mut config = CodeGenConfig {
        generate_json: true,
        ..Default::default()
    };
    config.bytes_fields = vec![
        (
            ".policy.test.Holder.key".to_string(),
            BytesRepr::Custom("crate::Secret".to_string()),
        ),
        (
            ".policy.test.Holder.plain".to_string(),
            BytesRepr::Custom("crate::Plain".to_string()),
        ),
    ];
    config.string_fields = vec![(
        ".policy.test.Holder.name".to_string(),
        StringRepr::Custom("crate::Text".to_string()),
    )];
    config
}

fn generate_policy(config: &CodeGenConfig) -> Result<(String, Vec<CodeGenWarning>), CodeGenError> {
    generate_with_diagnostics(&[policy_file()], &["policy.proto".to_string()], config)
        .map(|(files, warnings)| (joined(&files), warnings))
}

#[test]
fn listed_types_use_their_policy_and_unlisted_types_are_unchanged() {
    let mut config = policy_config();
    config.json_policy_types = vec!["crate :: Secret".to_string(), "crate::Text".to_string()];
    let (code, warnings) = generate_policy(&config).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(code.contains("json_helpers::bytes_policy"), "{code}");
    assert!(code.contains("json_helpers::string_policy"), "{code}");
    assert!(code.contains("BytesPolicyJson"), "{code}");
    assert!(code.contains("StringPolicyJson"), "{code}");
    // The unlisted custom type keeps buffa's base64 helper.
    assert!(code.contains("json_helpers::bytes\""), "{code}");

    let (unlisted, _) = generate_policy(&policy_config()).unwrap();
    assert!(!unlisted.contains("Policy"), "{unlisted}");
}

#[test]
fn a_listed_type_no_rule_selects_is_an_error() {
    let mut config = policy_config();
    config.json_policy_types = vec!["crate::Secret".into(), "crate::Missing".into()];
    assert!(
        matches!(generate_policy(&config),Err(CodeGenError::JsonPolicyTypeMatchedNothing { rust_type }) if rust_type=="crate::Missing")
    );
}

#[test]
fn a_shadowed_representation_does_not_count_as_a_selected_policy_type() {
    let mut config = policy_config();
    config.json_policy_types = vec!["crate::Secret".into()];
    config.bytes_fields.push((
        ".policy.test.Holder.key".into(),
        BytesRepr::Custom("crate::Plain".into()),
    ));
    assert!(matches!(
        generate_policy(&config),
        Err(CodeGenError::JsonPolicyTypeMatchedNothing { .. })
    ));
}

#[test]
fn a_second_rule_with_a_near_miss_is_an_error() {
    let mut config = policy_config();
    config.json_policy_types = vec!["crate::Secret".into()];
    config.bytes_fields.push((
        ".policy.test.Holder.plain".into(),
        BytesRepr::Custom("::crate::Secret".into()),
    ));
    assert!(
        matches!(generate_policy(&config),Err(CodeGenError::JsonPolicyTypeNearMiss {selected,listed}) if selected=="::crate::Secret" && listed=="crate::Secret")
    );
}

#[test]
fn global_policy_bindings_do_not_require_fields_in_each_directory() {
    let mut config = policy_config();
    config.string_fields.push((
        ".other_directory.Secret.name".into(),
        StringRepr::Custom("crate::OtherText".into()),
    ));
    config.json_policy_types = vec!["crate::Secret".into(), "crate::OtherText".into()];
    // This request contains policy.proto only. The other directory's rule is
    // still a valid configuration binding, and its usual unmatched-field
    // warning remains visible rather than becoming a false policy error.
    let (_, warnings) = generate_policy(&config).unwrap();
    assert!(warnings
        .iter()
        .any(|warning| matches!(warning, CodeGenWarning::FieldTypeRuleMatchedNothing { .. })));
    config.json_policy_types.push("crate::Misspelled".into());
    assert!(matches!(
        generate_policy(&config),
        Err(CodeGenError::JsonPolicyTypeMatchedNothing { .. })
    ));
}

#[test]
fn deliberately_distinct_same_name_types_must_each_be_listed() {
    let mut config = policy_config();
    config.bytes_fields.push((
        ".policy.test.Holder.plain".into(),
        BytesRepr::Custom("other::Secret".into()),
    ));
    config.json_policy_types = vec!["crate::Secret".into(), "other::Secret".into()];
    assert!(generate_policy(&config).is_ok());
}

#[test]
fn policy_types_without_json_warn_and_change_nothing() {
    let mut config = policy_config();
    config.generate_json = false;
    config.json_policy_types = vec!["crate::Secret".to_string()];
    let (code, warnings) = generate_policy(&config).unwrap();
    assert_eq!(warnings, [CodeGenWarning::JsonPolicyTypesRequireJson]);
    assert!(!code.contains("Policy"));
}

#[test]
fn a_policy_string_key_over_enum_values_is_rejected() {
    let mut config = policy_config();
    config.string_fields.push((
        ".policy.test.Holder.modes".to_string(),
        StringRepr::Custom("crate::Text".to_string()),
    ));
    config.json_policy_types = vec!["crate::Text".to_string()];
    let err = generate_policy(&config).unwrap_err();
    assert!(
        matches!(&err, CodeGenError::JsonPolicyMapKey(path) if path == ".policy.test.Holder.modes"),
        "{err:?}"
    );
}

/// A `map` entry message named `name` with the given key and value fields.
fn map_entry(
    name: &str,
    key: FieldDescriptorProto,
    value: FieldDescriptorProto,
) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.to_string()),
        field: vec![key, value],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

/// A `map` field of message `.{package}.{message}` with entry `entry`.
fn map_field(
    package: &str,
    message: &str,
    name: &str,
    number: i32,
    entry: &str,
) -> FieldDescriptorProto {
    FieldDescriptorProto {
        type_name: Some(format!(".{package}.{message}.{entry}")),
        ..make_field(name, number, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
    }
}

/// Package `package`, file `{package}.proto`:
///
/// ```proto
/// message Box {
///   repeated bytes chain = 1;
///   repeated string labels = 2;
///   map<string, bytes> by_name = 3;
///   map<int32, string> texts = 4;
///   map<string, string> pairs = 5;
///   map<string, bytes> keyed = 6;
///   map<string, bytes> tagged = 7;
/// }
/// ```
fn containers_file(package: &str) -> FileDescriptorProto {
    let string_key = || make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    let value = |ty| make_field("value", 2, Label::LABEL_OPTIONAL, ty);
    let message = DescriptorProto {
        name: Some("Box".to_string()),
        field: vec![
            make_field("chain", 1, Label::LABEL_REPEATED, Type::TYPE_BYTES),
            make_field("labels", 2, Label::LABEL_REPEATED, Type::TYPE_STRING),
            map_field(package, "Box", "by_name", 3, "ByNameEntry"),
            map_field(package, "Box", "texts", 4, "TextsEntry"),
            map_field(package, "Box", "pairs", 5, "PairsEntry"),
            map_field(package, "Box", "keyed", 6, "KeyedEntry"),
            map_field(package, "Box", "tagged", 7, "TaggedEntry"),
        ],
        nested_type: vec![
            map_entry("ByNameEntry", string_key(), value(Type::TYPE_BYTES)),
            map_entry(
                "TextsEntry",
                make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
                value(Type::TYPE_STRING),
            ),
            map_entry("PairsEntry", string_key(), value(Type::TYPE_STRING)),
            map_entry("KeyedEntry", string_key(), value(Type::TYPE_BYTES)),
            map_entry("TaggedEntry", string_key(), value(Type::TYPE_BYTES)),
        ],
        ..Default::default()
    };
    let mut file = proto3_file(&format!("{package}.proto"));
    file.package = Some(package.to_string());
    file.message_type = vec![message];
    file
}

/// Every container rule of [`containers_file`]: `crate::Secret` (bytes) and
/// `crate::Text` (string) are the listed policy types, `crate::Tag` an
/// unlisted custom string key.
fn containers_config(package: &str) -> CodeGenConfig {
    let mut config = CodeGenConfig {
        generate_json: true,
        ..Default::default()
    };
    let secret = || BytesRepr::Custom("crate::Secret".to_string());
    let text = || StringRepr::Custom("crate::Text".to_string());
    config.bytes_fields = vec![(format!(".{package}.Box"), secret())];
    config.string_fields = vec![
        (format!(".{package}.Box.labels"), text()),
        (format!(".{package}.Box.texts"), text()),
        (format!(".{package}.Box.pairs"), text()),
        (format!(".{package}.Box.keyed"), text()),
        (
            format!(".{package}.Box.tagged"),
            StringRepr::Custom("crate::Tag".to_string()),
        ),
    ];
    config.json_policy_types = vec!["crate::Secret".to_string(), "crate::Text".to_string()];
    config
}

/// The generated module text of `files_to_generate`, one run.
fn generate_containers(files: &[FileDescriptorProto], package: &str) -> String {
    let (generated, warnings) = generate_with_diagnostics(
        files,
        &[format!("{package}.proto")],
        &containers_config(package),
    )
    .unwrap();
    assert!(
        warnings
            .iter()
            .all(|w| matches!(w, CodeGenWarning::FieldTypeRuleMatchedNothing { .. })),
        "{warnings:?}"
    );
    joined(&generated)
}

/// The `with` module a generated serde attribute names for field `field`.
fn with_module_of(code: &str, field: &str) -> String {
    let compact: String = code.split_whitespace().collect();
    let anchor = format!("pub{field}:");
    let at = compact
        .find(&anchor)
        .unwrap_or_else(|| panic!("no field {field} in {code}"));
    let attr_start = compact[..at]
        .rfind("#[serde(")
        .unwrap_or_else(|| panic!("no serde attribute on {field}"));
    let attr = &compact[attr_start..at];
    // The attribute must belong to this field, not to an earlier one.
    assert!(
        !attr.contains("pub"),
        "no serde attribute directly on {field}: {attr}"
    );
    let with = attr
        .find("with=\"")
        .unwrap_or_else(|| panic!("no with module on {field}: {attr}"));
    let rest = &attr[with + "with=\"".len()..];
    rest[..rest.find('"').unwrap()].to_string()
}

#[test]
fn policy_containers_use_buffa_policy_modules_and_emit_no_element_impl() {
    let code = generate_containers(&[containers_file("pa")], "pa");
    for (field, module) in [
        ("chain", "::buffa::json_helpers::repeated_bytes_policy"),
        ("labels", "::buffa::json_helpers::repeated_string_policy"),
        ("by_name", "::buffa::json_helpers::map_bytes_policy"),
        ("texts", "::buffa::json_helpers::map_string_policy"),
        (
            "pairs",
            "::buffa::json_helpers::map_string_policy_policy_key",
        ),
        (
            "keyed",
            "::buffa::json_helpers::map_bytes_policy_policy_key",
        ),
        (
            "tagged",
            "::buffa::json_helpers::map_bytes_policy_serde_key",
        ),
    ] {
        assert_eq!(with_module_of(&code, field), module, "field {field}");
    }
    // The policy types get no codegen-emitted JSON element impl, so they may
    // come from another crate.
    assert!(!code.contains("ProtoElemJson for crate::"), "{code}");
    assert!(!code.contains("proto_seq"), "{code}");
    assert!(!code.contains("proto_map"), "{code}");
    assert!(!code.contains("proto_str_key_map"), "{code}");
    assert!(!code.contains("policy_str_key_map"), "{code}");
}

#[test]
fn separate_runs_sharing_a_policy_type_in_containers_emit_no_duplicate_impls() {
    // Two packages generated by two runs into one crate, as buf's directory
    // strategy or two `compile()` calls do. Each uses the listed types in
    // repeated fields and map values. JSON only: vtable reflection still emits
    // a `ReflectElement` impl per run, as documented.
    let files = [containers_file("pa"), containers_file("pb")];
    let first = generate_containers(&files, "pa");
    let second = generate_containers(&files, "pb");
    let both = format!("{first}\n{second}");
    assert!(!both.contains("ProtoElemJson for crate::"), "{both}");

    // An unlisted custom bytes element still gets its base64 element impl in
    // every run that uses it; the policy listing is what removes it.
    let mut unlisted = containers_config("pa");
    unlisted.json_policy_types.clear();
    let (generated, _) =
        generate_with_diagnostics(&files, &["pa.proto".to_string()], &unlisted).unwrap();
    assert!(joined(&generated).contains("ProtoElemJson for crate::Secret"));
}

#[test]
fn a_policy_value_under_a_byte_key_keeps_its_policy() {
    use crate::generated::descriptor::{
        feature_set::Utf8Validation, Edition, FeatureSet, FieldOptions,
    };
    // `map<string, string>` whose key alone has UTF-8 validation disabled:
    // with `strict_utf8_mapping` the key becomes bytes and the value stays a
    // string of the listed policy type.
    let key = FieldDescriptorProto {
        options: FieldOptions {
            features: FeatureSet {
                utf8_validation: Some(Utf8Validation::NONE),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
        .into(),
        ..make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING)
    };
    let file = FileDescriptorProto {
        name: Some("raw.proto".to_string()),
        package: Some("raw".to_string()),
        syntax: Some("editions".to_string()),
        edition: Some(Edition::EDITION_2023),
        message_type: vec![DescriptorProto {
            name: Some("Raw".to_string()),
            field: vec![map_field("raw", "Raw", "by_key", 1, "ByKeyEntry")],
            nested_type: vec![map_entry(
                "ByKeyEntry",
                key,
                make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            )],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut config = CodeGenConfig {
        generate_json: true,
        strict_utf8_mapping: true,
        ..Default::default()
    };
    config.string_fields = vec![(
        ".raw.Raw.by_key".to_string(),
        StringRepr::Custom("crate::Text".to_string()),
    )];
    config.json_policy_types = vec!["crate::Text".to_string()];
    let (generated, _) =
        generate_with_diagnostics(&[file], &["raw.proto".to_string()], &config).unwrap();
    let code = joined(&generated);
    assert_eq!(
        with_module_of(&code, "by_key"),
        "::buffa::json_helpers::map_string_policy_bytes_key"
    );
    assert!(!code.contains("bytes_key_map"), "{code}");
    assert!(!code.contains("ProtoElemJson for crate::"), "{code}");
}

#[test]
fn reflection_with_policy_types_warns_that_reflective_json_bypasses_them() {
    let mut config = policy_config();
    config.json_policy_types = vec!["crate::Secret".to_string()];
    config.generate_reflection = true;
    let (_, warnings) = generate_policy(&config).unwrap();
    assert_eq!(
        warnings,
        [CodeGenWarning::JsonPolicyReflectionBypass {
            policy_types: vec!["crate :: Secret".into()]
        }]
    );
    let text = warnings[0].to_string();
    assert!(text.contains("DynamicMessage"), "{text}");
    assert!(text.contains("refuses export"), "{text}");
    assert!(text.contains("import checks"), "{text}");
    assert!(text.contains("try_from_message"), "{text}");
    assert!(text.contains("not a confidentiality boundary"), "{text}");

    // Vtable reflection implies reflection: still one warning.
    config.generate_reflection_vtable = true;
    assert_eq!(
        generate_policy(&config).unwrap().1,
        [CodeGenWarning::JsonPolicyReflectionBypass {
            policy_types: vec!["crate :: Secret".into()]
        }]
    );
    config.generate_reflection_vtable = false;

    // No warning without reflection, without listed types, or without JSON
    // (where the policies are inactive and a different warning applies).
    config.generate_reflection = false;
    assert!(generate_policy(&config).unwrap().1.is_empty());
    let mut unlisted = policy_config();
    unlisted.generate_reflection = true;
    assert!(generate_policy(&unlisted).unwrap().1.is_empty());
    config.generate_reflection = true;
    config.generate_json = false;
    assert_eq!(
        generate_policy(&config).unwrap().1,
        [CodeGenWarning::JsonPolicyTypesRequireJson]
    );
}

#[test]
fn equivalent_custom_type_spellings_emit_one_impl_per_trait() {
    for policy in [false, true] {
        let mut config = containers_config("pa");
        config.generate_reflection = true;
        config.generate_reflection_vtable = true;
        config.bytes_fields.push((
            ".pa.Box.chain".into(),
            BytesRepr::Custom("crate :: Secret".into()),
        ));
        config.string_fields.push((
            ".pa.Box.pairs".into(),
            StringRepr::Custom("crate :: Text".into()),
        ));
        config.bytes_fields.push((
            ".pa.Box.by_name".into(),
            BytesRepr::Custom("((crate::r#Secret))".into()),
        ));
        config.string_fields.push((
            ".pa.Box.keyed".into(),
            StringRepr::Custom("(crate::Text)".into()),
        ));
        if !policy {
            config.json_policy_types.clear();
        }
        let generated = generate(&[containers_file("pa")], &["pa.proto".into()], &config).unwrap();
        fn count_impls(items: &[syn::Item], trait_name: &str, type_key: &str) -> usize {
            items
                .iter()
                .map(|item| match item {
                    syn::Item::Mod(module) => module
                        .content
                        .as_ref()
                        .map_or(0, |(_, items)| count_impls(items, trait_name, type_key)),
                    syn::Item::Impl(implementation) => {
                        let matches_trait = implementation
                            .trait_
                            .as_ref()
                            .and_then(|(_, path, _)| path.segments.last())
                            .is_some_and(|segment| segment.ident == trait_name);
                        let ty = &implementation.self_ty;
                        usize::from(
                            matches_trait
                                && crate::context::rust_type_key(&quote::quote!(#ty).to_string())
                                    == type_key,
                        )
                    }
                    _ => 0,
                })
                .sum()
        }
        let parsed = syn::parse_file(&joined(&generated)).unwrap();
        for (trait_name, ty, expected) in [
            ("ProtoElemJson", "crate::Secret", usize::from(!policy)),
            ("ReflectElement", "crate::Secret", 1),
            ("ReflectElement", "crate::Text", 1),
            ("ReflectMapKey", "crate::Text", 1),
            ("ReflectMapKey", "crate::Tag", 1),
        ] {
            assert_eq!(
                count_impls(
                    &parsed.items,
                    trait_name,
                    &crate::context::rust_type_key(ty)
                ),
                expected,
                "{trait_name} for {ty}"
            );
        }
    }
}

#[test]
fn unused_policy_rules_do_not_warn_about_reflection() {
    let mut config = policy_config();
    config.generate_reflection = true;
    config.bytes_fields = vec![(
        ".another.Directory".into(),
        BytesRepr::Custom("crate::Secret".into()),
    )];
    config.json_policy_types = vec!["crate::Secret".into()];
    let (_, warnings) = generate_policy(&config).unwrap();
    assert!(!warnings
        .iter()
        .any(|warning| matches!(warning, CodeGenWarning::JsonPolicyReflectionBypass { .. })));
}

#[test]
fn reflection_warning_names_only_selected_policy_types_once() {
    let mut config = policy_config();
    config.generate_reflection = true;
    config.generate_views = false;
    config.json_policy_types = vec!["crate::Text".into(), "((crate::Secret))".into()];
    let (_, warnings) = generate_policy(&config).unwrap();
    assert_eq!(
        warnings,
        [CodeGenWarning::JsonPolicyReflectionBypass {
            policy_types: vec!["crate :: Secret".into(), "crate :: Text".into()],
        }]
    );

    // Oneof-only use is tracked even with borrowed-view generation disabled.
    let mut file = policy_file();
    file.message_type[0]
        .oneof_decl
        .push(crate::generated::descriptor::OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        });
    file.message_type[0].field[0].oneof_index = Some(0);
    config.json_policy_types = vec!["crate::Secret".into()];
    let (_, warnings) =
        generate_with_diagnostics(&[file], &["policy.proto".into()], &config).unwrap();
    assert_eq!(
        warnings,
        [CodeGenWarning::JsonPolicyReflectionBypass {
            policy_types: vec!["crate :: Secret".into()],
        }]
    );
}

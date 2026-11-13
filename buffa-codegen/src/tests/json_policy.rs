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

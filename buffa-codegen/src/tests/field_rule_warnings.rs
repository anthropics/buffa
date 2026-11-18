//! `CodeGenWarning::FieldTypeRuleMatchedNothing`: a `bytes_fields`,
//! `string_fields`, `map_fields` or `repeated_fields` rule that matches no
//! field of a generated message.

use super::*;

/// Package `rules.test`:
///
/// ```proto
/// message Outer {
///   string name = 1;
///   map<string, bytes> blobs = 2;
///   message Inner { repeated int32 ids = 1; }
/// }
/// ```
fn rules_file() -> FileDescriptorProto {
    let blobs_entry = DescriptorProto {
        name: Some("BlobsEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let inner = DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field(
            "ids",
            1,
            Label::LABEL_REPEATED,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    };
    let outer = DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            FieldDescriptorProto {
                type_name: Some(".rules.test.Outer.BlobsEntry".to_string()),
                ..make_field("blobs", 2, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
        ],
        nested_type: vec![blobs_entry, inner],
        ..Default::default()
    };

    let mut file = proto3_file("rules.proto");
    file.package = Some("rules.test".to_string());
    file.message_type = vec![outer];
    file
}

/// Package `dep`, passed to the generator as a dependency only.
fn dep_file() -> FileDescriptorProto {
    let mut file = proto3_file("dep.proto");
    file.package = Some("dep".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Dep".to_string()),
        field: vec![make_field(
            "value",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    file
}

/// The `(option, rule, matches_ungenerated)` of every
/// `FieldTypeRuleMatchedNothing` in `warnings`, sorted.
fn inert_rules(warnings: Vec<CodeGenWarning>) -> Vec<(&'static str, String, bool)> {
    let mut inert: Vec<(&'static str, String, bool)> = warnings
        .into_iter()
        .filter_map(|w| match w {
            CodeGenWarning::FieldTypeRuleMatchedNothing {
                rule,
                option,
                matches_ungenerated,
                ..
            } => Some((option, rule, matches_ungenerated)),
            _ => None,
        })
        .collect();
    inert.sort_unstable();
    inert
}

/// Generates `rules.proto` with `dep.proto` as a dependency and returns the
/// inert field-type rules.
fn inert_rules_for(config: &CodeGenConfig) -> Vec<(&'static str, String, bool)> {
    let (_, warnings) = generate_with_diagnostics(
        &[rules_file(), dep_file()],
        &["rules.proto".to_string()],
        config,
    )
    .expect("should generate");
    inert_rules(warnings)
}

#[test]
fn no_rules_no_warning() {
    assert!(inert_rules_for(&CodeGenConfig::default()).is_empty());
}

#[test]
fn a_rule_that_matches_no_generated_field_warns() {
    let live = [
        ".",
        ".rules.test",
        ".rules.test.Outer",
        ".rules.test.Outer.name",
        ".rules.test.Outer.blobs",
        ".rules.test.Outer.Inner",
        ".rules.test.Outer.Inner.ids",
    ];
    // A field of a message that is not being generated, a map-entry message
    // and its value field, a field-name prefix, a typo, a prost-style bare
    // name, and a path without its leading dot. Sorted, to compare against
    // the sorted warnings. Only the first matches a field in `dep.proto`.
    let inert = [
        (".dep.Dep.value", true),
        (".rules.test.Outer.BlobsEntry", false),
        (".rules.test.Outer.BlobsEntry.value", false),
        (".rules.test.Outer.nam", false),
        (".rules.test.Outer.nope", false),
        ("name", false),
        ("rules.test.Outer.name", false),
    ];
    let config = CodeGenConfig {
        string_fields: live
            .iter()
            .copied()
            .chain(inert.iter().map(|(rule, _)| *rule))
            .map(|rule| (rule.to_string(), StringRepr::String))
            .collect(),
        ..Default::default()
    };

    let expected: Vec<(&str, String, bool)> = inert
        .iter()
        .map(|(rule, ungenerated)| ("string_fields", (*rule).to_string(), *ungenerated))
        .collect();
    assert_eq!(inert_rules_for(&config), expected);
}

#[test]
fn each_field_type_rule_list_is_checked() {
    // `Inner` has one `repeated int32` field, so the live rules change no
    // generated type.
    let live = ".rules.test.Outer.Inner".to_string();
    let inert = ".rules.test.Nope".to_string();
    let config = CodeGenConfig {
        bytes_fields: vec![
            (live.clone(), BytesRepr::Bytes),
            (inert.clone(), BytesRepr::Bytes),
        ],
        string_fields: vec![
            (live.clone(), StringRepr::String),
            (inert.clone(), StringRepr::String),
        ],
        map_fields: vec![
            (live.clone(), MapRepr::BTreeMap),
            (inert.clone(), MapRepr::BTreeMap),
        ],
        repeated_fields: vec![
            (live, RepeatedRepr::Vec),
            (inert.clone(), RepeatedRepr::Vec),
        ],
        ..Default::default()
    };

    assert_eq!(
        inert_rules_for(&config),
        [
            ("bytes_fields", inert.clone(), false),
            ("map_fields", inert.clone(), false),
            ("repeated_fields", inert.clone(), false),
            ("string_fields", inert, false),
        ]
    );
}

/// A file with no package:
///
/// ```proto
/// message Holder {
///   string name = 1;
///   map<string, int32> counts = 2;
///   oneof pick { string label = 3; int32 code = 4; }
///   message Inner { string tag = 1; }
/// }
/// ```
fn holder_file() -> FileDescriptorProto {
    let counts_entry = DescriptorProto {
        name: Some("CountsEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let inner = DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field(
            "tag",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    };
    let in_pick = |mut field: FieldDescriptorProto| {
        field.oneof_index = Some(0);
        field
    };
    let holder = DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            FieldDescriptorProto {
                type_name: Some(".Holder.CountsEntry".to_string()),
                ..make_field("counts", 2, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
            in_pick(make_field(
                "label",
                3,
                Label::LABEL_OPTIONAL,
                Type::TYPE_STRING,
            )),
            in_pick(make_field(
                "code",
                4,
                Label::LABEL_OPTIONAL,
                Type::TYPE_INT32,
            )),
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("pick".to_string()),
            ..Default::default()
        }],
        nested_type: vec![counts_entry, inner],
        ..Default::default()
    };

    let mut file = proto3_file("holder.proto");
    file.message_type = vec![holder];
    file
}

/// Ties the check to the generator: for each field shape, the path the check
/// accepts is the path that changes the generated type.
#[test]
fn a_rule_the_generator_applies_does_not_warn() {
    let generate_with = |rule: &str| {
        let config = CodeGenConfig {
            string_fields: vec![(
                rule.to_string(),
                StringRepr::Custom("::my::Str".to_string()),
            )],
            ..Default::default()
        };
        let (files, warnings) =
            generate_with_diagnostics(&[holder_file()], &["holder.proto".to_string()], &config)
                .expect("should generate");
        (joined(&files).contains("::my::Str"), inert_rules(warnings))
    };

    // A singular field, a map key, a oneof member and a nested message's
    // field, in a file without a package.
    for rule in [
        ".Holder.name",
        ".Holder.counts",
        ".Holder.label",
        ".Holder.Inner.tag",
    ] {
        let (applied, inert) = generate_with(rule);
        assert!(applied, "{rule} did not change a generated type");
        assert!(inert.is_empty(), "{rule} warned: {inert:?}");
    }

    // `box_type_in` and `unbox_oneof_in` address a oneof member through the
    // oneof's name. These lists do not, and the check agrees with the
    // generator.
    let (applied, inert) = generate_with(".Holder.pick.label");
    assert!(!applied);
    assert_eq!(
        inert,
        [("string_fields", ".Holder.pick.label".to_string(), false)]
    );
}

/// Package `ext.test`, proto2:
///
/// ```proto
/// message Holder {
///   optional bytes data = 1;
///   extensions 100 to 199;
///   extend Holder { optional bytes nested_ext = 101; }
/// }
/// extend Holder { optional bytes file_ext = 100; }
/// ```
fn extension_file() -> FileDescriptorProto {
    let ext = |name: &str, number: i32| FieldDescriptorProto {
        extendee: Some(".ext.test.Holder".to_string()),
        ..make_field(name, number, Label::LABEL_OPTIONAL, Type::TYPE_BYTES)
    };
    let holder = DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![make_field(
            "data",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_BYTES,
        )],
        extension: vec![ext("nested_ext", 101)],
        extension_range: vec![
            crate::generated::descriptor::descriptor_proto::ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    FileDescriptorProto {
        name: Some("ext.proto".to_string()),
        package: Some("ext.test".to_string()),
        syntax: Some("proto2".to_string()),
        message_type: vec![holder],
        extension: vec![ext("file_ext", 100)],
        ..Default::default()
    }
}

#[test]
fn a_rule_naming_an_extension_says_so() {
    let rules = [
        ".ext.test.file_ext",
        ".ext.test.Holder.nested_ext",
        ".ext.test.Holder.data",
        ".ext.test.Holder.nope",
    ];
    let config = CodeGenConfig {
        bytes_fields: rules
            .iter()
            .map(|rule| ((*rule).to_string(), BytesRepr::Bytes))
            .collect(),
        ..Default::default()
    };
    let (_, warnings) =
        generate_with_diagnostics(&[extension_file()], &["ext.proto".to_string()], &config)
            .expect("should generate");
    let mut inert: Vec<(String, bool)> = warnings
        .into_iter()
        .filter_map(|w| match w {
            CodeGenWarning::FieldTypeRuleMatchedNothing {
                rule,
                names_extension,
                ..
            } => Some((rule, names_extension)),
            _ => None,
        })
        .collect();
    inert.sort_unstable();
    assert_eq!(
        inert,
        [
            (".ext.test.Holder.nested_ext".to_string(), true),
            (".ext.test.Holder.nope".to_string(), false),
            (".ext.test.file_ext".to_string(), true),
        ]
    );
}

#[test]
fn the_warning_names_the_rule_kind_and_its_options() {
    let typo = CodeGenWarning::FieldTypeRuleMatchedNothing {
        rule: ".items".to_string(),
        option: "map_fields",
        matches_ungenerated: false,
        names_extension: false,
    }
    .to_string();
    assert!(
        typo.starts_with(
            "map type rule '.items' (map_type_in / map_type_custom_in) \
             matched no field of a generated message"
        ),
        "{typo}"
    );
    assert!(typo.contains("a separate run generates"), "{typo}");

    let dependency = CodeGenWarning::FieldTypeRuleMatchedNothing {
        rule: ".dep.Dep.value".to_string(),
        option: "bytes_fields",
        matches_ungenerated: true,
        names_extension: false,
    }
    .to_string();
    assert!(
        dependency.starts_with(
            "bytes type rule '.dep.Dep.value' (bytes_type_in / use_bytes_type_in / \
             bytes_type_custom_in) matches only fields of messages this run does not generate"
        ),
        "{dependency}"
    );

    let extension = CodeGenWarning::FieldTypeRuleMatchedNothing {
        rule: ".ext.test.file_ext".to_string(),
        option: "string_fields",
        matches_ungenerated: false,
        names_extension: true,
    }
    .to_string();
    assert!(
        extension.starts_with(
            "string type rule '.ext.test.file_ext' (string_type_in / string_type_custom_in) \
             names (or covers only) extension fields"
        ),
        "{extension}"
    );
    assert!(
        extension.contains("do not apply to extensions"),
        "{extension}"
    );

    // The text is shared by `buffa-build` and `protoc-gen-buffa`, so it names
    // neither front end, nor the internal `CodeGenConfig` list.
    for text in [&typo, &dependency, &extension] {
        assert!(!text.contains("buffa-build"), "{text}");
        assert!(!text.contains("_fields"), "{text}");
    }
}

#[test]
fn extension_detection_covers_files_without_a_package_and_ungenerated_files() {
    // No package: the extension's name is `.file_ext`.
    let mut unpackaged = extension_file();
    unpackaged.name = Some("unpackaged.proto".to_string());
    unpackaged.package = None;
    for ext in unpackaged
        .extension
        .iter_mut()
        .chain(unpackaged.message_type[0].extension.iter_mut())
    {
        ext.extendee = Some(".Holder".to_string());
    }
    let config = CodeGenConfig {
        bytes_fields: vec![(".file_ext".to_string(), BytesRepr::Bytes)],
        ..Default::default()
    };
    let (_, warnings) = generate_with_diagnostics(
        &[unpackaged.clone()],
        &["unpackaged.proto".to_string()],
        &config,
    )
    .expect("should generate");
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::FieldTypeRuleMatchedNothing {
                names_extension: true,
                matches_ungenerated: false,
                ..
            }
        )),
        "{warnings:?}"
    );

    // `ext.proto` only as a dependency: the package rule matches a field there
    // and its extensions, and the `Display` text gives the first reason.
    let config = CodeGenConfig {
        bytes_fields: vec![(".ext.test".to_string(), BytesRepr::Bytes)],
        ..Default::default()
    };
    let (_, warnings) = generate_with_diagnostics(
        &[extension_file(), unpackaged],
        &["unpackaged.proto".to_string()],
        &config,
    )
    .expect("should generate");
    let warning = warnings
        .iter()
        .find(|w| {
            matches!(
                w,
                CodeGenWarning::FieldTypeRuleMatchedNothing {
                    names_extension: true,
                    matches_ungenerated: true,
                    ..
                }
            )
        })
        .unwrap_or_else(|| panic!("{warnings:?}"));
    assert!(
        warning
            .to_string()
            .contains("matches only fields of messages this run does not generate"),
        "{warning}"
    );
}

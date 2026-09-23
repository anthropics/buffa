//! `CodecStrategy::Table`: which messages get a table, and what codegen says
//! about the ones that cannot.

use super::*;

/// A message field of message type `type_name` (a proto path with a leading dot).
fn message_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        type_name: Some(type_name.to_string()),
        ..make_field(name, number, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE)
    }
}

fn message(name: &str, fields: Vec<FieldDescriptorProto>) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.to_string()),
        field: fields,
        ..Default::default()
    }
}

fn scalar(name: &str, number: i32, ty: Type) -> FieldDescriptorProto {
    make_field(name, number, Label::LABEL_OPTIONAL, ty)
}

/// Package `t` with:
///
/// - `Plain`, `Leaf`, and `HasLeaf` (holds a `Leaf`), which can use the table;
/// - `Oneofy` (has a string with a custom type, which the table does not
///   support), which cannot, and `HasOneofy` (holds an `Oneofy`), which can,
///   because a table message may hold a message that has no table;
/// - `Outer` with a nested `Inner`, both plain.
fn schema() -> FileDescriptorProto {
    let oneofy = message("Oneofy", vec![scalar("a", 1, Type::TYPE_STRING)]);
    let mut outer = message("Outer", vec![scalar("x", 1, Type::TYPE_INT32)]);
    outer.nested_type = vec![message("Inner", vec![scalar("y", 1, Type::TYPE_STRING)])];
    FileDescriptorProto {
        package: Some("t".to_string()),
        message_type: vec![
            message(
                "Plain",
                vec![
                    scalar("a", 1, Type::TYPE_INT32),
                    scalar("s", 2, Type::TYPE_STRING),
                ],
            ),
            message("Leaf", vec![scalar("x", 1, Type::TYPE_INT32)]),
            message("HasLeaf", vec![message_field("leaf", 1, ".t.Leaf")]),
            oneofy,
            message("HasOneofy", vec![message_field("o", 1, ".t.Oneofy")]),
            outer,
        ],
        ..proto3_file("t.proto")
    }
}

/// `.t.Oneofy.a` has a custom string type, which the table cannot use.
fn with_custom_string(config: &CodeGenConfig) -> CodeGenConfig {
    let mut config = config.clone();
    config.string_fields.push((
        ".t.Oneofy.a".to_string(),
        StringRepr::Custom("crate::Str".to_string()),
    ));
    config
}

fn run(config: &CodeGenConfig) -> Result<(String, Vec<CodeGenWarning>), CodeGenError> {
    let (files, warnings) = generate_with_diagnostics(
        &[schema()],
        &["t.proto".to_string()],
        &with_custom_string(config),
    )?;
    Ok((joined(&files), warnings))
}

/// The reason `Oneofy` cannot use the table.
const CUSTOM: &str = "has a field with a custom string, bytes or collection type";

fn table_config(strategy: CodecStrategy) -> CodeGenConfig {
    CodeGenConfig {
        codec_strategy: strategy,
        ..Default::default()
    }
}

/// `code` without whitespace: prettyplease does not format the inside of a
/// macro call, so its line breaks are arbitrary.
fn squashed(code: &str) -> String {
    code.split_whitespace().collect()
}

/// The messages with a static table in `code`, in order of appearance.
fn tables(code: &str) -> Vec<String> {
    code.match_indices("static __BUFFA_TABLE_")
        .map(|(i, _)| {
            code[i + "static __BUFFA_TABLE_".len()..]
                .split(':')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// The reasons of the summary warning, as `(reason, number of messages)`, and the counts of
/// fallen-back and selected messages.
fn summary(warnings: &[CodeGenWarning]) -> ((usize, usize), Vec<(&str, usize)>) {
    let found: Vec<_> = warnings
        .iter()
        .filter_map(|w| match w {
            CodeGenWarning::TableCodecFallbackSummary {
                fallbacks,
                selected,
                reasons,
            } => Some((
                (*fallbacks, *selected),
                reasons
                    .iter()
                    .map(|r| (r.reason.as_str(), r.messages.len()))
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one summary in {warnings:?}");
    found.into_iter().next().unwrap()
}

fn table_warnings(warnings: &[CodeGenWarning]) -> Vec<&CodeGenWarning> {
    warnings
        .iter()
        .filter(|w| {
            matches!(
                w,
                CodeGenWarning::TableCodecFallbackSummary { .. }
                    | CodeGenWarning::CodecStrategyRuleMatchedNothing { .. }
            )
        })
        .collect()
}

#[test]
fn the_default_is_unrolled_and_says_nothing() {
    let (code, warnings) = run(&CodeGenConfig::default()).unwrap();
    assert!(tables(&code).is_empty());
    assert!(!code.contains("buffa::table"));
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
}

#[test]
fn the_global_setting_gives_a_table_to_every_message_that_can_use_one() {
    let (code, warnings) = run(&table_config(CodecStrategy::Table)).unwrap();
    assert_eq!(
        tables(&code),
        ["Plain", "Leaf", "HasLeaf", "HasOneofy", "Outer", "Inner"]
    );
    // The one with a custom string falls back, and one warning covers the run.
    let (counts, reasons) = summary(&warnings);
    assert_eq!(counts, (1, 7));
    assert_eq!(reasons, [(CUSTOM, 1)]);
    let text = table_warnings(&warnings)[0].to_string();
    assert!(text.starts_with("1 of 7 messages selected for the table codec"));
    assert!(text.contains(&format!("{CUSTOM} (1: .t.Oneofy)")), "{text}");
    assert!(text.contains("codec_strategy_in=<path>=unrolled"), "{text}");
}

#[test]
fn a_table_message_forwards_message_to_the_shared_interpreters() {
    let (code, _) = run(&table_config(CodecStrategy::Table)).unwrap();
    assert!(code.contains("::buffa::__table!"));
    assert!(code.contains("__BUFFA_TABLE_Plain.compute_size(self, cache)"));
    assert!(code.contains("__BUFFA_TABLE_Plain.merge_field(self, tag, buf, ctx)"));
    // None of the per-field code an unrolled impl has: the size and write
    // statements for `a` would name `self.a`.
    let plain = code
        .split("impl ::buffa::Message for Plain")
        .nth(1)
        .and_then(|rest| rest.split("impl ::buffa::Message for").next())
        .unwrap();
    assert!(!plain.contains("self.a"), "{plain}");
}

#[test]
fn a_child_message_is_referenced_through_its_own_table() {
    let (code, _) = run(&table_config(CodecStrategy::Table)).unwrap();
    let code = squashed(&code);
    let has_leaf = code.split("static__BUFFA_TABLE_HasLeaf").nth(1).unwrap();
    assert!(
        has_leaf.contains("Aux::Msg(&::buffa::table::MsgVt::new::<::buffa::MessageField<Leaf,::buffa::Inline<Leaf>>>(&__BUFFA_TABLE_Leaf))"),
        "{has_leaf}"
    );
}

#[test]
fn a_child_without_a_table_is_reached_through_its_message_impl() {
    let (code, _) = run(&table_config(CodecStrategy::Table)).unwrap();
    let code = squashed(&code);
    // `HasOneofy` is a table message that holds `Oneofy`, which is unrolled.
    let holder = code.split("static__BUFFA_TABLE_HasOneofy").nth(1).unwrap();
    let holder = holder
        .split("impl::buffa::MessageforHasOneofy")
        .next()
        .unwrap();
    assert!(
        holder.contains("Aux::Msg(&::buffa::table::MsgVt::new_via_message::<::buffa::MessageField<Oneofy,::buffa::Inline<Oneofy>>>())"),
        "{holder}"
    );
    assert!(!holder.contains("__BUFFA_TABLE_Oneofy"), "{holder}");
}

#[test]
fn a_repeated_child_without_a_table_is_reached_through_its_message_impl() {
    let mut file = schema();
    file.message_type[4].field[0].label = Some(Label::LABEL_REPEATED);
    let (files, _) = generate_with_diagnostics(
        &[file],
        &["t.proto".to_string()],
        &with_custom_string(&table_config(CodecStrategy::Table)),
    )
    .unwrap();
    let code = squashed(&joined(&files));
    let holder = code.split("static__BUFFA_TABLE_HasOneofy").nth(1).unwrap();
    let holder = holder
        .split("impl::buffa::MessageforHasOneofy")
        .next()
        .unwrap();
    assert!(
        holder.contains("Aux::Rep(&::buffa::table::RepVt::new_via_message::<Oneofy>())"),
        "{holder}"
    );
    assert!(holder.contains("Vec<Oneofy>"), "{holder}");
}

#[test]
fn a_child_in_another_package_is_reached_through_the_path_idiomatic_imports_shortens() {
    let other = FileDescriptorProto {
        package: Some("a".to_string()),
        message_type: vec![message("Cold", vec![scalar("c", 1, Type::TYPE_INT32)])],
        ..proto3_file("a.proto")
    };
    let holder = FileDescriptorProto {
        package: Some("b".to_string()),
        dependency: vec!["a.proto".to_string()],
        message_type: vec![message(
            "Holder",
            vec![
                message_field("cold", 1, ".a.Cold"),
                repeated_message_field("colds", 2, ".a.Cold"),
            ],
        )],
        ..proto3_file("b.proto")
    };
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".a.Cold".to_string(), CodecStrategy::Unrolled)],
        idiomatic_imports: true,
        file_per_package: true,
        ..table_config(CodecStrategy::Table)
    };
    let (files, warnings) = generate_with_diagnostics(
        &[other, holder],
        &["a.proto".to_string(), "b.proto".to_string()],
        &config,
    )
    .unwrap();
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
    let code = squashed(&joined(&files));
    let holder = code.split("static__BUFFA_TABLE_Holder").nth(1).unwrap();
    let holder = holder
        .split("impl::buffa::MessageforHolder")
        .next()
        .unwrap();
    assert!(
        holder.contains("MsgVt::new_via_message::<MessageField<Cold,::buffa::Inline<Cold>>>()"),
        "{holder}"
    );
    assert!(
        holder.contains("RepVt::new_via_message::<Cold>()"),
        "{holder}"
    );
}

#[test]
fn a_rule_selects_messages_when_the_global_setting_is_unrolled() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".t.Plain".to_string(), CodecStrategy::Table)],
        ..Default::default()
    };
    let (code, warnings) = run(&config).unwrap();
    assert_eq!(tables(&code), ["Plain"]);
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
}

#[test]
fn the_last_matching_rule_wins() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![
            (".t".to_string(), CodecStrategy::Table),
            (".t.Plain".to_string(), CodecStrategy::Unrolled),
            (".t.Outer".to_string(), CodecStrategy::Unrolled),
            (".t.Outer.Inner".to_string(), CodecStrategy::Table),
        ],
        ..Default::default()
    };
    let (code, _) = run(&config).unwrap();
    // `Outer` is unrolled, though its nested message is a table.
    assert_eq!(tables(&code), ["Leaf", "HasLeaf", "HasOneofy", "Inner"]);
}

#[test]
fn a_message_can_have_a_table_when_it_holds_a_message_set_to_unrolled() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![
            (".t.HasLeaf".to_string(), CodecStrategy::Table),
            (".t.Leaf".to_string(), CodecStrategy::Unrolled),
        ],
        ..Default::default()
    };
    let (code, warnings) = run(&config).unwrap();
    assert_eq!(tables(&code), ["HasLeaf"]);
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
    assert!(squashed(&code).contains("MsgVt::new_via_message::<"));
}

#[test]
fn a_rule_for_a_message_does_not_select_the_messages_it_holds() {
    // The global setting stays unrolled, so `Leaf` is not selected, and
    // `HasLeaf` reaches it through its `Message` impl.
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".t.HasLeaf".to_string(), CodecStrategy::Table)],
        ..Default::default()
    };
    let (code, warnings) = run(&config).unwrap();
    assert_eq!(tables(&code), ["HasLeaf"]);
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
    let code = squashed(&code);
    assert!(code.contains("MsgVt::new_via_message::<"), "{code}");
    assert!(!code.contains("(&__BUFFA_TABLE_Leaf)"), "{code}");

    // A rule for the child as well gives both a table, and the parent then
    // uses the child's.
    let config = CodeGenConfig {
        codec_strategy_in: vec![
            (".t.HasLeaf".to_string(), CodecStrategy::Table),
            (".t.Leaf".to_string(), CodecStrategy::Table),
        ],
        ..Default::default()
    };
    let (code, warnings) = run(&config).unwrap();
    assert_eq!(tables(&code), ["Leaf", "HasLeaf"]);
    assert!(table_warnings(&warnings).is_empty());
    let code = squashed(&code);
    assert!(code.contains("(&__BUFFA_TABLE_Leaf)"), "{code}");
    assert!(!code.contains("new_via_message"), "{code}");
}

#[test]
fn every_exact_path_rule_that_cannot_be_honoured_is_reported_at_once() {
    let mut file = schema();
    let mut second = file.message_type[3].clone();
    second.name = Some("Oneofy2".to_string());
    file.message_type.push(second);
    let config = CodeGenConfig {
        codec_strategy_in: vec![
            (".t.Oneofy".to_string(), CodecStrategy::Table),
            (".t.Oneofy2".to_string(), CodecStrategy::Table),
            (".t.HasOneofy".to_string(), CodecStrategy::Table),
        ],
        ..Default::default()
    };
    let mut config = with_custom_string(&config);
    config.string_fields.push((
        ".t.Oneofy2.a".to_string(),
        StringRepr::Custom("crate::Str".to_string()),
    ));
    let err = generate_with_diagnostics(&[file], &["t.proto".to_string()], &config)
        .unwrap_err()
        .to_string();
    // `HasOneofy` can use the table, so it is not among them.
    assert!(
        err.contains("rule '.t.Oneofy'")
            && err.contains("rule '.t.Oneofy2'")
            && !err.contains("rule '.t.HasOneofy'"),
        "{err}"
    );
}

#[test]
fn an_exact_path_rule_for_a_message_that_cannot_use_the_table_is_an_error() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".t.Oneofy".to_string(), CodecStrategy::Table)],
        ..Default::default()
    };
    let err = run(&config).unwrap_err().to_string();
    assert!(err.contains("cannot use it"), "{err}");
    assert!(
        err.contains("field `a` has a custom string, bytes or collection type"),
        "{err}"
    );
}

#[test]
fn a_broad_rule_that_covers_such_a_message_only_warns() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".t".to_string(), CodecStrategy::Table)],
        ..Default::default()
    };
    let (code, warnings) = run(&config).unwrap();
    assert_eq!(
        tables(&code),
        ["Plain", "Leaf", "HasLeaf", "HasOneofy", "Outer", "Inner"]
    );
    // Messages a rule selects are counted like the ones the global setting does.
    assert_eq!(summary(&warnings).0, (1, 7));
}

#[test]
fn setting_a_message_to_unrolled_does_not_affect_the_messages_that_hold_it() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".t.Leaf".to_string(), CodecStrategy::Unrolled)],
        ..table_config(CodecStrategy::Table)
    };
    let (code, warnings) = run(&config).unwrap();
    // `HasLeaf` holds the `Leaf` the user set to unrolled and has a table
    // anyway. The warning counts the messages selected, which leaves out `Leaf`.
    assert_eq!(
        tables(&code),
        ["Plain", "HasLeaf", "HasOneofy", "Outer", "Inner"]
    );
    assert_eq!(summary(&warnings).0, (1, 6));

    // Once the message that cannot use the table is set to unrolled too, no
    // fallback is left to report.
    let config = CodeGenConfig {
        codec_strategy_in: vec![
            (".t.Leaf".to_string(), CodecStrategy::Unrolled),
            (".t.Oneofy".to_string(), CodecStrategy::Unrolled),
        ],
        ..table_config(CodecStrategy::Table)
    };
    let (_, warnings) = run(&config).unwrap();
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
}

#[test]
fn a_rule_that_matches_no_message_warns() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![
            (".t.Plain".to_string(), CodecStrategy::Table),
            (".t.Nope".to_string(), CodecStrategy::Table),
            (".t.Plain.a".to_string(), CodecStrategy::Table),
            (".elsewhere".to_string(), CodecStrategy::Unrolled),
        ],
        ..Default::default()
    };
    let (_, warnings) = run(&config).unwrap();
    let mut inert: Vec<&str> = warnings
        .iter()
        .filter_map(|w| match w {
            CodeGenWarning::CodecStrategyRuleMatchedNothing { rule } => Some(rule.as_str()),
            _ => None,
        })
        .collect();
    inert.sort_unstable();
    assert_eq!(inert, [".elsewhere", ".t.Nope", ".t.Plain.a"]);
}

#[test]
fn a_message_type_from_another_crate_is_reached_through_its_message_impl() {
    // `HasLeaf.leaf` now names a type mapped to another crate.
    let mut file = schema();
    file.message_type[2].field[0].type_name = Some(".other.Foreign".to_string());
    let other = FileDescriptorProto {
        package: Some("other".to_string()),
        message_type: vec![message("Foreign", vec![])],
        ..proto3_file("other.proto")
    };
    let config = CodeGenConfig {
        extern_paths: vec![(".other".to_string(), "::other_crate".to_string())],
        ..table_config(CodecStrategy::Table)
    };
    let (files, warnings) = generate_with_diagnostics(
        &[file, other],
        &["t.proto".to_string()],
        &with_custom_string(&config),
    )
    .unwrap();
    let code = joined(&files);
    assert!(tables(&code).contains(&"HasLeaf".to_string()), "{code}");
    assert!(tables(&code).contains(&"Leaf".to_string()));
    assert_eq!(table_warnings(&warnings).len(), 1, "{warnings:?}");
    let code = squashed(&code);
    let holder = code.split("static__BUFFA_TABLE_HasLeaf").nth(1).unwrap();
    assert!(
        holder.contains("MsgVt::new_via_message::<::buffa::MessageField<::other_crate::Foreign"),
        "{holder}"
    );
}

#[test]
fn a_child_mapped_to_another_crate_is_not_a_table_though_this_run_generates_it_too() {
    let mut file = schema();
    file.message_type[2].field[0].type_name = Some(".other.Foreign".to_string());
    let other = FileDescriptorProto {
        package: Some("other".to_string()),
        message_type: vec![message("Foreign", vec![scalar("x", 1, Type::TYPE_INT32)])],
        ..proto3_file("other.proto")
    };
    let config = CodeGenConfig {
        extern_paths: vec![(".other".to_string(), "::other_crate".to_string())],
        ..table_config(CodecStrategy::Table)
    };
    // Both files are generated, but `HasLeaf` names `::other_crate::Foreign`,
    // which has no table here, so it is reached through its `Message` impl and
    // its table is not looked up, which would not build.
    let (files, _) = generate_with_diagnostics(
        &[file, other],
        &["t.proto".to_string(), "other.proto".to_string()],
        &config,
    )
    .unwrap();
    let code = joined(&files);
    assert!(tables(&code).contains(&"HasLeaf".to_string()), "{code}");
    assert!(!tables(&code).contains(&"Foreign".to_string()));
    assert!(!code.contains("__BUFFA_TABLE_Foreign"), "{code}");
}

#[test]
fn type_name_prefix_names_the_static_and_the_child_table() {
    let config = CodeGenConfig {
        type_name_prefix: "Rpc".to_string(),
        ..table_config(CodecStrategy::Table)
    };
    let (code, _) = run(&config).unwrap();
    assert!(tables(&code).contains(&"RpcHasLeaf".to_string()), "{code}");
    let has_leaf = squashed(&code);
    let has_leaf = has_leaf
        .split("static__BUFFA_TABLE_RpcHasLeaf")
        .nth(1)
        .unwrap();
    assert!(has_leaf.contains("(&__BUFFA_TABLE_RpcLeaf)"), "{has_leaf}");
}

#[test]
fn a_message_that_does_not_preserve_unknown_fields_has_no_unknown_slot() {
    let config = CodeGenConfig {
        preserve_unknown_fields: false,
        ..table_config(CodecStrategy::Table)
    };
    let (code, _) = run(&config).unwrap();
    let plain = squashed(&code);
    let plain = plain.split("static__BUFFA_TABLE_Plain").nth(1).unwrap();
    assert!(plain
        .split("impl::buffa::Message")
        .next()
        .unwrap()
        .contains("unknown=none"));
    assert!(!squashed(&code).contains("__buffa_unknown_fields"));
}

#[test]
fn field_numbers_index_the_dense_array_and_missing_ones_are_zero() {
    let mut file = schema();
    file.message_type[0].field = vec![
        scalar("c", 3, Type::TYPE_INT32),
        scalar("a", 1, Type::TYPE_INT32),
        scalar("far", 5000, Type::TYPE_INT32),
    ];
    let (files, _) = generate_with_diagnostics(
        &[file],
        &["t.proto".to_string()],
        &table_config(CodecStrategy::Table),
    )
    .unwrap();
    let code = joined(&files);
    let plain = code.split("static __BUFFA_TABLE_Plain").nth(1).unwrap();
    let plain = plain.split("impl ::buffa::Message").next().unwrap();
    // Entries are in field-number order, and only numbers below 64 are dense.
    let plain = squashed(plain);
    let (a, c, far) = (
        plain.find("(Plain,a,Int32Implicit,1u32)").expect("a"),
        plain.find("(Plain,c,Int32Implicit,3u32)").expect("c"),
        plain
            .find("(Plain,far,Int32Implicit,5000u32)")
            .expect("far"),
    );
    assert!(a < c && c < far, "{plain}");
    assert!(plain.contains("dense=&[0u8,1u8,0u8,2u8]"), "{plain}");
}

#[test]
fn the_warning_texts_say_what_to_do() {
    let rule = CodeGenWarning::CodecStrategyRuleMatchedNothing {
        rule: ".x".to_string(),
    };
    assert_eq!(
        rule.to_string(),
        "codec_strategy_in rule '.x' matched no generated message; those messages keep the \
         global codec_strategy setting — check the path against the fully-qualified proto \
         message names"
    );
    let reason = TableCodecFallbackReason {
        reason: "has a map field".to_string(),
        messages: [".t.A", ".t.B", ".t.C", ".t.D", ".t.E"]
            .map(String::from)
            .to_vec(),
    };
    let summary = CodeGenWarning::TableCodecFallbackSummary {
        fallbacks: 5,
        selected: 9,
        reasons: vec![reason],
    };
    let text = summary.to_string();
    // Three messages are named, and the rest are counted.
    assert!(
        text.contains("has a map field (5: .t.A, .t.B, .t.C, and 2 more)"),
        "{text}"
    );
}

#[test]
fn the_plan_judges_fields_under_the_same_features_as_the_generator() {
    // The file makes message fields DELIMITED (groups), and `Outer` sets
    // LENGTH_PREFIXED on itself. The generator gives a top-level message the
    // file's features and ignores its own message-level features, so `Outer.i`
    // is a group, and `Outer` cannot use the table. A plan that applied
    // `Outer`'s own features would select it, and the emitter would then fail
    // the build.
    use crate::generated::descriptor::{
        feature_set::MessageEncoding, Edition, FeatureSet, FileOptions,
    };
    let features = |encoding| {
        FeatureSet {
            message_encoding: Some(encoding),
            ..Default::default()
        }
        .into()
    };
    let inner = message("Inner", vec![scalar("x", 1, Type::TYPE_INT32)]);
    let mut outer = message("Outer", vec![message_field("i", 1, ".Inner")]);
    outer.options = MessageOptions {
        features: features(MessageEncoding::LENGTH_PREFIXED),
        ..Default::default()
    }
    .into();
    let file = FileDescriptorProto {
        name: Some("delim.proto".to_string()),
        edition: Some(Edition::EDITION_2023),
        options: FileOptions {
            features: features(MessageEncoding::DELIMITED),
            ..Default::default()
        }
        .into(),
        message_type: vec![inner, outer],
        ..Default::default()
    };
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["delim.proto".to_string()],
        &table_config(CodecStrategy::Table),
    )
    .expect("the plan and the emitter must agree");
    assert!(tables(&joined(&files)).is_empty());
    assert_eq!(summary(&warnings).0, (2, 2));
}

#[test]
fn the_generated_abi_is_a_literal_that_matches_the_runtime() {
    // The runtime refuses a table generated for another ABI, so the generator
    // must emit its own number: passing `buffa::table::ABI` would compare the
    // runtime's constant with itself.
    assert_eq!(crate::table_codec::TABLE_ABI, buffa::table::ABI);
    let (code, _) = run(&table_config(CodecStrategy::Table)).unwrap();
    let code = squashed(&code);
    assert!(
        code.contains(&format!("abi={},", crate::table_codec::TABLE_ABI)),
        "the table must carry the literal ABI: {code}"
    );
    assert!(!code.contains("abi=::buffa::table::ABI"), "{code}");
}

// ---------------------------------------------------------------------------
// Messages that hold a message stored with a non-default bytes type
// ---------------------------------------------------------------------------

const HOLDS_BYTES: &str = "holds a message with bytes fields of a non-default type";
const CUSTOM_FIELD: &str = "has a field with a custom string, bytes or collection type";

fn bytes_field(name: &str, number: i32) -> FieldDescriptorProto {
    scalar(name, number, Type::TYPE_BYTES)
}

fn repeated_message_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        label: Some(Label::LABEL_REPEATED),
        ..message_field(name, number, type_name)
    }
}

/// Package `b` with:
///
/// - `Blob` (a bytes field, which the rules below give the type `Bytes`) and
///   `PlainBytes` (a bytes field that keeps `Vec<u8>`);
/// - `HasBlob`, `HasBlobs` (repeated) and `HoldsHasBlob`, which hold a `Blob`
///   directly, in a list and through `HasBlob`;
/// - `OneofBlob`, which has a oneof member of type `Blob`, and `HoldsOneofBlob`
///   (holds `OneofBlob`);
/// - `MapBlob`, which has a map with `Blob` values, and `HoldsMapBlob`;
/// - `HoldsPlain` (holds `PlainBytes`), `Leaf` and `HoldsLeaf`, which are
///   unaffected.
fn bytes_schema() -> FileDescriptorProto {
    let mut oneof_blob = message(
        "OneofBlob",
        vec![FieldDescriptorProto {
            oneof_index: Some(0),
            ..message_field("blob", 1, ".b.Blob")
        }],
    );
    oneof_blob.oneof_decl = vec![OneofDescriptorProto {
        name: Some("choice".to_string()),
        ..Default::default()
    }];
    let mut map_blob = message(
        "MapBlob",
        vec![repeated_message_field("blobs", 1, ".b.MapBlob.BlobsEntry")],
    );
    map_blob.nested_type = vec![DescriptorProto {
        name: Some("BlobsEntry".to_string()),
        field: vec![
            scalar("key", 1, Type::TYPE_STRING),
            message_field("value", 2, ".b.Blob"),
        ],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    }];
    FileDescriptorProto {
        package: Some("b".to_string()),
        message_type: vec![
            message("Blob", vec![bytes_field("data", 1)]),
            message("PlainBytes", vec![bytes_field("data", 1)]),
            message("HasBlob", vec![message_field("blob", 1, ".b.Blob")]),
            message(
                "HasBlobs",
                vec![repeated_message_field("blobs", 1, ".b.Blob")],
            ),
            message("HoldsHasBlob", vec![message_field("has", 1, ".b.HasBlob")]),
            oneof_blob,
            message(
                "HoldsOneofBlob",
                vec![message_field("o", 1, ".b.OneofBlob")],
            ),
            map_blob,
            message("HoldsMapBlob", vec![message_field("m", 1, ".b.MapBlob")]),
            message("HoldsPlain", vec![message_field("p", 1, ".b.PlainBytes")]),
            message("Leaf", vec![scalar("x", 1, Type::TYPE_INT32)]),
            message("HoldsLeaf", vec![message_field("leaf", 1, ".b.Leaf")]),
        ],
        ..proto3_file("b.proto")
    }
}

fn run_bytes(config: &CodeGenConfig) -> Result<(String, Vec<CodeGenWarning>), CodeGenError> {
    let (files, warnings) =
        generate_with_diagnostics(&[bytes_schema()], &["b.proto".to_string()], config)?;
    Ok((joined(&files), warnings))
}

/// The table strategy, with `Blob.data` stored as `Bytes`.
fn blob_config() -> CodeGenConfig {
    CodeGenConfig {
        bytes_fields: vec![(".b.Blob.data".to_string(), BytesRepr::Bytes)],
        ..table_config(CodecStrategy::Table)
    }
}

#[test]
fn a_message_that_holds_a_message_with_a_bytes_type_stays_unrolled() {
    let (code, warnings) = run_bytes(&blob_config()).unwrap();
    // The holders of `Blob` fall back, directly, in a list, transitively, and
    // through a oneof member and a map value.
    assert_eq!(
        tables(&code),
        ["PlainBytes", "HoldsPlain", "Leaf", "HoldsLeaf"]
    );
    let (counts, reasons) = summary(&warnings);
    assert_eq!(counts, (8, 12));
    // `MapBlob` falls back for its map, so it is not counted as a holder.
    assert_eq!(
        reasons,
        [(HOLDS_BYTES, 6), (CUSTOM_FIELD, 1), ("has a map field", 1)]
    );
    let text = table_warnings(&warnings)[0].to_string();
    assert!(text.contains(HOLDS_BYTES), "{text}");
    assert!(text.contains(".b.HasBlob"), "{text}");
}

#[test]
fn a_message_with_a_plain_bytes_field_is_unaffected() {
    let (code, _) = run_bytes(&blob_config()).unwrap();
    let plain = squashed(&code);
    assert!(tables(&code).contains(&"PlainBytes".to_string()));
    // `HoldsPlain` reaches `PlainBytes` through its table.
    let holder = plain
        .split("static__BUFFA_TABLE_HoldsPlain")
        .nth(1)
        .unwrap();
    assert!(holder.contains("(&__BUFFA_TABLE_PlainBytes)"), "{holder}");
}

#[test]
fn a_child_set_to_unrolled_without_bytes_still_lets_its_holder_use_the_table() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".b.Leaf".to_string(), CodecStrategy::Unrolled)],
        ..blob_config()
    };
    let (code, _) = run_bytes(&config).unwrap();
    assert!(tables(&code).contains(&"HoldsLeaf".to_string()), "{code}");
    assert!(!tables(&code).contains(&"Leaf".to_string()));
    assert!(squashed(&code).contains("MsgVt::new_via_message::<"));
}

#[test]
fn a_child_set_to_unrolled_that_has_a_bytes_type_keeps_its_holder_unrolled() {
    // The rule does not decide it: `Blob` is stored as `Bytes` either way.
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".b.Blob".to_string(), CodecStrategy::Unrolled)],
        ..blob_config()
    };
    let (code, warnings) = run_bytes(&config).unwrap();
    assert!(!tables(&code).contains(&"HasBlob".to_string()), "{code}");
    assert!(summary(&warnings).1.contains(&(HOLDS_BYTES, 6)));
}

#[test]
fn a_bytes_type_for_every_field_keeps_the_holders_of_every_bytes_message_unrolled() {
    let config = CodeGenConfig {
        bytes_fields: vec![(".".to_string(), BytesRepr::Bytes)],
        ..table_config(CodecStrategy::Table)
    };
    let (code, _) = run_bytes(&config).unwrap();
    // With this rule `PlainBytes` has a `Bytes` field, so it stays unrolled and
    // so does every message that holds it.
    assert_eq!(tables(&code), ["Leaf", "HoldsLeaf"]);
}

#[test]
fn a_bytes_type_on_another_message_does_not_affect_a_holder() {
    let config = CodeGenConfig {
        bytes_fields: vec![(".b.PlainBytes.data".to_string(), BytesRepr::Bytes)],
        ..table_config(CodecStrategy::Table)
    };
    let (code, _) = run_bytes(&config).unwrap();
    assert!(tables(&code).contains(&"HasBlob".to_string()), "{code}");
    assert!(!tables(&code).contains(&"HoldsPlain".to_string()));
}

#[test]
fn an_exact_path_rule_for_a_holder_of_a_bytes_typed_message_is_an_error() {
    let config = CodeGenConfig {
        codec_strategy_in: vec![(".b.HasBlob".to_string(), CodecStrategy::Table)],
        ..blob_config()
    };
    let err = run_bytes(&config).unwrap_err().to_string();
    assert!(err.contains("cannot use it"), "{err}");
    assert!(err.contains("it holds `.b.Blob`"), "{err}");
}

// ---------------------------------------------------------------------------
// How far the bytes rule reaches
// ---------------------------------------------------------------------------

fn repeated_bytes_field(name: &str, number: i32) -> FieldDescriptorProto {
    FieldDescriptorProto {
        label: Some(Label::LABEL_REPEATED),
        ..bytes_field(name, number)
    }
}

/// A message with the one map field `m` whose entries have a key and a value
/// of the given types.
fn message_with_map(name: &str, key: Type, value: Type) -> DescriptorProto {
    let mut msg = message(
        name,
        vec![repeated_message_field("m", 1, &format!(".c.{name}.MEntry"))],
    );
    msg.nested_type = vec![DescriptorProto {
        name: Some("MEntry".to_string()),
        field: vec![scalar("key", 1, key), scalar("value", 2, value)],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    }];
    msg
}

/// Package `c` with:
///
/// - `ChainA` holds `ChainB` holds `ChainC` holds `Blob`, declared holder
///   first, so a holder is judged before the message it holds;
/// - `CycleP` and `CycleQ` hold each other, and `CycleQ` has a bytes field, and
///   `CycleX` and `CycleY` hold each other and have none;
/// - `RepBytes` (a repeated bytes field), `MapBytes` (a `map<string, bytes>`)
///   and `Outer.Inner` (a nested message), each with a holder;
/// - `BytesKeyMap`, a `map<bytes, bytes>`, whose values keep `Vec<u8>` whatever
///   the rule says, and its holder. Protoc rejects a bytes key, so only a
///   hand-built descriptor can have one.
fn taint_schema() -> FileDescriptorProto {
    let mut outer = message("Outer", vec![]);
    outer.nested_type = vec![message("Inner", vec![bytes_field("data", 1)])];
    FileDescriptorProto {
        package: Some("c".to_string()),
        message_type: vec![
            message("ChainA", vec![message_field("b", 1, ".c.ChainB")]),
            message("ChainB", vec![message_field("c", 1, ".c.ChainC")]),
            message("ChainC", vec![message_field("blob", 1, ".c.Blob")]),
            message("Blob", vec![bytes_field("data", 1)]),
            message("CycleP", vec![message_field("q", 1, ".c.CycleQ")]),
            message(
                "CycleQ",
                vec![message_field("p", 1, ".c.CycleP"), bytes_field("data", 2)],
            ),
            message("CycleX", vec![message_field("y", 1, ".c.CycleY")]),
            message("CycleY", vec![message_field("x", 1, ".c.CycleX")]),
            message("RepBytes", vec![repeated_bytes_field("chunks", 1)]),
            message("HoldsRepBytes", vec![message_field("r", 1, ".c.RepBytes")]),
            message_with_map("MapBytes", Type::TYPE_STRING, Type::TYPE_BYTES),
            message("HoldsMapBytes", vec![message_field("m", 1, ".c.MapBytes")]),
            message_with_map("BytesKeyMap", Type::TYPE_BYTES, Type::TYPE_BYTES),
            message(
                "HoldsBytesKeyMap",
                vec![message_field("m", 1, ".c.BytesKeyMap")],
            ),
            outer,
            message("HoldsInner", vec![message_field("i", 1, ".c.Outer.Inner")]),
        ],
        ..proto3_file("c.proto")
    }
}

/// What the plan made of `taint_schema`.
struct Taint {
    tables: Vec<String>,
    /// The messages that fell back and the messages selected.
    counts: (usize, usize),
    /// The messages that fell back for holding a message with a bytes type.
    holders: usize,
}

/// The table strategy with every bytes field of `taint_schema` stored as
/// `Bytes`.
fn run_taint() -> Taint {
    let config = CodeGenConfig {
        bytes_fields: [
            ".c.Blob.data",
            ".c.CycleQ.data",
            ".c.RepBytes.chunks",
            ".c.MapBytes.m",
            ".c.BytesKeyMap.m",
            ".c.Outer.Inner.data",
        ]
        .map(|path| (path.to_string(), BytesRepr::Bytes))
        .to_vec(),
        ..table_config(CodecStrategy::Table)
    };
    let (files, warnings) =
        generate_with_diagnostics(&[taint_schema()], &["c.proto".to_string()], &config).unwrap();
    let (counts, reasons) = summary(&warnings);
    let holders = reasons
        .iter()
        .find_map(|&(reason, n)| (reason == HOLDS_BYTES).then_some(n))
        .unwrap_or(0);
    Taint {
        tables: tables(&joined(&files)),
        counts,
        holders,
    }
}

#[test]
fn a_chain_of_holders_declared_holder_first_is_unrolled_all_the_way() {
    let tables = run_taint().tables;
    for name in ["ChainA", "ChainB", "ChainC", "Blob"] {
        assert!(!tables.contains(&name.to_string()), "{name}: {tables:?}");
    }
}

#[test]
fn a_cycle_that_reaches_a_bytes_message_is_unrolled_and_one_that_does_not_is_not() {
    let tables = run_taint().tables;
    for name in ["CycleP", "CycleQ"] {
        assert!(!tables.contains(&name.to_string()), "{name}: {tables:?}");
    }
    for name in ["CycleX", "CycleY"] {
        assert!(tables.contains(&name.to_string()), "{name}: {tables:?}");
    }
}

#[test]
fn a_message_holding_repeated_map_or_nested_bytes_is_unrolled() {
    let plan = run_taint();
    for name in ["HoldsRepBytes", "HoldsMapBytes", "HoldsInner"] {
        assert!(
            !plan.tables.contains(&name.to_string()),
            "{name}: {:?}",
            plan.tables
        );
    }
    // A message with a plain nested declaration is unaffected.
    assert!(
        plan.tables.contains(&"Outer".to_string()),
        "{:?}",
        plan.tables
    );
    // The holders counted under the one reason are the three above, the chain
    // and the cycle.
    assert_eq!(plan.holders, 7);
    assert_eq!(plan.counts, (13, 17));
}

#[test]
fn a_map_with_a_bytes_key_keeps_vec_values_so_its_holder_uses_the_table() {
    let tables = run_taint().tables;
    assert!(
        tables.contains(&"HoldsBytesKeyMap".to_string()),
        "{tables:?}"
    );
}

/// Package `o` with `WithOneof { oneof choice { int32 a = 1; string b = 2; Leaf leaf = 5; }; int32 c = 3; }`.
fn oneof_schema() -> FileDescriptorProto {
    let member = |name, number, ty| FieldDescriptorProto {
        oneof_index: Some(0),
        ..scalar(name, number, ty)
    };
    let mut with_oneof = message(
        "WithOneof",
        vec![
            member("a", 1, Type::TYPE_INT32),
            member("b", 2, Type::TYPE_STRING),
            FieldDescriptorProto {
                oneof_index: Some(0),
                ..message_field("leaf", 5, ".o.Leaf")
            },
            scalar("c", 3, Type::TYPE_INT32),
        ],
    );
    with_oneof.oneof_decl = vec![OneofDescriptorProto {
        name: Some("choice".to_string()),
        ..Default::default()
    }];
    FileDescriptorProto {
        package: Some("o".to_string()),
        message_type: vec![
            message("Leaf", vec![scalar("x", 1, Type::TYPE_INT32)]),
            with_oneof,
        ],
        ..proto3_file("o.proto")
    }
}

fn run_oneof(config: &CodeGenConfig) -> (String, Vec<CodeGenWarning>) {
    let (files, warnings) =
        generate_with_diagnostics(&[oneof_schema()], &["o.proto".to_string()], config).unwrap();
    (joined(&files), warnings)
}

#[test]
fn a_message_with_a_oneof_gets_a_table_with_one_entry_per_member() {
    let (code, warnings) = run_oneof(&table_config(CodecStrategy::Table));
    assert_eq!(tables(&code), ["Leaf", "WithOneof"]);
    assert!(table_warnings(&warnings).is_empty(), "{warnings:?}");
    let code = squashed(&code);
    let table = code.split("static__BUFFA_TABLE_WithOneof").nth(1).unwrap();
    let table = table.split("impl::buffa::Message").next().unwrap();
    // The members carry their payload kinds, in field-number order with `c`
    // (3) between them, and all name the field that holds the oneof.
    for entry in [
        "(WithOneof,choice,oneof(Int32Required),1u32,",
        "(WithOneof,choice,oneof(StrRequired),2u32,",
        "(WithOneof,c,Int32Implicit,3u32)",
        "(WithOneof,choice,oneof(MsgSingular),5u32,",
    ] {
        assert!(table.contains(entry), "{entry} in {table}");
    }
    // One descriptor for the oneof, at the lowest member number, and only the
    // first member leads it.
    assert_eq!(table.matches("Aux::Group(").count(), 1, "{table}");
    assert!(table.contains("OneofVt::new::<"), "{table}");
    assert!(
        table.contains("offset_of!(WithOneof,choice),1u32"),
        "{table}"
    );
    assert_eq!(table.matches("Member::new(0u16,").count(), 3, "{table}");
    assert_eq!(table.matches(",true,)").count(), 1, "{table}");
    assert_eq!(table.matches(",false,)").count(), 2, "{table}");
    // The message member's child is reached through its own table.
    assert!(
        table.contains("MsgVt::direct(&__BUFFA_TABLE_Leaf)"),
        "{table}"
    );
}

#[test]
fn the_oneof_enum_implements_the_accessors_the_table_reads_it_through() {
    let (code, _) = run_oneof(&table_config(CodecStrategy::Table));
    let code = squashed(&code);
    let imp = code
        .split("impl::buffa::table::OneofEnumfor__buffa::oneof::with_oneof::Choice{")
        .nth(1)
        .and_then(|rest| rest.split("impl::buffa::ExtensionSet").next())
        .unwrap();
    assert!(imp.contains("Self::A(_)=>1u32"), "{imp}");
    assert!(imp.contains("Self::B(_)=>2u32"), "{imp}");
    assert!(imp.contains("Self::Leaf(_)=>5u32"), "{imp}");
    // A boxed message is reached through its box (by deref coercion), and a
    // new one starts empty.
    assert!(imp.contains("Self::Leaf(v)=>{letvalue:&Leaf=v;"), "{imp}");
    assert!(
        imp.contains(
            "5u32=>{::core::option::Option::Some(Self::Leaf(::buffa::alloc::boxed::Box::default()"
        ),
        "{imp}"
    );
    assert!(imp.contains("_=>::core::option::Option::None"), "{imp}");
    // None of it is unsafe: the generated crate may forbid unsafe code.
    assert!(!imp.contains("unsafe"), "{imp}");
}

#[test]
fn an_unrolled_message_with_a_oneof_is_unchanged() {
    let (code, warnings) = run_oneof(&CodeGenConfig::default());
    assert!(tables(&code).is_empty());
    assert!(!code.contains("OneofEnum"));
    assert!(table_warnings(&warnings).is_empty());
}

#[test]
fn a_oneof_member_with_a_custom_type_keeps_the_message_unrolled() {
    let config = CodeGenConfig {
        string_fields: vec![(
            ".o.WithOneof.b".to_string(),
            StringRepr::Custom("crate::Str".to_string()),
        )],
        ..table_config(CodecStrategy::Table)
    };
    let (code, warnings) = run_oneof(&config);
    assert_eq!(tables(&code), ["Leaf"]);
    assert!(
        table_warnings(&warnings)[0]
            .to_string()
            .contains(".o.WithOneof"),
        "{warnings:?}"
    );
}

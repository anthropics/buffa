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
/// - `Oneofy` (has a oneof), which cannot, and `HasOneofy` (holds an `Oneofy`),
///   which can, because a table message may hold a message that has no table;
/// - `Outer` with a nested `Inner`, both plain.
fn schema() -> FileDescriptorProto {
    let mut oneofy = message(
        "Oneofy",
        vec![FieldDescriptorProto {
            oneof_index: Some(0),
            ..scalar("a", 1, Type::TYPE_INT32)
        }],
    );
    oneofy.oneof_decl = vec![OneofDescriptorProto {
        name: Some("choice".to_string()),
        ..Default::default()
    }];
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

fn run(config: &CodeGenConfig) -> Result<(String, Vec<CodeGenWarning>), CodeGenError> {
    let (files, warnings) =
        generate_with_diagnostics(&[schema()], &["t.proto".to_string()], config)?;
    Ok((joined(&files), warnings))
}

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
    // The one with a oneof falls back, and one warning covers the run.
    let (counts, reasons) = summary(&warnings);
    assert_eq!(counts, (1, 7));
    assert_eq!(reasons, [("has a oneof", 1)]);
    let text = table_warnings(&warnings)[0].to_string();
    assert!(text.starts_with("1 of 7 messages selected for the table codec"));
    assert!(text.contains("has a oneof (1: .t.Oneofy)"), "{text}");
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
    let holder = holder.split("impl::buffa::MessageforHasOneofy").next().unwrap();
    assert!(
        holder.contains("Aux::Msg(&::buffa::table::MsgVt::new_dyn::<::buffa::MessageField<Oneofy,::buffa::Inline<Oneofy>>>())"),
        "{holder}"
    );
    assert!(!holder.contains("__BUFFA_TABLE_Oneofy"), "{holder}");
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
    assert!(squashed(&code).contains("MsgVt::new_dyn::<"));
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
    assert!(code.contains("MsgVt::new_dyn::<"), "{code}");
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
    assert!(!code.contains("new_dyn"), "{code}");
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
    assert!(err.contains("field `a` is in a oneof"), "{err}");
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

    // With the one that cannot set to unrolled as well, nothing is left to say.
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
    let (files, warnings) =
        generate_with_diagnostics(&[file, other], &["t.proto".to_string()], &config).unwrap();
    let code = joined(&files);
    assert!(tables(&code).contains(&"HasLeaf".to_string()), "{code}");
    assert!(tables(&code).contains(&"Leaf".to_string()));
    assert!(table_warnings(&warnings).len() == 1, "{warnings:?}");
    let code = squashed(&code);
    let holder = code.split("static__BUFFA_TABLE_HasLeaf").nth(1).unwrap();
    assert!(
        holder.contains("MsgVt::new_dyn::<::buffa::MessageField<::other_crate::Foreign"),
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
        reason: "has a oneof".to_string(),
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
        text.contains("has a oneof (5: .t.A, .t.B, .t.C, and 2 more)"),
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

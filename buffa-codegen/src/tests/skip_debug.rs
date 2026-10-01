//! `skip_debug`: omit the generated `Debug` for selected messages, with their
//! oneof enums, and for enums named exactly.

use super::*;
use crate::generated::descriptor::FieldOptions;

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A message with one string field and a two-variant oneof.
fn message_with_oneof(name: &str, oneof: &str) -> DescriptorProto {
    let mut label = make_field("label", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    label.oneof_index = Some(0);
    let mut code = make_field("code", 3, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
    code.oneof_index = Some(0);
    DescriptorProto {
        name: Some(name.to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            label,
            code,
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some(oneof.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn two_value_enum(name: &str, prefix: &str) -> EnumDescriptorProto {
    EnumDescriptorProto {
        name: Some(name.to_string()),
        value: vec![
            enum_value(&format!("{prefix}_UNSPECIFIED"), 0),
            enum_value(&format!("{prefix}_ONE"), 1),
        ],
        ..Default::default()
    }
}

/// Package `skip.test`: `Outer` (oneof `choice`, nested message `Nested`,
/// nested enum `Scope`), `Other` (oneof `pick`) and the top-level enum
/// `Level`.
fn skip_debug_file() -> FileDescriptorProto {
    let mut outer = message_with_oneof("Outer", "choice");
    outer.nested_type.push(DescriptorProto {
        name: Some("Nested".to_string()),
        field: vec![make_field(
            "value",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    outer.enum_type.push(two_value_enum("Scope", "SCOPE"));

    let mut file = proto3_file("skip_debug.proto");
    file.package = Some("skip.test".to_string());
    file.message_type = vec![outer, message_with_oneof("Other", "pick")];
    file.enum_type.push(two_value_enum("Level", "LEVEL"));
    file
}

fn config(rules: &[&str]) -> CodeGenConfig {
    CodeGenConfig {
        skip_debug: rules.iter().map(|r| (*r).to_string()).collect(),
        ..Default::default()
    }
}

fn generate_squashed(config: &CodeGenConfig) -> String {
    let files = generate(
        &[skip_debug_file()],
        &["skip_debug.proto".to_string()],
        config,
    )
    .expect("should generate");
    squash(&joined(&files))
}

fn message_debug(name: &str) -> String {
    format!("impl::core::fmt::Debugfor{name}{{")
}

fn oneof_derive(name: &str, debug: bool) -> String {
    let debug = if debug { ",Debug" } else { "" };
    format!("#[derive(Clone,PartialEq{debug})]pubenum{name}{{")
}

fn enum_derive(name: &str, debug: bool) -> String {
    let debug = if debug { ",Debug" } else { "" };
    format!("#[derive(Clone,Copy,PartialEq,Eq,Hash{debug})]#[repr(i32)]pubenum{name}{{")
}

#[test]
fn debug_is_generated_by_default() {
    let content = generate_squashed(&CodeGenConfig::default());
    for name in ["Outer", "Nested", "Other"] {
        assert!(content.contains(&message_debug(name)), "{name}: {content}");
    }
    for name in ["Choice", "Pick"] {
        assert!(
            content.contains(&oneof_derive(name, true)),
            "{name}: {content}"
        );
    }
    for name in ["Scope", "Level"] {
        assert!(
            content.contains(&enum_derive(name, true)),
            "{name}: {content}"
        );
    }
}

#[test]
fn a_message_rule_covers_its_oneofs_and_nested_messages() {
    let content = generate_squashed(&config(&[".skip.test.Outer"]));

    for name in ["Outer", "Nested"] {
        assert!(!content.contains(&message_debug(name)), "{name}: {content}");
    }
    assert!(
        content.contains(&oneof_derive("Choice", false)),
        "{content}"
    );

    // The nested enum is not named by the rule, and types outside the rule
    // keep their generated `Debug`.
    assert!(content.contains(&enum_derive("Scope", true)), "{content}");
    assert!(content.contains(&message_debug("Other")), "{content}");
    assert!(content.contains(&oneof_derive("Pick", true)), "{content}");
    assert!(content.contains(&enum_derive("Level", true)), "{content}");

    assert!(
        content.contains("#[derive(Clone,Debug,Default)]pubstructOuterView"),
        "a view keeps its Debug: {content}"
    );
}

#[test]
fn an_enum_loses_debug_only_when_a_rule_is_its_name() {
    let content = generate_squashed(&config(&[".skip.test.Level", ".skip.test.Outer.Scope"]));

    assert!(content.contains(&enum_derive("Level", false)), "{content}");
    assert!(content.contains(&enum_derive("Scope", false)), "{content}");
    for name in ["Outer", "Nested", "Other"] {
        assert!(content.contains(&message_debug(name)), "{name}: {content}");
    }
}

#[test]
fn the_root_rule_covers_every_message_and_no_enum() {
    let content = generate_squashed(&config(&["."]));

    for name in ["Outer", "Nested", "Other"] {
        assert!(!content.contains(&message_debug(name)), "{name}: {content}");
    }
    for name in ["Choice", "Pick"] {
        assert!(
            content.contains(&oneof_derive(name, false)),
            "{name}: {content}"
        );
    }
    for name in ["Scope", "Level"] {
        assert!(
            content.contains(&enum_derive(name, true)),
            "{name}: {content}"
        );
    }
}

#[test]
fn a_matched_oneof_with_a_redacted_variant_gets_no_debug_impl() {
    let mut file = skip_debug_file();
    file.message_type[0].field[1].options = FieldOptions {
        debug_redact: Some(true),
        ..Default::default()
    }
    .into();
    let generate_with = |rules: &[&str]| {
        let files = generate(
            &[file.clone()],
            &["skip_debug.proto".to_string()],
            &config(rules),
        )
        .expect("should generate");
        squash(&joined(&files))
    };

    // Unmatched, the redacted variant swaps the derive for a manual impl.
    // The view oneof's impl is for `Choice<'a>` under `impl<'a>`, because
    // `label` borrows, so it matches neither assertion.
    assert!(generate_with(&[]).contains(&message_debug("Choice")));

    let content = generate_with(&[".skip.test.Outer"]);
    assert!(!content.contains(&message_debug("Choice")), "{content}");
    assert!(
        content.contains(&oneof_derive("Choice", false)),
        "{content}"
    );
}

#[test]
fn a_rule_that_matches_no_type_warns() {
    let live = [
        ".",
        ".skip.test",
        ".skip.test.Outer",
        ".skip.test.Outer.Nested",
        ".skip.test.Outer.Scope",
        ".skip.test.Level",
    ];
    // A typo, a oneof path, a field path, an enum-name prefix, a foreign
    // package, and a message in a descriptor that is not being generated.
    // Sorted, to compare against the sorted warnings.
    let inert = [
        ".dep.Dep",
        ".other",
        ".skip.test.Lev",
        ".skip.test.Nope",
        ".skip.test.Outer.choice",
        ".skip.test.Outer.name",
    ];
    let rules: Vec<&str> = live.iter().chain(&inert).copied().collect();

    let mut dep = proto3_file("dep.proto");
    dep.package = Some("dep".to_string());
    dep.message_type.push(DescriptorProto {
        name: Some("Dep".to_string()),
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[skip_debug_file(), dep],
        &["skip_debug.proto".to_string()],
        &config(&rules),
    )
    .expect("should generate");
    let mut warned: Vec<&str> = warnings
        .iter()
        .filter_map(|w| match w {
            CodeGenWarning::SkipDebugRuleMatchedNothing { rule } => Some(rule.as_str()),
            _ => None,
        })
        .collect();
    warned.sort_unstable();
    assert_eq!(warned, inert, "all warnings: {warnings:?}");
}

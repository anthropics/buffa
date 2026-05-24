//! Idiomatic `UpperCamelCase` enum alias generation
//! ([`CodeGenConfig::idiomatic_enum_aliases`]).
//!
//! The proto `SHOUTY_SNAKE_CASE` names stay the definitive variants; these
//! tests cover the additive alias `const`s and the all-or-nothing suppression
//! rule for the collision classes (lossy fold, mixed-convention shadow,
//! strip-empty, strip-digit, keyword, redundant).

use super::*;

fn idiomatic_config() -> CodeGenConfig {
    CodeGenConfig {
        idiomatic_enum_aliases: true,
        ..Default::default()
    }
}

fn enum_file(file: &str, name: &str, values: Vec<EnumValueDescriptorProto>) -> FileDescriptorProto {
    let mut f = proto3_file(file);
    f.enum_type.push(EnumDescriptorProto {
        name: Some(name.to_string()),
        value: values,
        ..Default::default()
    });
    f
}

#[test]
fn aliases_on_by_default() {
    let file = enum_file(
        "s.proto",
        "RuleLevel",
        vec![
            enum_value("RULE_LEVEL_UNKNOWN", 0),
            enum_value("RULE_LEVEL_HIGH", 1),
        ],
    );
    let files = generate(&[file], &["s.proto".to_string()], &CodeGenConfig::default()).unwrap();
    let c = joined(&files);
    // SHOUTY variant remains, and the idiomatic alias is emitted by default.
    assert!(c.contains("RULE_LEVEL_HIGH = 1"), "{c}");
    assert!(
        c.contains("const High: Self = Self::RULE_LEVEL_HIGH"),
        "{c}"
    );
}

#[test]
fn aliases_can_be_disabled() {
    let file = enum_file(
        "s.proto",
        "RuleLevel",
        vec![
            enum_value("RULE_LEVEL_UNKNOWN", 0),
            enum_value("RULE_LEVEL_HIGH", 1),
        ],
    );
    let config = CodeGenConfig {
        idiomatic_enum_aliases: false,
        ..Default::default()
    };
    let files = generate(&[file], &["s.proto".to_string()], &config).unwrap();
    let c = joined(&files);
    assert!(c.contains("RULE_LEVEL_HIGH = 1"), "{c}");
    assert!(
        !c.contains("const High"),
        "no idiomatic alias when disabled: {c}"
    );
}

#[test]
fn aliases_strip_prefix_and_camel_case() {
    let file = enum_file(
        "s.proto",
        "RuleLevel",
        vec![
            enum_value("RULE_LEVEL_UNKNOWN", 0),
            enum_value("RULE_LEVEL_HIGH", 1),
            enum_value("RULE_LEVEL_CRITICAL", 2),
        ],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    // The SHOUTY_SNAKE_CASE names remain the definitive variants.
    assert!(c.contains("RULE_LEVEL_HIGH = 1"), "{c}");
    // Idiomatic aliases are emitted as consts pointing at those variants.
    assert!(
        c.contains("const Unknown: Self = Self::RULE_LEVEL_UNKNOWN"),
        "{c}"
    );
    assert!(
        c.contains("const High: Self = Self::RULE_LEVEL_HIGH"),
        "{c}"
    );
    assert!(
        c.contains("const Critical: Self = Self::RULE_LEVEL_CRITICAL"),
        "{c}"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn aliases_without_matching_prefix_use_full_name() {
    let file = enum_file(
        "c.proto",
        "Color",
        vec![enum_value("RED", 0), enum_value("DARK_GREEN", 1)],
    );
    let files = generate(&[file], &["c.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    // Prefix `COLOR_` matches nothing, so full names are CamelCased.
    assert!(c.contains("const Red: Self = Self::RED"), "{c}");
    assert!(
        c.contains("const DarkGreen: Self = Self::DARK_GREEN"),
        "{c}"
    );
}

#[test]
fn lossy_collision_suppresses_entire_enum() {
    let file = enum_file(
        "s.proto",
        "Weird",
        vec![
            enum_value("FOO_BAR", 0),
            enum_value("FOO__BAR", 1), // collapses to the same CamelCase as FOO_BAR
            enum_value("BAZ", 2),
        ],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    // All-or-nothing: even the clean BAZ value gets no alias.
    assert!(
        !c.contains("const Baz"),
        "clean value must be suppressed too: {c}"
    );
    assert!(!c.contains("const FooBar"), "{c}");
    // The enum carries a doc note explaining the suppression.
    assert!(
        c.contains("Idiomatic CamelCase aliases are not generated for this enum"),
        "{c}"
    );
    // A single build warning names the exact clash.
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let w = warnings[0].to_string();
    assert!(
        w.contains("Weird")
            && w.contains("FOO_BAR")
            && w.contains("FOO__BAR")
            && w.contains("FooBar"),
        "{w}"
    );
    // The structured form exposes the conflict for programmatic handling.
    match &warnings[0] {
        CodeGenWarning::IdiomaticAliasesSuppressed {
            enum_name,
            conflicts,
            invalid,
            ..
        } => {
            assert_eq!(enum_name, "Weird");
            assert!(invalid.is_empty(), "{invalid:?}");
            assert!(
                conflicts.iter().any(|c| c.camel_target == "FooBar"
                    && c.proto_values.iter().any(|n| n == "FOO_BAR")
                    && c.proto_values.iter().any(|n| n == "FOO__BAR")),
                "{conflicts:?}"
            );
        }
    }
}

#[test]
fn mixed_convention_silent_shadow_is_detected() {
    // `FOO_BAR`'s CamelCase form (`FooBar`) collides with the literal `FooBar`
    // variant — a clash the Rust compiler would accept silently.
    let file = enum_file(
        "s.proto",
        "Mix",
        vec![enum_value("FOO_BAR", 0), enum_value("FooBar", 1)],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    assert!(
        c.contains("Use the `SHOUTY_SNAKE_CASE` variants directly"),
        "{c}"
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let w = warnings[0].to_string();
    assert!(w.contains("FOO_BAR") && w.contains("FooBar"), "{w}");
}

#[test]
fn strip_leading_digit_falls_back_to_unstripped() {
    let file = enum_file(
        "v.proto",
        "Version",
        vec![
            enum_value("VERSION_UNKNOWN", 0),
            enum_value("VERSION_2", 1), // strips to "2", an invalid identifier
            enum_value("VERSION_3", 2),
        ],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["v.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    // The whole enum keeps full (unstripped) names so every alias stays valid.
    assert!(
        c.contains("const VersionUnknown: Self = Self::VERSION_UNKNOWN"),
        "{c}"
    );
    assert!(c.contains("const Version2: Self = Self::VERSION_2"), "{c}");
    assert!(c.contains("const Version3: Self = Self::VERSION_3"), "{c}");
    assert!(
        warnings.is_empty(),
        "fallback should not warn: {warnings:?}"
    );
}

#[test]
fn strip_empty_remainder_falls_back_to_unstripped() {
    // `FOO_` equals the prefix, stripping to "" — fall back to unstripped names.
    let file = enum_file(
        "f.proto",
        "Foo",
        vec![enum_value("FOO_UNSET", 0), enum_value("FOO_", 1)],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["f.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("const FooUnset: Self = Self::FOO_UNSET"), "{c}");
    assert!(c.contains("const Foo: Self = Self::FOO_"), "{c}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn keyword_alias_is_escaped() {
    let file = enum_file(
        "k.proto",
        "Kind",
        vec![enum_value("KIND_UNKNOWN", 0), enum_value("KIND_SELF", 1)],
    );
    let files = generate(&[file], &["k.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    // `SELF` folds to the keyword `Self`, escaped to `Self_`.
    assert!(c.contains("const Self_: Self = Self::KIND_SELF"), "{c}");
}

#[test]
fn strip_decided_for_whole_enum_not_per_value() {
    // One value carries the enum prefix, one does not. Because stripping is an
    // all-or-nothing per-enum decision, neither is stripped — the result is not
    // a mix of `Bar` and `OtherValue`.
    let file = enum_file(
        "m.proto",
        "Mode",
        vec![enum_value("MODE_BAR", 0), enum_value("OTHER_VALUE", 1)],
    );
    let files = generate(&[file], &["m.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("const ModeBar: Self = Self::MODE_BAR"), "{c}");
    assert!(
        c.contains("const OtherValue: Self = Self::OTHER_VALUE"),
        "{c}"
    );
    assert!(!c.contains("const Bar"), "prefix must not be stripped: {c}");
}

#[test]
fn already_camel_with_acronym_is_skipped_not_duplicated() {
    // `MyValue` round-trips through the converter, so it is recognized as
    // redundant rather than emitted as a mangled `Myvalue` alias.
    let file = enum_file(
        "s.proto",
        "Shape",
        vec![enum_value("MyValue", 0), enum_value("OtherValue", 1)],
    );
    let files = generate(&[file], &["s.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    assert!(!c.contains("Myvalue"), "must not emit a mangled alias: {c}");
    assert!(!c.contains("impl Shape {"), "all aliases redundant: {c}");
}

#[test]
fn alias_const_doc_links_variant_not_duplicates_proto_doc() {
    let file = enum_file(
        "s.proto",
        "RuleLevel",
        vec![
            enum_value("RULE_LEVEL_UNKNOWN", 0),
            enum_value("RULE_LEVEL_HIGH", 1),
        ],
    );
    let files = generate(&[file], &["s.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    assert!(
        c.contains("Idiomatic alias for [`Self::RULE_LEVEL_HIGH`]"),
        "{c}"
    );
}

#[test]
fn redundant_alias_is_skipped() {
    // Values already in CamelCase need no alias and trigger no suppression.
    let file = enum_file(
        "s.proto",
        "State",
        vec![enum_value("Active", 0), enum_value("Inactive", 1)],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    assert!(
        !c.contains("impl State {"),
        "no alias impl block expected: {c}"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn allow_alias_values_get_idiomatic_aliases() {
    let mut file = proto3_file("code.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Code".to_string()),
        value: vec![
            enum_value("CODE_OK", 0),
            enum_value("CODE_SUCCESS", 0), // proto allow_alias of CODE_OK
            enum_value("CODE_ERROR", 1),
        ],
        options: (crate::generated::descriptor::EnumOptions {
            allow_alias: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });
    let files = generate(&[file], &["code.proto".to_string()], &idiomatic_config()).unwrap();
    let c = joined(&files);
    // Primary and its proto alias both get an idiomatic const, pointing at the
    // canonical variant for the number.
    assert!(c.contains("const Ok: Self = Self::CODE_OK"), "{c}");
    assert!(c.contains("const Success: Self = Self::CODE_OK"), "{c}");
    assert!(c.contains("const Error: Self = Self::CODE_ERROR"), "{c}");
}

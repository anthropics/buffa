//! The `(buffa.ext.field).name` option: a schema-chosen Rust name for a
//! field or a oneof variant.
//!
//! The option renames only Rust identifiers. `buffa-test`'s `ext_name`
//! fixture compiles the generated code and checks the wire format, JSON,
//! text format and reflection; these tests cover which names are generated
//! and which values are rejected.

use super::*;
use crate::generated::descriptor::FieldOptions;
use buffa::ExtensionSet;

const FILE: &str = "name.proto";

/// `field` with `(buffa.ext.field).name = name`.
fn named(mut field: FieldDescriptorProto, name: &str) -> FieldDescriptorProto {
    let mut options = FieldOptions::default();
    options.set_extension(
        &buffa_proto_options::FIELD,
        buffa_proto_options::FieldOptions {
            name: Some(name.to_string()),
            ..Default::default()
        },
    );
    field.options = options.into();
    field
}

fn string_field(name: &str, number: i32) -> FieldDescriptorProto {
    make_field(name, number, Label::LABEL_OPTIONAL, Type::TYPE_STRING)
}

/// A member of the first oneof of its message.
fn oneof_member(name: &str, number: i32) -> FieldDescriptorProto {
    FieldDescriptorProto {
        oneof_index: Some(0),
        ..string_field(name, number)
    }
}

fn message(name: &str, fields: Vec<FieldDescriptorProto>) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.to_string()),
        field: fields,
        ..Default::default()
    }
}

/// `message` with one oneof, which the fields built by [`oneof_member`]
/// belong to.
fn with_oneof(mut message: DescriptorProto, oneof: &str) -> DescriptorProto {
    message.oneof_decl.push(OneofDescriptorProto {
        name: Some(oneof.to_string()),
        ..Default::default()
    });
    message
}

/// A proto2 file in package `pkg`. proto2 gives every singular field
/// explicit presence, so each one has a `with_*` setter.
fn file_of(messages: Vec<DescriptorProto>) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(FILE.to_string()),
        package: Some("pkg".to_string()),
        syntax: Some("proto2".to_string()),
        message_type: messages,
        ..Default::default()
    }
}

fn generate_one(file: FileDescriptorProto, config: &CodeGenConfig) -> Result<String, CodeGenError> {
    generate(&[file], &[FILE.to_string()], config).map(|files| joined(&files))
}

fn idiomatic() -> CodeGenConfig {
    CodeGenConfig {
        idiomatic_field_names: true,
        ..Default::default()
    }
}

/// Whether `code` declares or matches an enum variant `name` with a payload.
/// A substring test would also match `PlainText(` for `Text`.
fn has_variant(code: &str, name: &str) -> bool {
    let declared = format!("{name}(");
    let matched = format!("::{name}(");
    code.lines()
        .any(|line| line.trim_start().starts_with(&declared) || line.contains(&matched))
}

#[track_caller]
fn assert_invalid(file: FileDescriptorProto, element: &str, name: &str, want: &NameOptionProblem) {
    let err = generate_one(file, &CodeGenConfig::default())
        .expect_err("an unusable name option must be rejected");
    let CodeGenError::InvalidNameOption {
        option,
        element: got_element,
        name: got_name,
        problem,
    } = &err
    else {
        panic!("expected InvalidNameOption, got: {err}");
    };
    assert_eq!(
        (*option, got_element.as_str(), got_name.as_str(), problem),
        ("(buffa.ext.field).name", element, name, want)
    );
}

/// Expect a conflict between two members of `pkg.Msg`: the option on
/// `element` gives it the Rust name that `other` has.
#[track_caller]
fn assert_conflict(
    file: FileDescriptorProto,
    config: &CodeGenConfig,
    element: &str,
    other: &str,
    rust_name: &str,
) {
    let err = generate_one(file, config).expect_err("a colliding name option must be rejected");
    let CodeGenError::NameOptionConflict {
        option,
        element: got_element,
        other: got_other,
        rust_name: got_rust_name,
    } = &err
    else {
        panic!("expected NameOptionConflict, got: {err}");
    };
    assert_eq!(
        (
            *option,
            got_element.as_str(),
            got_other.as_str(),
            got_rust_name.as_str()
        ),
        (
            "(buffa.ext.field).name",
            format!("pkg.Msg.{element}").as_str(),
            format!("pkg.Msg.{other}").as_str(),
            rust_name
        )
    );
}

#[test]
fn option_names_the_struct_field_and_its_setter() {
    let file = file_of(vec![message(
        "Msg",
        vec![named(string_field("type", 1), "kind")],
    )]);
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub kind:"), "{code}");
    assert!(code.contains("pub fn with_kind("), "{code}");
    assert!(!code.contains("r#type"), "{code}");
    // The doc tag still gives the proto name.
    assert!(code.contains("Field 1: `type`"), "{code}");
}

#[test]
fn option_leaves_json_and_text_names_alone() {
    let file = file_of(vec![message(
        "Msg",
        vec![named(string_field("type", 1), "kind")],
    )]);
    let config = CodeGenConfig {
        generate_json: true,
        generate_text: true,
        ..Default::default()
    };
    let code = generate_one(file, &config).unwrap();
    assert!(code.contains("rename = \"type\""), "{code}");
    assert!(!code.contains("rename = \"kind\""), "{code}");
    // The text decoder matches on the proto name.
    assert!(code.contains("\"type\" =>"), "{code}");
    assert!(!code.contains("\"kind\" =>"), "{code}");
}

#[test]
fn option_is_used_as_written_under_idiomatic_field_names() {
    // The flag would convert `remoteJid` to `remote_jid`. The option's value
    // is not converted, and the struct allows the non-snake name.
    let file = file_of(vec![message(
        "Msg",
        vec![named(string_field("remoteJid", 1), "peerId")],
    )]);
    let (files, warnings) =
        generate_with_diagnostics(&[file], &[FILE.to_string()], &idiomatic()).unwrap();
    let code = joined(&files);
    assert!(code.contains("pub peerId:"), "{code}");
    assert!(!code.contains("remote_jid"), "{code}");
    assert!(code.contains("#[allow(non_snake_case)]"), "{code}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn option_takes_the_field_out_of_the_collision_plan() {
    // Without the option, `userName` collides with `user_name` and becomes
    // `user_name_f12`, with a warning.
    let file = file_of(vec![message(
        "Msg",
        vec![
            string_field("user_name", 11),
            named(string_field("userName", 12), "login"),
        ],
    )]);
    let (files, warnings) =
        generate_with_diagnostics(&[file], &[FILE.to_string()], &idiomatic()).unwrap();
    let code = joined(&files);
    assert!(code.contains("pub user_name:"), "{code}");
    assert!(code.contains("pub login:"), "{code}");
    assert!(!code.contains("user_name_f12"), "{code}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn option_applies_to_one_message_only() {
    // `Other.type = 1` shares the name and number of `Msg.type = 1`, and an
    // adjustment in the collision plan would apply to both. The option does
    // not.
    let file = file_of(vec![
        message("Msg", vec![named(string_field("type", 1), "kind")]),
        message("Other", vec![string_field("type", 1)]),
    ]);
    let code = generate_one(file, &idiomatic()).unwrap();
    assert!(code.contains("pub kind:"), "{code}");
    assert!(code.contains("pub r#type:"), "{code}");
}

#[test]
fn option_names_a_oneof_variant_as_written() {
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                named(oneof_member("text", 1), "PlainText"),
                named(oneof_member("raw", 2), "raw_bytes"),
                named(oneof_member("url", 3), "HTTP_URL"),
                oneof_member("other", 4),
            ],
        ),
        "body",
    );
    let code = generate_one(file_of(vec![msg]), &CodeGenConfig::default()).unwrap();
    for variant in ["PlainText", "raw_bytes", "HTTP_URL", "Other"] {
        assert!(has_variant(&code, variant), "{variant}: {code}");
    }
    // The variants that the proto names would give, and the PascalCase
    // forms of the values.
    for variant in ["Text", "Raw", "Url", "RawBytes", "HTTPURL"] {
        assert!(!has_variant(&code, variant), "{variant}: {code}");
    }
}

#[test]
fn option_names_a_variant_under_idiomatic_field_names() {
    // The flag converts struct fields and oneof names, and leaves variants
    // alone. The option still sets the variant.
    let msg = with_oneof(
        message(
            "Msg",
            vec![named(oneof_member("textValue", 1), "PlainText")],
        ),
        "bodyKind",
    );
    let code = generate_one(file_of(vec![msg]), &idiomatic()).unwrap();
    assert!(has_variant(&code, "PlainText"), "{code}");
    assert!(!has_variant(&code, "TextValue"), "{code}");
    assert!(code.contains("pub body_kind:"), "{code}");
}

#[test]
fn option_on_a_proto3_optional_field_names_the_struct_field() {
    // A proto3 `optional` field is the one member of a synthetic oneof. It
    // is a struct field, and the synthetic oneof takes no name in the
    // struct, so another field can have the oneof's name `_f`.
    let field = FieldDescriptorProto {
        proto3_optional: Some(true),
        ..named(oneof_member("f", 1), "count")
    };
    let other = named(string_field("g", 2), "_f");
    let mut file = file_of(vec![with_oneof(message("Msg", vec![field, other]), "_f")]);
    file.syntax = Some("proto3".to_string());
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub count:"), "{code}");
    assert!(code.contains("pub _f:"), "{code}");
}

#[test]
fn value_that_is_not_an_identifier_is_rejected() {
    for value in ["", "1st", "a-b", "a b", "_", "naïve", "r#type", "a::b"] {
        let file = file_of(vec![message(
            "Msg",
            vec![named(string_field("f", 1), value)],
        )]);
        assert_invalid(
            file,
            "pkg.Msg.f",
            value,
            &NameOptionProblem::NotAnIdentifier,
        );
    }
    // The rule is the same for a field in a oneof.
    let msg = with_oneof(
        message("Msg", vec![named(oneof_member("f", 1), "plain-text")]),
        "body",
    );
    assert_invalid(
        file_of(vec![msg]),
        "pkg.Msg.f",
        "plain-text",
        &NameOptionProblem::NotAnIdentifier,
    );
}

#[test]
fn keyword_value_is_rejected() {
    for value in ["type", "self", "Self", "async", "gen"] {
        let file = file_of(vec![message(
            "Msg",
            vec![named(string_field("f", 1), value)],
        )]);
        assert_invalid(file, "pkg.Msg.f", value, &NameOptionProblem::Keyword);
    }
    // The rule is the same for a field in a oneof.
    for value in ["type", "Self"] {
        let msg = with_oneof(
            message("Msg", vec![named(oneof_member("f", 1), value)]),
            "body",
        );
        assert_invalid(
            file_of(vec![msg]),
            "pkg.Msg.f",
            value,
            &NameOptionProblem::Keyword,
        );
    }
}

#[test]
fn reserved_prefix_value_is_rejected() {
    let file = file_of(vec![message(
        "Msg",
        vec![named(string_field("f", 1), "__buffa_unknown_fields")],
    )]);
    assert_invalid(
        file,
        "pkg.Msg.f",
        "__buffa_unknown_fields",
        &NameOptionProblem::ReservedPrefix,
    );
    // The rule is the same for a field in a oneof.
    let msg = with_oneof(
        message("Msg", vec![named(oneof_member("f", 1), "__buffa_x")]),
        "body",
    );
    assert_invalid(
        file_of(vec![msg]),
        "pkg.Msg.f",
        "__buffa_x",
        &NameOptionProblem::ReservedPrefix,
    );
}

#[test]
fn underscore_values_name_a_variant_and_a_field_as_written() {
    // The proto names `__` and `_1` derive a variant that is not an
    // identifier (empty, `1`), and `self_` derives `Self`, which buffa
    // escapes to `Self_`. An option value is not converted, so each one is
    // the variant, or the struct field, as written.
    for value in ["__", "_1", "self_"] {
        let msg = with_oneof(
            message("Msg", vec![named(oneof_member("f", 1), value)]),
            "body",
        );
        let code = generate_one(file_of(vec![msg]), &CodeGenConfig::default()).unwrap();
        assert!(has_variant(&code, value), "{value}: {code}");

        let file = file_of(vec![message(
            "Msg",
            vec![named(string_field("f", 1), value)],
        )]);
        let code = generate_one(file, &CodeGenConfig::default()).unwrap();
        assert!(code.contains(&format!("pub {value}:")), "{code}");
    }
}

#[test]
fn invalid_value_in_a_nested_message_names_the_field() {
    let mut outer = message("Outer", vec![]);
    outer
        .nested_type
        .push(message("Inner", vec![named(string_field("f", 1), "type")]));
    assert_invalid(
        file_of(vec![outer]),
        "pkg.Outer.Inner.f",
        "type",
        &NameOptionProblem::Keyword,
    );
}

#[test]
fn option_on_an_extension_is_rejected() {
    let extension = |name: &str| FieldDescriptorProto {
        extendee: Some(".pkg.Msg".to_string()),
        ..named(string_field(name, 100), "renamed")
    };

    let mut file = file_of(vec![message("Msg", vec![])]);
    file.extension.push(extension("at_file"));
    assert_invalid(
        file,
        "pkg.at_file",
        "renamed",
        &NameOptionProblem::OnExtension,
    );

    let mut scope = message("Scope", vec![]);
    scope.extension.push(extension("in_message"));
    let file = file_of(vec![message("Msg", vec![]), scope]);
    assert_invalid(
        file,
        "pkg.Scope.in_message",
        "renamed",
        &NameOptionProblem::OnExtension,
    );
}

#[test]
fn two_fields_with_one_name_conflict() {
    let default = CodeGenConfig::default();

    // The value is another field's proto name.
    let file = file_of(vec![message(
        "Msg",
        vec![string_field("a", 1), named(string_field("b", 2), "a")],
    )]);
    assert_conflict(file, &default, "b", "a", "a");

    // The same pair declared in the other order: the option is still the
    // one to change.
    let file = file_of(vec![message(
        "Msg",
        vec![named(string_field("b", 1), "a"), string_field("a", 2)],
    )]);
    assert_conflict(file, &default, "b", "a", "a");

    // Two options with one value: the error names the second.
    let file = file_of(vec![message(
        "Msg",
        vec![
            named(string_field("a", 1), "same"),
            named(string_field("b", 2), "same"),
        ],
    )]);
    assert_conflict(file, &default, "b", "a", "same");

    // The value is the escaped form of a keyword field.
    let file = file_of(vec![message(
        "Msg",
        vec![
            string_field("self", 1),
            named(string_field("b", 2), "self_"),
        ],
    )]);
    assert_conflict(file, &default, "b", "self", "self_");
}

#[test]
fn two_derived_names_that_collide_still_generate() {
    // `self` and `self_` both derive `self_`. An option elsewhere in the
    // message runs the check, and the check leaves the pair to rustc.
    let file = file_of(vec![message(
        "Msg",
        vec![
            string_field("self", 1),
            string_field("self_", 2),
            named(string_field("type", 3), "kind"),
        ],
    )]);
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub kind:"), "{code}");

    // The same for two variants: `foo_bar` and `fooBar` both derive `FooBar`.
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                oneof_member("foo_bar", 1),
                oneof_member("fooBar", 2),
                named(oneof_member("raw", 3), "Binary"),
            ],
        ),
        "body",
    );
    let code = generate_one(file_of(vec![msg]), &CodeGenConfig::default()).unwrap();
    assert!(has_variant(&code, "Binary"), "{code}");
}

#[test]
fn field_and_converted_field_conflict() {
    // `idiomatic_field_names` converts `fooBar` to `foo_bar`. The option does
    // not make the plan rename `fooBar`.
    let file = file_of(vec![message(
        "Msg",
        vec![
            string_field("fooBar", 1),
            named(string_field("b", 2), "foo_bar"),
        ],
    )]);
    assert_conflict(file, &idiomatic(), "b", "fooBar", "foo_bar");
}

#[test]
fn field_and_converted_oneof_conflict() {
    // `idiomatic_field_names` converts the oneof `dataValue` to `data_value`.
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                oneof_member("text", 1),
                named(string_field("b", 2), "data_value"),
            ],
        ),
        "dataValue",
    );
    assert_conflict(
        file_of(vec![msg]),
        &idiomatic(),
        "b",
        "dataValue",
        "data_value",
    );
}

#[test]
fn field_and_oneof_conflict() {
    let msg = with_oneof(
        message(
            "Msg",
            vec![oneof_member("text", 1), named(string_field("b", 2), "body")],
        ),
        "body",
    );
    assert_conflict(
        file_of(vec![msg]),
        &CodeGenConfig::default(),
        "b",
        "body",
        "body",
    );
}

#[test]
fn two_variants_with_one_name_conflict() {
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                oneof_member("plain_text", 1),
                named(oneof_member("raw", 2), "PlainText"),
            ],
        ),
        "body",
    );
    assert_conflict(
        file_of(vec![msg]),
        &CodeGenConfig::default(),
        "raw",
        "plain_text",
        "PlainText",
    );

    // Two options with one value: the error names the second.
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                named(oneof_member("a", 1), "Same"),
                named(oneof_member("b", 2), "Same"),
            ],
        ),
        "body",
    );
    assert_conflict(
        file_of(vec![msg]),
        &CodeGenConfig::default(),
        "b",
        "a",
        "Same",
    );

    // The value is the escaped variant of a keyword field: `self` derives
    // `Self_`.
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                oneof_member("self", 1),
                named(oneof_member("b", 2), "Self_"),
            ],
        ),
        "body",
    );
    assert_conflict(
        file_of(vec![msg]),
        &CodeGenConfig::default(),
        "b",
        "self",
        "Self_",
    );
}

#[test]
fn variants_that_differ_in_case_do_not_conflict() {
    // `text` derives `Text`, and the values are not converted, so the four
    // variants are distinct.
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                oneof_member("text", 1),
                named(oneof_member("a", 2), "text"),
                named(oneof_member("b", 3), "TEXT"),
                named(oneof_member("c", 4), "plain_text"),
                named(oneof_member("d", 5), "PlainText"),
            ],
        ),
        "body",
    );
    let code = generate_one(file_of(vec![msg]), &CodeGenConfig::default()).unwrap();
    for variant in ["Text", "text", "TEXT", "plain_text", "PlainText"] {
        assert!(has_variant(&code, variant), "{variant}: {code}");
    }
}

#[test]
fn variant_and_struct_field_do_not_conflict() {
    // A variant and a struct field are in different namespaces.
    let msg = with_oneof(
        message(
            "Msg",
            vec![
                named(oneof_member("text", 1), "note"),
                named(string_field("b", 2), "note"),
            ],
        ),
        "body",
    );
    let code = generate_one(file_of(vec![msg]), &CodeGenConfig::default()).unwrap();
    assert!(has_variant(&code, "note"), "{code}");
    assert!(code.contains("pub note:"), "{code}");
}

#[test]
fn option_in_a_file_that_is_not_generated_is_not_checked() {
    let mut dependency = file_of(vec![message(
        "Dep",
        vec![named(string_field("f", 1), "type")],
    )]);
    dependency.name = Some("dep.proto".to_string());
    let mut file = file_of(vec![message("Msg", vec![string_field("f", 1)])]);
    file.dependency.push("dep.proto".to_string());

    let files = generate(
        &[dependency, file],
        &[FILE.to_string()],
        &CodeGenConfig::default(),
    );
    assert!(files.is_ok(), "{files:?}");
}

#[test]
fn field_without_a_name_is_rejected() {
    for name in [None, Some(String::new())] {
        let field = FieldDescriptorProto {
            name,
            ..string_field("f", 1)
        };
        let err = generate_one(
            file_of(vec![message("Msg", vec![field])]),
            &CodeGenConfig::default(),
        )
        .expect_err("a field needs a name");
        assert!(
            matches!(err, CodeGenError::MissingField("field.name")),
            "{err}"
        );
    }
}

#[test]
fn error_messages_name_the_option_the_element_and_the_fix() {
    let invalid = |field: FieldDescriptorProto| {
        let msg = message("Msg", vec![field]);
        generate_one(file_of(vec![msg]), &CodeGenConfig::default())
            .expect_err("an unusable name option must be rejected")
            .to_string()
    };
    let prefix =
        |value: &str| format!("invalid `(buffa.ext.field).name` = {value:?} on 'pkg.Msg.f': ");

    assert_eq!(
        invalid(named(string_field("f", 1), "1st")),
        prefix("1st")
            + "the value is not an ASCII Rust identifier (letters, digits and `_`, \
               not starting with a digit, and not `_` alone)"
    );
    assert_eq!(
        invalid(named(string_field("f", 1), "fn")),
        prefix("fn") + "the value is a Rust keyword, and buffa does not escape a `name` value"
    );
    assert_eq!(
        invalid(named(string_field("f", 1), "__buffa_x")),
        prefix("__buffa_x")
            + "names that start with `__buffa_` are reserved for buffa's own identifiers"
    );

    let mut file = file_of(vec![message("Msg", vec![])]);
    file.extension.push(FieldDescriptorProto {
        extendee: Some(".pkg.Msg".to_string()),
        ..named(string_field("f", 100), "renamed")
    });
    let err = generate_one(file, &CodeGenConfig::default()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid `(buffa.ext.field).name` = \"renamed\" on 'pkg.f': \
         the option does not rename an extension"
    );

    let file = file_of(vec![message(
        "Msg",
        vec![string_field("a", 1), named(string_field("b", 2), "a")],
    )]);
    let err = generate_one(file, &CodeGenConfig::default()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "name conflict: `(buffa.ext.field).name` gives 'pkg.Msg.b' the Rust name `a`, \
         which 'pkg.Msg.a' also has; change the option"
    );
}

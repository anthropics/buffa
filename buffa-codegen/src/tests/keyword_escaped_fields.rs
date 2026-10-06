//! A member named `self`, `super`, `crate` or `Self` is escaped with a `_`
//! suffix, which can be the name of another member of the same message.

use super::*;

fn file_with_members(
    fields: Vec<FieldDescriptorProto>,
    oneof_names: &[&str],
) -> FileDescriptorProto {
    let mut file = FileDescriptorProto {
        name: Some("keyword_names.proto".to_string()),
        package: Some("pkg".to_string()),
        syntax: Some("proto2".to_string()),
        ..Default::default()
    };
    file.message_type.push(DescriptorProto {
        name: Some("Collision".to_string()),
        field: fields,
        oneof_decl: oneof_names
            .iter()
            .map(|name| OneofDescriptorProto {
                name: Some((*name).to_string()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    });
    file
}

fn regular_field(name: &str, number: i32) -> FieldDescriptorProto {
    make_field(name, number, Label::LABEL_OPTIONAL, Type::TYPE_INT32)
}

fn config(idiomatic_field_names: bool) -> CodeGenConfig {
    CodeGenConfig {
        idiomatic_field_names,
        ..Default::default()
    }
}

fn adjusted_warnings(warnings: &[CodeGenWarning]) -> Vec<(&str, &[(String, String)])> {
    warnings
        .iter()
        .filter_map(|warning| match warning {
            CodeGenWarning::KeywordEscapedNamesAdjusted {
                message_name,
                assignments,
                ..
            } => Some((message_name.as_str(), assignments.as_slice())),
            _ => None,
        })
        .collect()
}

#[test]
fn keyword_field_next_to_its_escaped_name_takes_a_second_underscore() {
    for idiomatic_field_names in [false, true] {
        for (keyword, authored) in [
            ("self", "self_"),
            ("super", "super_"),
            ("crate", "crate_"),
            ("Self", "Self_"),
        ] {
            let file = file_with_members(
                vec![regular_field(keyword, 1), regular_field(authored, 2)],
                &[],
            );
            let (files, warnings) = generate_with_diagnostics(
                &[file],
                &["keyword_names.proto".to_string()],
                &config(idiomatic_field_names),
            )
            .expect("generation succeeds");
            let content = joined(&files);
            // `idiomatic_field_names` lowercases `Self` and `Self_` first.
            let (stem, authored) = if idiomatic_field_names && keyword == "Self" {
                ("self", "self_")
            } else {
                (keyword, authored)
            };

            assert!(content.contains(&format!("pub {stem}__:")), "{content}");
            assert!(content.contains(&format!("pub {authored}:")), "{content}");
            assert!(
                content.contains(&format!("fn with_{stem}__(")),
                "the setter follows the field name: {content}"
            );
            assert_eq!(
                adjusted_warnings(&warnings),
                [(
                    "pkg.Collision",
                    &[(keyword.to_string(), format!("{stem}__"))][..]
                )]
            );
        }
    }
}

#[test]
fn upper_case_keyword_is_planned_under_idiomatic_names() {
    // proto3 accepts this pair: the JSON names `SELF` and `self` differ.
    let file = file_with_members(
        vec![regular_field("SELF", 1), regular_field("self_", 2)],
        &[],
    );
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["keyword_names.proto".to_string()], &config(true))
            .expect("generation succeeds");
    let content = joined(&files);

    assert!(content.contains("pub self__:"), "{content}");
    assert!(content.contains("pub self_:"), "{content}");
    assert_eq!(
        adjusted_warnings(&warnings),
        [(
            "pkg.Collision",
            &[("SELF".to_string(), "self__".to_string())][..]
        )]
    );
}

#[test]
fn keyword_field_without_a_collision_keeps_one_underscore() {
    let file = file_with_members(
        vec![regular_field("self", 1), regular_field("other", 2)],
        &[],
    );
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation succeeds");
    let content = joined(&files);

    assert!(content.contains("pub self_:"), "{content}");
    assert!(!content.contains("self__"), "{content}");
    assert_eq!(adjusted_warnings(&warnings), []);
}

#[test]
fn keyword_oneof_next_to_a_field_with_its_escaped_name_is_renamed() {
    for idiomatic_field_names in [false, true] {
        let mut choice = regular_field("choice", 2);
        choice.oneof_index = Some(0);
        let file = file_with_members(vec![regular_field("self_", 1), choice], &["self"]);
        let (files, warnings) = generate_with_diagnostics(
            &[file],
            &["keyword_names.proto".to_string()],
            &config(idiomatic_field_names),
        )
        .expect("generation succeeds");
        let content = joined(&files);

        assert!(content.contains("pub self_: "), "{content}");
        assert!(content.contains("pub self__: "), "{content}");
        assert_eq!(
            adjusted_warnings(&warnings),
            [(
                "pkg.Collision",
                &[("self".to_string(), "self__".to_string())][..]
            )]
        );
    }
}

#[test]
fn renamed_field_skips_names_that_other_members_have() {
    let file = file_with_members(
        vec![
            regular_field("self", 1),
            regular_field("self_", 2),
            regular_field("self__", 3),
        ],
        &[],
    );
    let files = generate(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation succeeds");
    let content = joined(&files);

    for field in ["pub self_:", "pub self__:", "pub self___:"] {
        assert!(content.contains(field), "{field}: {content}");
    }
}

#[test]
fn message_with_the_same_field_inherits_the_name_and_a_warning() {
    let mut file = file_with_members(
        vec![regular_field("self", 1), regular_field("self_", 2)],
        &[],
    );
    file.message_type.push(DescriptorProto {
        name: Some("Inherited".to_string()),
        // `self__` is taken here, so both messages get `self___`.
        field: vec![regular_field("self", 1), regular_field("self__", 2)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("OtherNumber".to_string()),
        field: vec![regular_field("self", 7)],
        ..Default::default()
    });
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["keyword_names.proto".to_string()],
        // Owned structs only, so each field appears once in the output.
        &CodeGenConfig {
            generate_views: false,
            ..Default::default()
        },
    )
    .expect("generation succeeds");
    let content = joined(&files);

    assert_eq!(content.matches("pub self___:").count(), 2, "{content}");
    // A field with another number is a different key and keeps `self_`.
    assert_eq!(content.matches("pub self_:").count(), 2, "{content}");
    let renamed = [("self".to_string(), "self___".to_string())];
    assert_eq!(
        adjusted_warnings(&warnings),
        [
            ("pkg.Collision", &renamed[..]),
            ("pkg.Inherited", &renamed[..])
        ]
    );
}

#[test]
fn collision_in_an_imported_file_leaves_generated_messages_alone() {
    let mut dependency = file_with_members(
        vec![regular_field("self", 1), regular_field("self_", 2)],
        &[],
    );
    dependency.name = Some("dependency.proto".to_string());
    dependency.package = Some("dependency".to_string());

    let consumer = FileDescriptorProto {
        name: Some("consumer.proto".to_string()),
        package: Some("consumer".to_string()),
        syntax: Some("proto2".to_string()),
        message_type: vec![DescriptorProto {
            name: Some("Consumer".to_string()),
            // The same key as the dependency's keyword field.
            field: vec![regular_field("self", 1)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let (files, warnings) = generate_with_diagnostics(
        &[dependency, consumer],
        &["consumer.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation succeeds");
    let content = joined(&files);

    assert!(content.contains("pub self_:"), "{content}");
    assert!(!content.contains("self__"), "{content}");
    assert_eq!(adjusted_warnings(&warnings), []);
}

#[test]
fn keyword_field_next_to_a_oneof_with_its_escaped_name_is_renamed() {
    let mut choice = regular_field("choice", 2);
    choice.oneof_index = Some(0);
    let file = file_with_members(vec![regular_field("self", 1), choice], &["self_"]);
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig {
            generate_views: false,
            ..Default::default()
        },
    )
    .expect("generation succeeds");
    let content = joined(&files);

    // The oneof keeps `self_`; the field moves.
    assert!(
        content.contains("pub self_: ::core::option::Option<"),
        "{content}"
    );
    assert!(
        content.contains("pub self__: ::core::option::Option<i32>"),
        "{content}"
    );
    assert_eq!(
        adjusted_warnings(&warnings),
        [(
            "pkg.Collision",
            &[("self".to_string(), "self__".to_string())][..]
        )]
    );
}

#[test]
fn idiomatic_plan_names_are_the_input_of_the_keyword_plan() {
    // `Self` and `self` both convert to `self`; the idiomatic plan suffixes
    // the converted one. The keyword plan then sees `self_f1`, `self`, `self_`.
    let file = file_with_members(
        vec![
            regular_field("Self", 1),
            regular_field("self", 2),
            regular_field("self_", 3),
        ],
        &[],
    );
    let files = generate(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig {
            generate_views: false,
            ..config(true)
        },
    )
    .expect("generation succeeds");
    let content = joined(&files);
    for field in ["pub self_f1:", "pub self__:", "pub self_:"] {
        assert_eq!(content.matches(field).count(), 1, "{field}: {content}");
    }

    // A oneof whose conversion collides keeps its proto name, `Self`, which
    // escapes to `Self_` and collides with no member.
    let mut choice = regular_field("choice", 3);
    choice.oneof_index = Some(0);
    let file = file_with_members(
        vec![regular_field("self", 1), regular_field("self_", 2), choice],
        &["Self"],
    );
    let files = generate(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig {
            generate_views: false,
            ..config(true)
        },
    )
    .expect("generation succeeds");
    let content = joined(&files);
    for field in ["pub Self_:", "pub self__:", "pub self_:"] {
        assert_eq!(content.matches(field).count(), 1, "{field}: {content}");
    }
}

#[test]
fn renamed_members_carry_a_doc_note_and_the_warning_names_them() {
    let mut choice = regular_field("choice", 3);
    choice.oneof_index = Some(0);
    let file = file_with_members(
        vec![
            regular_field("super", 1),
            regular_field("super_", 2),
            regular_field("self_", 4),
            choice,
        ],
        &["self"],
    );
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation succeeds");
    let content = joined(&files);

    assert!(
        content.contains("`super_`, the usual Rust name of this field,"),
        "{content}"
    );
    assert!(
        content.contains("`self_`, the usual Rust name of this oneof,"),
        "{content}"
    );
    let [warning] = warnings.as_slice() else {
        panic!("one warning: {warnings:?}");
    };
    let text = warning.to_string();
    assert!(text.starts_with("message `pkg.Collision`: "), "{text}");
    assert!(
        text.contains("adjusted: `self` → `self__`, `super` → `super__`"),
        "{text}"
    );
}

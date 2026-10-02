//! Rust keyword escaping can collapse distinct proto field and oneof names.

use super::*;

fn file_with_members(
    fields: Vec<FieldDescriptorProto>,
    oneof_names: &[&str],
) -> FileDescriptorProto {
    let mut file = FileDescriptorProto {
        name: Some("keyword_names.proto".to_string()),
        syntax: Some("proto2".to_string()),
        ..Default::default()
    };
    file.package = Some("pkg".to_string());
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

#[test]
fn keyword_escaped_fields_are_deconflicted_in_both_naming_modes() {
    for idiomatic_field_names in [false, true] {
        for (keyword, authored_name, adjusted_name) in [
            ("self", "self_", "self_f1"),
            ("super", "super_", "super_f1"),
            ("crate", "crate_", "crate_f1"),
            ("Self", "Self_", "Self_f1"),
        ] {
            let file = file_with_members(
                vec![regular_field(keyword, 1), regular_field(authored_name, 2)],
                &[],
            );
            let (files, warnings) = generate_with_diagnostics(
                &[file],
                &["keyword_names.proto".to_string()],
                &config(idiomatic_field_names),
            )
            .expect("keyword-escaped field names must be deconflicted");
            let content = joined(&files);
            let adjusted_name = if idiomatic_field_names && keyword == "Self" {
                "self_f1"
            } else {
                adjusted_name
            };
            let authored_name = if idiomatic_field_names && authored_name == "Self_" {
                "self_"
            } else {
                authored_name
            };

            assert!(
                content.contains(&format!("pub {adjusted_name}:")),
                "{content}"
            );
            assert!(
                content.contains(&format!("pub {authored_name}:")),
                "{content}"
            );
            assert!(
                warnings.iter().any(|warning| matches!(
                    warning,
                    CodeGenWarning::RustIdentifierCollisionAdjusted { message_name, .. }
                        if message_name == "pkg.Collision"
                )),
                "{warnings:?}"
            );
        }
    }
}

#[test]
fn crate_keyword_collision_keeps_the_authored_non_keyword_name() {
    let file = file_with_members(
        vec![regular_field("crate", 3), regular_field("crate_", 4)],
        &[],
    );
    let files = generate(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("keyword-escaped field names must be deconflicted");
    let content = joined(&files);

    assert!(content.contains("pub crate_f3:"), "{content}");
    assert!(content.contains("pub crate_:"), "{content}");
}

#[test]
fn keyword_escaped_oneof_field_is_deconflicted_from_regular_field() {
    for idiomatic_field_names in [false, true] {
        let mut choice = regular_field("choice", 2);
        choice.oneof_index = Some(0);
        let file = file_with_members(vec![regular_field("self_", 1), choice], &["self"]);
        let files = generate(
            &[file],
            &["keyword_names.proto".to_string()],
            &config(idiomatic_field_names),
        )
        .expect("oneof and field names must be deconflicted");
        let content = joined(&files);

        assert!(content.contains("pub self_:"), "{content}");
        assert!(content.contains("pub self_oneof:"), "{content}");
    }
}

#[test]
fn field_collision_suffix_skips_an_existing_field_name() {
    let file = file_with_members(
        vec![
            regular_field("self", 1),
            regular_field("self_", 2),
            regular_field("self_f1", 3),
        ],
        &[],
    );
    let files = generate(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generated field names must remain unique");
    let content = joined(&files);

    assert!(content.contains("pub self_f1_2:"), "{content}");
    assert!(content.contains("pub self_f1:"), "{content}");
    assert!(content.contains("pub self_:"), "{content}");
}

#[test]
fn shared_field_name_adjustment_is_reported_in_each_message() {
    let mut file = file_with_members(
        vec![regular_field("self", 1), regular_field("self_", 2)],
        &[],
    );
    file.message_type.push(DescriptorProto {
        name: Some("Inherited".to_string()),
        field: vec![regular_field("self", 1)],
        ..Default::default()
    });
    let (files, warnings) = generate_with_diagnostics(
        &[file],
        &["keyword_names.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("a shared name adjustment must be usable in each message");
    let content = joined(&files);

    assert!(content.contains("pub self_f1:"), "{content}");
    assert!(
        warnings.iter().any(|warning| matches!(
            warning,
            CodeGenWarning::RustIdentifierCollisionAdjusted { message_name, .. }
                if message_name == "pkg.Inherited"
        )),
        "{warnings:?}"
    );
}

#[test]
fn keyword_collision_warning_is_omitted_for_files_not_generated() {
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
            field: vec![regular_field("value", 1)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let (_, warnings) = generate_with_diagnostics(
        &[dependency, consumer],
        &["consumer.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("consumer generation must ignore naming diagnostics for dependencies");

    assert!(
        !warnings.iter().any(|warning| matches!(
            warning,
            CodeGenWarning::RustIdentifierCollisionAdjusted { message_name, .. }
                if message_name == "dependency.Collision"
        )),
        "{warnings:?}"
    );
}

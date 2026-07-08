//! Idiomatic snake_case field-name conversion
//! ([`CodeGenConfig::idiomatic_field_names`]).
//!
//! The flag renames only Rust source names (struct fields, view accessors,
//! `has_*`/`with_*` methods, oneof struct fields); the wire format, JSON
//! names, text-format names, and reflection lookups keep the descriptor's
//! names. Collisions are adjusted deterministically (`_f<number>` suffix,
//! verbatim fallback for oneofs) with a build warning.

use super::*;

fn fields_config() -> CodeGenConfig {
    CodeGenConfig {
        idiomatic_field_names: true,
        ..Default::default()
    }
}

/// A proto2 file (collisions require proto2 — protoc rejects them for
/// proto3/editions via conflicting `json_name`s).
fn proto2_file(name: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.to_string()),
        syntax: Some("proto2".to_string()),
        ..Default::default()
    }
}

fn message_file(
    file: &str,
    msg_name: &str,
    fields: Vec<FieldDescriptorProto>,
) -> FileDescriptorProto {
    let mut f = proto3_file(file);
    f.message_type.push(DescriptorProto {
        name: Some(msg_name.to_string()),
        field: fields,
        ..Default::default()
    });
    f
}

fn string_field(name: &str, number: i32) -> FieldDescriptorProto {
    make_field(name, number, Label::LABEL_OPTIONAL, Type::TYPE_STRING)
}

#[test]
fn off_by_default_names_are_verbatim() {
    let file = message_file("s.proto", "Msg", vec![string_field("remoteJid", 1)]);
    let files = generate(&[file], &["s.proto".to_string()], &CodeGenConfig::default()).unwrap();
    let c = joined(&files);
    assert!(c.contains("pub remoteJid:"), "{c}");
    assert!(!c.contains("pub remote_jid:"), "{c}");
}

#[test]
fn camel_case_fields_convert_to_snake_case() {
    let file = message_file(
        "s.proto",
        "Msg",
        vec![
            string_field("remoteJid", 1),
            make_field(
                "messageTimestamp",
                2,
                Label::LABEL_OPTIONAL,
                Type::TYPE_UINT64,
            ),
            string_field("already_snake", 3),
        ],
    );
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("pub remote_jid:"), "{c}");
    assert!(c.contains("pub message_timestamp:"), "{c}");
    assert!(c.contains("pub already_snake:"), "{c}");
    assert!(!c.contains("remoteJid:"), "no verbatim ident remains: {c}");
}

#[test]
fn json_names_are_preserved_under_rename() {
    let file = message_file("s.proto", "Msg", vec![string_field("remoteJid", 1)]);
    let config = CodeGenConfig {
        idiomatic_field_names: true,
        generate_json: true,
        ..Default::default()
    };
    let files = generate(&[file], &["s.proto".to_string()], &config).unwrap();
    let c = joined(&files);
    // Owned struct: serde rename pins the JSON name to the descriptor's name.
    assert!(c.contains(r#"rename = "remoteJid""#), "{c}");
    // View serialize: the string key is the JSON name, not the Rust ident.
    assert!(c.contains(r#""remoteJid""#), "{c}");
}

#[test]
fn text_format_names_are_preserved_under_rename() {
    let file = message_file("s.proto", "Msg", vec![string_field("remoteJid", 1)]);
    let config = CodeGenConfig {
        idiomatic_field_names: true,
        generate_text: true,
        ..Default::default()
    };
    let files = generate(&[file], &["s.proto".to_string()], &config).unwrap();
    let c = joined(&files);
    // The text-format name literal stays the proto name while the accessed
    // ident is renamed.
    assert!(
        c.contains(r#""remoteJid" => "#) || c.contains(r#""remoteJid""#),
        "{c}"
    );
    assert!(c.contains("self.remote_jid"), "{c}");
}

#[test]
fn with_setter_follows_rename() {
    // proto2 optional → explicit presence → `Option<T>` field with a
    // `with_*` setter (proto3 implicit-presence fields get no setter).
    let mut file = proto2_file("s.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![string_field("remoteJid", 1)],
        ..Default::default()
    });
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("fn with_remote_jid"), "{c}");
    assert!(!c.contains("with_remoteJid"), "{c}");
}

#[test]
fn oneof_names_convert() {
    let mut file = proto3_file("s.proto");
    let mut member = string_field("textBody", 2);
    member.oneof_index = Some(0);
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![string_field("plainField", 1), member],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("messageKind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    // Oneof struct field renamed; variant names (Pascal) are unchanged.
    assert!(c.contains("pub message_kind:"), "{c}");
    assert!(c.contains("TextBody"), "{c}");
}

#[test]
fn keyword_conversion_is_escaped() {
    // `Type` converts to `type`, which must come out as `r#type`.
    let file = message_file("s.proto", "Msg", vec![string_field("Type", 1)]);
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("pub r#type:"), "{c}");
}

#[test]
fn collision_suffixes_changed_field_and_warns() {
    let mut file = proto2_file("s.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![string_field("user_name", 11), string_field("userName", 12)],
        ..Default::default()
    });
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    // The already-snake field keeps its name; the converted one is suffixed
    // with its field number.
    assert!(c.contains("pub user_name:"), "{c}");
    assert!(c.contains("pub user_name_f12:"), "{c}");
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::IdiomaticFieldNamesAdjusted { message_name, .. }
                if message_name == "Msg"
        )),
        "{warnings:?}"
    );
}

#[test]
fn adjusted_field_gets_doc_note() {
    let mut file = proto2_file("s.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![string_field("user_name", 11), string_field("userName", 12)],
        ..Default::default()
    });
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(
        c.contains("adjusted to `user_name_f12`"),
        "adjusted field carries a doc note: {c}"
    );
}

#[test]
fn verbatim_fallback_gets_non_snake_allow() {
    // `userName = 12` would suffix to `user_name_f12`, which collides with
    // the literal third field — the changed member reverts to its verbatim
    // camelCase name and must carry #[allow(non_snake_case)] so the
    // consumer crate compiles warning-free.
    let mut file = proto2_file("s.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            string_field("user_name", 11),
            string_field("userName", 12),
            string_field("user_name_f12", 13),
        ],
        ..Default::default()
    });
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("pub userName:"), "{c}");
    assert!(c.contains("#[allow(non_snake_case)]"), "{c}");
}

#[test]
fn non_snake_allow_is_detection_scoped() {
    // The allow is detection-based and independent of the flag: a verbatim
    // camelCase proto compiled with the flag OFF gets the scoped allow...
    let camel = message_file("s.proto", "Msg", vec![string_field("remoteJid", 1)]);
    let files = generate(
        &[camel],
        &["s.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();
    assert!(joined(&files).contains("#[allow(non_snake_case)]"));
    // ...a snake-conforming proto never gets it (zero output diff), flag off
    // or on...
    for config in [CodeGenConfig::default(), fields_config()] {
        let snake = message_file("s.proto", "Msg", vec![string_field("plain_field", 1)]);
        let files = generate(&[snake], &["s.proto".to_string()], &config).unwrap();
        assert!(!joined(&files).contains("non_snake_case"));
    }
    // ...and a camelCase proto with the flag ON converts cleanly, so no
    // allow is needed there either.
    let camel = message_file("s.proto", "Msg", vec![string_field("remoteJid", 1)]);
    let files = generate(&[camel], &["s.proto".to_string()], &fields_config()).unwrap();
    assert!(!joined(&files).contains("non_snake_case"));
}

#[test]
fn non_snake_allow_fires_for_oneof_only() {
    // A message whose only non-snake member is a real oneof exercises the
    // oneof branch of the detection (the fields are all conforming).
    let mut file = proto3_file("s.proto");
    let mut member = string_field("text_body", 2);
    member.oneof_index = Some(0);
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![string_field("plain_field", 1), member],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("oneofKind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(&[file], &["s.proto".to_string()], &CodeGenConfig::default()).unwrap();
    let c = joined(&files);
    assert!(c.contains("pub oneofKind:"), "{c}");
    assert!(c.contains("#[allow(non_snake_case)]"), "{c}");
}

#[test]
fn no_warning_without_collision() {
    let file = message_file("s.proto", "Msg", vec![string_field("remoteJid", 1)]);
    let (_, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::IdiomaticFieldNamesAdjusted { .. })),
        "{warnings:?}"
    );
}

#[test]
fn owned_view_reserved_check_uses_converted_name() {
    // `toOwnedMessage` converts to `to_owned_message`, a reserved wrapper
    // method — the accessor must be suppressed (not emitted with a colliding
    // name).
    let file = message_file("s.proto", "Msg", vec![string_field("toOwnedMessage", 1)]);
    let (files, warnings) =
        generate_with_diagnostics(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::OwnedViewAccessorSuppressed { .. })),
        "{warnings:?}"
    );
    // The struct field itself is still renamed and present.
    assert!(c.contains("pub to_owned_message:"), "{c}");
}

#[test]
fn required_has_method_follows_rename() {
    let mut file = proto2_file("s.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "requiredField",
            1,
            Label::LABEL_REQUIRED,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    let files = generate(&[file], &["s.proto".to_string()], &fields_config()).unwrap();
    let c = joined(&files);
    assert!(c.contains("fn has_required_field"), "{c}");
    assert!(!c.contains("has_requiredField"), "{c}");
}

//! Custom type/field/message attribute injection into generated code.

use super::*;

// ── type_attribute tests ────────────────────────────────────────────

fn attr_config(
    type_attrs: Vec<(&str, &str)>,
    field_attrs: Vec<(&str, &str)>,
    message_attrs: Vec<(&str, &str)>,
) -> CodeGenConfig {
    CodeGenConfig {
        generate_views: false,
        type_attributes: type_attrs
            .into_iter()
            .map(|(p, a)| (p.to_string(), a.to_string()))
            .collect(),
        field_attributes: field_attrs
            .into_iter()
            .map(|(p, a)| (p.to_string(), a.to_string()))
            .collect(),
        message_attributes: message_attrs
            .into_iter()
            .map(|(p, a)| (p.to_string(), a.to_string()))
            .collect(),
        ..CodeGenConfig::default()
    }
}

#[test]
fn test_type_attribute_on_message() {
    let mut file = proto3_file("msg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let config = attr_config(vec![(".", "#[derive(Hash)]")], vec![], vec![]);
    let files = generate(&[file], &["msg.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    assert!(
        content.contains("derive(Hash)"),
        "type_attribute should appear on struct: {content}"
    );
}

#[test]
fn test_type_attribute_on_enum() {
    let mut file = proto3_file("color.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("GREEN", 1)],
        ..Default::default()
    });
    // Use an attribute not in the default enum derive set.
    let config = attr_config(vec![(".", "#[derive(serde::Serialize)]")], vec![], vec![]);
    let files = generate(&[file], &["color.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    assert!(
        content.contains("derive(serde::Serialize)"),
        "type_attribute should appear on enum: {content}"
    );
}

#[test]
fn test_type_attribute_scoped_to_specific_type() {
    let mut file = proto3_file("multi.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Targeted".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Other".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let config = attr_config(
        vec![(".pkg.Targeted", "#[derive(serde::Serialize)]")],
        vec![],
        vec![],
    );
    let files = generate(&[file], &["multi.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    // Attribute should only appear in the Targeted region, not near Other.
    let targeted_pos = content
        .find("pub struct Targeted")
        .expect("Targeted struct");
    let other_pos = content.find("pub struct Other").expect("Other struct");
    // prettyplease renders the attribute on its own line above the struct.
    assert!(
        content[..targeted_pos].contains("derive(serde::Serialize)"),
        "Targeted should have the derive: {content}"
    );
    assert!(
        !content[other_pos..].contains("derive(serde::Serialize)"),
        "Other should not have the derive: {content}"
    );
}

// ── message_attribute tests ─────────────────────────────────────────

#[test]
fn test_message_attribute_on_struct_not_enum() {
    let mut file = proto3_file("mixed.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![enum_value("UNKNOWN", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    });
    let config = attr_config(vec![], vec![], vec![(".", "#[serde(default)]")]);
    let files = generate(&[file], &["mixed.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    // Exactly one occurrence: on the struct, not the enum.
    let total = content.matches("serde(default)").count();
    assert_eq!(
        total, 1,
        "serde(default) should appear once (struct only), found {total}: {content}"
    );
    // It should appear between the enum and the struct def (enums come first).
    let enum_pos = content.find("pub enum Status").expect("Status enum");
    let attr_pos = content.find("serde(default)").unwrap();
    let struct_pos = content.find("pub struct Msg").expect("Msg struct");
    assert!(
        attr_pos > enum_pos && attr_pos < struct_pos,
        "serde(default) should appear after enum, before struct: {content}"
    );
}

// ── field_attribute tests ───────────────────────────────────────────

#[test]
fn test_field_attribute_on_specific_field() {
    let mut file = proto3_file("fields.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            make_field("public_name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("secret_key", 2, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
        ],
        ..Default::default()
    });
    let config = attr_config(
        vec![],
        vec![(".pkg.Msg.secret_key", "#[serde(skip)]")],
        vec![],
    );
    let files = generate(&[file], &["fields.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    // Exactly one occurrence, and it must be near secret_key not public_name.
    let total = content.matches("serde(skip)").count();
    assert_eq!(
        total, 1,
        "serde(skip) should appear exactly once: {content}"
    );
    let attr_pos = content.find("serde(skip)").unwrap();
    let secret_pos = content.find("pub secret_key").expect("secret_key field");
    let public_pos = content.find("pub public_name").expect("public_name field");
    assert!(
        attr_pos > public_pos && attr_pos < secret_pos,
        "serde(skip) should appear after public_name, before secret_key: {content}"
    );
}

#[test]
fn test_field_attribute_catchall() {
    let mut file = proto3_file("allfields.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            make_field("a", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
            make_field("b", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    });
    // "." applies to all fields.
    let config = attr_config(vec![], vec![(".", "#[doc = \"custom\"]")], vec![]);
    let files =
        generate(&[file], &["allfields.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    // Both fields should have the attribute.
    let count = content.matches("custom").count();
    assert!(
        count >= 2,
        "catch-all field_attribute should appear on all fields, found {count}: {content}"
    );
}

// ── oneof coverage ──────────────────────────────────────────────────

fn oneof_message(name: &str, oneof_name: &str, variant_names: &[&str]) -> DescriptorProto {
    let mut fields = Vec::new();
    for (i, v) in variant_names.iter().enumerate() {
        let mut f = make_field(v, (i + 1) as i32, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
        f.oneof_index = Some(0);
        fields.push(f);
    }
    DescriptorProto {
        name: Some(name.to_string()),
        field: fields,
        oneof_decl: vec![OneofDescriptorProto {
            name: Some(oneof_name.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn test_type_attribute_reaches_oneof_enum() {
    let mut file = proto3_file("oo.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("Msg", "payload", &["a", "b"]));
    // Target the oneof enum by its fully-qualified proto path.
    let config = attr_config(
        vec![(".pkg.Msg.payload", "#[derive(Hash)]")],
        vec![],
        vec![],
    );
    let files = generate(&[file], &["oo.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    assert!(
        content.contains("#[derive(Hash)]"),
        "type_attribute should reach oneof enum: {content}"
    );
}

#[test]
fn test_field_attribute_reaches_oneof_variant() {
    let mut file = proto3_file("oo.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("Msg", "payload", &["a", "b"]));
    // Target variant `a` only.
    let config = attr_config(
        vec![],
        vec![(".pkg.Msg.payload.a", "#[doc = \"only_a\"]")],
        vec![],
    );
    let files = generate(&[file], &["oo.proto".to_string()], &config).expect("should generate");
    let content = &files[0].content;
    assert!(
        content.contains("only_a"),
        "field_attribute should reach oneof variant: {content}"
    );
    assert_eq!(
        content.matches("only_a").count(),
        1,
        "exactly one variant matched"
    );
}

// ── malformed attributes fail loudly ────────────────────────────────

#[test]
fn test_invalid_attribute_produces_error() {
    let mut file = proto3_file("bad.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let config = attr_config(vec![(".", "not a valid #[attribute")], vec![], vec![]);
    let err = generate(&[file], &["bad.proto".to_string()], &config)
        .expect_err("malformed attribute should error");
    let msg = err.to_string();
    assert!(
        msg.contains("invalid custom attribute"),
        "error should mention invalid custom attribute: {msg}"
    );
    assert!(
        msg.contains("not a valid #[attribute"),
        "error should include the offending string: {msg}"
    );
}

// ── no attributes when config is empty ──────────────────────────────

#[test]
fn test_no_custom_attributes_by_default() {
    let mut file = proto3_file("plain.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["plain.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &files[0].content;
    // No custom derives beyond the standard set.
    assert!(
        !content.contains("serde"),
        "no serde attrs without custom config: {content}"
    );
}

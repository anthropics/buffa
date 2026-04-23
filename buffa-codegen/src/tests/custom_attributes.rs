//! Custom type/field/message attribute injection into generated code.

use super::*;

// ── type_attribute tests ────────────────────────────────────────────

fn attr_config(
    type_attrs: Vec<(&str, &str)>,
    field_attrs: Vec<(&str, &str)>,
    message_attrs: Vec<(&str, &str)>,
) -> CodeGenConfig {
    attr_config_full(type_attrs, field_attrs, message_attrs, vec![])
}

fn attr_config_full(
    type_attrs: Vec<(&str, &str)>,
    field_attrs: Vec<(&str, &str)>,
    message_attrs: Vec<(&str, &str)>,
    enum_attrs: Vec<(&str, &str)>,
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
        enum_attributes: enum_attrs
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
    let content = &joined(&files);
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
    let content = &joined(&files);
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
    let content = &joined(&files);
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
    let content = &joined(&files);
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

// ── enum_attribute tests ────────────────────────────────────────────

fn mixed_msg_enum_file() -> FileDescriptorProto {
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
    file
}

#[test]
fn test_enum_attribute_on_enum_not_struct() {
    let file = mixed_msg_enum_file();
    let config = attr_config_full(vec![], vec![], vec![], vec![(".", "#[derive(Hash)]")]);
    let files = generate(&[file], &["mixed.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    // Hash already appears in the enum's built-in derive, so we use the
    // expanded `,` separator inside the user-supplied derive to avoid a
    // false-positive substring match. enum_attribute injects a *separate*
    // `#[derive(Hash)]` line after the built-in derive, so look for the
    // standalone form.
    assert!(
        content.contains("#[derive(Hash)]"),
        "enum_attribute should appear on the enum: {content}"
    );
    let enum_pos = content.find("pub enum Status").expect("Status enum");
    let attr_pos = content.find("#[derive(Hash)]").unwrap();
    let struct_pos = content.find("pub struct Msg").expect("Msg struct");
    // The injected attribute lands above the `pub enum`, not above the struct.
    assert!(
        attr_pos < enum_pos,
        "#[derive(Hash)] should sit above the enum: {content}"
    );
    assert!(
        attr_pos < struct_pos,
        "#[derive(Hash)] should not appear above the struct: {content}"
    );
}

#[test]
fn test_enum_attribute_scoped_to_specific_enum() {
    let mut file = proto3_file("two_enums.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Targeted".to_string()),
        value: vec![enum_value("A", 0)],
        ..Default::default()
    });
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Untouched".to_string()),
        value: vec![enum_value("B", 0)],
        ..Default::default()
    });
    let config = attr_config_full(
        vec![],
        vec![],
        vec![],
        vec![(".Targeted", "#[derive(Ord, PartialOrd)]")],
    );
    let files =
        generate(&[file], &["two_enums.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    let count = content.matches("derive(Ord, PartialOrd)").count();
    assert_eq!(
        count, 1,
        "attribute should land on Targeted only, found {count} matches: {content}"
    );
    // Verify the single match is associated with `Targeted`, not `Untouched`.
    let attr_pos = content.find("derive(Ord, PartialOrd)").unwrap();
    let targeted_pos = content.find("pub enum Targeted").expect("Targeted enum");
    let untouched_pos = content.find("pub enum Untouched").expect("Untouched enum");
    assert!(
        attr_pos < targeted_pos && attr_pos < untouched_pos,
        "attribute should sit above Targeted (and therefore not above Untouched): {content}"
    );
    assert!(
        targeted_pos < untouched_pos,
        "test relies on Targeted being emitted before Untouched"
    );
}

#[test]
fn test_enum_attribute_does_not_apply_to_struct() {
    let file = mixed_msg_enum_file();
    // Catch-all enum_attribute must not bleed onto messages.
    let config = attr_config_full(
        vec![],
        vec![],
        vec![],
        vec![(".", "#[doc = \"enum_only_marker\"]")],
    );
    let files = generate(&[file], &["mixed.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    let total = content.matches("enum_only_marker").count();
    assert_eq!(
        total, 1,
        "enum_only_marker must appear exactly once (on the enum): {content}"
    );
    let attr_pos = content.find("enum_only_marker").unwrap();
    let struct_pos = content.find("pub struct Msg").expect("Msg struct");
    assert!(
        attr_pos < struct_pos,
        "enum_attribute must not land on the struct: {content}"
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
    let content = &joined(&files);
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
    let content = &joined(&files);
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
    let content = &joined(&files);
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
    let content = &joined(&files);
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
    let content = &joined(&files);
    // No custom derives beyond the standard set.
    assert!(
        !content.contains("serde"),
        "no serde attrs without custom config: {content}"
    );
}

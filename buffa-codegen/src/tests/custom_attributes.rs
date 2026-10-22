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

/// A message with a plain `id` field ahead of one oneof, so an attribute on the
/// struct and an attribute on the struct's first field are different places
/// from the oneof field.
fn oneof_message(name: &str, oneof_name: &str, variant_names: &[&str]) -> DescriptorProto {
    let mut fields = vec![make_field(
        "id",
        100,
        Label::LABEL_OPTIONAL,
        Type::TYPE_INT32,
    )];
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

#[test]
fn test_oneof_attribute_on_oneof_not_message_or_enum() {
    let mut file = proto3_file("mix.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("Msg", "payload", &["a", "b"]));
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0)],
        ..Default::default()
    });
    let config = CodeGenConfig {
        generate_views: false,
        oneof_attributes: vec![(".".to_string(), "#[derive(serde::Serialize)]".to_string())],
        ..CodeGenConfig::default()
    };
    let files = generate(&[file], &["mix.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    // Appears exactly once: on the oneof enum, not on the message struct or the
    // plain enum.
    assert_eq!(
        content.matches("derive(serde::Serialize)").count(),
        1,
        "oneof_attribute should appear only on the oneof enum: {content}"
    );
}

// ── oneof_struct_field_attribute tests ──────────────────────────────

const MARKER: &str = "#[allow(clippy::pedantic)]";
const SECOND_MARKER: &str = "#[allow(clippy::nursery)]";

/// Every item of the generated files, including the items of inline modules.
fn all_items(files: &[GeneratedFile]) -> Vec<syn::Item> {
    fn walk(items: Vec<syn::Item>, out: &mut Vec<syn::Item>) {
        for item in items {
            if let syn::Item::Mod(module) = &item {
                if let Some((_, inner)) = &module.content {
                    walk(inner.clone(), out);
                }
            }
            out.push(item);
        }
    }
    let mut out = Vec::new();
    for f in files {
        let parsed = syn::parse_file(&f.content)
            .unwrap_or_else(|e| panic!("{} must parse: {e}\n{}", f.name, f.content));
        walk(parsed.items, &mut out);
    }
    out
}

/// The attributes as source text without whitespace, in declaration order.
fn rendered(attrs: &[syn::Attribute]) -> Vec<String> {
    use quote::ToTokens;
    attrs
        .iter()
        .map(|a| {
            let mut text = a.to_token_stream().to_string();
            text.retain(|c| !c.is_whitespace());
            text
        })
        .collect()
}

fn find_struct<'a>(items: &'a [syn::Item], name: &str) -> &'a syn::ItemStruct {
    let mut found = items.iter().filter_map(|item| match item {
        syn::Item::Struct(s) if s.ident == name => Some(s),
        _ => None,
    });
    let first = found
        .next()
        .unwrap_or_else(|| panic!("struct {name} is generated"));
    assert!(found.next().is_none(), "one struct is named {name}");
    first
}

/// The rendered attributes of the named field of `item`.
fn field_attrs(item: &syn::ItemStruct, field: &str) -> Vec<String> {
    let field = item
        .fields
        .iter()
        .find(|f| f.ident.as_ref().is_some_and(|ident| ident == field))
        .unwrap_or_else(|| panic!("{} has a field {field}", item.ident));
    rendered(&field.attrs)
}

/// Generated code as parsed items and as source text without whitespace.
struct Generated {
    items: Vec<syn::Item>,
    source: String,
}

/// Asserts that [`MARKER`] is on the field `oneof_field` of each struct in
/// `on_structs` and nowhere else in the generated code.
fn assert_marker_exactly_on_fields(generated: &Generated, oneof_field: &str, on_structs: &[&str]) {
    let has_marker = |attrs: &[syn::Attribute]| rendered(attrs).iter().any(|a| a == MARKER);
    for item in &generated.items {
        let syn::Item::Struct(s) = item else {
            continue;
        };
        for f in &s.fields {
            let name = f.ident.as_ref().map(ToString::to_string);
            let expected =
                name.as_deref() == Some(oneof_field) && on_structs.iter().any(|n| s.ident == n);
            assert_eq!(has_marker(&f.attrs), expected, "{}.{name:?}", s.ident);
        }
    }
    // The fields above account for every occurrence, so a struct, an enum, a
    // variant, an impl and a method are all unmarked.
    assert_eq!(
        generated.source.matches(MARKER).count(),
        on_structs.len(),
        "{}",
        generated.source
    );
}

fn oneof_field_config(rules: &[(&str, &str)]) -> CodeGenConfig {
    CodeGenConfig {
        oneof_struct_field_attributes: rules
            .iter()
            .map(|(p, a)| (p.to_string(), a.to_string()))
            .collect(),
        ..CodeGenConfig::default()
    }
}

/// `pkg.Msg` and `pkg.Outer.Inner`, each with an `id` field and a oneof
/// `payload`.
fn two_oneof_messages_file() -> FileDescriptorProto {
    let mut file = proto3_file("oneofs.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("Msg", "payload", &["a", "b"]));
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![oneof_message("Inner", "payload", &["a", "b"])],
        ..Default::default()
    });
    file
}

fn generate_items(file: FileDescriptorProto, config: &CodeGenConfig) -> Generated {
    let name = file.name.clone().unwrap_or_default();
    let files = generate(&[file], &[name], config).expect("should generate");
    let mut source = joined(&files);
    source.retain(|c| !c.is_whitespace());
    Generated {
        items: all_items(&files),
        source,
    }
}

#[test]
fn test_oneof_struct_field_attribute_on_oneof_field_only() {
    // Views are on: the view struct has a `payload` field too, and stays bare.
    let generated = generate_items(
        two_oneof_messages_file(),
        &oneof_field_config(&[(".pkg.Msg.payload", MARKER)]),
    );
    assert_eq!(
        field_attrs(find_struct(&generated.items, "Msg"), "payload"),
        [MARKER],
        "the oneof field carries exactly the custom attribute"
    );
    assert!(field_attrs(find_struct(&generated.items, "MsgView"), "payload").is_empty());
    assert_marker_exactly_on_fields(&generated, "payload", &["Msg"]);
}

/// With JSON on, buffa's own `serde(flatten)` comes first on the oneof field
/// and the custom attribute follows it.
#[test]
fn test_oneof_struct_field_attribute_nested_with_json_and_views() {
    let config = CodeGenConfig {
        generate_json: true,
        ..oneof_field_config(&[(".pkg.Outer.Inner.payload", MARKER)])
    };
    let generated = generate_items(two_oneof_messages_file(), &config);
    assert_eq!(
        field_attrs(find_struct(&generated.items, "Inner"), "payload"),
        ["#[serde(flatten)]", MARKER]
    );
    assert_eq!(
        field_attrs(find_struct(&generated.items, "Msg"), "payload"),
        ["#[serde(flatten)]"]
    );
    assert_marker_exactly_on_fields(&generated, "payload", &["Inner"]);
}

#[test]
fn test_oneof_struct_field_attribute_prefix_and_catch_all() {
    for rule in [".pkg", "."] {
        let generated = generate_items(
            two_oneof_messages_file(),
            &oneof_field_config(&[(rule, MARKER)]),
        );
        assert_marker_exactly_on_fields(&generated, "payload", &["Msg", "Inner"]);
    }
    // A prefix ends on a path segment, and a variant's path is longer than the
    // oneof's, so these rules match nothing.
    for rule in [".pkg.Msg.pay", ".pkg.Msg.payload.a", ".pk"] {
        let generated = generate_items(
            two_oneof_messages_file(),
            &oneof_field_config(&[(rule, MARKER)]),
        );
        assert_marker_exactly_on_fields(&generated, "payload", &[]);
    }
}

#[test]
fn test_oneof_struct_field_attribute_skips_synthetic_oneof() {
    // `optional string note` is a member of the synthetic oneof `_note`, which
    // has no struct field of its own.
    let mut msg = oneof_message("Msg", "payload", &["a", "b"]);
    let mut note = make_field("note", 50, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    note.oneof_index = Some(1);
    note.proto3_optional = Some(true);
    msg.field.push(note);
    msg.oneof_decl.push(OneofDescriptorProto {
        name: Some("_note".to_string()),
        ..Default::default()
    });
    let mut file = proto3_file("synthetic.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(msg);
    let generated = generate_items(file, &oneof_field_config(&[(".", MARKER)]));
    assert!(field_attrs(find_struct(&generated.items, "Msg"), "note")
        .iter()
        .all(|a| a != MARKER));
    assert_marker_exactly_on_fields(&generated, "payload", &["Msg"]);
}

#[test]
fn test_oneof_struct_field_attribute_follows_rename_note() {
    // The oneof `self` would be `self_`, which the field has, so the oneof
    // field is `self__` and carries a doc note about the rename.
    let mut msg = oneof_message("Msg", "self", &["a"]);
    msg.field[0].name = Some("self_".to_string());
    let mut file = proto3_file("renamed.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(msg);
    let generated = generate_items(file, &oneof_field_config(&[(".pkg.Msg.self", MARKER)]));
    let attrs = field_attrs(find_struct(&generated.items, "Msg"), "self__");
    assert_eq!(attrs.len(), 2, "{attrs:?}");
    assert!(
        attrs[0].starts_with("#[doc=") && attrs[0].contains("`self__`"),
        "the rename note comes first: {attrs:?}"
    );
    assert_eq!(attrs[1], MARKER);
}

#[test]
fn test_oneof_struct_field_attributes_accumulate_in_insertion_order() {
    for (rules, expected) in [
        (
            [(".", MARKER), (".pkg.Msg.payload", SECOND_MARKER)],
            [MARKER, SECOND_MARKER],
        ),
        (
            [(".pkg.Msg.payload", SECOND_MARKER), (".", MARKER)],
            [SECOND_MARKER, MARKER],
        ),
    ] {
        let generated = generate_items(two_oneof_messages_file(), &oneof_field_config(&rules));
        assert_eq!(
            field_attrs(find_struct(&generated.items, "Msg"), "payload"),
            expected
        );
    }
}

#[test]
fn test_oneof_struct_field_attribute_invalid_attribute_errors() {
    let file = two_oneof_messages_file();
    let config = oneof_field_config(&[(".pkg.Msg.payload", "not a valid #[attribute")]);
    let err = generate(&[file], &["oneofs.proto".to_string()], &config)
        .expect_err("malformed attribute should error");
    assert!(
        matches!(
            &err,
            CodeGenError::InvalidCustomAttribute { path, attribute, .. }
                if path == ".pkg.Msg.payload" && attribute == "not a valid #[attribute"
        ),
        "{err:?}"
    );
}

#[test]
fn test_oneof_struct_field_attribute_rejects_tokens_that_are_not_an_attribute() {
    // A struct field is valid in this position, so it used to be generated
    // as a second field of `Msg`.
    let file = two_oneof_messages_file();
    let config = oneof_field_config(&[(".pkg.Msg.payload", "pub extra: u32,")]);
    let err = generate(&[file], &["oneofs.proto".to_string()], &config)
        .expect_err("a field declaration is not an attribute");
    assert!(
        matches!(
            &err,
            CodeGenError::InvalidCustomAttribute { path, attribute, .. }
                if path == ".pkg.Msg.payload" && attribute == "pub extra: u32,"
        ),
        "{err:?}"
    );
}

#[test]
fn test_attribute_string_is_checked_when_its_path_matches_nothing() {
    // The rule is about the string, so a build does not start failing only
    // once a `.proto` gains an item the path matches.
    let file = two_oneof_messages_file();
    let rule = vec![(".pkg.NoSuchType".to_string(), String::new())];
    let configs = [
        CodeGenConfig {
            type_attributes: rule.clone(),
            ..CodeGenConfig::default()
        },
        CodeGenConfig {
            field_attributes: rule.clone(),
            ..CodeGenConfig::default()
        },
        CodeGenConfig {
            message_attributes: rule.clone(),
            ..CodeGenConfig::default()
        },
        CodeGenConfig {
            enum_attributes: rule.clone(),
            ..CodeGenConfig::default()
        },
        CodeGenConfig {
            oneof_attributes: rule.clone(),
            ..CodeGenConfig::default()
        },
        CodeGenConfig {
            oneof_struct_field_attributes: rule,
            ..CodeGenConfig::default()
        },
    ];
    for (i, config) in configs.iter().enumerate() {
        let err = generate(
            std::slice::from_ref(&file),
            &["oneofs.proto".to_string()],
            config,
        )
        .expect_err("an empty string is not an attribute");
        assert!(
            matches!(
                &err,
                CodeGenError::InvalidCustomAttribute { path, attribute, .. }
                    if path == ".pkg.NoSuchType" && attribute.is_empty()
            ),
            "list {i}: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "invalid custom attribute for path '.pkg.NoSuchType': '' must be one or more \
             outer `#[...]` attributes (expected at least one `#[...]` attribute)",
            "list {i}"
        );
    }
}

#[test]
fn test_oneof_attribute_specific_path_matches_one_oneof() {
    let mut file = proto3_file("two.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("First", "payload", &["a"]));
    file.message_type
        .push(oneof_message("Second", "other", &["b"]));
    let config = CodeGenConfig {
        generate_views: false,
        oneof_attributes: vec![(
            ".pkg.First.payload".to_string(),
            "#[derive(serde::Serialize)]".to_string(),
        )],
        ..CodeGenConfig::default()
    };
    let files = generate(&[file], &["two.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    assert_eq!(
        content.matches("derive(serde::Serialize)").count(),
        1,
        "exact-path rule should match only First.payload: {content}"
    );
    let enum_pos = content.find("pub enum Payload").expect("oneof enum");
    let attr_pos = content.find("derive(serde::Serialize)").expect("attr");
    assert!(
        attr_pos < enum_pos && enum_pos - attr_pos < 200,
        "attribute should sit on the Payload enum: {content}"
    );
}

#[test]
fn test_oneof_attribute_owned_enum_only_with_views() {
    // With views enabled, the attribute must land on the owned oneof enum
    // only — the view-of-oneof enum receives no custom attributes (the
    // documented family-wide scoping).
    let mut file = proto3_file("viewed.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("Msg", "payload", &["a", "b"]));
    let config = CodeGenConfig {
        generate_views: true,
        oneof_attributes: vec![(".".to_string(), "#[derive(serde::Serialize)]".to_string())],
        ..CodeGenConfig::default()
    };
    let files = generate(&[file], &["viewed.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    assert_eq!(
        content.matches("derive(serde::Serialize)").count(),
        1,
        "attribute must not leak onto the view oneof enum: {content}"
    );
}

#[test]
fn test_oneof_attribute_stacks_with_type_attribute() {
    let mut file = proto3_file("stack.proto");
    file.package = Some("pkg".to_string());
    file.message_type
        .push(oneof_message("Msg", "payload", &["a"]));
    let config = CodeGenConfig {
        generate_views: false,
        type_attributes: vec![(".".to_string(), "#[derive(Hash)]".to_string())],
        oneof_attributes: vec![(".".to_string(), "#[derive(serde::Serialize)]".to_string())],
        ..CodeGenConfig::default()
    };
    let files = generate(&[file], &["stack.proto".to_string()], &config).expect("should generate");
    let content = &joined(&files);
    // type_attribute hits the message struct and the oneof enum; the
    // oneof_attribute stacks on the enum alongside it.
    assert_eq!(content.matches("derive(Hash)").count(), 2, "{content}");
    assert_eq!(
        content.matches("derive(serde::Serialize)").count(),
        1,
        "{content}"
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

#[test]
fn test_unmatched_custom_attributes_warn_for_each_list_without_changing_output() {
    let file = two_oneof_messages_file();
    let names = ["oneofs.proto".to_string()];
    let baseline = generate(
        std::slice::from_ref(&file),
        &names,
        &CodeGenConfig::default(),
    )
    .expect("baseline generates");
    let rule = vec![(".pkg.Mesage.secret".to_string(), MARKER.to_string())];
    let config = CodeGenConfig {
        type_attributes: rule.clone(),
        message_attributes: rule.clone(),
        enum_attributes: rule.clone(),
        oneof_attributes: rule.clone(),
        field_attributes: rule.clone(),
        oneof_struct_field_attributes: rule,
        ..CodeGenConfig::default()
    };
    let (files, warnings) = generate_with_diagnostics(&[file], &names, &config).unwrap();
    assert_eq!(files.len(), baseline.len());
    for (actual, expected) in files.iter().zip(&baseline) {
        assert_eq!(actual.name, expected.name);
        assert_eq!(
            actual.content, expected.content,
            "inert rules must not change generated Rust"
        );
    }
    assert_eq!(warnings.len(), 6, "each list must warn: {warnings:?}");
    for (warning, option) in warnings.iter().zip([
        "type_attributes",
        "message_attributes",
        "enum_attributes",
        "oneof_attributes",
        "field_attributes",
        "oneof_struct_field_attributes",
    ]) {
        assert!(matches!(
            warning,
            CodeGenWarning::CustomAttributeRuleMatchedNothing { rule, option: actual }
                if *actual == option && rule == ".pkg.Mesage.secret"
        ));
        let text = warning.to_string();
        assert!(
            text.contains(option) && text.contains(".pkg.Mesage.secret"),
            "{text}"
        );
    }
}

fn custom_rule_config(option: &str, paths: &[&str]) -> CodeGenConfig {
    let mut config = CodeGenConfig::default();
    let rules = paths
        .iter()
        .map(|p| (p.to_string(), MARKER.to_string()))
        .collect();
    match option {
        "type_attributes" => config.type_attributes = rules,
        "message_attributes" => config.message_attributes = rules,
        "enum_attributes" => config.enum_attributes = rules,
        "oneof_attributes" => config.oneof_attributes = rules,
        "field_attributes" => config.field_attributes = rules,
        "oneof_struct_field_attributes" => config.oneof_struct_field_attributes = rules,
        _ => panic!("unknown list {option}"),
    }
    config
}

fn attribute_warning_texts(file: FileDescriptorProto, config: &CodeGenConfig) -> Vec<String> {
    let name = file.name.clone().unwrap();
    let (_, warnings) = generate_with_diagnostics(&[file], &[name], config).unwrap();
    warnings.into_iter().map(|w| w.to_string()).collect()
}

#[test]
fn test_custom_attribute_warnings_respect_each_lists_generated_target_kind() {
    let mut file = two_oneof_messages_file();
    file.message_type[1].nested_type[0]
        .enum_type
        .push(EnumDescriptorProto {
            name: Some("Status".to_string()),
            value: vec![enum_value("UNKNOWN", 0)],
            ..Default::default()
        });
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "type_attributes",
            &[
                ".",
                ".pkg",
                ".pkg.Msg",
                ".pkg.Outer.Inner",
                ".pkg.Outer.Inner.Status",
                ".pkg.Msg.payload",
            ],
            &[".pkg.Msg.id", ".pkg.Msg.payload.a", ".pkg.MsgView"],
        ),
        (
            "message_attributes",
            &[".", ".pkg", ".pkg.Msg", ".pkg.Outer.Inner"],
            &[".pkg.Outer.Inner.Status", ".pkg.Msg.payload", ".pkg.Msg.id"],
        ),
        (
            "enum_attributes",
            &[".", ".pkg", ".pkg.Outer.Inner.Status"],
            &[".pkg.Msg", ".pkg.Msg.payload", ".pkg.Msg.id"],
        ),
        (
            "oneof_attributes",
            &[".", ".pkg", ".pkg.Msg.payload", ".pkg.Outer.Inner.payload"],
            &[
                ".pkg.Msg.id",
                ".pkg.Msg.payload.a",
                ".pkg.Outer.Inner.Status",
            ],
        ),
        (
            "field_attributes",
            &[
                ".",
                ".pkg",
                ".pkg.Msg.id",
                ".pkg.Msg.payload.a",
                ".pkg.Outer.Inner.payload.b",
            ],
            &[
                ".pkg.Msg.a",
                ".pkg.Msg.pay",
                ".pkg.MsgView.id",
                ".pkg.Outer.Inner.Status.UNKNOWN",
            ],
        ),
        (
            "oneof_struct_field_attributes",
            &[".", ".pkg", ".pkg.Msg.payload", ".pkg.Outer.Inner.payload"],
            &[".pkg.Msg.id", ".pkg.Msg.payload.a", ".pkg.Msg.pay", ".pk"],
        ),
    ];
    for &(option, live, inert) in cases {
        let paths: Vec<_> = live.iter().chain(inert).copied().collect();
        let warnings = attribute_warning_texts(file.clone(), &custom_rule_config(option, &paths));
        assert_eq!(warnings.len(), inert.len(), "{option}: {warnings:?}");
        for (text, path) in warnings.iter().zip(inert) {
            assert!(text.contains(option) && text.contains(path), "{text}");
        }
    }
}

#[test]
fn test_custom_attribute_warnings_preserve_duplicate_entries_and_rule_order() {
    let config = custom_rule_config(
        "field_attributes",
        &[
            ".pkg.Nope.z",
            ".pkg.Msg.id",
            ".pkg.Nope.a",
            ".pkg.Nope.z",
            ".pkg.Msg.id",
        ],
    );
    let warnings = attribute_warning_texts(two_oneof_messages_file(), &config);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    for (text, path) in warnings
        .iter()
        .zip([".pkg.Nope.z", ".pkg.Nope.a", ".pkg.Nope.z"])
    {
        assert!(text.contains(path), "{text}");
    }
}

#[test]
fn test_custom_attribute_rules_match_across_generated_files_and_import_collection() {
    let first = two_oneof_messages_file();
    let mut second = proto3_file("second.proto");
    second.package = Some("other".to_string());
    second
        .message_type
        .push(oneof_message("Second", "payload", &["a"]));
    let names = ["oneofs.proto".to_string(), "second.proto".to_string()];
    for idiomatic in [false, true] {
        let config = CodeGenConfig {
            idiomatic_imports: idiomatic,
            file_per_package: idiomatic,
            ..custom_rule_config(
                "field_attributes",
                &[".other.Second.payload.a", ".pkg.Msg.id", "."],
            )
        };
        let (_, warnings) =
            generate_with_diagnostics(&[first.clone(), second.clone()], &names, &config).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}

#[test]
fn test_custom_attribute_rules_do_not_match_imported_excluded_or_extern_types() {
    let kept = two_oneof_messages_file();
    let mut dep = kept.clone();
    dep.name = Some("dep.proto".to_string());
    dep.package = Some("dep".to_string());
    dep.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![enum_value("UNKNOWN", 0)],
        ..Default::default()
    });
    for option in [
        "type_attributes",
        "message_attributes",
        "enum_attributes",
        "oneof_attributes",
        "field_attributes",
        "oneof_struct_field_attributes",
    ] {
        for mode in ["imported", "excluded", "extern"] {
            let mut config = custom_rule_config(option, &[".dep"]);
            let mut names = vec!["oneofs.proto".to_string()];
            if mode == "excluded" {
                names.push("dep.proto".to_string());
            }
            if mode == "excluded" {
                config.exclude_packages.push("dep".to_string());
            }
            if mode == "extern" {
                config
                    .extern_paths
                    .push((".dep".to_string(), "::foreign".to_string()));
            }
            let (_, warnings) =
                generate_with_diagnostics(&[kept.clone(), dep.clone()], &names, &config).unwrap();
            assert_eq!(warnings.len(), 1, "{option} {mode}: {warnings:?}");
            let text = warnings[0].to_string();
            assert!(text.contains(option) && text.contains(".dep"), "{text}");
        }
    }
}

#[test]
fn test_custom_attribute_rules_do_not_match_synthetic_oneofs_or_map_entries() {
    let mut file = two_oneof_messages_file();
    let msg = &mut file.message_type[0];
    msg.oneof_decl.push(OneofDescriptorProto {
        name: Some("_note".to_string()),
        ..Default::default()
    });
    let mut note = make_field("note", 50, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    note.oneof_index = Some(1);
    note.proto3_optional = Some(true);
    msg.field.push(note);
    msg.nested_type.push(DescriptorProto {
        name: Some("EntriesEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    });
    msg.field.push(FieldDescriptorProto {
        type_name: Some(".pkg.Msg.EntriesEntry".to_string()),
        ..make_field("entries", 51, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
    });
    for (option, paths) in [
        (
            "type_attributes",
            vec![".pkg.Msg.EntriesEntry", ".pkg.Msg._note"],
        ),
        ("message_attributes", vec![".pkg.Msg.EntriesEntry"]),
        ("oneof_attributes", vec![".pkg.Msg._note"]),
        ("oneof_struct_field_attributes", vec![".pkg.Msg._note"]),
        (
            "field_attributes",
            vec![".pkg.Msg.EntriesEntry.key", ".pkg.Msg._note.note"],
        ),
    ] {
        let warnings = attribute_warning_texts(file.clone(), &custom_rule_config(option, &paths));
        assert_eq!(warnings.len(), paths.len(), "{option}: {warnings:?}");
    }
    assert!(attribute_warning_texts(
        file,
        &custom_rule_config("field_attributes", &[".pkg.Msg.note", ".pkg.Msg.entries"])
    )
    .is_empty());
}

#[test]
fn test_explicitly_generated_extern_mapped_types_count_as_attribute_matches() {
    // extern_path changes reference resolution, not requested-file emission.
    let config = CodeGenConfig {
        extern_paths: vec![(".pkg".to_string(), "::foreign".to_string())],
        ..custom_rule_config("message_attributes", &[".pkg.Msg"])
    };
    let (files, warnings) = generate_with_diagnostics(
        &[two_oneof_messages_file()],
        &["oneofs.proto".to_string()],
        &config,
    )
    .unwrap();
    assert!(joined(&files).contains("clippy::pedantic"));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn test_custom_attribute_root_rule_warns_when_no_targets_are_generated() {
    for option in [
        "type_attributes",
        "message_attributes",
        "enum_attributes",
        "oneof_attributes",
        "field_attributes",
        "oneof_struct_field_attributes",
    ] {
        let config = custom_rule_config(option, &["."]);
        let (files, warnings) = generate_with_diagnostics(&[], &[], &config).unwrap();
        assert!(files.is_empty());
        assert_eq!(warnings.len(), 1, "{option}: {warnings:?}");
        assert!(warnings[0].to_string().contains(option));
    }
}

//! Proto2 codegen: optional → Option<T>, required always-encoded,
//! unpacked repeated, closed enums.

use super::*;

// ── Proto2 tests ─────────────────────────────────────────────────────

fn proto2_file(name: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.to_string()),
        syntax: Some("proto2".to_string()),
        ..Default::default()
    }
}

#[test]
fn test_proto2_optional_scalar_is_option() {
    let mut file = proto2_file("p2opt.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "count",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2opt.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 optional scalar should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub count: Option<i32>"),
        "proto2 optional int32 must be Option<i32>: {content}"
    );
}

#[test]
fn test_proto2_required_scalar_is_bare_type_and_always_encoded() {
    let mut file = proto2_file("p2req.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "count",
            1,
            Label::LABEL_REQUIRED,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2req.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 required scalar should generate");
    let content = &joined(&files);
    // Required fields use the bare type, not Option<T>.
    assert!(
        content.contains("pub count: i32"),
        "proto2 required int32 must be bare i32: {content}"
    );
    // Required fields must always be encoded; zero-default suppression must
    // not appear for this field.
    assert!(
        !content.contains("self.count != 0"),
        "proto2 required field must not have zero-default guard: {content}"
    );
}

#[test]
fn test_proto2_repeated_scalar_is_unpacked_by_default() {
    let mut file = proto2_file("p2rep.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![make_field(
            "ids",
            1,
            Label::LABEL_REPEATED,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2rep.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 repeated scalar should generate");
    let content = &joined(&files);
    // Unpacked write_to: no packed-payload size accumulation.
    // (The view decode arm uses `let payload = borrow_bytes(...)` for its
    // lenient packed-accept path, so we look for the typed accumulator
    // `let payload: u32` that appears only in the packed write_to path.)
    assert!(
        !content.contains("let payload: u32"),
        "proto2 repeated scalar must be unpacked by default: {content}"
    );
    // Each element gets its own tag in write_to.
    assert!(
        content.contains("encode_int32"),
        "missing encode_int32 in unpacked write_to: {content}"
    );
}

#[test]
fn test_proto2_optional_enum_is_option_enum_value() {
    let mut file = proto2_file("p2enum.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("color".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Color".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2enum.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 optional enum should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub color: Option<Color>"),
        "proto2 optional enum must be Option<Color> (closed enum): {content}"
    );
}

#[test]
fn test_proto2_enum_default_is_first_declared_variant() {
    // Enum with a zero-valued variant that is NOT listed first.  Proto2 default
    // is the first declared value regardless of its number; proto3 would prefer
    // the zero-valued one.
    let mut file = proto2_file("p2enumdef.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Priority".to_string()),
        value: vec![
            enum_value("HIGH", 1), // first declared, non-zero
            enum_value("NONE", 0), // zero-valued but listed second
            enum_value("LOW", 3),
        ],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["p2enumdef.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto2 enum default should generate");
    let content = &joined(&files);
    // Default impl must use HIGH (first declared), not NONE (zero-valued).
    assert!(
        content.contains("impl ::core::default::Default for Priority"),
        "missing Default impl: {content}"
    );
    // The default() body must reference HIGH, not NONE.
    let default_pos = content.find("fn default()").expect("missing fn default()");
    let after_default = &content[default_pos..default_pos + 80];
    assert!(
        after_default.contains("HIGH"),
        "proto2 enum Default must be first variant (HIGH), got: {after_default}"
    );
    assert!(
        !after_default.contains("NONE"),
        "proto2 enum Default must not be zero variant (NONE): {after_default}"
    );
}

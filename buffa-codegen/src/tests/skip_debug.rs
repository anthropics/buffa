//! `skip_debug`: suppress the owned-message `Debug` impl for selected proto paths.

use super::*;

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn skip_debug_file() -> FileDescriptorProto {
    let nested = DescriptorProto {
        name: Some("Nested".to_string()),
        field: vec![make_field(
            "value",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    };
    let outer = DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![make_field(
            "name",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        nested_type: vec![nested],
        ..Default::default()
    };
    let other = DescriptorProto {
        name: Some("Other".to_string()),
        field: vec![make_field(
            "name",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    };

    let mut file = proto3_file("skip_debug.proto");
    file.package = Some("skip.test".to_string());
    file.message_type = vec![outer, other];
    file
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

#[test]
fn debug_is_generated_by_default() {
    let content = generate_squashed(&CodeGenConfig::default());
    assert!(content.contains("impl::core::fmt::DebugforOuter"));
    assert!(content.contains("impl::core::fmt::DebugforNested"));
    assert!(content.contains("impl::core::fmt::DebugforOther"));
}

#[test]
fn skip_debug_matches_proto_prefixes_and_leaves_views_alone() {
    let config = CodeGenConfig {
        skip_debug: vec![".skip.test.Outer".to_string()],
        ..Default::default()
    };
    let content = generate_squashed(&config);

    assert!(!content.contains("impl::core::fmt::DebugforOuter"));
    assert!(!content.contains("impl::core::fmt::DebugforNested"));
    assert!(content.contains("impl::core::fmt::DebugforOther"));
    assert!(
        content.contains("#[derive(Clone,Debug,Default)]pubstructOuterView"),
        "skip_debug must not change view Debug generation: {content}"
    );
}

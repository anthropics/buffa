//! Tests for source code comment propagation into generated Rust code.

use super::*;
use crate::generated::descriptor::{source_code_info::Location, SourceCodeInfo};

fn make_location(path: Vec<i32>, leading: &str) -> Location {
    Location {
        path,
        leading_comments: Some(leading.to_string()),
        ..Default::default()
    }
}

#[test]
fn test_message_comment_in_generated_code() {
    let mut file = proto3_file("commented.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Person".to_string()),
        field: vec![make_field(
            "name",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    // Path [4, 0] = FileDescriptorProto.message_type[0]
    let mut sci = SourceCodeInfo::default();
    sci.location.push(make_location(
        vec![4, 0],
        " Represents a person in the system.\n",
    ));
    file.source_code_info = sci.into();

    let result = generate(
        &[file],
        &["commented.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation should succeed");

    let content = &joined(&result);
    assert!(
        content.contains("Represents a person in the system."),
        "message doc comment should appear in generated code, got:\n{content}"
    );
}

#[test]
fn test_field_comment_in_generated_code() {
    let mut file = proto3_file("field_comment.proto");
    file.message_type.push(DescriptorProto {
        name: Some("User".to_string()),
        field: vec![make_field(
            "email",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    // Path [4, 0, 2, 0] = message_type[0].field[0]
    let mut sci = SourceCodeInfo::default();
    sci.location.push(make_location(
        vec![4, 0, 2, 0],
        " The user's email address.\n",
    ));
    file.source_code_info = sci.into();

    let result = generate(
        &[file],
        &["field_comment.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation should succeed");

    let content = &joined(&result);
    assert!(
        content.contains("The user's email address."),
        "field doc comment should appear in generated code, got:\n{content}"
    );
}

#[test]
fn test_enum_comment_in_generated_code() {
    let mut file = proto3_file("enum_comment.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("UNSPECIFIED", 0), enum_value("RED", 1)],
        ..Default::default()
    });
    let mut sci = SourceCodeInfo::default();
    // Path [5, 0] = enum_type[0]
    sci.location
        .push(make_location(vec![5, 0], " Available colors.\n"));
    // Path [5, 0, 2, 1] = enum_type[0].value[1] (RED)
    sci.location
        .push(make_location(vec![5, 0, 2, 1], " The color red.\n"));
    file.source_code_info = sci.into();

    let result = generate(
        &[file],
        &["enum_comment.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation should succeed");

    let content = &joined(&result);
    assert!(
        content.contains("Available colors."),
        "enum doc comment should appear, got:\n{content}"
    );
    assert!(
        content.contains("The color red."),
        "enum value doc comment should appear, got:\n{content}"
    );
}

#[test]
fn test_oneof_comment_in_generated_code() {
    let mut file = proto3_file("oneof_comment.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Event".to_string()),
        field: vec![
            {
                let mut f = make_field("text", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
                f.oneof_index = Some(0);
                f
            },
            {
                let mut f = make_field("number", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
                f.oneof_index = Some(0);
                f
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("payload".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let mut sci = SourceCodeInfo::default();
    // Path [4, 0, 8, 0] = message_type[0].oneof_decl[0]
    sci.location.push(make_location(
        vec![4, 0, 8, 0],
        " The event payload variant.\n",
    ));
    file.source_code_info = sci.into();

    let result = generate(
        &[file],
        &["oneof_comment.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generation should succeed");

    let content = &joined(&result);
    assert!(
        content.contains("The event payload variant."),
        "oneof doc comment should appear, got:\n{content}"
    );
}

#[test]
fn test_no_source_code_info_still_generates() {
    // Ensure we don't crash when source_code_info is absent
    let mut file = proto3_file("no_sci.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Empty".to_string()),
        ..Default::default()
    });
    // No source_code_info set

    let result = generate(
        &[file],
        &["no_sci.proto".to_string()],
        &CodeGenConfig::default(),
    );
    assert!(result.is_ok(), "should generate without source_code_info");
}

#[test]
fn test_view_gets_same_comment_as_message() {
    let mut file = proto3_file("view_comment.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Greeter".to_string()),
        field: vec![make_field(
            "name",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    let mut sci = SourceCodeInfo::default();
    sci.location
        .push(make_location(vec![4, 0], " A greeter message.\n"));
    file.source_code_info = sci.into();

    let config = CodeGenConfig {
        generate_views: true,
        ..Default::default()
    };
    let result = generate(&[file], &["view_comment.proto".to_string()], &config)
        .expect("generation should succeed");

    let content = &joined(&result);
    // The comment should appear on both the owned struct and the view struct
    let count = content.matches("A greeter message.").count();
    assert!(
        count >= 2,
        "comment should appear on both Greeter and GreeterView, found {count} occurrence(s)"
    );
}

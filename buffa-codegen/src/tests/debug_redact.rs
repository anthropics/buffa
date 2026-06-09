//! `[debug_redact = true]`: generated `Debug` impls print a placeholder
//! instead of the annotated field's value — owned messages, owned oneofs,
//! views, and view-oneofs.

use super::*;
use crate::generated::descriptor::FieldOptions;

/// Strip all whitespace so assertions are immune to prettyplease line wrapping.
fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn redacted(mut field: FieldDescriptorProto) -> FieldDescriptorProto {
    field.options = FieldOptions {
        debug_redact: Some(true),
        ..Default::default()
    }
    .into();
    field
}

/// `Credentials` with a redacted singular field, an unannotated singular
/// field, and a oneof mixing a redacted and an unannotated variant.
fn redact_file() -> FileDescriptorProto {
    let mut credentials = DescriptorProto {
        name: Some("Credentials".to_string()),
        field: vec![
            redacted(make_field(
                "api_key",
                1,
                Label::LABEL_OPTIONAL,
                Type::TYPE_STRING,
            )),
            make_field("org_id", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    };
    let mut access_token = redacted(make_field(
        "access_token",
        3,
        Label::LABEL_OPTIONAL,
        Type::TYPE_STRING,
    ));
    access_token.oneof_index = Some(0);
    let mut token_uuid = make_field("token_uuid", 4, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    token_uuid.oneof_index = Some(0);
    credentials.field.push(access_token);
    credentials.field.push(token_uuid);
    credentials.oneof_decl.push(OneofDescriptorProto {
        name: Some("lookup".to_string()),
        ..Default::default()
    });

    let mut file = proto3_file("debug_redact.proto");
    file.package = Some("redact.test".to_string());
    file.message_type.push(credentials);
    file
}

fn generate_squashed(config: &CodeGenConfig) -> String {
    let files = generate(
        &[redact_file()],
        &["debug_redact.proto".to_string()],
        config,
    )
    .expect("should generate");
    squash(&joined(&files))
}

#[test]
fn message_debug_redacts_annotated_field_only() {
    let content = generate_squashed(&CodeGenConfig::default());
    assert!(
        content.contains(r#""api_key",&::core::format_args!("[REDACTED]")"#),
        "redacted field must print the placeholder: {content}"
    );
    assert!(
        !content.contains(r#""api_key",&self.api_key"#),
        "redacted field must not print its value"
    );
    assert!(
        content.contains(r#""org_id",&self.org_id"#),
        "unannotated fields keep printing their value: {content}"
    );
}

#[test]
fn oneof_debug_redacts_annotated_variant_only() {
    let content = generate_squashed(&CodeGenConfig::default());
    assert!(
        content.contains(
            r#"Self::AccessToken(_)=>{f.debug_tuple("AccessToken").field(&::core::format_args!("[REDACTED]")).finish()}"#
        ),
        "redacted oneof variant must print the placeholder: {content}"
    );
    assert!(
        content.contains(
            r#"Self::TokenUuid(value)=>{f.debug_tuple("TokenUuid").field(value).finish()}"#
        ),
        "unannotated oneof variant keeps printing its payload: {content}"
    );
    assert!(
        !content.contains("#[derive(Clone,PartialEq,Debug)]pubenumLookup"),
        "oneof with a redacted variant must not derive Debug: {content}"
    );
}

#[test]
fn view_debug_redacts_annotated_field() {
    let config = CodeGenConfig {
        generate_views: true,
        ..Default::default()
    };
    let content = generate_squashed(&config);
    assert!(
        content.contains("impl<'a>::core::fmt::DebugforCredentialsView<'a>"),
        "view with a redacted field must get a manual Debug impl: {content}"
    );
    assert!(
        !content.contains(r#""api_key",&self.api_key"#),
        "view Debug must not print the redacted field's value"
    );
    assert!(
        content.contains(
            r#"Self::AccessToken(_)=>{f.debug_tuple("AccessToken").field(&::core::format_args!("[REDACTED]")).finish()}"#
        ),
        "redacted view-oneof variant must print the placeholder: {content}"
    );
}

#[test]
fn unannotated_message_keeps_derived_debug() {
    let mut msg = DescriptorProto {
        name: Some("Plain".to_string()),
        field: vec![make_field(
            "name",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    };
    let mut text = make_field("text", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    text.oneof_index = Some(0);
    msg.field.push(text);
    msg.oneof_decl.push(OneofDescriptorProto {
        name: Some("kind".to_string()),
        ..Default::default()
    });
    let mut file = proto3_file("plain.proto");
    file.package = Some("redact.plain".to_string());
    file.message_type.push(msg);

    let config = CodeGenConfig {
        generate_views: true,
        ..Default::default()
    };
    let files = generate(&[file], &["plain.proto".to_string()], &config).expect("should generate");
    let content = squash(&joined(&files));
    assert!(
        content.contains("#[derive(Clone,PartialEq,Debug)]pubenumKind"),
        "unannotated oneofs keep the Debug derive: {content}"
    );
    assert!(
        content.contains("#[derive(Clone,Debug,Default)]pubstructPlainView"),
        "unannotated views keep the Debug derive: {content}"
    );
    assert!(
        !content.contains("[REDACTED]"),
        "no placeholder may appear without an annotation: {content}"
    );
}

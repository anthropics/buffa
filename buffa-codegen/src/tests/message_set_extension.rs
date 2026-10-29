//! MessageSet extensions may use field numbers above `(1 << 29) - 1`.

use super::*;
use crate::generated::descriptor::descriptor_proto::ExtensionRange;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, MessageOptions};

const TYPE_ID: i32 = 1_000_000_000;

fn message_set_extension_file() -> FileDescriptorProto {
    let mut file = proto3_file("message-set-ext.proto");
    file.package = Some("mset.codegen".into());
    file.syntax = Some("proto2".into());
    file.message_type = vec![
        DescriptorProto {
            name: Some("Set".into()),
            extension_range: vec![ExtensionRange {
                start: Some(4),
                end: Some(i32::MAX),
                ..Default::default()
            }],
            options: MessageOptions {
                message_set_wire_format: Some(true),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        DescriptorProto {
            name: Some("Item".into()),
            ..Default::default()
        },
    ];
    file.extension = vec![FieldDescriptorProto {
        name: Some("ext".into()),
        number: Some(TYPE_ID),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(".mset.codegen.Item".into()),
        extendee: Some(".mset.codegen.Set".into()),
        ..Default::default()
    }];
    file
}

#[test]
fn message_set_extension_const_uses_large_field_number() {
    let file = message_set_extension_file();
    let config = CodeGenConfig {
        allow_message_set: true,
        ..Default::default()
    };
    let files = generate(&[file], &["message-set-ext.proto".into()], &config)
        .expect("MessageSet extension should generate");
    let content = joined(&files);
    assert!(
        content.contains(&format!("::buffa::Extension::new({TYPE_ID}")),
        "extension const must use the MessageSet type id: {content}"
    );
}

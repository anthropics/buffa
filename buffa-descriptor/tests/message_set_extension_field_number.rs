#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet, MessageOptions,
};
use buffa_descriptor::DescriptorPool;

const TYPE_ID: i32 = 1_000_000_000;

fn message_set_with_extension() -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("message-set-ext.proto".into()),
            package: Some("mset.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![
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
                    field: vec![FieldDescriptorProto {
                        name: Some("payload".into()),
                        number: Some(1),
                        label: Some(Label::LABEL_OPTIONAL),
                        r#type: Some(Type::TYPE_STRING),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            extension: vec![FieldDescriptorProto {
                name: Some("ext".into()),
                number: Some(TYPE_ID),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".mset.test.Item".into()),
                extendee: Some(".mset.test.Set".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn message_set_extension_above_max_field_number_links() {
    let set = message_set_with_extension();
    for pool in [
        DescriptorPool::new(set.clone()).unwrap(),
        DescriptorPool::decode(&set.encode_to_vec()).unwrap(),
    ] {
        let ext = pool
            .extension_by_name("mset.test.ext")
            .expect("extension should link");
        assert_eq!(ext.field().number(), TYPE_ID as u32);
        assert_eq!(pool.message(ext.extendee()).full_name(), "mset.test.Set");
    }
}

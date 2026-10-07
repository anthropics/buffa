//! FieldMask JSON behavior through the descriptor-driven WKT codec.

#![cfg(all(feature = "reflect", feature = "json"))]

use std::sync::Arc;

use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::reflect::{DynamicMessage, ReflectMessageMut, Value};
use buffa_descriptor::DescriptorPool;

fn pool() -> Arc<DescriptorPool> {
    Arc::new(
        DescriptorPool::new(FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("google/protobuf/field_mask.proto".into()),
                package: Some("google.protobuf".into()),
                syntax: Some("proto3".into()),
                message_type: vec![DescriptorProto {
                    name: Some("FieldMask".into()),
                    field: vec![FieldDescriptorProto {
                        name: Some("paths".into()),
                        number: Some(1),
                        label: Some(Label::LABEL_REPEATED),
                        r#type: Some(Type::TYPE_STRING),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        })
        .expect("pool builds from the FieldMask descriptor"),
    )
}

#[test]
fn reflective_field_mask_json_ignores_empty_paths() {
    let pool = pool();
    let index = pool.message_index("google.protobuf.FieldMask").unwrap();
    let parsed = DynamicMessage::from_json(Arc::clone(&pool), index, "\",fooBar,,\"")
        .expect("empty comma-separated paths are ignored");
    assert_eq!(
        parsed.field_by_number(1),
        Some(&Value::List(vec![Value::String("foo_bar".into()),]))
    );
    assert_eq!(parsed.to_json().unwrap(), "\"fooBar\"");

    let mut all_empty = DynamicMessage::new(Arc::clone(&pool), index);
    all_empty.set(
        pool.message_by_name("google.protobuf.FieldMask")
            .unwrap()
            .field(1)
            .unwrap(),
        Value::List(vec![
            Value::String(String::new()),
            Value::String(String::new()),
        ]),
    );
    assert_eq!(all_empty.to_json().unwrap(), "\"\"");
}

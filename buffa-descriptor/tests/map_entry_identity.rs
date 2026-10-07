//! Map-entry name collisions against a protoc-compiled descriptor set.
//! Regenerate from the repository root with:
//! ```sh
//! protoc -I buffa-descriptor/tests/protos --include_imports \
//!     --descriptor_set_out=buffa-descriptor/tests/protos/map_entry_identity.fds \
//!     map_entry_identity.proto
//! ```

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::Message;
use buffa_descriptor::generated::descriptor::{DescriptorProto, FileDescriptorSet};
use buffa_descriptor::reflect::{DynamicMessage, Value};
use buffa_descriptor::{DescriptorPool, FieldKind, ScalarType, SingularKind};

const FDS_BYTES: &[u8] = include_bytes!("protos/map_entry_identity.fds");

fn rewrite_type_names(message: &mut DescriptorProto, infer_types: bool, leading_dot: bool) {
    for field in &mut message.field {
        if let Some(name) = &mut field.type_name {
            if infer_types {
                field.r#type = None;
            }
            if !leading_dot {
                *name = name.trim_start_matches('.').to_owned();
            }
        }
    }
    for nested in &mut message.nested_type {
        rewrite_type_names(nested, infer_types, leading_dot);
    }
}

#[test]
fn repeated_messages_with_matching_map_entry_names_remain_lists() {
    for infer_types in [false, true] {
        for leading_dot in [false, true] {
            let mut set = FileDescriptorSet::decode(&mut &FDS_BYTES[..]).unwrap();
            for file in &mut set.file {
                for message in &mut file.message_type {
                    rewrite_type_names(message, infer_types, leading_dot);
                }
            }
            let pool = DescriptorPool::new(set).unwrap();
            for name in [
                "reflect.map_identity.Holder",
                "reflect.map_identity.Holder.Nested",
            ] {
                let holder = pool.message_by_name(name).unwrap();
                assert_eq!(
                    holder.field(1).unwrap().kind(),
                    FieldKind::Map {
                        key: ScalarType::String,
                        value: SingularKind::Scalar(ScalarType::Int32),
                    },
                    "{name}, infer_types={infer_types}, leading_dot={leading_dot}"
                );
                assert_eq!(
                    holder.field(2).unwrap().kind(),
                    FieldKind::List(SingularKind::Message(
                        pool.message_index("reflect.map_identity.Other.ItemsEntry")
                            .unwrap()
                    )),
                    "{name}, infer_types={infer_types}, leading_dot={leading_dot}"
                );
            }
            let holder = pool.message_by_name("reflect.map_identity.Holder").unwrap();
            assert_eq!(
                holder.field(3).unwrap().kind(),
                FieldKind::List(SingularKind::Message(
                    pool.message_index("reflect.map_identity.HolderExtra.ItemsEntry")
                        .unwrap()
                ))
            );
        }
    }
}

#[test]
fn repeated_messages_with_matching_map_entry_names_preserve_wire_data() {
    let pool = Arc::new(DescriptorPool::decode(FDS_BYTES).unwrap());
    let index = pool.message_index("reflect.map_identity.Holder").unwrap();
    let wire = b"\x12\x05\x08\x07\x12\x01a\x12\x05\x08\x08\x12\x01b";
    let message = DynamicMessage::decode(Arc::clone(&pool), index, wire).unwrap();
    let Some(Value::List(records)) = message.field_by_number(2) else {
        panic!("records must be a list of messages");
    };
    assert_eq!(records.len(), 2);
    for (record, (id, note)) in records.iter().zip([(7, "a"), (8, "b")]) {
        let Value::Message(record) = record else {
            panic!("record must be a message");
        };
        assert_eq!(record.field_by_number(1), Some(&Value::I32(id)));
        assert_eq!(record.field_by_number(2), Some(&Value::String(note.into())));
    }
    assert_eq!(message.encode_to_vec(), wire);
}

#[cfg(feature = "json")]
#[test]
fn repeated_messages_with_matching_map_entry_names_accept_json_arrays() {
    let pool = Arc::new(DescriptorPool::decode(FDS_BYTES).unwrap());
    let index = pool.message_index("reflect.map_identity.Holder").unwrap();
    let input = serde_json::json!({
        "items": {"key": 42},
        "records": [{"id": 7, "note": "a"}, {"id": 8, "note": "b"}],
        "prefixedRecords": [{"id": "first", "count": 3}, {"id": "first", "count": 4}],
        "nested": {"items": {"nested": 9}, "records": [{"id": 10, "note": "c"}]}
    });
    let message = DynamicMessage::from_json(Arc::clone(&pool), index, &input.to_string()).unwrap();
    let output: serde_json::Value = serde_json::from_str(&message.to_json().unwrap()).unwrap();
    assert_eq!(output, input);
    let decoded = DynamicMessage::decode(pool, index, &message.encode_to_vec()).unwrap();
    assert_eq!(decoded, message);
}

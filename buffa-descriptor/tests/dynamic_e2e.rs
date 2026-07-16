//! End-to-end tests for [`DynamicMessage`] encode/decode and the
//! [`ReflectMessage`] trait surface against a `protoc`-compiled
//! `FileDescriptorSet`.

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::DecodeError;
use buffa_descriptor::reflect::{
    DynamicMessage, MapKey, MapValue, ReflectError, ReflectMessage, ReflectMessageMut, Value,
};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

fn message_with_valid_nested() -> DynamicMessage {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let inner_md = p.message_by_name("reflect.test.Inner").unwrap();
    let containers_md = p.message_by_name("reflect.test.Containers").unwrap();

    let mut inner = DynamicMessage::new(Arc::clone(&p), inner_idx);
    inner.set(inner_md.field(1).unwrap(), Value::String("first".into()));
    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    msg.set(containers_md.field(5).unwrap(), Value::Message(inner));

    let bytes = msg.encode_to_vec();
    DynamicMessage::decode(Arc::clone(&p), containers_idx, &bytes).unwrap()
}

fn assert_nested_id(msg: &DynamicMessage, expected: &str) {
    let Some(Value::Message(nested)) = msg.field_by_number(5) else {
        panic!("nested message missing");
    };
    assert!(matches!(
        nested.field_by_number(1),
        Some(Value::String(value)) if value.as_str() == expected
    ));
}

fn nested_occurrence(inner: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    Tag::new(5, WireType::LengthDelimited).encode(&mut bytes);
    encode_varint(inner.len() as u64, &mut bytes);
    bytes.extend_from_slice(inner);
    bytes
}

fn message_with_valid_group() -> DynamicMessage {
    let p = pool();
    let idx = p.message_index("reflect.ext.Extendable").unwrap();
    let mut bytes = Vec::new();
    Tag::new(120, WireType::StartGroup).encode(&mut bytes);
    Tag::new(1, WireType::Varint).encode(&mut bytes);
    encode_varint(77, &mut bytes);
    Tag::new(120, WireType::EndGroup).encode(&mut bytes);
    DynamicMessage::decode(Arc::clone(&p), idx, &bytes).unwrap()
}

fn assert_group_value(msg: &DynamicMessage) {
    let Some(Value::Message(group)) = msg.field_by_number(120) else {
        panic!("group message missing");
    };
    assert_eq!(group.field_by_number(1), Some(&Value::I32(77)));
}

#[test]
fn dynamic_message_scalar_round_trip() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    let md = p.message_by_name("reflect.test.Scalars").unwrap();

    // Set every field through the descriptor-keyed API.
    msg.set(md.field(1).unwrap(), Value::F64(1.5));
    msg.set(md.field(2).unwrap(), Value::F32(2.5));
    msg.set(md.field(3).unwrap(), Value::I32(-3));
    msg.set(md.field(4).unwrap(), Value::I64(-4));
    msg.set(md.field(5).unwrap(), Value::U32(5));
    msg.set(md.field(6).unwrap(), Value::U64(6));
    msg.set(md.field(7).unwrap(), Value::I32(-7));
    msg.set(md.field(8).unwrap(), Value::I64(-8));
    msg.set(md.field(9).unwrap(), Value::U32(9));
    msg.set(md.field(10).unwrap(), Value::U64(10));
    msg.set(md.field(11).unwrap(), Value::I32(-11));
    msg.set(md.field(12).unwrap(), Value::I64(-12));
    msg.set(md.field(13).unwrap(), Value::Bool(true));
    msg.set(md.field(14).unwrap(), Value::String("hello".into()));
    msg.set(md.field(15).unwrap(), Value::Bytes(vec![1, 2, 3]));
    msg.set(md.field(16).unwrap(), Value::I32(99));

    let bytes = msg.encode_to_vec();
    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &bytes).unwrap();
    assert_eq!(msg, decoded);

    // Spot-check a few values.
    assert_eq!(decoded.field_by_number(3), Some(&Value::I32(-3)));
    assert_eq!(
        decoded.field_by_number(14),
        Some(&Value::String("hello".into()))
    );
    assert_eq!(decoded.field_by_number(16), Some(&Value::I32(99)));
}

#[test]
fn dynamic_message_containers_round_trip() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    let md = p.message_by_name("reflect.test.Containers").unwrap();

    // Repeated packed ints.
    msg.set(
        md.field(1).unwrap(),
        Value::List(vec![Value::I32(1), Value::I32(2), Value::I32(300)]),
    );

    // Repeated strings (unpacked).
    msg.set(
        md.field(2).unwrap(),
        Value::List(vec![Value::String("a".into()), Value::String("b".into())]),
    );

    // map<string, int32>.
    let mut tags = MapValue::new();
    tags.insert(MapKey::String("k1".into()), Value::I32(10));
    tags.insert(MapKey::String("k2".into()), Value::I32(20));
    msg.set(md.field(3).unwrap(), Value::Map(tags));

    // map<int32, Inner>.
    let inner_md = p.message_by_name("reflect.test.Inner").unwrap();
    let mut child = DynamicMessage::new(Arc::clone(&p), inner_idx);
    child.set(inner_md.field(1).unwrap(), Value::String("c1".into()));
    child.set(inner_md.field(2).unwrap(), Value::I32(42));
    let mut children = MapValue::new();
    children.insert(MapKey::I32(1), Value::Message(child.clone()));
    msg.set(md.field(4).unwrap(), Value::Map(children));

    // Nested singular message.
    msg.set(md.field(5).unwrap(), Value::Message(child));

    // Enum.
    msg.set(md.field(6).unwrap(), Value::EnumNumber(2));

    // Repeated enum (packed).
    msg.set(
        md.field(7).unwrap(),
        Value::List(vec![Value::EnumNumber(1), Value::EnumNumber(3)]),
    );

    // Round-trip.
    let bytes = msg.encode_to_vec();
    let decoded = DynamicMessage::decode(Arc::clone(&p), containers_idx, &bytes).unwrap();
    assert_eq!(msg, decoded);

    // The encoded length should match the actual bytes written.
    assert_eq!(msg.encoded_len(), bytes.len());
}

#[test]
fn merge_keeps_existing_nested_message_when_length_varint_is_truncated() {
    let mut msg = message_with_valid_nested();
    let mut malformed = Vec::new();
    Tag::new(5, WireType::LengthDelimited).encode(&mut malformed);
    malformed.push(0x80);

    assert_eq!(msg.merge(&malformed), Err(DecodeError::UnexpectedEof));
    assert_nested_id(&msg, "first");
}

#[test]
fn merge_keeps_existing_nested_message_when_declared_length_is_too_large() {
    let mut msg = message_with_valid_nested();
    let mut malformed = Vec::new();
    Tag::new(5, WireType::LengthDelimited).encode(&mut malformed);
    malformed.push(2);
    malformed.push(0x0a);

    assert_eq!(msg.merge(&malformed), Err(DecodeError::UnexpectedEof));
    assert_nested_id(&msg, "first");
}

#[cfg(target_pointer_width = "32")]
#[test]
fn merge_keeps_existing_nested_message_when_length_does_not_fit_usize() {
    let mut msg = message_with_valid_nested();
    let mut malformed = Vec::new();
    Tag::new(5, WireType::LengthDelimited).encode(&mut malformed);
    encode_varint(u64::MAX, &mut malformed);

    assert_eq!(msg.merge(&malformed), Err(DecodeError::MessageTooLarge));
    assert_nested_id(&msg, "first");
}

#[test]
fn merge_keeps_existing_nested_message_when_nested_payload_is_malformed() {
    let mut msg = message_with_valid_nested();
    let malformed = nested_occurrence(&[0x0a, 0x02, b'x']);

    assert_eq!(msg.merge(&malformed), Err(DecodeError::UnexpectedEof));
    assert_nested_id(&msg, "first");
}

#[test]
fn merge_keeps_existing_nested_message_when_wrong_wire_type_is_truncated() {
    let mut msg = message_with_valid_nested();
    let mut malformed = Vec::new();
    Tag::new(5, WireType::Varint).encode(&mut malformed);

    assert_eq!(msg.merge(&malformed), Err(DecodeError::UnexpectedEof));
    assert_nested_id(&msg, "first");
}

#[test]
fn merge_keeps_existing_group_when_group_payload_is_malformed() {
    let mut msg = message_with_valid_group();
    let mut malformed = Vec::new();
    Tag::new(120, WireType::StartGroup).encode(&mut malformed);
    Tag::new(1, WireType::Varint).encode(&mut malformed);

    assert_eq!(msg.merge(&malformed), Err(DecodeError::UnexpectedEof));
    assert_group_value(&msg);
}

#[test]
fn try_set_rejects_values_that_do_not_match_field_kind() {
    let p = pool();
    let scalars_idx = p.message_index("reflect.test.Scalars").unwrap();
    let scalars = p.message_by_name("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), scalars_idx);

    msg.set(scalars.field(3).unwrap(), Value::I32(7));
    let err = msg
        .try_set(scalars.field(3).unwrap(), Value::String("bad".into()))
        .unwrap_err();
    assert!(matches!(
        err,
        ReflectError::WrongValueKind {
            ref message,
            ref field_name,
            number,
            ref expected,
            ref actual,
        } if message == "reflect.test.Scalars"
            && field_name == "f_int32"
            && number == 3
            && expected == "int32"
            && actual == "string"
    ));
    assert_eq!(msg.field_by_number(3), Some(&Value::I32(7)));
}

#[test]
fn try_set_rejects_values_with_mismatched_container_contents() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let md = p.message_by_name("reflect.test.Containers").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);

    let err = msg
        .try_set(
            md.field(1).unwrap(),
            Value::List(vec![Value::I32(1), Value::String("bad".into())]),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        ReflectError::WrongValueKind {
            ref field_name,
            ref expected,
            ref actual,
            ..
        } if field_name == "packed_ints" && expected == "list<int32>" && actual == "list element string"
    ));

    let mut wrong_key = MapValue::new();
    wrong_key.insert(MapKey::I32(1), Value::I32(7));
    let err = msg
        .try_set(md.field(3).unwrap(), Value::Map(wrong_key))
        .unwrap_err();
    assert!(matches!(
        err,
        ReflectError::WrongValueKind {
            ref field_name,
            ref expected,
            ref actual,
            ..
        } if field_name == "tags" && expected == "map<string, int32>" && actual == "map key i32"
    ));

    let mut wrong_value = MapValue::new();
    wrong_value.insert(MapKey::String("k".into()), Value::String("bad".into()));
    let err = msg
        .try_set(md.field(3).unwrap(), Value::Map(wrong_value))
        .unwrap_err();
    assert!(matches!(
        err,
        ReflectError::WrongValueKind {
            ref field_name,
            ref expected,
            ref actual,
            ..
        } if field_name == "tags"
            && expected == "map<string, int32>"
            && actual == "map value string"
    ));
}

#[test]
fn try_set_rejects_message_values_with_wrong_descriptor() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let scalars_idx = p.message_index("reflect.test.Scalars").unwrap();
    let md = p.message_by_name("reflect.test.Containers").unwrap();

    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    let wrong_message = DynamicMessage::new(Arc::clone(&p), scalars_idx);
    let err = msg
        .try_set(md.field(5).unwrap(), Value::Message(wrong_message))
        .unwrap_err();
    assert!(matches!(
        err,
        ReflectError::WrongValueKind {
            ref field_name,
            ref expected,
            ref actual,
            ..
        } if field_name == "nested"
            && expected == "message reflect.test.Inner"
            && actual == "message reflect.test.Scalars"
    ));
}

#[test]
#[should_panic(expected = "expects int32, got string")]
fn set_panics_if_value_does_not_match_field_kind() {
    let p = pool();
    let scalars_idx = p.message_index("reflect.test.Scalars").unwrap();
    let scalars = p.message_by_name("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), scalars_idx);

    msg.set(scalars.field(3).unwrap(), Value::String("bad".into()));
}

#[test]
fn encode_skips_invalid_values_left_by_mutable_field_access() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let md = p.message_by_name("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);

    msg.set(md.field(3).unwrap(), Value::I32(7));
    *msg.field_by_number_mut(3).unwrap() = Value::String("bad".into());

    let bytes = msg.encode_to_vec();
    assert!(bytes.is_empty());
    assert_eq!(msg.encoded_len(), bytes.len());
}

#[test]
fn dynamic_message_unknown_fields_preserved() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    // Build wire bytes with a known field (int32 #3) and an unknown field
    // (#17, varint). Use buffa's own Tag encoder so the wire bytes are
    // correct by construction.
    use buffa::encoding::{Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(3, WireType::Varint).encode(&mut wire);
    wire.push(7u8); // f_int32 = 7
    Tag::new(17, WireType::Varint).encode(&mut wire);
    wire.push(0x05u8); // unknown field 17 = 5

    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &wire).unwrap();
    assert_eq!(decoded.field_by_number(3), Some(&Value::I32(7)));
    assert_eq!(decoded.unknown_fields().len(), 1);

    // Round-trip preserves the unknown field.
    let re_encoded = decoded.encode_to_vec();
    assert_eq!(re_encoded.len(), wire.len());
}

#[test]
fn reflect_message_get_has_for_each() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    let md = p.message_by_name("reflect.test.Scalars").unwrap();

    msg.set(md.field(3).unwrap(), Value::I32(42));
    msg.set(md.field(14).unwrap(), Value::String("abc".into()));

    // get returns the set value.
    let v = msg.get(md.field(3).unwrap());
    assert!(matches!(v, buffa_descriptor::reflect::ValueRef::I32(42)));

    // get on an absent field returns the default.
    let v = msg.get(md.field(13).unwrap());
    assert!(matches!(
        v,
        buffa_descriptor::reflect::ValueRef::Bool(false)
    ));

    // has reflects presence.
    assert!(msg.has(md.field(3).unwrap()));
    assert!(!msg.has(md.field(13).unwrap()));

    // for_each_set visits exactly the set fields.
    let mut seen = Vec::new();
    msg.for_each_set(&mut |fd, _| seen.push(fd.number()));
    seen.sort();
    assert_eq!(seen, vec![3, 14]);
}

#[test]
fn dynamic_message_empty_containers_have_returns_false() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    let md = p.message_by_name("reflect.test.Containers").unwrap();

    // Empty list and map — has() should be false, for_each_set should skip.
    msg.set(md.field(1).unwrap(), Value::List(Vec::new()));
    msg.set(md.field(3).unwrap(), Value::Map(MapValue::new()));

    assert!(!msg.has(md.field(1).unwrap()));
    assert!(!msg.has(md.field(3).unwrap()));

    let mut count = 0;
    msg.for_each_set(&mut |_, _| count += 1);
    assert_eq!(count, 0);
}

#[test]
fn which_oneof_resolves_set_member() {
    let p = pool();
    let oneof_idx = p.message_index("reflect.test.OneOf").unwrap();
    let md = p.message_by_name("reflect.test.OneOf").unwrap();
    let oneof = &md.oneofs()[0];

    // Empty message — no oneof member set.
    let empty = DynamicMessage::new(Arc::clone(&p), oneof_idx);
    assert!(empty.which_oneof(oneof).is_none());

    // Set one member.
    let mut msg = DynamicMessage::new(Arc::clone(&p), oneof_idx);
    msg.set(md.field(2).unwrap(), Value::String("hello".into()));
    let active = msg.which_oneof(oneof).expect("a member is set");
    assert_eq!(active.number(), 2);
    assert_eq!(active.name(), "text");

    // Switch to a different member — last write wins.
    msg.set(md.field(1).unwrap(), Value::I32(42));
    let active = msg.which_oneof(oneof).expect("a member is set");
    assert_eq!(active.number(), 1);
    assert_eq!(active.name(), "num");
}

#[test]
fn unknown_fields_reachable_through_dyn_reflect_message() {
    // The PII-interceptor case: a recursive walk over `&dyn ReflectMessage`
    // must be able to reach the unknown fields of *nested* messages, not
    // just the root. `unknown_fields()` is on the trait for exactly this.
    use buffa::{UnknownFieldData, UnknownFields};

    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let md = p.message_by_name("reflect.test.Containers").unwrap();

    // Build an Inner whose wire bytes carry a field its descriptor doesn't
    // declare (number 99, a string), then nest it in a Containers.
    let mut inner = DynamicMessage::new(Arc::clone(&p), inner_idx);
    inner.set(
        p.message(inner_idx).field(1).unwrap(),
        Value::String("known".into()),
    );
    let mut inner_bytes = inner.encode_to_vec();
    buffa::encoding::Tag::new(99, buffa::encoding::WireType::LengthDelimited)
        .encode(&mut inner_bytes);
    buffa::encoding::encode_varint(11, &mut inner_bytes);
    inner_bytes.extend_from_slice(b"555-12-3456");
    let inner_with_unknown =
        DynamicMessage::decode(Arc::clone(&p), inner_idx, &inner_bytes).unwrap();
    assert_eq!(inner_with_unknown.unknown_fields().len(), 1);

    let mut outer = DynamicMessage::new(Arc::clone(&p), containers_idx);
    outer.set(md.field(5).unwrap(), Value::Message(inner_with_unknown));

    // Walk through the trait object only — the way a generic interceptor
    // sees the message — and collect every length-delimited unknown payload
    // at any depth.
    fn collect_unknown_strings(msg: &dyn ReflectMessage, out: &mut Vec<String>) {
        for uf in msg.unknown_fields().iter() {
            if let UnknownFieldData::LengthDelimited(b) = &uf.data {
                if let Ok(s) = core::str::from_utf8(b) {
                    out.push(s.to_owned());
                }
            }
        }
        msg.for_each_set(&mut |_, v| {
            if let buffa_descriptor::reflect::ValueRef::Message(cow) = v {
                collect_unknown_strings(&*cow, out);
            }
        });
    }
    let mut found = Vec::new();
    collect_unknown_strings(&outer, &mut found);
    assert_eq!(found, vec!["555-12-3456".to_string()]);

    // The root itself has no unknown fields — only the nested Inner does —
    // so a non-recursive check would have missed the payload entirely.
    assert!(ReflectMessage::unknown_fields(&outer).is_empty());
    let _: &UnknownFields = outer.unknown_fields();
}

#[test]
fn field_mut_redacts_strings_at_any_depth() {
    // The mutating-interceptor use case: redact every string in a message
    // tree in place, through `&mut DynamicMessage`, without read-clone-set-back.
    use buffa_descriptor::FieldKind;

    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let cmd = p.message_by_name("reflect.test.Containers").unwrap();
    let imd = p.message_by_name("reflect.test.Inner").unwrap();

    // strings (field 2, repeated string), nested.id (5→1), inners[].id (8→1).
    let mut nested = DynamicMessage::new(Arc::clone(&p), inner_idx);
    nested.set(imd.field(1).unwrap(), Value::String("secret-nested".into()));
    let mut elem = DynamicMessage::new(Arc::clone(&p), inner_idx);
    elem.set(imd.field(1).unwrap(), Value::String("secret-elem".into()));

    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    msg.set(
        cmd.field(2).unwrap(),
        Value::List(vec![Value::String("secret-top".into())]),
    );
    msg.set(cmd.field(5).unwrap(), Value::Message(nested));
    msg.set(
        cmd.field(8).unwrap(),
        Value::List(vec![Value::Message(elem)]),
    );

    redact_strings(&mut msg);

    // The descriptor-keyed `field_mut` entry point also mutates in place.
    if let Some(Value::List(items)) = msg.field_mut(cmd.field(2).unwrap()) {
        items.push(Value::String("appended".into()));
    }

    // Top-level repeated string redacted.
    let Some(Value::List(strings)) = msg.field_by_number(2) else {
        panic!("strings missing");
    };
    assert_eq!(strings[0], Value::String("[REDACTED]".into()));
    assert_eq!(strings[1], Value::String("appended".into()));
    // Nested singular message's string redacted in place.
    let Some(Value::Message(n)) = msg.field_by_number(5) else {
        panic!("nested missing");
    };
    assert_eq!(
        n.field_by_number(1),
        Some(&Value::String("[REDACTED]".into()))
    );
    // Repeated message element's string redacted in place.
    let Some(Value::List(items)) = msg.field_by_number(8) else {
        panic!("inners missing");
    };
    let Value::Message(e) = &items[0] else {
        panic!("elem not a message");
    };
    assert_eq!(
        e.field_by_number(1),
        Some(&Value::String("[REDACTED]".into()))
    );

    // Recursive redactor: clone the Arc pool so the descriptor borrow is
    // independent of the `&mut DynamicMessage` borrow.
    fn redact_strings(msg: &mut DynamicMessage) {
        let pool = Arc::clone(msg.pool());
        let md = pool.message(msg.message_index());
        for fd in md.fields() {
            let Some(value) = msg.field_by_number_mut(fd.number()) else {
                continue;
            };
            match fd.kind() {
                FieldKind::Singular(_) => redact_value(value),
                FieldKind::List(_) => {
                    if let Value::List(items) = value {
                        for v in items {
                            redact_value(v);
                        }
                    }
                }
                FieldKind::Map { .. } => {}
            }
        }
    }

    fn redact_value(v: &mut Value) {
        match v {
            Value::String(s) => *s = "[REDACTED]".into(),
            Value::Message(inner) => redact_strings(inner),
            _ => {}
        }
    }
}

#[test]
fn debug_output_redacts_debug_redact_fields() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, FieldOptions, FileDescriptorProto, FileDescriptorSet,
    };

    // Hand-built descriptor (rather than the shared protoc-compiled .fds) so
    // the `[debug_redact = true]` option is exercised without regenerating the
    // checked-in descriptor set.
    let file = FileDescriptorProto {
        name: Some("redact.proto".into()),
        package: Some("redact.test".into()),
        syntax: Some("proto3".into()),
        message_type: vec![DescriptorProto {
            name: Some("Credentials".into()),
            field: vec![
                FieldDescriptorProto {
                    name: Some("api_key".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    options: FieldOptions {
                        debug_redact: Some(true),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                },
                FieldDescriptorProto {
                    name: Some("org_id".into()),
                    number: Some(2),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let p = Arc::new(
        DescriptorPool::new(FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        })
        .expect("pool builds from hand-built descriptor"),
    );

    let idx = p.message_index("redact.test.Credentials").unwrap();
    let md = p.message_by_name("redact.test.Credentials").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    msg.set(
        md.field(1).unwrap(),
        Value::String("sk-super-secret".into()),
    );
    msg.set(md.field(2).unwrap(), Value::String("org_123".into()));

    let out = format!("{msg:?}");
    assert!(
        !out.contains("sk-super-secret"),
        "redacted field leaked: {out}"
    );
    assert!(out.contains("[REDACTED]"), "placeholder missing: {out}");
    assert!(
        out.contains("org_123"),
        "unannotated field must still print: {out}"
    );
}

fn foreign_field_pool() -> Arc<DescriptorPool> {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let file = FileDescriptorProto {
        name: Some("foreign.proto".into()),
        package: Some("foreign.test".into()),
        syntax: Some("proto3".into()),
        message_type: vec![
            DescriptorProto {
                name: Some("Owner".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("owned".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    ..Default::default()
                }],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Foreign".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("alien".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    Arc::new(
        DescriptorPool::new(FileDescriptorSet {
            file: vec![file],
            ..Default::default()
        })
        .expect("pool builds from hand-built descriptor"),
    )
}

#[test]
fn try_set_and_try_clear_reject_foreign_field_descriptors() {
    let p = foreign_field_pool();
    let p2 = foreign_field_pool();
    let owner_idx = p.message_index("foreign.test.Owner").unwrap();
    let owner_md = p.message_by_name("foreign.test.Owner").unwrap();
    let owner_field = owner_md.field(1).unwrap();
    let foreign_same_pool = p
        .message_by_name("foreign.test.Foreign")
        .unwrap()
        .field(1)
        .unwrap();
    let foreign_other_pool = p2
        .message_by_name("foreign.test.Owner")
        .unwrap()
        .field(1)
        .unwrap();

    let mut msg = DynamicMessage::new(Arc::clone(&p), owner_idx);
    msg.try_set(owner_field, Value::String("kept".into()))
        .expect("owned field sets cleanly");

    let err = msg
        .try_set(foreign_same_pool, Value::String("wrong".into()))
        .unwrap_err();
    assert!(matches!(
        err,
        ReflectError::FieldNotMember {
            ref message,
            ref field_name,
            number,
        } if message == "foreign.test.Owner" && field_name == "alien" && number == 1
    ));
    assert_eq!(msg.field_by_number(1), Some(&Value::String("kept".into())));

    let err = msg.try_clear(foreign_other_pool).unwrap_err();
    assert!(matches!(
        err,
        ReflectError::FieldNotMember {
            ref message,
            ref field_name,
            number,
        } if message == "foreign.test.Owner" && field_name == "owned" && number == 1
    ));
    assert_eq!(msg.field_by_number(1), Some(&Value::String("kept".into())));

    msg.try_clear(owner_field)
        .expect("owned field clears cleanly");
    assert_eq!(msg.field_by_number(1), None);
}

#[test]
#[should_panic(expected = "is not a member of foreign.test.Owner")]
fn set_panics_on_foreign_field_descriptor() {
    let p = foreign_field_pool();
    let owner_idx = p.message_index("foreign.test.Owner").unwrap();
    let foreign = p
        .message_by_name("foreign.test.Foreign")
        .unwrap()
        .field(1)
        .unwrap();

    let mut msg = DynamicMessage::new(Arc::clone(&p), owner_idx);
    msg.set(foreign, Value::String("boom".into()));
}

// ── Cross-pool message values (issue #297) ─────────────────────────────────

/// A nested message built against a *different* pool instance of the same
/// schema is re-homed on `set`, not rejected — the adoption rule documented on
/// `ReflectMessageMut::try_set`. Two pools decoded from the same bytes stand in
/// for the cross-crate shape, where the nested type's pool is its defining
/// crate's.
#[test]
fn set_rehomes_singular_message_from_a_foreign_pool() {
    let parent_pool = pool();
    let foreign_pool = pool(); // same bytes, different Arc — the cross-crate shape

    let containers = parent_pool
        .message_by_name("reflect.test.Containers")
        .unwrap();
    let inner_idx = foreign_pool.message_index("reflect.test.Inner").unwrap();
    let foreign_inner_md = foreign_pool.message_by_name("reflect.test.Inner").unwrap();

    let mut foreign_inner = DynamicMessage::new(Arc::clone(&foreign_pool), inner_idx);
    foreign_inner.set(foreign_inner_md.field(2).unwrap(), Value::I32(7));

    let mut parent = DynamicMessage::new(
        Arc::clone(&parent_pool),
        parent_pool
            .message_index("reflect.test.Containers")
            .unwrap(),
    );
    parent
        .try_set(containers.field(5).unwrap(), Value::Message(foreign_inner))
        .expect("a same-schema message from another pool is re-homed, not rejected");

    // Stored value is now homed in the parent's pool, and survives a round-trip.
    let Some(Value::Message(stored)) = parent.field_by_number(5) else {
        panic!("nested field not set");
    };
    assert!(
        Arc::ptr_eq(stored.pool(), &parent_pool),
        "re-homed into parent pool"
    );
    let bytes = parent.encode_to_vec();
    let back = DynamicMessage::decode(
        Arc::clone(&parent_pool),
        parent_pool
            .message_index("reflect.test.Containers")
            .unwrap(),
        &bytes,
    )
    .unwrap();
    assert_eq!(back, parent);
}

/// The same re-homing applies inside repeated and map fields — a vtable walk
/// surfaces those elements through `ValueRef::to_owned` too.
#[test]
fn set_rehomes_message_elements_in_lists_and_maps() {
    let parent_pool = pool();
    let foreign_pool = pool();

    let containers = parent_pool
        .message_by_name("reflect.test.Containers")
        .unwrap();
    let inner_idx = foreign_pool.message_index("reflect.test.Inner").unwrap();
    let foreign_inner_md = foreign_pool.message_by_name("reflect.test.Inner").unwrap();

    let mut foreign_inner = DynamicMessage::new(Arc::clone(&foreign_pool), inner_idx);
    foreign_inner.set(foreign_inner_md.field(2).unwrap(), Value::I32(9));

    let mut parent = DynamicMessage::new(
        Arc::clone(&parent_pool),
        parent_pool
            .message_index("reflect.test.Containers")
            .unwrap(),
    );

    parent
        .try_set(
            containers.field(8).unwrap(),
            Value::List(vec![Value::Message(foreign_inner.clone())]),
        )
        .expect("foreign list element is re-homed");
    parent
        .try_set(
            containers.field(4).unwrap(),
            Value::Map(MapValue::from_entries(vec![(
                MapKey::I32(1),
                Value::Message(foreign_inner),
            )])),
        )
        .expect("foreign map value is re-homed");

    let bytes = parent.encode_to_vec();
    let back = DynamicMessage::decode(
        Arc::clone(&parent_pool),
        parent_pool
            .message_index("reflect.test.Containers")
            .unwrap(),
        &bytes,
    )
    .unwrap();
    assert_eq!(back, parent);
}

/// Re-homing is keyed on the message's full name: a *different* message type
/// from another pool is still rejected, so #272's validation keeps its teeth.
#[test]
fn set_still_rejects_a_different_message_type_from_a_foreign_pool() {
    let parent_pool = pool();
    let foreign_pool = pool();

    let containers = parent_pool
        .message_by_name("reflect.test.Containers")
        .unwrap();
    let scalars_idx = foreign_pool.message_index("reflect.test.Scalars").unwrap();
    let wrong_type = DynamicMessage::new(Arc::clone(&foreign_pool), scalars_idx);

    let err = parent_pool
        .message_index("reflect.test.Containers")
        .map(|idx| DynamicMessage::new(Arc::clone(&parent_pool), idx))
        .unwrap()
        .try_set(containers.field(5).unwrap(), Value::Message(wrong_type))
        .expect_err("Scalars is not an Inner, whatever pool it came from");
    assert!(matches!(err, ReflectError::WrongValueKind { .. }));
}

/// Two pools that disagree about a same-named type reinterpret the value's
/// bytes against the target's schema, exactly as if they had arrived from a
/// peer built on the other schema: what the target does not recognize lands in
/// unknown fields and is re-emitted on the next encode. Adoption is keyed on
/// the full name, so this is the documented consequence of taking equal names
/// to mean equal schemas — nothing is lost, but a field whose type is not
/// wire-compatible reads as unset rather than raising.
#[test]
fn set_reinterprets_a_same_named_message_whose_schema_diverges() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    // `skew.Holder{ skew.Payload sub = 1 }`, with Payload.v typed per `v_type`
    // and an optional field 7 the other side may not know.
    let build = |v_type: Type, with_extra: bool| {
        let mut fields = vec![FieldDescriptorProto {
            name: Some("v".into()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(v_type),
            ..Default::default()
        }];
        if with_extra {
            fields.push(FieldDescriptorProto {
                name: Some("extra".into()),
                number: Some(7),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                ..Default::default()
            });
        }
        let file = FileDescriptorProto {
            name: Some("skew.proto".into()),
            package: Some("skew".into()),
            syntax: Some("proto3".into()),
            message_type: vec![
                DescriptorProto {
                    name: Some("Holder".into()),
                    field: vec![FieldDescriptorProto {
                        name: Some("sub".into()),
                        number: Some(1),
                        label: Some(Label::LABEL_OPTIONAL),
                        r#type: Some(Type::TYPE_MESSAGE),
                        type_name: Some(".skew.Payload".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                DescriptorProto {
                    name: Some("Payload".into()),
                    field: fields,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        Arc::new(
            DescriptorPool::new(FileDescriptorSet {
                file: vec![file],
                ..Default::default()
            })
            .expect("pool builds from hand-built descriptor"),
        )
    };

    let adopt = |target: &Arc<DescriptorPool>, payload: DynamicMessage| {
        let holder_md = target.message_by_name("skew.Holder").unwrap();
        let mut holder = DynamicMessage::new(
            Arc::clone(target),
            target.message_index("skew.Holder").unwrap(),
        );
        holder
            .try_set(holder_md.field(1).unwrap(), Value::Message(payload))
            .expect("same full name is the adoption key, whatever the schema says");
        let Some(Value::Message(stored)) = holder.field_by_number(1) else {
            panic!("nested field not set");
        };
        stored.clone()
    };

    let payload_of = |p: &Arc<DescriptorPool>, v: Value| {
        let md = p.message_by_name("skew.Payload").unwrap();
        let mut m = DynamicMessage::new(Arc::clone(p), p.message_index("skew.Payload").unwrap());
        m.try_set(md.field(1).unwrap(), v).unwrap();
        m
    };

    // Wire-compatible types are reinterpreted, as protobuf itself defines them
    // to be: an int32 read against an int64 field is that same value.
    let stored = adopt(
        &build(Type::TYPE_INT64, false),
        payload_of(&build(Type::TYPE_INT32, false), Value::I32(-1)),
    );
    assert_eq!(stored.field_by_number(1), Some(&Value::I64(-1)));

    // A wire-incompatible type is not an error: the bytes go to unknown fields,
    // so the field reads unset and the value survives the next encode.
    let stored = adopt(
        &build(Type::TYPE_INT32, false),
        payload_of(&build(Type::TYPE_STRING, false), Value::String("hi".into())),
    );
    assert_eq!(stored.field_by_number(1), None, "not readable as an int32");
    assert_eq!(stored.unknown_fields().iter().count(), 1, "kept verbatim");

    // A field the target's schema lacks likewise round-trips intact.
    let src = build(Type::TYPE_INT32, true);
    let src_md = src.message_by_name("skew.Payload").unwrap();
    let mut rich =
        DynamicMessage::new(Arc::clone(&src), src.message_index("skew.Payload").unwrap());
    rich.try_set(src_md.field(1).unwrap(), Value::I32(5))
        .unwrap();
    rich.try_set(src_md.field(7).unwrap(), Value::String("keepme".into()))
        .unwrap();
    let stored = adopt(&build(Type::TYPE_INT32, false), rich);
    assert_eq!(stored.field_by_number(1), Some(&Value::I32(5)));
    let bytes = stored.encode_to_vec();
    assert!(
        bytes.windows(6).any(|w| w == b"keepme"),
        "the field this pool cannot name is re-emitted intact"
    );
}

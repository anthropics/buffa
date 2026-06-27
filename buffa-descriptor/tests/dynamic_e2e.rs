//! End-to-end tests for [`DynamicMessage`] encode/decode and the
//! [`ReflectMessage`] trait surface against a `protoc`-compiled
//! `FileDescriptorSet`.

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa_descriptor::reflect::{
    DynamicMessage, MapKey, MapValue, ReflectError, ReflectMessage, ReflectMessageMut, Value,
};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
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

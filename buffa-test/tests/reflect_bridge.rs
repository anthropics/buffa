//! Bridge test: generated buffa message ↔ DynamicMessage.
//!
//! Validates the encode/decode round-trip bridge from generated types to
//! `DynamicMessage` and back. This is what `Reflectable::reflect()` would do
//! for a bridge-mode message; here we wire it by hand to prove viability
//! without touching codegen.

use std::sync::Arc;

use buffa_descriptor::reflect::{DynamicMessage, MapKey, ReflectMessage, Value, ValueRef};
use buffa_descriptor::DescriptorPool;
use buffa_test::basic::*;

const FDS_BYTES: &[u8] = include_bytes!("protos/basic.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

#[test]
fn bridge_generated_to_dynamic() {
    let p = pool();
    let person_idx = p.message_index("basic.Person").expect("Person registered");

    // Build a generated Person with a representative field set.
    let person = Person {
        id: 42,
        name: "Ada".into(),
        avatar: vec![0xDE, 0xAD],
        verified: true,
        score: 9.5,
        status: Status::ACTIVE.into(),
        address: buffa::MessageField::some(Address {
            street: "1 Main".into(),
            city: "Somewhere".into(),
            zip_code: 12345,
            ..Default::default()
        }),
        tags: vec!["x".into(), "y".into()],
        lucky_numbers: vec![7, 11, 13],
        addresses: vec![Address {
            street: "2 Side".into(),
            ..Default::default()
        }],
        maybe_age: Some(30),
        contact: Some(person::Contact::Email("ada@example.com".into())),
        ..Default::default()
    };

    // Bridge: encode → decode into DynamicMessage.
    let dyn_msg = DynamicMessage::from_message(&person, Arc::clone(&p), person_idx);
    let person_md = p.message_by_name("basic.Person").unwrap();

    // Read fields reflectively. Field 1: id (int32).
    assert!(matches!(
        dyn_msg.get(person_md.field(1).unwrap()),
        ValueRef::I32(42)
    ));
    // Field 2: name (string).
    assert!(matches!(
        dyn_msg.get(person_md.field(2).unwrap()),
        ValueRef::String("Ada")
    ));
    // Field 4: verified (bool).
    assert!(matches!(
        dyn_msg.get(person_md.field(4).unwrap()),
        ValueRef::Bool(true)
    ));
    // Field 6: status (enum) — ACTIVE is 1.
    assert!(matches!(
        dyn_msg.get(person_md.field(6).unwrap()),
        ValueRef::EnumNumber(1)
    ));
    // Field 7: address (nested message).
    let addr_ref = dyn_msg.get(person_md.field(7).unwrap());
    let ValueRef::Message(cow) = addr_ref else {
        panic!("expected Message")
    };
    let addr_md = p.message_by_name("basic.Address").unwrap();
    let street = cow.get(addr_md.field(1).unwrap());
    assert!(matches!(street, ValueRef::String("1 Main")));
    // Field 8: tags (repeated string). `ValueRef::List` carries a
    // `&dyn ReflectList`, indexed via `.get()` not `[]`.
    let ValueRef::List(tags) = dyn_msg.get(person_md.field(8).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(tags.len(), 2);
    assert!(matches!(tags.get(0), Some(ValueRef::String("x"))));
    // Field 9: lucky_numbers (repeated int32, packed).
    let ValueRef::List(nums) = dyn_msg.get(person_md.field(9).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(nums.len(), 3);
    // Field 11: maybe_age (proto3 optional int32).
    assert!(dyn_msg.has(person_md.field(11).unwrap()));
    assert!(matches!(
        dyn_msg.get(person_md.field(11).unwrap()),
        ValueRef::I32(30)
    ));
    // Field 12: maybe_nickname — not set.
    assert!(!dyn_msg.has(person_md.field(12).unwrap()));
    // Field 13: contact.email (oneof string variant).
    assert!(dyn_msg.has(person_md.field(13).unwrap()));
    assert!(matches!(
        dyn_msg.get(person_md.field(13).unwrap()),
        ValueRef::String("ada@example.com")
    ));

    // Bridge back: DynamicMessage → generated Person.
    let roundtripped: Person = dyn_msg.to_message().expect("round-trip succeeds");
    assert_eq!(person, roundtripped);
}

#[test]
fn bridge_for_each_set_visits_set_fields() {
    let p = pool();
    let person_idx = p.message_index("basic.Person").unwrap();
    let person = Person {
        id: 1,
        name: "B".into(),
        ..Default::default()
    };
    let dyn_msg = DynamicMessage::from_message(&person, Arc::clone(&p), person_idx);

    let mut numbers = Vec::new();
    dyn_msg.for_each_set(&mut |fd, _| numbers.push(fd.number()));
    numbers.sort();
    // Only the non-default fields are present after wire round-trip.
    assert_eq!(numbers, vec![1, 2]);
}

#[test]
fn bridge_map_fields() {
    let p = pool();
    let inv_idx = p
        .message_index("basic.Inventory")
        .expect("Inventory registered");
    let inv_md = p.message_by_name("basic.Inventory").unwrap();

    let mut stock = buffa::Map::default();
    stock.insert("apples".to_string(), 3);
    stock.insert("oranges".to_string(), 7);
    let inv = Inventory {
        stock,
        ..Default::default()
    };
    let dyn_msg = DynamicMessage::from_message(&inv, Arc::clone(&p), inv_idx);

    // Find the stock field (map<string, int32>).
    let stock_fd = inv_md
        .fields()
        .iter()
        .find(|f| f.name() == "stock")
        .expect("stock field");
    let ValueRef::Map(m) = dyn_msg.get(stock_fd) else {
        panic!("expected Map")
    };
    assert_eq!(m.len(), 2);
    // The CEL hot-path lookup: borrowed `&str`, no `MapKey` allocation.
    // `ValueRef::Map` carries `&dyn ReflectMap` so `get_str` returns
    // `Option<ValueRef>`.
    assert!(matches!(m.get_str("apples"), Some(ValueRef::I32(3))));
    assert!(matches!(m.get_str("oranges"), Some(ValueRef::I32(7))));
    // The descriptor-keyed path also works.
    assert!(matches!(
        m.get(&MapKey::String("apples".into())),
        Some(ValueRef::I32(3))
    ));
    // Iteration via `for_each` — the dyn-safe non-allocating form.
    let mut total = 0;
    m.for_each(&mut |_k, v| {
        if let ValueRef::I32(n) = v {
            total += n;
        }
    });
    assert_eq!(total, 10);

    // Round-trip back.
    let roundtripped: Inventory = dyn_msg.to_message().expect("round-trip succeeds");
    assert_eq!(inv, roundtripped);
}

#[test]
fn dynamic_message_to_generated_with_set() {
    use buffa_descriptor::reflect::ReflectMessageMut;
    let p = pool();
    let person_idx = p.message_index("basic.Person").unwrap();
    let person_md = p.message_by_name("basic.Person").unwrap();

    // Build a DynamicMessage from scratch — the dynamic-schema path
    // (schema registry, transcoding).
    let mut dyn_msg = DynamicMessage::new(Arc::clone(&p), person_idx);
    dyn_msg.set(person_md.field(1).unwrap(), Value::I32(99));
    dyn_msg.set(person_md.field(2).unwrap(), Value::String("Carol".into()));
    // tags is field 8 (repeated string), not a map — use lucky_numbers (9).
    dyn_msg.set(
        person_md.field(9).unwrap(),
        Value::List(vec![Value::I32(7), Value::I32(11)]),
    );

    // Encode and decode into the generated type.
    let p2: Person = dyn_msg.to_message().unwrap();
    assert_eq!(p2.id, 99);
    assert_eq!(p2.name, "Carol");
    assert_eq!(p2.lucky_numbers, vec![7, 11]);
}

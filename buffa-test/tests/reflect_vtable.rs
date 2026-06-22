//! Vtable test: reflect directly over a decoded zero-copy view.
//!
//! Unlike `reflect_bridge.rs` (which round-trips through `DynamicMessage`),
//! these tests exercise the generated `impl ReflectMessage for FooView<'a>` —
//! `get`/`has`/`for_each_set` read view struct fields directly, with no
//! encode/decode round-trip and no per-field allocation. The view is produced
//! by `decode_view` straight from wire bytes, which is the entry point a
//! reflection consumer holding raw bytes (an interceptor, a field-mask
//! evaluator) uses.

use buffa::{Message, MessageView, OwnedView};
use buffa_descriptor::reflect::{MapKey, ReflectMessage, ValueRef};
use buffa_test::basic::*;

/// A representative Person, encoded to wire bytes.
fn person_bytes() -> Vec<u8> {
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
        maybe_age: Some(30),
        contact: Some(person::Contact::Email("ada@example.com".into())),
        ..Default::default()
    };
    person.encode_to_vec()
}

#[test]
fn vtable_view_scalar_and_string_fields() {
    let bytes = person_bytes();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    // Exercise the dyn-safe path: a reflection consumer (interceptor,
    // field-mask evaluator) holds `&dyn ReflectMessage`, not the concrete view.
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();

    // Field 1: id (int32) — scalar copy.
    assert!(matches!(r.get(md.field(1).unwrap()), ValueRef::I32(42)));
    // Field 2: name (string) — borrowed from the wire buffer, no allocation.
    assert!(matches!(
        r.get(md.field(2).unwrap()),
        ValueRef::String("Ada")
    ));
    // Field 3: avatar (bytes) — borrowed.
    let ValueRef::Bytes(avatar) = r.get(md.field(3).unwrap()) else {
        panic!("expected Bytes")
    };
    assert_eq!(avatar, &[0xDE, 0xAD]);
    // Field 4: verified (bool).
    assert!(matches!(r.get(md.field(4).unwrap()), ValueRef::Bool(true)));
    // Field 5: score (double).
    assert!(matches!(r.get(md.field(5).unwrap()), ValueRef::F64(9.5)));
    // Field 6: status (open enum) — ACTIVE is 1.
    assert!(matches!(
        r.get(md.field(6).unwrap()),
        ValueRef::EnumNumber(1)
    ));
}

#[test]
fn vtable_view_message_field_borrows() {
    let bytes = person_bytes();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    // Exercise the dyn-safe path: a reflection consumer (interceptor,
    // field-mask evaluator) holds `&dyn ReflectMessage`, not the concrete view.
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();

    // Field 7: address (nested message) — reflected without materializing.
    let ValueRef::Message(cow) = r.get(md.field(7).unwrap()) else {
        panic!("expected Message")
    };
    let addr_md = cow.message_descriptor();
    assert!(matches!(
        cow.get(addr_md.field(1).unwrap()),
        ValueRef::String("1 Main")
    ));
    assert!(matches!(
        cow.get(addr_md.field(2).unwrap()),
        ValueRef::String("Somewhere")
    ));
}

#[test]
fn vtable_view_repeated_fields() {
    let bytes = person_bytes();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    // Exercise the dyn-safe path: a reflection consumer (interceptor,
    // field-mask evaluator) holds `&dyn ReflectMessage`, not the concrete view.
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();

    // Field 8: tags (repeated string).
    let ValueRef::List(tags) = r.get(md.field(8).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(tags.len(), 2);
    assert!(matches!(tags.get(0), Some(ValueRef::String("x"))));
    assert!(matches!(tags.get(1), Some(ValueRef::String("y"))));

    // Field 9: lucky_numbers (repeated int32, packed).
    let ValueRef::List(nums) = r.get(md.field(9).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(nums.len(), 3);
    let mut sum = 0;
    nums.for_each(&mut |v| {
        if let ValueRef::I32(n) = v {
            sum += n;
        }
    });
    assert_eq!(sum, 31);
}

#[test]
fn vtable_view_presence_and_oneof() {
    let bytes = person_bytes();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    // Exercise the dyn-safe path: a reflection consumer (interceptor,
    // field-mask evaluator) holds `&dyn ReflectMessage`, not the concrete view.
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();

    // Field 11: maybe_age (proto3 optional int32) — set.
    assert!(r.has(md.field(11).unwrap()));
    assert!(matches!(r.get(md.field(11).unwrap()), ValueRef::I32(30)));
    // Field 12: maybe_nickname — not set; has() false, get() returns default.
    assert!(!r.has(md.field(12).unwrap()));
    assert!(matches!(r.get(md.field(12).unwrap()), ValueRef::String("")));
    // Field 13: contact.email (oneof string variant) — active.
    assert!(r.has(md.field(13).unwrap()));
    assert!(matches!(
        r.get(md.field(13).unwrap()),
        ValueRef::String("ada@example.com")
    ));
}

#[test]
fn vtable_for_each_set_visits_set_fields() {
    // Only id and name set; after wire round-trip the rest are default.
    let person = Person {
        id: 1,
        name: "B".into(),
        ..Default::default()
    };
    let bytes = person.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    // Exercise the dyn-safe path: a reflection consumer (interceptor,
    // field-mask evaluator) holds `&dyn ReflectMessage`, not the concrete view.
    let r: &dyn ReflectMessage = &view;

    let mut numbers = Vec::new();
    r.for_each_set(&mut |fd, _| numbers.push(fd.number()));
    numbers.sort_unstable();
    assert_eq!(numbers, vec![1u32, 2u32]);
}

#[test]
fn vtable_map_field() {
    let mut stock = buffa::Map::default();
    stock.insert("apples".to_string(), 3);
    stock.insert("oranges".to_string(), 7);
    let inv = Inventory {
        stock,
        ..Default::default()
    };
    let bytes = inv.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();

    let stock_fd = md
        .fields()
        .iter()
        .find(|f| f.name() == "stock")
        .expect("stock field");
    let ValueRef::Map(m) = r.get(stock_fd) else {
        panic!("expected Map")
    };
    assert_eq!(m.len(), 2);
    // No-alloc string lookup (the CEL hot path).
    assert!(matches!(m.get_str("apples"), Some(ValueRef::I32(3))));
    assert!(matches!(m.get_str("oranges"), Some(ValueRef::I32(7))));
    assert!(m.get_str("durian").is_none());
    // Descriptor-keyed lookup.
    assert!(matches!(
        m.get(&MapKey::String("apples".into())),
        Some(ValueRef::I32(3))
    ));
}

#[test]
fn vtable_to_dynamic_snapshot() {
    // `to_dynamic()` falls back to a bridge-style materialization for consumers
    // that need an owned snapshot.
    let bytes = person_bytes();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    // Exercise the dyn-safe path: a reflection consumer (interceptor,
    // field-mask evaluator) holds `&dyn ReflectMessage`, not the concrete view.
    let r: &dyn ReflectMessage = &view;

    let snapshot = r.to_dynamic();
    let snap_md = snapshot.message_descriptor();
    assert!(matches!(
        snapshot.get(snap_md.field(2).unwrap()),
        ValueRef::String("Ada")
    ));
    assert!(matches!(
        snapshot.get(snap_md.field(1).unwrap()),
        ValueRef::I32(42)
    ));
}

#[test]
fn vtable_owned_view_entry_point() {
    // The entry point a reflection consumer holding raw wire bytes uses: wrap
    // them in an `OwnedView` (lifetime-erased), reborrow to a tied-lifetime
    // view, and reflect through `&dyn ReflectMessage`.
    let bytes = buffa::bytes::Bytes::from(person_bytes());
    let owned = OwnedView::<PersonView<'static>>::decode(bytes).expect("OwnedView::decode");
    let view = owned.reborrow();
    let r: &dyn ReflectMessage = view;
    let md = r.message_descriptor();
    assert!(matches!(r.get(md.field(1).unwrap()), ValueRef::I32(42)));
    assert!(matches!(
        r.get(md.field(2).unwrap()),
        ValueRef::String("Ada")
    ));
}

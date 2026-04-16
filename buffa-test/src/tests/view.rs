//! View type tests: decode_view, MessageFieldView Deref, to_owned_message,
//! MapView iteration, oneof views, unknown-field preservation, recursion limit.

use crate::basic::*;
use buffa::{Message, MessageView};

#[test]
fn test_view_decodes_scalar_fields() {
    let mut msg = Person::default();
    msg.id = 99;
    msg.name = "Alice".into();
    msg.verified = true;
    msg.score = 3.14;
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view failed");
    assert_eq!(view.id, 99);
    assert_eq!(view.name, "Alice");
    assert!(view.verified);
    assert!((view.score - 3.14).abs() < 1e-10);
}

#[test]
fn test_view_decodes_string_and_bytes_zero_copy() {
    let mut msg = Person::default();
    msg.name = "zero-copy".into();
    msg.avatar = vec![0xDE, 0xAD];
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.name, "zero-copy");
    assert_eq!(view.avatar, &[0xDE, 0xAD]);
}

#[test]
fn test_view_decodes_nested_message() {
    let mut msg = Person::default();
    msg.address.get_or_insert_default().street = "1 Main St".into();
    msg.address.get_or_insert_default().zip_code = 90210;
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    assert!(view.address.is_set());
    let addr = view.address.as_option().unwrap();
    assert_eq!(addr.street, "1 Main St");
    assert_eq!(addr.zip_code, 90210);
}

#[test]
fn test_view_decodes_repeated_string() {
    let mut msg = Person::default();
    msg.tags = vec!["a".into(), "b".into(), "c".into()];
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    let tags: Vec<&str> = view.tags.iter().copied().collect();
    assert_eq!(tags, vec!["a", "b", "c"]);
}

#[test]
fn test_view_decodes_repeated_packed_int() {
    let mut msg = Person::default();
    msg.lucky_numbers = vec![7, 13, 42];
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.lucky_numbers.as_ref(), &[7, 13, 42]);
}

#[test]
fn test_view_decodes_repeated_message() {
    let mut msg = Person::default();
    let mut a = Address::default();
    a.street = "X".into();
    msg.addresses = vec![a];
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.addresses.len(), 1);
    assert_eq!(view.addresses[0].street, "X");
}

#[test]
fn test_view_decodes_proto3_optional() {
    let mut msg = Person::default();
    msg.maybe_age = Some(25);
    msg.maybe_nickname = Some("Al".into());
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.maybe_age, Some(25));
    assert_eq!(view.maybe_nickname, Some("Al"));
}

#[test]
fn test_view_proto3_optional_unset_is_none() {
    let bytes = Person::default().encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.maybe_age, None);
    assert_eq!(view.maybe_nickname, None);
}

#[test]
fn test_view_decodes_oneof() {
    let mut msg = Person::default();
    msg.contact = Some(person::ContactOneof::Email("bob@example.com".into()));
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    match view.contact {
        Some(person::ContactOneofView::Email(s)) => assert_eq!(s, "bob@example.com"),
        other => panic!("expected Email, got {other:?}"),
    }
}

#[test]
fn test_view_to_owned_roundtrip() {
    let mut msg = Person::default();
    msg.id = 42;
    msg.name = "Carol".into();
    msg.tags = vec!["x".into(), "y".into()];
    msg.maybe_age = Some(30);
    msg.contact = Some(person::ContactOneof::Phone("+1-555-0000".into()));
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    let owned = view.to_owned_message();
    assert_eq!(owned.id, 42);
    assert_eq!(owned.name, "Carol");
    assert_eq!(owned.tags, vec!["x", "y"]);
    assert_eq!(owned.maybe_age, Some(30));
    assert_eq!(
        owned.contact,
        Some(person::ContactOneof::Phone("+1-555-0000".into()))
    );
}

#[test]
fn test_view_empty_message() {
    let bytes = Empty::default().encode_to_vec();
    assert!(bytes.is_empty());
    let view = EmptyView::decode_view(&bytes).expect("decode_view");
    assert!(view.__buffa_unknown_fields.is_empty());
}

#[test]
fn test_view_preserves_unknown_fields() {
    use buffa::encoding::{encode_varint, Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(1, WireType::LengthDelimited).encode(&mut wire);
    buffa::types::encode_string("Hello", &mut wire);
    Tag::new(99, WireType::Varint).encode(&mut wire);
    encode_varint(7, &mut wire);
    let view = AddressView::decode_view(&wire).expect("decode_view");
    assert_eq!(view.street, "Hello");
    assert!(!view.__buffa_unknown_fields.is_empty());
}

#[test]
fn test_view_recursion_limit_exceeded() {
    // Encode a Person with an Address sub-message, then attempt to decode
    // the view with depth=0.  The address arm checks depth==0 and must
    // return RecursionLimitExceeded before recursing.
    let mut msg = Person::default();
    msg.address.get_or_insert_default().street = "1 Main St".into();
    let bytes = msg.encode_to_vec();
    let result = PersonView::_decode_depth(&bytes, 0);
    assert!(
        matches!(result, Err(buffa::DecodeError::RecursionLimitExceeded)),
        "expected RecursionLimitExceeded, got {result:?}"
    );
}

#[test]
fn test_unknown_group_respects_depth_budget() {
    // Regression: skip_field previously reset the recursion budget to
    // RECURSION_LIMIT (100) for unknown fields, allowing depth-doubling
    // attacks via unknown groups. Now uses skip_field_depth(tag, buf, depth).
    use buffa::encoding::{Tag, WireType};
    // Unknown field 99 as a StartGroup containing a nested StartGroup:
    // this requires depth >= 2 to skip. With depth=1, skip_field_depth
    // decrements to 0 for the outer group, then the inner StartGroup
    // must return RecursionLimitExceeded.
    let mut wire = Vec::new();
    Tag::new(99, WireType::StartGroup).encode(&mut wire); // depth -> 0
    Tag::new(98, WireType::StartGroup).encode(&mut wire); // exceeds
    Tag::new(98, WireType::EndGroup).encode(&mut wire);
    Tag::new(99, WireType::EndGroup).encode(&mut wire);

    // Via Person merge (owned path, preserve_unknown_fields=true by default
    // — decode_unknown_field is used there, which was already correct).
    // The view path is what was broken.
    let result = PersonView::_decode_depth(&wire, 1);
    assert!(
        matches!(result, Err(buffa::DecodeError::RecursionLimitExceeded)),
        "unknown nested group must respect depth budget, got {result:?}"
    );

    // With depth=2, should succeed (one level per group).
    let result = PersonView::_decode_depth(&wire, 2);
    assert!(result.is_ok(), "depth=2 should suffice, got {result:?}");
}

// -----------------------------------------------------------------------
// Map field view tests
// -----------------------------------------------------------------------

#[test]
fn test_view_decodes_map_scalar_value() {
    let mut inv = Inventory::default();
    inv.stock.insert("apples".into(), 10);
    inv.stock.insert("bananas".into(), 5);
    let bytes = inv.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.stock.get(&"apples"), Some(&10));
    assert_eq!(view.stock.get(&"bananas"), Some(&5));
    assert_eq!(view.stock.len(), 2);
}

#[test]
fn test_view_decodes_map_message_value() {
    let mut inv = Inventory::default();
    let mut addr = Address::default();
    addr.street = "1 Warehouse Rd".into();
    inv.locations.insert("depot".into(), addr);
    let bytes = inv.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");
    let loc = view.locations.get(&"depot").expect("depot missing");
    assert_eq!(loc.street, "1 Warehouse Rd");
}

#[test]
fn test_view_map_to_owned_roundtrip() {
    let mut inv = Inventory::default();
    inv.stock.insert("x".into(), 42);
    inv.stock.insert("y".into(), 7);
    let bytes = inv.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");
    let owned = view.to_owned_message();
    assert_eq!(owned.stock.get("x"), Some(&42));
    assert_eq!(owned.stock.get("y"), Some(&7));
}

#[test]
fn test_view_map_message_to_owned_roundtrip() {
    let mut inv = Inventory::default();
    let mut addr = Address::default();
    addr.street = "100 Main St".into();
    addr.city = "Springfield".into();
    inv.locations.insert("hq".into(), addr.clone());
    let bytes = inv.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");
    let owned = view.to_owned_message();
    assert_eq!(owned.locations.get("hq"), Some(&addr));
}

#[test]
fn test_view_map_empty() {
    let inv = Inventory::default();
    let bytes = inv.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");
    assert!(view.stock.is_empty());
    assert!(view.locations.is_empty());
}

#[test]
fn test_compute_size_matches_encode_len() {
    let mut msg = Person::default();
    msg.id = 99;
    msg.name = "Bob".into();
    msg.tags = vec!["a".into(), "b".into()];
    let size = msg.compute_size() as usize;
    let bytes = msg.encode_to_vec();
    assert_eq!(size, bytes.len());
}

#[test]
fn test_view_map_with_open_enum_value() {
    // map<string, Status> — proto3 open enum as map value.
    // Exercises EnumValue<E> branch in view map value type/decode codegen.
    let mut msg = Inventory::default();
    msg.statuses
        .insert("svc1".into(), buffa::EnumValue::Known(Status::ACTIVE));
    msg.statuses
        .insert("svc2".into(), buffa::EnumValue::Known(Status::INACTIVE));
    // Unknown value — open enum preserves it.
    msg.statuses
        .insert("svc3".into(), buffa::EnumValue::Unknown(99));

    let bytes = msg.encode_to_vec();
    let view = InventoryView::decode_view(&bytes).expect("decode_view");

    let collected: std::collections::HashMap<_, _> = view
        .statuses
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    assert_eq!(
        collected.get("svc1"),
        Some(&buffa::EnumValue::Known(Status::ACTIVE))
    );
    assert_eq!(
        collected.get("svc2"),
        Some(&buffa::EnumValue::Known(Status::INACTIVE))
    );
    // Unknown value survives view decode + to_owned.
    assert_eq!(collected.get("svc3"), Some(&buffa::EnumValue::Unknown(99)));

    let owned = view.to_owned_message();
    assert_eq!(
        owned.statuses.get("svc1"),
        Some(&buffa::EnumValue::Known(Status::ACTIVE))
    );
    assert_eq!(
        owned.statuses.get("svc3"),
        Some(&buffa::EnumValue::Unknown(99))
    );
}

// ── Views + preserve_unknown_fields=false ────────────────────────────
// Regression: this config previously produced E0392 (unused lifetime 'a)
// for all-scalar messages — the UnknownFieldsView<'a> field was the only
// thing borrowing 'a, so disabling it left the lifetime unused.
// Fixed by injecting PhantomData<&'a ()> when no field borrows.

#[test]
fn test_view_no_unknown_fields_all_scalar_compiles() {
    use crate::basic_no_uf::{AllScalars, AllScalarsView, Empty, EmptyView};
    // EmptyView<'a> has NO fields; AllScalarsView<'a> has only scalars.
    // Both now carry a PhantomData marker.
    let e = Empty::default();
    let bytes = e.encode_to_vec();
    let view = EmptyView::decode_view(&bytes).unwrap();
    let _owned = view.to_owned_message();

    let mut s = AllScalars::default();
    s.f_int32 = 42;
    s.f_bool = true;
    let bytes = s.encode_to_vec();
    let view = AllScalarsView::decode_view(&bytes).unwrap();
    assert_eq!(view.f_int32, 42);
    assert!(view.f_bool);
    let owned = view.to_owned_message();
    assert_eq!(owned.f_int32, 42);
}

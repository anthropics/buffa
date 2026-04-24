//! View type tests: decode_view, MessageFieldView Deref, to_owned_message,
//! MapView iteration, oneof views, unknown-field preservation, recursion limit.

use crate::basic::__buffa::oneof;
use crate::basic::__buffa::view::oneof as view_oneof;
use crate::basic::__buffa::view::*;
use crate::basic::*;
use buffa::{Message, MessageView, ViewEncode};

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
    msg.contact = Some(oneof::person::Contact::Email("bob@example.com".into()));
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    match view.contact {
        Some(crate::basic::__buffa::view::oneof::person::Contact::Email(s)) => {
            assert_eq!(s, "bob@example.com")
        }
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
    msg.contact = Some(oneof::person::Contact::Phone("+1-555-0000".into()));
    let bytes = msg.encode_to_vec();
    let view = PersonView::decode_view(&bytes).expect("decode_view");
    let owned = view.to_owned_message();
    assert_eq!(owned.id, 42);
    assert_eq!(owned.name, "Carol");
    assert_eq!(owned.tags, vec!["x", "y"]);
    assert_eq!(owned.maybe_age, Some(30));
    assert_eq!(
        owned.contact,
        Some(oneof::person::Contact::Phone("+1-555-0000".into()))
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
fn test_compute_size_matches_encode_len_view() {
    // Covers nested-message, repeated-message, and oneof-message cached-size
    // dispatch through MessageFieldView/Box.
    let addr = AddressView {
        street: "1 Main St",
        city: "Springfield",
        zip_code: 12345,
        ..Default::default()
    };
    let view = PersonView {
        id: 99,
        name: "Bob",
        tags: ["a", "b"].iter().copied().collect(),
        address: addr.clone().into(),
        addresses: [addr.clone()].into_iter().collect(),
        contact: Some(view_oneof::person::Contact::HomeAddress(Box::new(addr))),
        ..Default::default()
    };
    let size = view.compute_size() as usize;
    let bytes = view.encode_to_vec();
    assert_eq!(size, bytes.len());
    // Round-trips through owned.
    let owned = Person::decode(&mut bytes.as_slice()).expect("decode owned");
    assert_eq!(owned.encode_to_vec().len(), size);
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
    use crate::basic_no_uf::__buffa::view::{AllScalarsView, EmptyView};
    use crate::basic_no_uf::{AllScalars, Empty};
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

// ── view encode ──────────────────────────────────────────────────────────

#[test]
fn test_view_encode_roundtrip_address() {
    let owned = Address {
        street: "1 Infinite Loop".into(),
        city: "Cupertino".into(),
        zip_code: 95014,
        ..Default::default()
    };
    let owned_bytes = owned.encode_to_vec();
    let view = AddressView::decode_view(&owned_bytes).unwrap();
    let view_bytes = view.encode_to_vec();
    assert_eq!(owned_bytes, view_bytes);
}

#[test]
fn test_view_encode_construct_from_borrows() {
    let view = AddressView {
        street: "borrowed st",
        city: "ref town",
        zip_code: 12345,
        ..Default::default()
    };
    let bytes = view.encode_to_vec();
    let decoded = Address::decode_from_slice(&bytes).unwrap();
    assert_eq!(decoded.street, "borrowed st");
    assert_eq!(decoded.city, "ref town");
    assert_eq!(decoded.zip_code, 12345);
}

#[test]
fn test_view_encode_roundtrip_inventory_maps() {
    let mut owned = Inventory::default();
    owned.stock.insert("widgets".into(), 7);
    owned.stock.insert("gadgets".into(), 3);
    owned.locations.insert(
        "hq".into(),
        Address {
            street: "main".into(),
            city: "metro".into(),
            zip_code: 1,
            ..Default::default()
        },
    );
    owned.statuses.insert("hq".into(), Status::ACTIVE.into());
    let owned_bytes = owned.encode_to_vec();
    let view = InventoryView::decode_view(&owned_bytes).unwrap();
    let view_bytes = view.encode_to_vec();
    // Map ordering: HashMap iter order is unspecified, but a decoded
    // MapView preserves wire order, so re-encoding the view of the
    // owned encoding produces the SAME bytes.
    assert_eq!(owned_bytes, view_bytes);
}

#[test]
fn test_view_encode_construct_map_from_iter() {
    let pairs = [("a", 1i32), ("b", 2)];
    let view = InventoryView {
        stock: pairs.iter().copied().collect(),
        ..Default::default()
    };
    let bytes = view.encode_to_vec();
    let decoded = Inventory::decode_from_slice(&bytes).unwrap();
    assert_eq!(decoded.stock.get("a"), Some(&1));
    assert_eq!(decoded.stock.get("b"), Some(&2));
}

#[test]
fn test_view_encode_oneof_roundtrip() {
    // String variant.
    let mut owned = Person::default();
    owned.contact = Some(oneof::person::Contact::Email("a@b.c".into()));
    let owned_bytes = owned.encode_to_vec();
    let view = PersonView::decode_view(&owned_bytes).unwrap();
    assert_eq!(owned_bytes, view.encode_to_vec());

    // Nested-message variant — exercises ViewEncode dispatch through Box<View>.
    let mut owned = Person::default();
    owned.contact = Some(oneof::person::Contact::HomeAddress(Box::new(Address {
        street: "1 main".into(),
        city: "metro".into(),
        zip_code: 9,
        ..Default::default()
    })));
    let owned_bytes = owned.encode_to_vec();
    let view = PersonView::decode_view(&owned_bytes).unwrap();
    assert_eq!(owned_bytes, view.encode_to_vec());
}

#[test]
fn test_view_encode_repeated_nested_and_oneof_from_borrows() {
    use buffa::{MessageFieldView, RepeatedView};
    let addr = AddressView {
        street: "x",
        city: "y",
        zip_code: 9,
        ..Default::default()
    };
    let view = PersonView {
        tags: RepeatedView::new(vec!["t1", "t2"]),
        lucky_numbers: RepeatedView::new(vec![7, 11]),
        address: MessageFieldView::set(addr.clone()),
        addresses: RepeatedView::new(vec![addr.clone()]),
        contact: Some(view_oneof::person::Contact::HomeAddress(Box::new(addr))),
        ..Default::default()
    };
    let bytes = view.encode_to_vec();
    let decoded = Person::decode_from_slice(&bytes).unwrap();
    assert_eq!(decoded.tags, ["t1", "t2"]);
    assert_eq!(decoded.lucky_numbers, [7, 11]);
    assert_eq!(decoded.address.street, "x");
    assert_eq!(decoded.addresses[0].zip_code, 9);
    let Some(oneof::person::Contact::HomeAddress(a)) = &decoded.contact else {
        panic!("expected HomeAddress variant")
    };
    assert_eq!(a.city, "y");
}

#[test]
fn test_view_encode_compute_size_matches_len() {
    let owned = Address {
        street: "1 Infinite Loop".into(),
        city: "Cupertino".into(),
        zip_code: 95014,
        ..Default::default()
    };
    let bytes = owned.encode_to_vec();
    let view = AddressView::decode_view(&bytes).unwrap();
    assert_eq!(view.compute_size() as usize, view.encode_to_vec().len());
}

#[test]
fn test_view_encode_proto2_groups_roundtrip() {
    use crate::proto2::__buffa::view::WithGroupsView;
    use crate::proto2::{with_groups, WithGroups};
    let owned = WithGroups {
        mygroup: with_groups::MyGroup {
            a: Some(7),
            b: Some("inner".into()),
            ..Default::default()
        }
        .into(),
        item: vec![
            with_groups::Item {
                id: Some(1),
                name: Some("first".into()),
                ..Default::default()
            },
            with_groups::Item {
                id: Some(2),
                name: Some("second".into()),
                ..Default::default()
            },
        ],
        label: Some("alongside".into()),
        ..Default::default()
    };
    let owned_bytes = owned.encode_to_vec();
    let view = WithGroupsView::decode_view(&owned_bytes).unwrap();
    let view_bytes = view.encode_to_vec();
    assert_eq!(owned_bytes, view_bytes);
    assert_eq!(view.compute_size() as usize, view_bytes.len());
}

#[test]
fn test_view_encode_proto2_oneof_group_closed_enum_roundtrip() {
    use crate::proto2::__buffa::oneof::view_coverage as vc_oneof;
    use crate::proto2::__buffa::view::ViewCoverageView;
    use crate::proto2::{view_coverage, Priority, ViewCoverage};
    let mut owned = ViewCoverage {
        level: Priority::HIGH,
        choice: Some(vc_oneof::Choice::Payload(Box::new(
            view_coverage::Payload {
                x: Some(42),
                y: Some("oneof-group".into()),
                ..Default::default()
            },
        ))),
        ..Default::default()
    };
    owned.by_id.insert(10, "ten".into());
    owned.priorities.insert("k".into(), Priority::LOW);
    let owned_bytes = owned.encode_to_vec();
    let view = ViewCoverageView::decode_view(&owned_bytes).unwrap();
    let view_bytes = view.encode_to_vec();
    assert_eq!(owned_bytes, view_bytes);
    // Re-decode through owned to confirm semantic equality, not just byte
    // identity (which here happens to hold because each map has 1 entry).
    let redecoded = ViewCoverage::decode_from_slice(&view_bytes).unwrap();
    assert_eq!(redecoded, owned);
}

#[test]
fn test_view_encode_closed_enum_unknown_value_preserved() {
    // Unknown closed-enum value on the wire: view decode cannot represent
    // 99 as a `Priority` so the typed `level` field stays at default and the
    // raw bytes go to UnknownFieldsView. ViewEncode must re-emit those
    // unknown bytes so a downstream owned decoder sees the same result as
    // decoding the original wire directly.
    use crate::proto2::__buffa::view::ViewCoverageView;
    use crate::proto2::{Priority, ViewCoverage};
    // field 1 (level) varint = tag 0x08, value 99 (not a Priority variant).
    let wire: &[u8] = &[0x08, 99];
    let owned_direct = ViewCoverage::decode_from_slice(wire).unwrap();
    let view = ViewCoverageView::decode_view(wire).unwrap();
    // The typed field is NOT 99 — it's the proto2 first-variant default.
    assert_eq!(view.level, Priority::LOW);
    let view_bytes = view.encode_to_vec();
    let owned_via_view = ViewCoverage::decode_from_slice(&view_bytes).unwrap();
    assert_eq!(
        owned_via_view, owned_direct,
        "view-encode must preserve unknown closed-enum semantics"
    );
}

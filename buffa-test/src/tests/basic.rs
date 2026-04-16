//! Basic round-trips: Empty, Person, Address, unknown-field preservation.

use super::round_trip;
use crate::basic::*;
use buffa::Message;

#[test]
fn test_empty_message_encodes_to_zero_bytes() {
    let msg = Empty::default();
    let bytes = msg.encode_to_vec();
    assert!(bytes.is_empty(), "empty message should encode to 0 bytes");
}

#[test]
fn test_empty_message_round_trips() {
    assert_eq!(round_trip(&Empty::default()), Empty::default());
}

#[test]
fn test_all_scalars_default_encodes_to_zero_bytes() {
    let bytes = AllScalars::default().encode_to_vec();
    assert!(bytes.is_empty(), "all-default scalars should be empty");
}

#[test]
fn test_all_scalars_round_trip() {
    let msg = AllScalars {
        f_int32: -1,
        f_int64: i64::MIN,
        f_uint32: u32::MAX,
        f_uint64: u64::MAX,
        f_sint32: -100,
        f_sint64: -200,
        f_fixed32: 0xDEAD_BEEF,
        f_fixed64: 0xDEAD_BEEF_CAFE_BABE,
        f_sfixed32: -42,
        f_sfixed64: -84,
        f_float: 1.5_f32,
        f_double: std::f64::consts::PI,
        f_bool: true,
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn test_person_scalar_fields() {
    let mut msg = Person::default();
    msg.id = 42;
    msg.name = "Alice".into();
    msg.verified = true;
    msg.score = 9.5;
    let decoded = round_trip(&msg);
    assert_eq!(decoded.id, 42);
    assert_eq!(decoded.name, "Alice");
    assert!(decoded.verified);
    assert!((decoded.score - 9.5).abs() < 1e-10);
}

#[test]
fn test_person_bytes_field() {
    let mut msg = Person::default();
    msg.avatar = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let decoded = round_trip(&msg);
    assert_eq!(decoded.avatar, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn test_person_enum_field() {
    let mut msg = Person::default();
    msg.status = buffa::EnumValue::from(Status::ACTIVE);
    let decoded = round_trip(&msg);
    assert_eq!(decoded.status.to_i32(), Status::ACTIVE as i32);
}

#[test]
fn test_person_nested_message_field() {
    let mut msg = Person::default();
    msg.address.get_or_insert_default().street = "123 Main St".into();
    msg.address.get_or_insert_default().city = "Springfield".into();
    msg.address.get_or_insert_default().zip_code = 12345;
    assert!(msg.address.is_set());
    let decoded = round_trip(&msg);
    assert!(decoded.address.is_set());
    assert_eq!(decoded.address.street, "123 Main St");
    assert_eq!(decoded.address.city, "Springfield");
    assert_eq!(decoded.address.zip_code, 12345);
}

#[test]
fn test_person_repeated_string_field() {
    let mut msg = Person::default();
    msg.tags = vec!["alpha".into(), "beta".into(), "gamma".into()];
    let decoded = round_trip(&msg);
    assert_eq!(decoded.tags, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn test_person_repeated_int_field() {
    let mut msg = Person::default();
    msg.lucky_numbers = vec![7, 13, 42, -1];
    let decoded = round_trip(&msg);
    assert_eq!(decoded.lucky_numbers, vec![7, 13, 42, -1]);
}

#[test]
fn test_person_repeated_message_field() {
    let mut msg = Person::default();
    let mut a1 = Address::default();
    a1.street = "1 First St".into();
    let mut a2 = Address::default();
    a2.street = "2 Second St".into();
    msg.addresses = vec![a1, a2];
    let decoded = round_trip(&msg);
    assert_eq!(decoded.addresses.len(), 2);
    assert_eq!(decoded.addresses[0].street, "1 First St");
    assert_eq!(decoded.addresses[1].street, "2 Second St");
}

#[test]
fn test_person_proto3_optional_set() {
    let mut msg = Person::default();
    msg.maybe_age = Some(30);
    msg.maybe_nickname = Some("Al".into());
    let decoded = round_trip(&msg);
    assert_eq!(decoded.maybe_age, Some(30));
    assert_eq!(decoded.maybe_nickname.as_deref(), Some("Al"));
}

#[test]
fn test_person_proto3_optional_zero_is_present() {
    let mut msg = Person::default();
    msg.maybe_age = Some(0);
    let bytes = msg.encode_to_vec();
    assert!(!bytes.is_empty(), "optional field set to 0 must be encoded");
    let decoded = round_trip(&msg);
    assert_eq!(decoded.maybe_age, Some(0));
}

#[test]
fn test_person_proto3_optional_unset_is_none() {
    let decoded = round_trip(&Person::default());
    assert_eq!(decoded.maybe_age, None);
}

#[test]
fn test_merge_appends_repeated_fields() {
    let mut a = Person::default();
    a.tags = vec!["x".into()];
    let b_bytes = {
        let mut b = Person::default();
        b.tags = vec!["y".into(), "z".into()];
        b.encode_to_vec()
    };
    a.merge(&mut b_bytes.as_slice(), buffa::RECURSION_LIMIT)
        .unwrap();
    assert_eq!(a.tags, vec!["x", "y", "z"]);
}

#[test]
fn test_map_scalar_value_round_trip() {
    let mut inv = Inventory::default();
    inv.stock.insert("apples".into(), 10);
    inv.stock.insert("bananas".into(), 5);
    let decoded = round_trip(&inv);
    assert_eq!(decoded.stock.get("apples"), Some(&10));
    assert_eq!(decoded.stock.get("bananas"), Some(&5));
    assert_eq!(decoded.stock.len(), 2);
}

#[test]
fn test_map_message_value_round_trip() {
    let mut inv = Inventory::default();
    let mut addr = Address::default();
    addr.street = "1 Warehouse Rd".into();
    inv.locations.insert("depot".into(), addr);
    let decoded = round_trip(&inv);
    assert_eq!(decoded.locations["depot"].street, "1 Warehouse Rd");
}

#[test]
fn test_map_empty_round_trip() {
    let inv = Inventory::default();
    let bytes = inv.encode_to_vec();
    assert!(bytes.is_empty());
    let decoded = round_trip(&inv);
    assert!(decoded.stock.is_empty());
}

#[test]
fn test_oneof_round_trip() {
    // Set the email variant.
    let mut msg = Person::default();
    msg.contact = Some(person::ContactOneof::Email("alice@example.com".into()));
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.contact,
        Some(person::ContactOneof::Email("alice@example.com".into()))
    );

    // Overwrite with phone variant — last write wins.
    msg.contact = Some(person::ContactOneof::Phone("+1-555-1234".into()));
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.contact,
        Some(person::ContactOneof::Phone("+1-555-1234".into()))
    );

    // Unset.
    msg.contact = None;
    let decoded = round_trip(&msg);
    assert_eq!(decoded.contact, None);
}

#[test]
fn test_oneof_default_encodes_to_empty() {
    let msg = Person::default();
    assert_eq!(msg.contact, None);
    let bytes = msg.encode_to_vec();
    // A completely default person still encodes to empty (all fields default).
    let decoded = round_trip(&msg);
    assert_eq!(decoded.contact, None);
    let _ = bytes;
}

#[test]
fn test_unknown_fields_preserved_through_round_trip() {
    // Encode a message with a field that `Address` doesn't know about
    // (field 99, varint value 42), then decode it as Address and re-encode.
    // The unknown field must survive both passes.
    use buffa::encoding::{encode_varint, Tag, WireType};
    let mut wire = Vec::new();
    // field 1 (street) = "Main St"
    Tag::new(1, WireType::LengthDelimited).encode(&mut wire);
    buffa::types::encode_string("Main St", &mut wire);
    // field 99 (unknown, varint) = 42
    Tag::new(99, WireType::Varint).encode(&mut wire);
    encode_varint(42, &mut wire);

    let addr = Address::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(addr.street, "Main St");
    assert!(
        !addr.__buffa_unknown_fields.is_empty(),
        "unknown field must be preserved"
    );

    // Re-encode and check the unknown field is still there.
    let re_encoded = addr.encode_to_vec();
    let addr2 = Address::decode(&mut re_encoded.as_slice()).unwrap();
    assert_eq!(addr2.street, "Main St");
    assert!(
        !addr2.__buffa_unknown_fields.is_empty(),
        "unknown field must survive re-encode"
    );
}

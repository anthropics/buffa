//! Eq and Hash support for well-known types without floating-point fields.

use buffa::Message;
use buffa_types::google::protobuf as wkt;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

fn hash(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn float_free_wkts_implement_eq_and_hash() {
    fn assert_eq_hash<T: Eq + Hash>() {}

    assert_eq_hash::<wkt::Timestamp>();
    assert_eq_hash::<wkt::Duration>();
    assert_eq_hash::<wkt::FieldMask>();
    assert_eq_hash::<wkt::Empty>();
    assert_eq_hash::<wkt::Any>();
    assert_eq_hash::<wkt::SourceContext>();
    assert_eq_hash::<wkt::Int32Value>();
    assert_eq_hash::<wkt::Int64Value>();
    assert_eq_hash::<wkt::UInt32Value>();
    assert_eq_hash::<wkt::UInt64Value>();
    assert_eq_hash::<wkt::BoolValue>();
    assert_eq_hash::<wkt::StringValue>();
    assert_eq_hash::<wkt::BytesValue>();
}

#[test]
fn timestamp_is_a_hash_set_and_map_key() {
    let timestamp = wkt::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_456_789,
        ..Default::default()
    };
    let decoded = wkt::Timestamp::decode_from_slice(&timestamp.encode_to_vec()).unwrap();
    assert_eq!(timestamp, decoded);
    assert_eq!(hash(&timestamp), hash(&decoded));

    let mut set = HashSet::new();
    assert!(set.insert(timestamp.clone()));
    assert!(!set.insert(decoded.clone()));
    assert!(set.insert(wkt::Timestamp::default()));

    let mut map = HashMap::new();
    map.insert(timestamp, "created");
    assert_eq!(map.get(&decoded), Some(&"created"));
    assert_eq!(map.insert(decoded, "updated"), Some("created"));
}

#[test]
fn any_hashes_equal_payloads_equally() {
    let first = wkt::Any {
        type_url: "type.googleapis.com/google.protobuf.Int32Value".into(),
        value: vec![0x08, 0x2a].into(),
        ..Default::default()
    };
    // Decode into a separate allocation: hashing must compare payload bytes.
    let second = wkt::Any::decode_from_slice(&first.encode_to_vec()).unwrap();
    assert_eq!(first, second);
    assert_eq!(hash(&first), hash(&second));
}

#[test]
fn unknown_fields_participate_in_equality_and_hashing() {
    let plain = wkt::Timestamp::default();
    // Unknown field 99, with varint values 7 and 8.
    let first = wkt::Timestamp::decode_from_slice(&[0x98, 0x06, 0x07]).unwrap();
    let same = wkt::Timestamp::decode_from_slice(&[0x98, 0x06, 0x07]).unwrap();
    let different = wkt::Timestamp::decode_from_slice(&[0x98, 0x06, 0x08]).unwrap();

    assert_eq!(first.seconds, plain.seconds);
    assert_eq!(first.nanos, plain.nanos);
    assert_eq!(first.__buffa_unknown_fields.len(), 1);
    assert_eq!(first, same);
    assert_eq!(hash(&first), hash(&same));
    assert_ne!(first, plain);
    assert_ne!(hash(&first), hash(&plain));
    assert_ne!(first, different);
    assert_ne!(hash(&first), hash(&different));
}

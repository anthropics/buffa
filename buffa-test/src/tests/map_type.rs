//! map_type(): configurable owned collection for `map` fields.
//!
//! `map_type.proto` is compiled with `.map_type(MapRepr::BTreeMap)`, so every
//! `map` field is a `BTreeMap<K, V>` (buffa-provided, no consumer code) instead
//! of `HashMap<K, V>`. Compiling `crate::map_type` is most of the test — the
//! merge (`storage_insert`), size/write (`storage_iter`/`storage_len`), JSON
//! skip (`is_empty_map`), reflect `has` (`MapStorage::storage_len`), and
//! view→owned (`.collect()`) paths must all work for the non-`HashMap`
//! container. The runtime checks below pin the field types, the binary / JSON /
//! view→owned round-trips, and `BTreeMap`'s deterministic encode order.
//!
//! It is compiled with `generate_json` + `generate_arbitrary` +
//! `bytes_type(Bytes)`, and includes `map<int64, int64>` and `map<int32, bytes>`
//! fields. Those non-string-key / non-derived-value combinations previously
//! required the `HashMap`-typed JSON with-modules and `arbitrary` shim; now that
//! both are generic over the container, they round-trip through `BTreeMap` —
//! the lift this fixture proves.

use crate::map_type::{Item, Maps};
use buffa::alloc::collections::BTreeMap;
use buffa::Message;

fn sample() -> Maps {
    let mut scores = BTreeMap::new();
    scores.insert("zeta".to_string(), 1);
    scores.insert("alpha".to_string(), -2);
    scores.insert("mu".to_string(), 300);

    let mut items = BTreeMap::new();
    items.insert(
        "one".to_string(),
        Item {
            id: 11,
            ..Default::default()
        },
    );
    items.insert(
        "two".to_string(),
        Item {
            id: 22,
            ..Default::default()
        },
    );

    let mut big_scores = BTreeMap::new();
    big_scores.insert(9_000_000_000i64, -1i64);
    big_scores.insert(-42i64, 7i64);

    let mut blobs = BTreeMap::new();
    blobs.insert(1i32, bytes::Bytes::from_static(b"\x00\x01\xff"));
    blobs.insert(-5i32, bytes::Bytes::from_static(b"hi"));

    Maps {
        scores,
        items,
        big_scores,
        blobs,
        ..Default::default()
    }
}

#[test]
fn field_types_are_btreemap() {
    // Fails to compile if codegen emitted the wrong collection for either the
    // scalar-valued or the message-valued map.
    let m = Maps::default();
    let _: &BTreeMap<buffa::alloc::string::String, i32> = &m.scores;
    let _: &BTreeMap<buffa::alloc::string::String, Item> = &m.items;
    let _: &BTreeMap<i64, i64> = &m.big_scores;
    // `bytes_type(Bytes)` promotes the map value to `Bytes`.
    let _: &BTreeMap<i32, bytes::Bytes> = &m.blobs;
}

#[test]
fn binary_round_trip() {
    let msg = sample();
    let bytes = msg.encode_to_vec();
    let decoded = Maps::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    // Spot-check the merge populated the BTreeMap.
    assert_eq!(decoded.scores.get("mu"), Some(&300));
    assert_eq!(decoded.items.get("two").map(|i| i.id), Some(22));
}

#[test]
fn empty_round_trips_clean() {
    let msg = Maps::default();
    let bytes = msg.encode_to_vec();
    assert!(bytes.is_empty(), "empty map fields encode to nothing");
    let decoded = Maps::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
}

#[test]
fn btreemap_encode_order_is_deterministic() {
    // The headline reason to pick BTreeMap: stable key order means stable wire
    // bytes across runs. HashMap would not guarantee this.
    let msg = sample();
    let a = msg.encode_to_vec();
    let b = msg.encode_to_vec();
    assert_eq!(a, b, "BTreeMap encode order is stable");

    // The scalar map's entries are emitted in sorted key order. Decode the keys
    // back in wire order and confirm they ascend.
    let only_scores = Maps {
        scores: msg.scores.clone(),
        ..Default::default()
    };
    let decoded = Maps::decode(&mut only_scores.encode_to_vec().as_slice()).expect("decode");
    let keys: buffa::alloc::vec::Vec<&str> = decoded.scores.keys().map(|s| s.as_str()).collect();
    assert_eq!(keys, ["alpha", "mu", "zeta"]);
}

#[test]
fn view_to_owned_round_trip() {
    // Exercises the view→owned path: entries `.collect()` into the BTreeMap
    // (not `HashMap`), and message values convert via `to_owned_from_source`.
    let bytes = bytes::Bytes::from(sample().encode_to_vec());
    let owned: Maps = crate::map_type::MapsOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message()
        .expect("to_owned");
    assert_eq!(owned, sample());
}

#[test]
fn json_round_trip_and_empty_skip() {
    // The `is_empty_map` skip predicate must omit empty maps from JSON, and a
    // populated BTreeMap must serialize/deserialize through serde.
    let empty = Maps::default();
    let empty_json = serde_json::to_string(&empty).expect("serialize empty");
    assert_eq!(empty_json, "{}", "empty maps are skipped in JSON");

    let msg = sample();
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: Maps = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn non_string_key_and_bytes_value_json_round_trip() {
    // The lift: `map<int64, int64>` (non-string key + quoted int64 value) and
    // `map<int32, bytes>` (non-string key + base64 value) route through the
    // `string_key_map` / `proto_map` JSON with-modules, which are now generic
    // over the container — so they round-trip through `BTreeMap`. Before the
    // associated-types change this combination did not compile with a
    // non-`HashMap` container.
    let msg = sample();
    let json = serde_json::to_string(&msg).expect("serialize");

    // Proto3 JSON renders all map keys as strings and int64 values as quoted
    // strings; bytes values as base64.
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(v["bigScores"]["9000000000"], serde_json::json!("-1"));
    assert_eq!(v["blobs"]["1"], serde_json::json!("AAH/")); // base64 of 00 01 ff

    let back: Maps = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.big_scores, msg.big_scores);
    assert_eq!(back.blobs, msg.blobs);
}

#[cfg(feature = "arbitrary")]
mod arbitrary_tests {
    use crate::map_type::Maps;
    use arbitrary::{Arbitrary, Unstructured};

    /// The generic `arbitrary_proto_bytes_map` shim must build a `BTreeMap<i32,
    /// Bytes>` for the `map<int32, bytes>` field (value promoted to `Bytes` by
    /// `bytes_type`). Building one `Maps` and touching the field via `slice(..)`
    /// (a `Bytes`-specific method) pins that the shim ran and produced `Bytes`
    /// values in a non-`HashMap` container.
    #[test]
    fn btreemap_bytes_value_arbitrary() {
        let raw: [u8; 256] = core::array::from_fn(|i| i as u8);
        let mut u = Unstructured::new(&raw);
        let msg = Maps::arbitrary(&mut u).unwrap();
        for b in msg.blobs.values() {
            let _ = b.slice(..);
        }
    }
}

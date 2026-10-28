//! map_type_custom(): a crate-local `CustomMap<K, V>` newtype for every `map`
//! field, the worked example of the `MapRepr::Custom` path.
//!
//! `map_type_custom.proto` is compiled with
//! `.map_type_custom("crate::map_type_custom::CustomMap")`, so every `map` field
//! is a `CustomMap<K, V>` (a crate-local `MapStorage` impl) instead of
//! `HashMap`. Compiling `crate::map_type_custom` is most of the test — the merge
//! (`storage_insert`), size/write (`storage_iter`/`storage_len`), clear
//! (`storage_clear`), reflect `has` (`MapStorage::storage_len`) and `get`
//! (consumer `ReflectMap`), and view→owned (`FromIterator`) paths must all emit
//! code that resolves against the consumer-provided trait impls. The runtime
//! checks pin the field types and the binary / view→owned / clear / text
//! round-trips.

use crate::map_type_custom::{CustomMaps, Item};
use buffa::Message;

fn sample() -> CustomMaps {
    // `.into_iter().collect()` builds the CustomMap via its `FromIterator` impl —
    // the same path the view→owned conversion uses.
    let scores = [("alpha".to_string(), -2), ("mu".to_string(), 300)]
        .into_iter()
        .collect();
    let items = [
        (
            "one".to_string(),
            Item {
                id: 11,
                ..Default::default()
            },
        ),
        (
            "two".to_string(),
            Item {
                id: 22,
                ..Default::default()
            },
        ),
    ]
    .into_iter()
    .collect();
    CustomMaps {
        scores,
        items,
        ..Default::default()
    }
}

#[test]
fn field_types_are_custom_map() {
    // Fails to compile if codegen emitted the wrong collection for either map.
    let m = CustomMaps::default();
    let _: &crate::map_type_custom::CustomMap<buffa::alloc::string::String, i32> = &m.scores;
    let _: &crate::map_type_custom::CustomMap<buffa::alloc::string::String, Item> = &m.items;
}

#[test]
fn binary_round_trip() {
    let msg = sample();
    let bytes = msg.encode_to_vec();
    let decoded = CustomMaps::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    // Spot-check the merge populated the custom container.
    assert_eq!(decoded.scores.0.get("mu"), Some(&300));
    assert_eq!(decoded.items.0.get("two").map(|i| i.id), Some(22));
}

#[test]
fn clear_empties_the_custom_map() {
    // Exercises the generated `clear()` routing through `MapStorage::storage_clear`.
    let mut msg = sample();
    msg.clear();
    assert_eq!(msg, CustomMaps::default());
    assert_eq!(msg.scores.0.len(), 0);
    assert_eq!(msg.items.0.len(), 0);
}

#[test]
fn view_to_owned_round_trip() {
    // Entries `.collect()` into CustomMap via FromIterator; message values
    // convert via `to_owned_from_source`.
    let bytes = bytes::Bytes::from(sample().encode_to_vec());
    let owned: CustomMaps = crate::map_type_custom::CustomMapsOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message();
    assert_eq!(owned, sample());
}

#[test]
fn text_round_trip() {
    // The text encoder iterates through `MapStorage::storage_iter` (in the inner
    // `BTreeMap`'s key order) and the decoder inserts through `storage_insert`.
    let msg = sample();
    let text = buffa::text::encode_to_string(&msg);
    assert_eq!(
        text,
        r#"scores {key: "alpha" value: -2} scores {key: "mu" value: 300} items {key: "one" value {id: 11}} items {key: "two" value {id: 22}}"#
    );
    let decoded: CustomMaps = buffa::text::decode_from_str(&text).expect("decode text");
    assert_eq!(decoded, msg);
}

#[test]
fn text_merge_inserts_last_write_wins() {
    // The list form and a repeated key: the later entry replaces the earlier one,
    // and entries already in the map stay.
    let mut msg = sample();
    buffa::text::merge_from_str(
        &mut msg,
        r#"scores: [{key: "mu" value: 1}, {key: "nu" value: 2}] scores {key: "nu" value: 3}"#,
    )
    .expect("merge text");
    assert_eq!(msg.scores.0.get("alpha"), Some(&-2));
    assert_eq!(msg.scores.0.get("mu"), Some(&1));
    assert_eq!(msg.scores.0.get("nu"), Some(&3));
    assert_eq!(msg.scores.0.len(), 3);
}

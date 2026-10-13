//! JSON through generated messages whose custom string type takes its serde
//! impls from the remote derive's `serde` key.
//!
//! `LocalStr` (`vtable_string_repr`) and `MapStr` (`string_map`) both carry
//! `#[buffa(remote = String, serde)]`. Generated code calls those impls for a
//! `repeated` element, the `Some` of an `optional` field, a oneof variant, and
//! a map key or value. A singular field goes through the `proto_string`
//! with-module instead.

use crate::string_map::{MapStr, Maps};
use crate::vtable_string_repr::__buffa::oneof::labels::Choice;
use crate::vtable_string_repr::{Labels, LocalStr};

#[test]
fn repeated_optional_and_oneof_fields_round_trip() {
    let labels = Labels {
        name: LocalStr::from("svc"),
        items: vec![LocalStr::from("a"), LocalStr::from("b")],
        alias: Some(LocalStr::from("present")),
        choice: Some(Choice::Tag(LocalStr::from("chosen"))),
        ..Default::default()
    };

    let json = serde_json::to_value(&labels).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "name": "svc",
            "items": ["a", "b"],
            "alias": "present",
            "tag": "chosen",
        })
    );
    // `from_value` hands the visitor owned strings, `from_str` borrowed ones.
    assert_eq!(
        serde_json::from_value::<Labels>(json.clone()).expect("deserialize owned"),
        labels
    );
    assert_eq!(
        serde_json::from_str::<Labels>(&json.to_string()).expect("deserialize borrowed"),
        labels
    );
}

#[test]
fn map_keys_and_values_round_trip() {
    let maps = Maps {
        ss: [(MapStr::from("k"), MapStr::from("v"))]
            .into_iter()
            .collect(),
        i32s: [(7, MapStr::from("seven"))].into_iter().collect(),
        ..Default::default()
    };

    let json = serde_json::to_value(&maps).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({"ss": {"k": "v"}, "i32s": {"7": "seven"}})
    );
    assert_eq!(
        serde_json::from_value::<Maps>(json.clone()).expect("deserialize owned"),
        maps
    );
    assert_eq!(
        serde_json::from_str::<Maps>(&json.to_string()).expect("deserialize borrowed"),
        maps
    );
}

#[test]
fn null_repeated_element_is_rejected() {
    for json in [r#"{"items":[null]}"#, r#"{"items":["a",null]}"#] {
        let err = serde_json::from_str::<Labels>(json).expect_err("null element is rejected");
        assert!(
            err.to_string()
                .contains("invalid type: null, expected a string"),
            "{json}: {err}"
        );
    }
}

#[test]
fn non_string_repeated_element_is_rejected() {
    let err = serde_json::from_str::<Labels>(r#"{"items":[1]}"#).expect_err("a number");
    assert!(
        err.to_string()
            .contains("invalid type: integer `1`, expected a string"),
        "{err}"
    );
}

#[test]
fn null_map_value_is_rejected() {
    // `ss` deserializes through the container's own serde, `i32s` through the
    // `string_key_map` with-module. Both call `MapStr`'s `Deserialize`.
    for json in [r#"{"ss":{"k":null}}"#, r#"{"i32s":{"1":null}}"#] {
        let err = serde_json::from_str::<Maps>(json).expect_err("null value is rejected");
        assert!(
            err.to_string()
                .contains("invalid type: null, expected a string"),
            "{json}: {err}"
        );
    }
}

#[test]
fn null_field_is_the_default() {
    let labels: Labels =
        serde_json::from_str(r#"{"name":null,"items":null,"alias":null,"tag":null}"#)
            .expect("null fields are accepted");
    assert_eq!(labels, Labels::default());
    assert_eq!(labels.alias, None);
    // A `null` oneof variant leaves the oneof unset.
    assert_eq!(labels.choice, None);

    let maps: Maps = serde_json::from_str(r#"{"ss":null,"i32s":null}"#).expect("null maps");
    assert_eq!(maps, Maps::default());
}

#![cfg(feature = "json")]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct SerdeValues {
    #[serde(with = "buffa::json_helpers::bytes_key_map")]
    entries: BTreeMap<Vec<u8>, serde_json::Value>,
}

#[test]
fn generic_bytes_key_map_accepts_external_serde_values() {
    let value = SerdeValues {
        entries: [(
            b"test".to_vec(),
            serde_json::json!({"nested": [1, true, null]}),
        )]
        .into(),
    };
    let encoded = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<SerdeValues>(&encoded).unwrap(),
        value
    );
    assert_eq!(
        serde_json::from_str::<SerdeValues>(r#"{"entries":null}"#)
            .unwrap()
            .entries
            .len(),
        0
    );
}

#[test]
fn generic_bytes_key_map_keeps_null_serde_values() {
    let parsed: SerdeValues = serde_json::from_str(r#"{"entries":{"dGVzdA==":null}}"#).unwrap();
    assert!(parsed.entries[b"test".as_slice()].is_null());
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct BorrowedValues<'a> {
    #[serde(borrow, with = "buffa::json_helpers::bytes_key_map")]
    entries: BTreeMap<Vec<u8>, &'a str>,
}

#[test]
fn generic_bytes_key_map_preserves_borrowed_serde_values() {
    let encoded = r#"{"entries":{"dGVzdA==":"borrowed"}}"#;
    let parsed: BorrowedValues<'_> = serde_json::from_str(encoded).unwrap();
    assert_eq!(parsed.entries[b"test".as_slice()], "borrowed");
    assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
}

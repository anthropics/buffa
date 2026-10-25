#![cfg(feature = "json")]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct NumericValues {
    #[serde(with = "buffa::json_helpers::bytes_key_map")]
    entries: BTreeMap<Vec<u8>, i64>,
}

#[test]
fn generic_bytes_key_map_preserves_serde_integer_shape() {
    let value = NumericValues {
        entries: [(b"test".to_vec(), 42)].into(),
    };
    let encoded = serde_json::to_string(&value).unwrap();
    assert_eq!(encoded, r#"{"entries":{"dGVzdA==":42}}"#);
    assert_eq!(
        serde_json::from_str::<NumericValues>(&encoded).unwrap(),
        value
    );
}

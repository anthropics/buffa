#![cfg(feature = "json")]

use serde::Serialize;

#[derive(Serialize)]
struct Signed {
    #[serde(with = "buffa::json_helpers::int64")]
    value: i64,
}

#[derive(Serialize)]
struct Unsigned {
    #[serde(with = "buffa::json_helpers::uint64")]
    value: u64,
}

#[test]
fn signed_int64_serializes_as_quoted_decimal() {
    for value in [
        i64::MIN,
        -9_223_372_036_854_775_807,
        -1,
        0,
        1,
        127,
        128,
        i64::MAX,
    ] {
        let json = serde_json::to_string(&Signed { value }).unwrap();
        assert_eq!(json, format!(r#"{{"value":"{value}"}}"#));
    }
}

#[test]
fn unsigned_uint64_serializes_as_quoted_decimal() {
    for value in [0, 1, 127, 128, u32::MAX as u64, u64::MAX - 1, u64::MAX] {
        let json = serde_json::to_string(&Unsigned { value }).unwrap();
        assert_eq!(json, format!(r#"{{"value":"{value}"}}"#));
    }
}

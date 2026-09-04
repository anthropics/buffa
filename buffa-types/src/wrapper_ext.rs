//! Ergonomic `From`/`Into` impls for the `google.protobuf` wrapper types.
//!
//! Each wrapper message has a single `value` field.  These impls allow seamless
//! conversion between the wrapper message and the underlying primitive.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::google::protobuf::{
    BoolValue, BytesValue, DoubleValue, FloatValue, Int32Value, Int64Value, StringValue,
    UInt32Value, UInt64Value,
};

macro_rules! impl_wrapper {
    ($wrapper:ty, $inner:ty) => {
        impl From<$inner> for $wrapper {
            #[doc = concat!("Wraps `", stringify!($inner), "` in [`", stringify!($wrapper), "`].")]
            fn from(v: $inner) -> Self {
                Self {
                    value: v,
                    ..Default::default()
                }
            }
        }

        impl From<$wrapper> for $inner {
            #[doc = concat!("Extracts the inner `", stringify!($inner), "` from [`", stringify!($wrapper), "`].")]
            fn from(w: $wrapper) -> Self {
                w.value
            }
        }
    };
}

impl_wrapper!(BoolValue, bool);
impl_wrapper!(DoubleValue, f64);
impl_wrapper!(FloatValue, f32);
impl_wrapper!(Int32Value, i32);
impl_wrapper!(Int64Value, i64);
impl_wrapper!(UInt32Value, u32);
impl_wrapper!(UInt64Value, u64);
impl_wrapper!(StringValue, String);
impl_wrapper!(BytesValue, Vec<u8>);

impl From<&str> for StringValue {
    /// Converts a string slice into a [`StringValue`], allocating a new `String`.
    fn from(s: &str) -> Self {
        Self {
            value: s.to_string(),
            ..Default::default()
        }
    }
}

impl From<&[u8]> for BytesValue {
    /// Converts a byte slice into a [`BytesValue`], copying the bytes.
    fn from(b: &[u8]) -> Self {
        Self {
            value: b.to_vec(),
            ..Default::default()
        }
    }
}

impl AsRef<str> for StringValue {
    /// Borrows the inner string slice, allowing `StringValue` to be passed to
    /// any function that accepts `&str` (e.g. via `.as_ref()`).
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl AsRef<[u8]> for BytesValue {
    /// Borrows the inner byte slice, allowing `BytesValue` to be passed to
    /// any function that accepts `&[u8]` (e.g. via `.as_ref()`).
    fn as_ref(&self) -> &[u8] {
        &self.value
    }
}

// ── serde impls ──────────────────────────────────────────────────────────────

// Primitive wrapper values use the same ProtoJSON scalar helpers as generated
// fields, including their handling of null.
#[cfg(feature = "json")]
impl serde::Serialize for BoolValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for BoolValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::proto_bool::deserialize(d).map(Self::from)
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for StringValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for StringValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::proto_string::deserialize::<String, _>(d).map(Self::from)
    }
}

// Int32Value / UInt32Value: ProtoJSON accepts integer numbers, quoted decimal
// strings, exact decimal/exponent forms, and null. Reuse the same helpers as
// generated scalar fields so wrappers follow the runtime's integer policy.
#[cfg(feature = "json")]
impl serde::Serialize for Int32Value {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Int32Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::int32::deserialize(d).map(Self::from)
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for UInt32Value {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for UInt32Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::uint32::deserialize(d).map(Self::from)
    }
}

// Int64Value: quoted decimal string per proto3 JSON spec.
#[cfg(feature = "json")]
impl serde::Serialize for Int64Value {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::int64::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Int64Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::int64::deserialize(d).map(Self::from)
    }
}

// UInt64Value: quoted decimal string per proto3 JSON spec.
#[cfg(feature = "json")]
impl serde::Serialize for UInt64Value {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::uint64::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for UInt64Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::uint64::deserialize(d).map(Self::from)
    }
}

// FloatValue: number, or "NaN" / "Infinity" / "-Infinity" per proto3 JSON spec.
#[cfg(feature = "json")]
impl serde::Serialize for FloatValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::float::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for FloatValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::float::deserialize(d).map(Self::from)
    }
}

// DoubleValue: number, or "NaN" / "Infinity" / "-Infinity" per proto3 JSON spec.
#[cfg(feature = "json")]
impl serde::Serialize for DoubleValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::double::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for DoubleValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        buffa::json_helpers::double::deserialize(d).map(Self::from)
    }
}

// BytesValue: base64-encoded string per proto3 JSON spec.
#[cfg(feature = "json")]
impl serde::Serialize for BytesValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::bytes::serialize(&self.value, s)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for BytesValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // The helper is generic over T: From<Vec<u8>>; pin T = Vec<u8> since
        // BytesValue has multiple From impls that would otherwise be ambiguous.
        buffa::json_helpers::bytes::deserialize::<Vec<u8>, _>(d).map(Self::from)
    }
}

#[cfg(feature = "json")]
macro_rules! impl_wrapper_proto_elem_json {
    ($wrapper:ty) => {
        impl buffa::json_helpers::ProtoElemJson for $wrapper {
            fn serialize_proto_json<S: serde::Serializer>(
                value: &Self,
                s: S,
            ) -> Result<S::Ok, S::Error> {
                serde::Serialize::serialize(value, s)
            }

            fn deserialize_proto_json<'de, D: serde::Deserializer<'de>>(
                d: D,
            ) -> Result<Self, D::Error> {
                <Self as serde::Deserialize>::deserialize(d)
            }
        }
    };
}

#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(BoolValue);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(BytesValue);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(DoubleValue);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(FloatValue);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(Int32Value);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(Int64Value);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(StringValue);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(UInt32Value);
#[cfg(feature = "json")]
impl_wrapper_proto_elem_json!(UInt64Value);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_value_roundtrip() {
        let w = BoolValue::from(true);
        assert!(w.value);
        let back: bool = w.into();
        assert!(back);
    }

    #[test]
    fn double_value_roundtrip() {
        let w = DoubleValue::from(2.5_f64);
        assert_eq!(w.value, 2.5);
        let back: f64 = w.into();
        assert_eq!(back, 2.5);
    }

    #[test]
    fn float_value_roundtrip() {
        let w = FloatValue::from(1.5_f32);
        assert_eq!(w.value, 1.5);
        let back: f32 = w.into();
        assert_eq!(back, 1.5);
    }

    #[test]
    fn int32_value_roundtrip() {
        let w = Int32Value::from(-42_i32);
        assert_eq!(w.value, -42);
        let back: i32 = w.into();
        assert_eq!(back, -42);
    }

    #[test]
    fn int64_value_roundtrip() {
        let w = Int64Value::from(i64::MAX);
        assert_eq!(w.value, i64::MAX);
        let back: i64 = w.into();
        assert_eq!(back, i64::MAX);
    }

    #[test]
    fn uint32_value_roundtrip() {
        let w = UInt32Value::from(100_u32);
        assert_eq!(w.value, 100);
        let back: u32 = w.into();
        assert_eq!(back, 100);
    }

    #[test]
    fn uint64_value_roundtrip() {
        let w = UInt64Value::from(u64::MAX);
        assert_eq!(w.value, u64::MAX);
        let back: u64 = w.into();
        assert_eq!(back, u64::MAX);
    }

    #[test]
    fn string_value_roundtrip() {
        let w = StringValue::from("hello".to_string());
        assert_eq!(w.value, "hello");
        let back: String = w.into();
        assert_eq!(back, "hello");
    }

    #[test]
    fn string_value_from_str_ref() {
        let w = StringValue::from("hello");
        assert_eq!(w.value, "hello");
    }

    #[test]
    fn string_value_as_ref_str() {
        let w = StringValue::from("world".to_string());
        // AsRef<str> allows passing to functions that accept &str.
        fn takes_str(s: &str) -> usize {
            s.len()
        }
        assert_eq!(takes_str(w.as_ref()), 5);
    }

    #[test]
    fn bytes_value_roundtrip() {
        let w = BytesValue::from(vec![1_u8, 2, 3]);
        assert_eq!(w.value, vec![1, 2, 3]);
        let back: Vec<u8> = w.into();
        assert_eq!(back, vec![1, 2, 3]);
    }

    #[test]
    fn bytes_value_from_slice() {
        let w = BytesValue::from(&[4_u8, 5, 6][..]);
        assert_eq!(w.value, vec![4, 5, 6]);
    }

    #[test]
    fn bytes_value_as_ref_u8_slice() {
        let w = BytesValue::from(vec![10_u8, 20, 30]);
        fn takes_bytes(b: &[u8]) -> usize {
            b.len()
        }
        assert_eq!(takes_bytes(w.as_ref()), 3);
    }

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        #[test]
        fn bool_value_serde_roundtrip() {
            let w = BoolValue::from(true);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, "true");
            let back: BoolValue = serde_json::from_str(&json).unwrap();
            assert!(back.value);
        }

        #[test]
        fn bool_value_accepts_protojson_null() {
            let value: BoolValue = serde_json::from_str("null").unwrap();
            assert!(!value.value);
        }

        #[test]
        fn int32_value_serde_roundtrip() {
            let w = Int32Value::from(-42_i32);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, "-42");
            let back: Int32Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, -42);
        }

        #[test]
        fn int32_value_accepts_protojson_integer_forms() {
            for (json, expected) in [
                ("1", 1),
                (r#""1""#, 1),
                ("1.0", 1),
                (r#""1.0""#, 1),
                ("1e2", 100),
                (r#""1e2""#, 100),
                ("null", 0),
            ] {
                let value: Int32Value = serde_json::from_str(json).unwrap();
                assert_eq!(value.value, expected, "input: {json}");
            }
        }

        #[test]
        fn int32_value_rejects_invalid_protojson_integers() {
            for json in ["1.5", r#""1.5""#, "2147483648", r#""""#, r#""2147483648""#] {
                assert!(
                    serde_json::from_str::<Int32Value>(json).is_err(),
                    "input should be rejected: {json}"
                );
            }
        }

        #[test]
        fn uint32_value_serde_roundtrip() {
            let w = UInt32Value::from(100_u32);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, "100");
            let back: UInt32Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, 100);
        }

        #[test]
        fn uint32_value_accepts_protojson_integer_forms() {
            for (json, expected) in [
                ("1", 1),
                (r#""1""#, 1),
                ("1.0", 1),
                (r#""1.0""#, 1),
                ("1e2", 100),
                (r#""1e2""#, 100),
                ("4294967295", u32::MAX),
                ("null", 0),
            ] {
                let value: UInt32Value = serde_json::from_str(json).unwrap();
                assert_eq!(value.value, expected, "input: {json}");
            }
        }

        #[test]
        fn uint32_value_rejects_invalid_protojson_integers() {
            for json in [
                "-1",
                r#""-1""#,
                "1.5",
                r#""1.5""#,
                "4294967296",
                r#""4294967296""#,
                r#""""#,
            ] {
                assert!(
                    serde_json::from_str::<UInt32Value>(json).is_err(),
                    "input should be rejected: {json}"
                );
            }
        }

        #[test]
        fn string_value_serde_roundtrip() {
            let w = StringValue::from("hello".to_string());
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, r#""hello""#);
            let back: StringValue = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, "hello");
        }

        #[test]
        fn string_value_accepts_protojson_null() {
            let value: StringValue = serde_json::from_str("null").unwrap();
            assert_eq!(value.value, "");
        }

        #[test]
        fn int64_value_serializes_as_quoted_string() {
            let w = Int64Value::from(i64::MAX);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, r#""9223372036854775807""#);
            let back: Int64Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, i64::MAX);
        }

        #[test]
        fn uint64_value_serializes_as_quoted_string() {
            let w = UInt64Value::from(u64::MAX);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, r#""18446744073709551615""#);
            let back: UInt64Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, u64::MAX);
        }

        #[test]
        fn float_value_serde_roundtrip() {
            let w = FloatValue::from(1.5_f32);
            let json = serde_json::to_string(&w).unwrap();
            let back: FloatValue = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, 1.5_f32);
        }

        #[test]
        fn float_value_nan_as_string() {
            let w = FloatValue::from(f32::NAN);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, r#""NaN""#);
        }

        #[test]
        fn double_value_serde_roundtrip() {
            let w = DoubleValue::from(2.5_f64);
            let json = serde_json::to_string(&w).unwrap();
            let back: DoubleValue = serde_json::from_str(&json).unwrap();
            assert!((back.value - 2.5).abs() < 1e-10);
        }

        #[test]
        fn double_value_infinity_as_string() {
            let w = DoubleValue::from(f64::INFINITY);
            let json = serde_json::to_string(&w).unwrap();
            assert_eq!(json, r#""Infinity""#);
        }

        #[test]
        fn bytes_value_serde_as_base64() {
            let w = BytesValue::from(vec![1_u8, 2, 3]);
            let json = serde_json::to_string(&w).unwrap();
            // base64 of [1, 2, 3] is "AQID"
            assert_eq!(json, r#""AQID""#);
            let back: BytesValue = serde_json::from_str(&json).unwrap();
            assert_eq!(back.value, vec![1_u8, 2, 3]);
        }
    }
}

//! Buffering a JSON value before its proto type is known.
//!
//! Some proto3 JSON shapes cannot be decoded in one pass: an `Any` names its
//! type in `@type`, which may follow the fields it describes, and an element
//! of an enum list is dropped instead of rejected when
//! `ignore_unknown_enum_values` is set. Those paths hold the value as a
//! [`serde_json::Value`] first.
//!
//! `serde_json`'s own `Deserialize` impl for `Value` must not be used for
//! that on untrusted input. When the `raw_value` feature of `serde_json` is
//! enabled by any crate in the build, that impl reads an object whose first
//! key is `$serde_json::private::RawValue` as "parse the string under this
//! key as JSON". The decoded value then differs from the one that a filter,
//! a log or a signature check sees in the request text. Each such string is
//! also parsed with a new recursion limit, so nesting them removes the bound
//! on depth.
//!
//! [`BufferedValue`] and [`BufferedObject`] build the same `Value` and read
//! every object key as data, at every depth. The depth of what they build is
//! bounded by the deserializer that feeds them.
//!
//! With the `arbitrary_precision` feature of `serde_json`, a number that is
//! not an integer in the `i64` or `u64` range reaches a visitor as an object
//! with a private key, and is buffered here as that object. `buffa`'s JSON
//! decoding does not support that feature: its `float` and `double` helpers
//! reject such numbers. `serde_json::Value` reads that object as a number,
//! so with the feature on, such a number did decode in an `Any` payload and
//! in an extension value when they were buffered as `serde_json::Value`. It
//! is now rejected there too, and a `google.protobuf.Value` payload reads it
//! as an object. Reading that key as a number here would let an object in
//! the JSON text decode as a number in a build without the feature.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// One JSON value of any kind, with every object key read as data.
pub struct BufferedValue(pub Value);

/// One JSON object, with every key read as data at every depth.
///
/// Accepts what `serde_json::Map<String, Value>` accepts.
pub struct BufferedObject(pub Map<String, Value>);

impl<'de> Deserialize<'de> for BufferedValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor).map(Self)
    }
}

impl<'de> Deserialize<'de> for BufferedObject {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(ObjectVisitor).map(Self)
    }
}

fn read_object<'de, A: MapAccess<'de>>(mut map: A) -> Result<Map<String, Value>, A::Error> {
    let mut out = Map::new();
    while let Some(key) = map.next_key::<String>()? {
        let BufferedValue(value) = map.next_value()?;
        out.insert(key, value);
    }
    Ok(out)
}

struct ObjectVisitor;

impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = Map<String, Value>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a map")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Map::new())
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        read_object(map)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_i128<E: Error>(self, v: i128) -> Result<Value, E> {
        Number::deserialize(serde::de::value::I128Deserializer::new(v)).map(Value::Number)
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u128<E: Error>(self, v: u128) -> Result<Value, E> {
        Number::deserialize(serde::de::value::U128Deserializer::new(v)).map(Value::Number)
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        // A non-finite float has no JSON form; `serde_json` buffers it as
        // `null`, and so does this.
        Ok(Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(String::from(v)))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        BufferedValue::deserialize(d).map(|BufferedValue(v)| v)
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(BufferedValue(element)) = seq.next_element()? {
            out.push(element);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Value, A::Error> {
        read_object(map).map(Value::Object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;
    use serde_json::json;

    const RAW_VALUE_KEY: &str = "$serde_json::private::RawValue";

    fn buffer(text: &str) -> serde_json::Result<Value> {
        serde_json::from_str(text).map(|BufferedValue(v)| v)
    }

    fn depth(v: &Value) -> usize {
        match v {
            Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
            Value::Object(o) => 1 + o.values().map(depth).max().unwrap_or(0),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
        }
    }

    /// `levels` objects with the private key, each inside `arrays` arrays and
    /// holding the next level as a JSON string.
    fn nested_raw_values(levels: usize, arrays: usize) -> alloc::string::String {
        let mut inner = "0".to_string();
        for _ in 0..levels {
            let object = json!({ RAW_VALUE_KEY: inner }).to_string();
            inner = format!("{}{object}{}", "[".repeat(arrays), "]".repeat(arrays));
        }
        inner
    }

    /// The tests below pass with the fix reverted unless `serde_json` has
    /// `raw_value` on. It is a dev-dependency feature of this crate, so a
    /// test build always has it.
    #[test]
    fn the_test_build_has_serde_json_raw_value_enabled() {
        let text = json!({ RAW_VALUE_KEY: "[1]" }).to_string();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value, json!([1]));
    }

    #[test]
    fn ordinary_json_buffers_as_serde_json_does() {
        for text in [
            "null",
            "true",
            "0",
            "-1",
            "18446744073709551615",
            "-9223372036854775808",
            "1.5",
            "1e400",
            r#""a\"é\n""#,
            "[]",
            "{}",
            r#"[1, [2, [3, {"a": [null]}]], "x"]"#,
            r#"{"a": 1, "b": {"c": [true, false]}, "a": 2}"#,
            r#"{"$serde_json::private::Number": "1"}"#,
        ] {
            let expected = serde_json::from_str::<Value>(text).map_err(|e| e.to_string());
            let got = buffer(text).map_err(|e| e.to_string());
            assert_eq!(got, expected, "{text}");
        }
    }

    #[test]
    fn the_raw_value_key_is_data_in_json_text() {
        let text = json!({ RAW_VALUE_KEY: "[1]" }).to_string();
        let mut expected = Map::new();
        expected.insert(RAW_VALUE_KEY.to_string(), Value::String("[1]".to_string()));
        assert_eq!(buffer(&text).unwrap(), Value::Object(expected.clone()));

        let in_list = format!("[{text}]");
        assert_eq!(
            buffer(&in_list).unwrap(),
            Value::Array(alloc::vec![Value::Object(expected.clone())])
        );

        let BufferedObject(object) = serde_json::from_str(&format!(r#"{{"f": {text}}}"#)).unwrap();
        assert_eq!(object["f"], Value::Object(expected));
    }

    #[test]
    fn the_raw_value_key_is_data_in_a_value_that_is_already_built() {
        // A payload buffered once is decoded again from the `Value`, where
        // `serde_json`'s own impl would parse the string.
        let built = json!({ "f": [{ RAW_VALUE_KEY: "[1]" }] });
        let BufferedValue(again) = BufferedValue::deserialize(built.clone()).unwrap();
        assert_eq!(again, built);
        let BufferedObject(object) = BufferedObject::deserialize(built.clone()).unwrap();
        assert_eq!(Value::Object(object), built);
    }

    #[test]
    fn nested_raw_value_strings_do_not_add_depth() {
        // Parsed as JSON, this is 3 * 121 levels deep. As data it is 120
        // arrays around one object.
        let text = nested_raw_values(3, 120);
        let value = buffer(&text).unwrap();
        assert_eq!(depth(&value), 121);

        let too_deep = nested_raw_values(1, 128);
        let err = buffer(&too_deep).unwrap_err();
        assert!(err.to_string().contains("recursion limit"), "{err}");
    }

    #[test]
    fn an_object_buffer_accepts_what_a_serde_json_map_accepts() {
        for text in ["{}", r#"{"a": [1]}"#, "null", "[]", "1", r#""x""#, "true"] {
            let expected =
                serde_json::from_str::<Map<String, Value>>(text).map_err(|e| e.to_string());
            let got = serde_json::from_str(text)
                .map(|BufferedObject(o)| o)
                .map_err(|e| e.to_string());
            assert_eq!(got, expected, "{text}");

            // The same input as a `Value` that is already built.
            let built: Value = serde_json::from_str(text).unwrap();
            let expected =
                Map::<String, Value>::deserialize(built.clone()).map_err(|e| e.to_string());
            let got = BufferedObject::deserialize(built)
                .map(|BufferedObject(o)| o)
                .map_err(|e| e.to_string());
            assert_eq!(got, expected, "{text}");
        }
    }

    #[test]
    fn a_float_with_no_json_form_buffers_as_null() {
        use serde::de::value::{Error, F64Deserializer};
        for v in [f64::NAN, f64::INFINITY] {
            let BufferedValue(value) =
                BufferedValue::deserialize(F64Deserializer::<Error>::new(v)).unwrap();
            assert_eq!(value, Value::Null);
        }
    }

    #[test]
    fn wide_integers_buffer_as_serde_json_does() {
        use serde::de::value::{Error, I128Deserializer, U128Deserializer};
        let small = BufferedValue::deserialize(I128Deserializer::<Error>::new(-5)).unwrap();
        assert_eq!(small.0, json!(-5));
        let small = BufferedValue::deserialize(U128Deserializer::<Error>::new(5)).unwrap();
        assert_eq!(small.0, json!(5));
        let wide = U128Deserializer::<Error>::new(u128::MAX);
        assert_eq!(
            BufferedValue::deserialize(wide).map(|v| v.0).ok(),
            Value::deserialize(U128Deserializer::<Error>::new(u128::MAX)).ok(),
        );
    }
}

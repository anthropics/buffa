//! JSON values buffered with every object key read as data.
//!
//! Use this module wherever a `Deserialize` impl keeps untrusted input as a
//! [`serde_json::Value`] before it decodes it:
//!
//! - In a hand-written visitor, read a [`BufferedValue`], as in
//!   `let BufferedValue(value) = map.next_value()?;`. Read a
//!   [`BufferedObject`] when the input must be an object.
//! - On a field of a derived impl, name [`value`], [`opt_value`] or
//!   [`object`] in `#[serde(deserialize_with = "...")]`.
//!
//! The `Deserialize` impl of `Value` itself must not read untrusted input.
//! The same is true of the impls of types that hold a `Value`, such as
//! `Map<String, Value>` and `Vec<Value>`. When any crate in the build enables
//! the `raw_value` feature of `serde_json`, that impl reads an object whose
//! first key is `$serde_json::private::RawValue` as the JSON text in the
//! string under that key. The decoded value then differs from the one that a
//! filter, a log or a signature check sees in the request text. Each such
//! string is parsed with a new recursion limit, so nested strings build a
//! value deeper than `serde_json` allows in one parse.
//!
//! The types in this module build the same `Value` and read every object key
//! as data, at every depth. They set no depth limit of their own. The depth of
//! what they build is the depth that the deserializer allows, which is 128
//! for `serde_json` by default.
//!
//! `buffa` buffers through them where a proto3 JSON shape cannot be decoded in
//! one pass. A `google.protobuf.Any` names its type in `@type`, which can
//! follow the fields that it describes. An element of an enum list is
//! dropped, not rejected, when
//! [`ignore_unknown_enum_values`](crate::json::JsonParseOptions::ignore_unknown_enum_values)
//! is set.
//!
//! # Fields of other types
//!
//! For a field that holds a `Value` in another container, write the function
//! for `deserialize_with` from the types:
//!
//! ```
//! use buffa::json_helpers::buffered::BufferedValue;
//! use serde::{Deserialize, Deserializer};
//! use serde_json::Value;
//!
//! fn values<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Value>, D::Error> {
//!     let buffered = Vec::<BufferedValue>::deserialize(deserializer)?;
//!     Ok(buffered.into_iter().map(Value::from).collect())
//! }
//!
//! #[derive(Deserialize)]
//! struct Batch {
//!     #[serde(deserialize_with = "values")]
//!     items: Vec<Value>,
//! }
//!
//! let text = r#"{"items": [{"$serde_json::private::RawValue": "[1, 2]"}]}"#;
//! let batch: Batch = serde_json::from_str(text)?;
//! assert_eq!(batch.items[0]["$serde_json::private::RawValue"], "[1, 2]");
//! # Ok::<(), serde_json::Error>(())
//! ```
//!
//! # The `arbitrary_precision` feature of `serde_json`
//!
//! `buffa`'s JSON decoding does not support that feature. With it, a number
//! that has a fraction or an exponent, or a whole number outside the `i64`
//! and `u64` ranges, reaches a visitor as an object with the key
//! `$serde_json::private::Number`. The numeric helpers, such as
//! [`double`](super::double) and [`int64`](super::int64), reject that object,
//! so an integer field also rejects `1.0` and `1e3`.
//!
//! The types in this module read that key as data, so the JSON text
//! `{"$serde_json::private::Number": "1"}` stays an object in every build.
//! With the feature on, the number `1.5` therefore buffers as an object with
//! that key, where `Value` holds a number.

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::fmt;

use serde::de::{Deserialize, DeserializeSeed, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// A [`serde_json::Value`], deserialized with every object key read as data.
///
/// The wrapper selects the `Deserialize` impl and carries no invariant: any
/// `Value` can be wrapped. It accepts the input that `Value` accepts, and
/// builds the same `Value` except in two cases:
///
/// - An object with the key `$serde_json::private::RawValue` stays an object.
/// - In a build with `serde_json`'s `arbitrary_precision` feature, a number
///   that has a fraction or an exponent, or that is outside the `i64` and
///   `u64` ranges, buffers as an object and not as a number.
///
/// The [module documentation](self) explains both.
///
/// # Examples
///
/// ```
/// use buffa::json_helpers::buffered::BufferedValue;
///
/// let text = r#"{"$serde_json::private::RawValue": "[1, 2]"}"#;
/// let BufferedValue(value) = serde_json::from_str(text)?;
/// assert_eq!(value["$serde_json::private::RawValue"], "[1, 2]");
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct BufferedValue(pub Value);

/// A JSON object, deserialized with every key read as data at every depth.
///
/// Use it instead of [`BufferedValue`] when the input must be a JSON object,
/// such as a payload whose fields are looked up by name. It accepts the input
/// that `serde_json::Map<String, Value>` accepts. From `serde_json`, that is
/// only a JSON object, so `null` is an error.
#[derive(Clone, Debug, PartialEq)]
pub struct BufferedObject(pub Map<String, Value>);

impl From<BufferedValue> for Value {
    fn from(BufferedValue(value): BufferedValue) -> Self {
        value
    }
}

impl From<BufferedObject> for Map<String, Value> {
    fn from(BufferedObject(object): BufferedObject) -> Self {
        object
    }
}

/// Deserializes a [`serde_json::Value`] as [`BufferedValue`] does.
///
/// Name it in `#[serde(deserialize_with = "...")]` on a field of type `Value`.
/// The result differs from what `Value`'s own impl builds in the two cases
/// that [`BufferedValue`] lists.
///
/// # Errors
///
/// Returns an error if `deserializer` fails, for example when `serde_json`
/// finds input nested past its recursion limit. Returns an error if
/// `deserializer` produces a value that a `Value` cannot hold, such as bytes.
///
/// # Examples
///
/// ```
/// #[derive(serde::Deserialize)]
/// struct Event {
///     #[serde(deserialize_with = "buffa::json_helpers::buffered::value")]
///     payload: serde_json::Value,
/// }
///
/// let text = r#"{"payload": {"$serde_json::private::RawValue": "[1, 2]"}}"#;
/// let event: Event = serde_json::from_str(text)?;
/// assert_eq!(event.payload["$serde_json::private::RawValue"], "[1, 2]");
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn value<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
    BufferedValue::deserialize(deserializer).map(Value::from)
}

/// Deserializes an `Option<serde_json::Value>` as [`BufferedValue`] does.
///
/// JSON `null` deserializes to `None`, and so does a missing field when the
/// field has `#[serde(default)]`. Without `default`, serde rejects input that
/// omits a field with `deserialize_with`.
///
/// # Errors
///
/// Returns the errors that [`value`] returns.
///
/// # Examples
///
/// ```
/// #[derive(serde::Deserialize)]
/// struct Event {
///     #[serde(default, deserialize_with = "buffa::json_helpers::buffered::opt_value")]
///     payload: Option<serde_json::Value>,
/// }
///
/// let event: Event = serde_json::from_str("{}")?;
/// assert_eq!(event.payload, None);
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn opt_value<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Option::<BufferedValue>::deserialize(deserializer).map(|value| value.map(Value::from))
}

/// Deserializes a `serde_json::Map<String, Value>` as [`BufferedObject`] does.
///
/// Name it in `#[serde(deserialize_with = "...")]` on a field of type
/// `Map<String, Value>`, including a field with `#[serde(flatten)]` that
/// collects the keys that no other field names.
///
/// # Errors
///
/// Returns the errors that [`value`] returns. Returns an error if the input
/// is not an object.
pub fn object<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Map<String, Value>, D::Error> {
    BufferedObject::deserialize(deserializer).map(Map::from)
}

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

/// A running byte budget for the charging buffers [`ChargingValue`] and
/// [`ChargingObject`].
///
/// Each JSON value admitted into an array or object charges `per_value` bytes
/// from `remaining`; a charge that would exceed it fails deserialization with
/// `limit_msg` as the error text, so the caller recognises its own limit
/// error. `remaining` is shared — the same cell threads through every nested
/// value — so one budget bounds the whole buffered tree and an oversized
/// payload fails *before* it is fully buffered, rather than only after the
/// `serde_json::Value` tree has been allocated in full.
///
/// A failed charge leaves `remaining` unchanged.
#[derive(Clone, Copy)]
pub struct Budget<'a> {
    /// Bytes still available to the buffer; decremented as values are admitted.
    pub remaining: &'a Cell<usize>,
    /// Bytes charged for each value admitted into an array or object.
    pub per_value: usize,
    /// Error text produced when a charge would exceed `remaining`.
    pub limit_msg: &'static str,
}

impl Budget<'_> {
    fn charge<E: Error>(&self) -> Result<(), E> {
        let left = self.remaining.get();
        if self.per_value > left {
            return Err(E::custom(self.limit_msg));
        }
        self.remaining.set(left - self.per_value);
        Ok(())
    }
}

/// A [`DeserializeSeed`] that buffers a [`serde_json::Value`] exactly as
/// [`BufferedValue`] does — reading every object key as data, at every depth —
/// while charging a [`Budget`] per array element and object member as it
/// builds. Use it, rather than [`BufferedValue`], when the input is untrusted
/// and its buffered size must be bounded.
#[derive(Clone, Copy)]
pub struct ChargingValue<'a>(pub Budget<'a>);

/// A [`DeserializeSeed`] that buffers a JSON object exactly as
/// [`BufferedObject`] does, charging a [`Budget`] per member. The charging
/// counterpart of [`BufferedObject`].
#[derive(Clone, Copy)]
pub struct ChargingObject<'a>(pub Budget<'a>);

impl<'de> DeserializeSeed<'de> for ChargingValue<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(ChargingValueVisitor(self.0))
    }
}

impl<'de> DeserializeSeed<'de> for ChargingObject<'_> {
    type Value = Map<String, Value>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_map(ChargingObjectVisitor(self.0))
    }
}

/// Read an object, charging `budget` per member and buffering each value
/// through [`ChargingValue`] so nested elements charge the same budget.
///
/// A member is charged once its value has been admitted: its own contents were
/// charged as they were built, so the overshoot is at most one member.
fn read_charging_object<'de, A: MapAccess<'de>>(
    mut map: A,
    budget: Budget<'_>,
) -> Result<Map<String, Value>, A::Error> {
    let mut out = Map::new();
    while let Some(key) = map.next_key::<String>()? {
        let value = map.next_value_seed(ChargingValue(budget))?;
        budget.charge()?;
        out.insert(key, value);
    }
    Ok(out)
}

struct ChargingObjectVisitor<'a>(Budget<'a>);

impl<'de> Visitor<'de> for ChargingObjectVisitor<'_> {
    type Value = Map<String, Value>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a map")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Map::new())
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        read_charging_object(map, self.0)
    }
}

struct ChargingValueVisitor<'a>(Budget<'a>);

impl<'de> Visitor<'de> for ChargingValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any valid JSON value")
    }

    // Scalars have no children and are charged by the container that admits
    // them, so they build exactly as `ValueVisitor` does — forwarded to it so
    // the two cannot drift on NaN, wide integers or the private keys.
    fn visit_bool<E: Error>(self, v: bool) -> Result<Value, E> {
        ValueVisitor.visit_bool(v)
    }
    fn visit_i64<E: Error>(self, v: i64) -> Result<Value, E> {
        ValueVisitor.visit_i64(v)
    }
    fn visit_i128<E: Error>(self, v: i128) -> Result<Value, E> {
        ValueVisitor.visit_i128(v)
    }
    fn visit_u64<E: Error>(self, v: u64) -> Result<Value, E> {
        ValueVisitor.visit_u64(v)
    }
    fn visit_u128<E: Error>(self, v: u128) -> Result<Value, E> {
        ValueVisitor.visit_u128(v)
    }
    fn visit_f64<E: Error>(self, v: f64) -> Result<Value, E> {
        ValueVisitor.visit_f64(v)
    }
    fn visit_str<E: Error>(self, v: &str) -> Result<Value, E> {
        ValueVisitor.visit_str(v)
    }
    fn visit_string<E: Error>(self, v: String) -> Result<Value, E> {
        ValueVisitor.visit_string(v)
    }
    fn visit_none<E: Error>(self) -> Result<Value, E> {
        ValueVisitor.visit_none()
    }
    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        ValueVisitor.visit_unit()
    }

    // Composite values recurse through the charging seed so the whole subtree
    // draws on the one budget.
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        ChargingValue(self.0).deserialize(d)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let budget = self.0;
        let mut out = Vec::new();
        while let Some(element) = seq.next_element_seed(ChargingValue(budget))? {
            budget.charge()?;
            out.push(element);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Value, A::Error> {
        read_charging_object(map, self.0).map(Value::Object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;
    use serde::de::DeserializeSeed;
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

    /// The tests of the private key also pass against `serde_json::Value`'s
    /// own `Deserialize` impl unless `serde_json` has `raw_value` on. It is a
    /// dev-dependency feature of this crate, so a test build always has it.
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
        assert!(serde_json::from_str::<BufferedObject>("null").is_err());
        assert!(BufferedObject::deserialize(Value::Null).is_err());
    }

    #[test]
    fn the_field_functions_read_the_raw_value_key_as_data() {
        #[derive(Debug, serde::Deserialize)]
        struct Fields {
            #[serde(deserialize_with = "value")]
            required: Value,
            #[serde(default, deserialize_with = "opt_value")]
            optional: Option<Value>,
            #[serde(deserialize_with = "object")]
            named: Map<String, Value>,
            #[serde(flatten, deserialize_with = "object")]
            rest: Map<String, Value>,
        }

        let hidden = json!({ RAW_VALUE_KEY: "[1]" });
        let built = json!({
            "required": hidden,
            "optional": hidden,
            "named": { "a": hidden },
            "other": hidden,
        });
        let from_text: Fields = serde_json::from_str(&built.to_string()).unwrap();
        let from_value: Fields = serde_json::from_value(built).unwrap();
        for fields in [from_text, from_value] {
            assert_eq!(fields.required, hidden);
            assert_eq!(fields.optional, Some(hidden.clone()));
            assert_eq!(fields.named["a"], hidden);
            assert_eq!(fields.rest["other"], hidden);
        }

        for text in [
            r#"{"required": 1, "named": {}}"#,
            r#"{"required": 1, "named": {}, "optional": null}"#,
        ] {
            let fields: Fields = serde_json::from_str(text).unwrap();
            assert_eq!(fields.optional, None, "{text}");
        }
        let err = serde_json::from_str::<Fields>(r#"{"required": 1, "named": null}"#).unwrap_err();
        assert!(err.to_string().contains("expected a map"), "{err}");
    }

    #[test]
    fn a_field_with_a_field_function_and_no_default_is_required() {
        #[derive(Debug, serde::Deserialize)]
        struct Fields {
            #[serde(deserialize_with = "opt_value")]
            #[allow(dead_code)]
            optional: Option<Value>,
        }

        let err = serde_json::from_str::<Fields>("{}").unwrap_err();
        assert!(
            err.to_string().contains("missing field `optional`"),
            "{err}"
        );
    }

    #[test]
    fn a_buffer_converts_into_what_it_holds() {
        let built = json!({ "a": [1] });
        let buffered = BufferedValue::deserialize(built.clone()).unwrap();
        assert_eq!(buffered, BufferedValue(built.clone()));
        assert_eq!(Value::from(buffered), built);

        let object = BufferedObject::deserialize(built.clone()).unwrap();
        assert_eq!(Value::Object(object.into()), built);
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

    const CHARGED_OUT: &str = "charged out";

    /// Buffer `text` through [`ChargingValue`], charging `per_value` per
    /// admitted value against a budget of `limit`.
    fn charge(text: &str, per_value: usize, limit: usize) -> serde_json::Result<Value> {
        let remaining = Cell::new(limit);
        let budget = Budget {
            remaining: &remaining,
            per_value,
            limit_msg: CHARGED_OUT,
        };
        ChargingValue(budget).deserialize(&mut serde_json::Deserializer::from_str(text))
    }

    #[test]
    fn a_charging_buffer_builds_what_a_plain_buffer_does() {
        for text in [
            "null",
            "true",
            "-1",
            "1.5",
            r#""x""#,
            "[1,[2,[3,{}]]]",
            r#"{"a":1,"b":{"c":[true,false]},"a":2}"#,
            r#"{"$serde_json::private::RawValue":"[1]"}"#,
            r#"{"$serde_json::private::Number":"1"}"#,
        ] {
            let got = charge(text, 1, usize::MAX).map_err(|e| e.to_string());
            let expected = serde_json::from_str::<BufferedValue>(text)
                .map(|BufferedValue(v)| v)
                .map_err(|e| e.to_string());
            assert_eq!(got, expected, "{text}");
        }
    }

    /// The reason the buffer exists at all survives the charging variant: the
    /// private `raw_value` key is read as data, not re-parsed.
    #[test]
    fn a_charging_buffer_reads_the_raw_value_key_as_data() {
        let text = json!({ RAW_VALUE_KEY: "[1]" }).to_string();
        let value = charge(&text, 1, usize::MAX).unwrap();
        assert_eq!(value[RAW_VALUE_KEY], Value::String("[1]".to_string()));
    }

    /// An oversized payload is refused, and a payload that fits is not.
    #[test]
    fn a_charging_buffer_is_bounded_by_its_budget() {
        // Four admitted values: the members `a` and `b`, and the two elements
        // of `b`'s array.
        let text = r#"{"a":1,"b":[1,2]}"#;
        assert_eq!(charge(text, 1, 4).unwrap(), json!({"a":1,"b":[1,2]}));

        let err = charge(text, 1, 3).unwrap_err();
        assert!(err.to_string().starts_with(CHARGED_OUT), "{err}");

        // The budget is left untouched by the charge that failed: 3 admitted,
        // then the fourth is refused with the whole remaining budget intact.
        let remaining = Cell::new(3);
        let budget = Budget {
            remaining: &remaining,
            per_value: 1,
            limit_msg: CHARGED_OUT,
        };
        let _ = ChargingValue(budget).deserialize(&mut serde_json::Deserializer::from_str(text));
        assert_eq!(remaining.get(), 0);
    }
}

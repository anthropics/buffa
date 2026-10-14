//! Ergonomic helpers for [`google::protobuf::Value`](crate::google::protobuf::Value),
//! [`google::protobuf::Struct`](crate::google::protobuf::Struct), and
//! [`google::protobuf::ListValue`](crate::google::protobuf::ListValue).

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use core::iter::FusedIterator;

use crate::google::protobuf::__buffa::oneof::value::Kind as KindOneof;
use crate::google::protobuf::{ListValue, NullValue, Struct, Value};

impl Value {
    /// Construct a [`Value`] that represents a protobuf `null`.
    pub fn null() -> Self {
        Self {
            kind: Some(KindOneof::NullValue(buffa::EnumValue::from(
                NullValue::NULL_VALUE,
            ))),
            ..Default::default()
        }
    }

    /// Returns `true` if this value is the null variant.
    pub fn is_null(&self) -> bool {
        matches!(self.kind, Some(KindOneof::NullValue(_)))
    }

    /// Returns the `f64` value if this is a number, otherwise `None`.
    pub fn as_number(&self) -> Option<f64> {
        match &self.kind {
            Some(KindOneof::NumberValue(n)) => Some(*n),
            _ => None,
        }
    }

    /// Returns the string value if this is a string, otherwise `None`.
    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            Some(KindOneof::StringValue(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the bool value if this is a bool, otherwise `None`.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.kind {
            Some(KindOneof::BoolValue(b)) => Some(*b),
            _ => None,
        }
    }

    /// Returns a reference to the [`Struct`] if this is a struct value.
    pub fn as_struct(&self) -> Option<&Struct> {
        match &self.kind {
            Some(KindOneof::StructValue(s)) => Some(s),
            _ => None,
        }
    }

    /// Returns a reference to the [`ListValue`] if this is a list value.
    pub fn as_list(&self) -> Option<&ListValue> {
        match &self.kind {
            Some(KindOneof::ListValue(l)) => Some(l),
            _ => None,
        }
    }

    /// Returns a mutable reference to the [`Struct`] if this is a struct value.
    pub fn as_struct_mut(&mut self) -> Option<&mut Struct> {
        match &mut self.kind {
            Some(KindOneof::StructValue(s)) => Some(s),
            _ => None,
        }
    }

    /// Returns a mutable reference to the [`ListValue`] if this is a list value.
    pub fn as_list_mut(&mut self) -> Option<&mut ListValue> {
        match &mut self.kind {
            Some(KindOneof::ListValue(l)) => Some(l),
            _ => None,
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Self {
            kind: Some(KindOneof::NumberValue(n)),
            ..Default::default()
        }
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self {
            kind: Some(KindOneof::StringValue(s)),
            ..Default::default()
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self {
            kind: Some(KindOneof::StringValue(s.to_string())),
            ..Default::default()
        }
    }
}

impl From<f32> for Value {
    /// Converts an `f32` to a [`Value`] via `f64` widening.
    ///
    /// The conversion is lossless for values representable as both `f32` and
    /// `f64`; the extra precision bits are filled with zeros.
    fn from(n: f32) -> Self {
        Self::from(n as f64)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self {
            kind: Some(KindOneof::BoolValue(b)),
            ..Default::default()
        }
    }
}

impl From<i32> for Value {
    /// Converts an `i32` to a [`Value`] via `f64`.
    ///
    /// All `i32` values are representable exactly as `f64`.
    fn from(n: i32) -> Self {
        Self::from(n as f64)
    }
}

impl From<u32> for Value {
    /// Converts a `u32` to a [`Value`] via `f64`.
    ///
    /// All `u32` values are representable exactly as `f64`.
    fn from(n: u32) -> Self {
        Self::from(n as f64)
    }
}

impl From<i64> for Value {
    /// Converts an `i64` to a [`Value`] via `f64`.
    ///
    /// # Precision
    ///
    /// `f64` has 53 bits of mantissa. `i64` values outside `[-2^53, 2^53]`
    /// will be rounded to the nearest representable `f64`.
    fn from(n: i64) -> Self {
        Self::from(n as f64)
    }
}

impl From<u64> for Value {
    /// Converts a `u64` to a [`Value`] via `f64`.
    ///
    /// # Precision
    ///
    /// `f64` has 53 bits of mantissa. `u64` values greater than `2^53`
    /// will be rounded to the nearest representable `f64`.
    fn from(n: u64) -> Self {
        Self::from(n as f64)
    }
}

impl From<Struct> for Value {
    fn from(s: Struct) -> Self {
        Self {
            kind: Some(KindOneof::StructValue(Box::new(s))),
            ..Default::default()
        }
    }
}

impl From<ListValue> for Value {
    fn from(l: ListValue) -> Self {
        Self {
            kind: Some(KindOneof::ListValue(Box::new(l))),
            ..Default::default()
        }
    }
}

impl ListValue {
    /// Construct a [`ListValue`] from an iterator of items convertible to [`Value`].
    pub fn from_values(values: impl IntoIterator<Item = impl Into<Value>>) -> Self {
        Self {
            values: values.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }

    /// Returns the number of elements in the list.
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns `true` if the list contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns an iterator over the values in the list.
    #[inline]
    pub fn iter(&self) -> core::slice::Iter<'_, Value> {
        self.values.iter()
    }

    /// Returns an iterator over mutable references to the values in the list.
    #[inline]
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, Value> {
        self.values.iter_mut()
    }
}

impl<'a> IntoIterator for &'a ListValue {
    type Item = &'a Value;
    type IntoIter = core::slice::Iter<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<'a> IntoIterator for &'a mut ListValue {
    type Item = &'a mut Value;
    type IntoIter = core::slice::IterMut<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

impl IntoIterator for ListValue {
    type Item = Value;
    type IntoIter = alloc::vec::IntoIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<V: Into<Value>> Extend<V> for ListValue {
    /// Appends each item, converted to a [`Value`], in iteration order.
    fn extend<T: IntoIterator<Item = V>>(&mut self, iter: T) {
        self.values.extend(iter.into_iter().map(Into::into));
    }
}

impl<V: Into<Value>> FromIterator<V> for ListValue {
    /// Collects items convertible to [`Value`] into a [`ListValue`], in
    /// iteration order, as [`from_values`](ListValue::from_values) does.
    ///
    /// The item type has to be known: `.map(Into::into).collect::<ListValue>()`
    /// does not compile, because the conversion has many possible targets.
    /// Collect the items as they are, or name the target with
    /// `.map(Value::from)`. Borrowed values need a clone:
    /// `list.iter().cloned().collect::<ListValue>()`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::ListValue;
    ///
    /// let list: ListValue = [1.0, 2.5].into_iter().collect();
    /// assert_eq!(list.len(), 2);
    /// ```
    ///
    /// A conversion with no named target does not compile:
    ///
    /// ```compile_fail,E0283
    /// use buffa_types::google::protobuf::ListValue;
    ///
    /// let list: ListValue = [1.0, 2.5].into_iter().map(Into::into).collect();
    /// ```
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        Self::from_values(iter)
    }
}

impl Struct {
    /// Construct a new empty [`Struct`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a [`Struct`] from an iterator of key-value pairs.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::{Struct, Value};
    ///
    /// let s = Struct::from_fields([("x", Value::from(1.0_f64)), ("y", Value::from(2.0_f64))]);
    /// assert!(s.get("x").is_some());
    /// ```
    pub fn from_fields(
        fields: impl IntoIterator<Item = (impl Into<String>, impl Into<Value>)>,
    ) -> Self {
        let mut s = Self::new();
        for (k, v) in fields {
            s.insert(k, v);
        }
        s
    }

    /// Insert a key-value pair into the struct.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.fields.insert(key.into(), value.into());
    }

    /// Returns the value for `key` if present.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// Returns a mutable reference to the value for `key` if present.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::{Struct, Value};
    ///
    /// let mut s = Struct::from_fields([("enabled", false)]);
    /// *s.get_mut("enabled").unwrap() = Value::from(true);
    /// assert_eq!(s.get("enabled").and_then(Value::as_bool), Some(true));
    /// ```
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.fields.get_mut(key)
    }

    /// Removes `key`, returning its value if present.
    ///
    /// A missing key leaves the struct unchanged.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::Struct;
    ///
    /// let mut s = Struct::from_fields([("name", "Alice")]);
    /// let value = s.remove("name").unwrap();
    /// assert_eq!(value.as_str(), Some("Alice"));
    /// assert!(s.get("name").is_none());
    /// ```
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.fields.remove(key)
    }

    /// Returns `true` if the struct has a value for `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.fields.contains_key(key)
    }

    /// Returns the number of fields in the struct.
    #[inline]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns `true` if the struct has no fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Returns an iterator over the key-value pairs of the struct.
    ///
    /// [`fields`](Struct::fields) is a hash map, so the iteration order is
    /// unspecified and can differ between two structs with the same fields.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::Struct;
    ///
    /// let s = Struct::from_fields([("a", 1.0_f64), ("b", 2.0_f64)]);
    /// let total: f64 = s.iter().filter_map(|(_, v)| v.as_number()).sum();
    /// assert_eq!(total, 3.0);
    /// ```
    #[inline]
    pub fn iter(&self) -> StructIter<'_> {
        StructIter(self.fields.iter())
    }

    /// Returns an iterator over the keys of the struct and mutable references
    /// to their values, in unspecified order.
    #[inline]
    pub fn iter_mut(&mut self) -> StructIterMut<'_> {
        StructIterMut(self.fields.iter_mut())
    }
}

type Fields = ::buffa::__private::HashMap<String, Value>;

/// Implements `Iterator`, `ExactSizeIterator` and `FusedIterator` for a
/// newtype over one of the hash map's iterators.
macro_rules! forward_iterator {
    ($name:ident $(<$lt:lifetime>)?, $item:ty) => {
        impl$(<$lt>)? Iterator for $name$(<$lt>)? {
            type Item = $item;

            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                self.0.next()
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }

            // The map's own `fold` walks the table a group at a time;
            // `for_each` and `sum` reach it through this.
            #[inline]
            fn fold<B, F: FnMut(B, Self::Item) -> B>(self, init: B, f: F) -> B {
                self.0.fold(init, f)
            }

            #[inline]
            fn count(self) -> usize {
                self.0.count()
            }
        }

        impl$(<$lt>)? ExactSizeIterator for $name$(<$lt>)? {
            #[inline]
            fn len(&self) -> usize {
                self.0.len()
            }
        }

        impl$(<$lt>)? FusedIterator for $name$(<$lt>)? {}
    };
}

/// An iterator over the key-value pairs of a [`Struct`], in unspecified
/// order.
///
/// Returned by [`Struct::iter`] and by iterating over `&Struct`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
#[derive(Clone, Debug)]
pub struct StructIter<'a>(<&'a Fields as IntoIterator>::IntoIter);

forward_iterator!(StructIter<'a>, (&'a String, &'a Value));

/// An iterator over the keys of a [`Struct`] and mutable references to their
/// values, in unspecified order.
///
/// Returned by [`Struct::iter_mut`] and by iterating over `&mut Struct`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
#[derive(Debug)]
pub struct StructIterMut<'a>(<&'a mut Fields as IntoIterator>::IntoIter);

forward_iterator!(StructIterMut<'a>, (&'a String, &'a mut Value));

/// An owning iterator over the key-value pairs of a [`Struct`], in
/// unspecified order.
///
/// Returned by iterating over a `Struct` by value.
#[must_use = "iterators are lazy and do nothing unless consumed"]
#[derive(Debug)]
pub struct StructIntoIter(<Fields as IntoIterator>::IntoIter);

forward_iterator!(StructIntoIter, (String, Value));

impl<'a> IntoIterator for &'a Struct {
    type Item = (&'a String, &'a Value);
    type IntoIter = StructIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Struct {
    type Item = (&'a String, &'a mut Value);
    type IntoIter = StructIterMut<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl IntoIterator for Struct {
    type Item = (String, Value);
    type IntoIter = StructIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        StructIntoIter(self.fields.into_iter())
    }
}

impl<K: Into<String>, V: Into<Value>> Extend<(K, V)> for Struct {
    /// Inserts each key-value pair, as [`insert`](Struct::insert) does: a key
    /// that is already present has its value replaced. `a.extend(b)` moves
    /// the fields of another struct `b` into `a`, and
    /// `a.extend(b.iter().map(|(k, v)| (k, v.clone())))` copies them. Neither
    /// carries over the unknown fields of `b`.
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        self.fields
            .extend(iter.into_iter().map(|(k, v)| (k.into(), v.into())));
    }
}

impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Struct {
    /// Collects key-value pairs into a [`Struct`], as
    /// [`from_fields`](Struct::from_fields) does: of two pairs with the same
    /// key, the later one is kept.
    ///
    /// The types of the pair have to be known: mapping to
    /// `(k.into(), v.into())` before `.collect::<Struct>()` does not compile,
    /// because each conversion has many possible targets. Collect the pairs
    /// as they are, or name both targets, as in
    /// `(k.to_string(), Value::from(v))`. Pairs with borrowed values, such as
    /// those [`Struct::iter`] yields, need the value cloned:
    /// `.map(|(k, v)| (k, v.clone()))`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::Struct;
    ///
    /// let s: Struct = [("x", 1.0), ("y", 2.0)].into_iter().collect();
    /// assert_eq!(s.len(), 2);
    /// ```
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self::from_fields(iter)
    }
}

// ── serde impls ─────────────────────────────────────────────────────────────

#[cfg(feature = "json")]
use alloc::vec::Vec;

#[cfg(feature = "json")]
impl serde::Serialize for Value {
    /// Serializes as the corresponding JSON value.
    ///
    /// The `null`, bool, string, object, and array variants map directly to
    /// their JSON counterparts.  The `number` variant serializes as a JSON
    /// number via `serialize_f64`.
    ///
    /// # Errors
    ///
    /// Serialization fails if no variant is set. It also fails if the
    /// `number` variant holds a non-finite value
    /// (`NaN`, `Infinity`, `-Infinity`), because JSON numbers cannot represent
    /// those values.  Use [`DoubleValue`](crate::google::protobuf::DoubleValue) if you need to
    /// serialize non-finite floating-point values (which uses the proto3 JSON
    /// string encoding `"NaN"` / `"Infinity"` / `"-Infinity"`).
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.kind {
            None => Err(serde::ser::Error::custom(
                "google.protobuf.Value.kind must be set",
            )),
            Some(KindOneof::NullValue(_)) => s.serialize_unit(),
            Some(KindOneof::NumberValue(n)) => {
                if !n.is_finite() {
                    return Err(serde::ser::Error::custom(
                        "Value.number_value must be finite; NaN and Infinity are not valid JSON numbers",
                    ));
                }
                s.serialize_f64(*n)
            }
            Some(KindOneof::StringValue(v)) => s.serialize_str(v),
            Some(KindOneof::BoolValue(b)) => s.serialize_bool(*b),
            Some(KindOneof::StructValue(st)) => st.serialize(s),
            Some(KindOneof::ListValue(l)) => l.serialize(s),
        }
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Value;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::null())
            }
            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::null())
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::from(v as f64))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(Value::from(v as f64))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
                let mut st = Struct::default();
                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    st.fields.insert(k, v);
                }
                Ok(Value::from(st))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = seq.next_element::<Value>()? {
                    values.push(v);
                }
                Ok(Value::from(ListValue {
                    values,
                    ..Default::default()
                }))
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for Struct {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.fields.len()))?;
        for (k, v) in &self.fields {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Struct {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Struct;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Struct, A::Error> {
                let mut st = Struct::default();
                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    st.fields.insert(k, v);
                }
                Ok(st)
            }
        }
        d.deserialize_map(V)
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for ListValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(self.values.len()))?;
        for v in &self.values {
            seq.serialize_element(v)?;
        }
        seq.end()
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for ListValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ListValue;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a JSON array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ListValue, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = seq.next_element::<Value>()? {
                    values.push(v);
                }
                Ok(ListValue {
                    values,
                    ..Default::default()
                })
            }
        }
        d.deserialize_seq(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{vec, vec::Vec};

    #[test]
    fn value_null() {
        let v = Value::null();
        assert!(v.is_null());
    }

    #[test]
    fn value_from_f64() {
        let v = Value::from(2.5_f64);
        assert_eq!(v.as_number(), Some(2.5));
    }

    #[test]
    fn value_from_string() {
        let v = Value::from("hello".to_string());
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn value_from_str_ref() {
        let v = Value::from("world");
        assert_eq!(v.as_str(), Some("world"));
    }

    #[test]
    fn value_from_bool() {
        assert_eq!(Value::from(true).as_bool(), Some(true));
        assert_eq!(Value::from(false).as_bool(), Some(false));
    }

    #[test]
    fn value_from_struct() {
        let mut s = Struct::new();
        s.insert("x", 1.0_f64);
        let v = Value::from(s);
        assert!(v.as_struct().is_some());
    }

    #[test]
    fn value_from_list() {
        let l = ListValue::from_values([1.0_f64, 2.0, 3.0]);
        let v = Value::from(l);
        assert!(v.as_list().is_some());
        assert_eq!(v.as_list().unwrap().values.len(), 3);
    }

    #[test]
    fn struct_insert_get() {
        let mut s = Struct::new();
        s.insert("key", "value");
        assert_eq!(s.get("key").and_then(|v| v.as_str()), Some("value"));
        assert!(s.get("missing").is_none());
    }

    #[test]
    fn struct_get_mut_replaces_existing_value() {
        let mut s = Struct::from_fields([("enabled", false), ("other", true)]);
        *s.get_mut("enabled").unwrap() = Value::from(true);
        assert_eq!(s.get("enabled").and_then(Value::as_bool), Some(true));
        assert_eq!(s.get("other").and_then(Value::as_bool), Some(true));
        assert_eq!(s.fields.len(), 2);
    }

    #[test]
    fn struct_get_mut_missing_key_does_not_insert() {
        let mut empty = Struct::new();
        assert!(empty.get_mut("missing").is_none());
        assert!(empty.fields.is_empty());

        let mut s = Struct::from_fields([("key", 1_i32)]);
        let before = s.clone();
        assert!(s.get_mut("Key").is_none());
        assert!(s.get_mut("missing").is_none());
        assert_eq!(s, before);
    }

    #[test]
    fn struct_get_mut_edits_nested_value() {
        let mut s = Struct::from_fields([("nested", Struct::from_fields([("count", 1_i32)]))]);
        s.get_mut("nested")
            .unwrap()
            .as_struct_mut()
            .unwrap()
            .insert("count", 2_i32);
        assert_eq!(
            s.get("nested")
                .unwrap()
                .as_struct()
                .unwrap()
                .get("count")
                .and_then(Value::as_number),
            Some(2.0)
        );
    }

    #[test]
    fn struct_remove_returns_value_and_keeps_other_fields() {
        let mut s = Struct::from_fields([("key", "value"), ("other", "unchanged")]);
        let value = s.remove("key").unwrap();
        assert_eq!(value.as_str(), Some("value"));
        assert!(s.get("key").is_none());
        assert_eq!(s.get("other").and_then(Value::as_str), Some("unchanged"));
        assert_eq!(s.fields.len(), 1);
        assert!(s.remove("key").is_none());
    }

    #[test]
    fn struct_remove_missing_key_leaves_fields_unchanged() {
        let mut s = Struct::new();
        assert!(s.remove("missing").is_none());
        assert!(s.fields.is_empty());
        s.insert("present", true);
        assert!(s.remove("missing").is_none());
        assert_eq!(s.fields.len(), 1);
        assert_eq!(s.get("present").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn struct_from_fields() {
        let s = Struct::from_fields([("a", Value::from(1.0_f64)), ("b", Value::from(2.0_f64))]);
        assert_eq!(s.get("a").and_then(|v| v.as_number()), Some(1.0));
        assert_eq!(s.get("b").and_then(|v| v.as_number()), Some(2.0));
    }

    #[test]
    fn struct_from_fields_empty() {
        let s = Struct::from_fields(core::iter::empty::<(&str, Value)>());
        assert!(s.fields.is_empty());
    }

    #[test]
    fn struct_from_iter() {
        let pairs = vec![
            ("x".to_string(), Value::from("hello")),
            ("y".to_string(), Value::from(true)),
        ];
        let s: Struct = pairs.into_iter().collect();
        assert_eq!(s.get("x").and_then(|v| v.as_str()), Some("hello"));
        assert_eq!(s.get("y").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn struct_from_iter_converts_keys_and_values() {
        let s: Struct = [("a", 1.0), ("b", 2.0)].into_iter().collect();
        assert_eq!(s.get("a").and_then(Value::as_number), Some(1.0));
        assert_eq!(s.get("b").and_then(Value::as_number), Some(2.0));

        // Of two pairs with one key the later is kept, as `from_fields` does.
        let s: Struct = [("k", "first"), ("k", "second")].into_iter().collect();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get("k").and_then(Value::as_str), Some("second"));

        // A struct collects from the pairs another struct yields.
        let copy: Struct = s.iter().map(|(k, v)| (k, v.clone())).collect();
        assert_eq!(copy, s);
    }

    #[test]
    fn list_value_from_iter_converts_items() {
        let l: ListValue = ["a", "b"].into_iter().collect();
        assert_eq!(l, ListValue::from_values(["a", "b"]));

        let l: ListValue = vec![Value::from(true), Value::null()].into_iter().collect();
        assert_eq!(l.len(), 2);
        assert_eq!(l.values[0].as_bool(), Some(true));

        // `&Value` items are cloned first.
        let copy: ListValue = l.iter().cloned().collect();
        assert_eq!(copy, l);

        let empty: ListValue = core::iter::empty::<f64>().collect();
        assert!(empty.is_empty());
    }

    #[test]
    fn list_value_from_values() {
        let l = ListValue::from_values(["a", "b", "c"]);
        assert_eq!(l.values.len(), 3);
    }

    // ---- From<f32> --------------------------------------------------------

    #[test]
    fn value_from_f32() {
        let v = Value::from(1.5_f32);
        assert_eq!(v.as_number(), Some(1.5_f64));
    }

    // ---- ListValue collection methods ------------------------------------

    #[test]
    fn list_value_len_and_is_empty() {
        let empty = ListValue::from_values(core::iter::empty::<f64>());
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let three = ListValue::from_values([1.0_f64, 2.0, 3.0]);
        assert!(!three.is_empty());
        assert_eq!(three.len(), 3);
    }

    #[test]
    fn list_value_iter() {
        let l = ListValue::from_values([1.0_f64, 2.0]);
        let nums: Vec<f64> = l.iter().map(|v| v.as_number().unwrap()).collect();
        assert_eq!(nums, [1.0, 2.0]);
    }

    #[test]
    fn list_value_ref_into_iter() {
        let l = ListValue::from_values(["a", "b"]);
        let strs: Vec<&str> = (&l).into_iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(strs, ["a", "b"]);
    }

    #[test]
    fn list_value_iter_mut() {
        let mut list = ListValue::from_values([1.0_f64, 2.0, 3.0]);
        let mut iter = list.iter_mut();
        assert_eq!(iter.len(), 3);
        *iter.next().unwrap() = Value::from(10.0_f64);
        *iter.next_back().unwrap() = Value::null();
        assert_eq!(iter.len(), 1);
        *iter.next().unwrap() = Value::from("middle");
        assert!(iter.next().is_none());
        assert!(iter.next_back().is_none());
        assert_eq!(
            list.values,
            [Value::from(10.0_f64), Value::from("middle"), Value::null()]
        );
    }

    #[test]
    fn list_value_mut_ref_into_iter() {
        let mut list = ListValue::from_values([false, true]);
        for value in &mut list {
            *value = Value::from(!value.as_bool().unwrap());
        }
        assert_eq!(list.values, [Value::from(true), Value::from(false)]);
    }

    #[test]
    fn list_value_empty_mutable_iteration() {
        let mut list = ListValue::default();
        assert_eq!(list.iter_mut().len(), 0);
        assert!(list.iter_mut().next().is_none());
        let mut iter = (&mut list).into_iter();
        assert_eq!(iter.len(), 0);
        assert!(iter.next_back().is_none());
        assert!(iter.next().is_none());
        assert!(list.is_empty());
    }

    #[test]
    fn list_value_extend_converts_and_appends() {
        let mut list = ListValue::from_values([1.0_f64]);
        list.extend(["a", "b"]);
        list.extend(vec![Value::from(true)]);
        assert_eq!(list.len(), 4);
        assert_eq!(list.values[0].as_number(), Some(1.0));
        assert_eq!(list.values[1].as_str(), Some("a"));
        assert_eq!(list.values[2].as_str(), Some("b"));
        assert_eq!(list.values[3].as_bool(), Some(true));
        list.extend(core::iter::empty::<Value>());
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn struct_len_is_empty_and_contains_key() {
        let mut s = Struct::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert!(!s.contains_key("a"));
        s.insert("a", 1.0_f64);
        s.insert("b", true);
        assert!(!s.is_empty());
        assert_eq!(s.len(), 2);
        assert!(s.contains_key("a"));
        assert!(!s.contains_key("A"));
    }

    fn sorted_keys<'a>(pairs: impl Iterator<Item = (&'a String, &'a Value)>) -> Vec<&'a str> {
        let mut keys: Vec<_> = pairs.map(|(k, _)| k.as_str()).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn struct_iter_and_ref_into_iter_visit_every_field() {
        let s = Struct::from_fields([("b", 2.0_f64), ("a", 1.0_f64), ("c", 3.0_f64)]);
        let iter = s.iter();
        assert_eq!(iter.len(), 3);
        assert_eq!(sorted_keys(iter), ["a", "b", "c"]);
        assert_eq!(sorted_keys((&s).into_iter()), ["a", "b", "c"]);
        let mut total = 0.0;
        for (_, value) in &s {
            total += value.as_number().unwrap();
        }
        assert_eq!(total, 6.0);
        assert_eq!(Struct::new().iter().next(), None);
    }

    #[test]
    fn struct_iter_mut_and_mut_ref_into_iter_edit_values() {
        let mut s = Struct::from_fields([("a", 1.0_f64), ("b", 2.0_f64)]);
        assert_eq!(s.iter_mut().len(), 2);
        for (_, value) in s.iter_mut() {
            *value = Value::from(value.as_number().unwrap() * 10.0);
        }
        for (key, value) in &mut s {
            if key == "a" {
                *value = Value::from("x");
            }
        }
        assert_eq!(s.get("a").and_then(Value::as_str), Some("x"));
        assert_eq!(s.get("b").and_then(Value::as_number), Some(20.0));
    }

    #[test]
    fn struct_owned_into_iter_yields_every_pair() {
        let s = Struct::from_fields([("a", 1.0_f64), ("b", 2.0_f64)]);
        let iter = s.into_iter();
        assert_eq!(iter.len(), 2);
        let mut pairs: Vec<(String, Value)> = iter.collect();
        pairs.sort_unstable_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(pairs[0].0, "a");
        assert_eq!(pairs[0].1.as_number(), Some(1.0));
        assert_eq!(pairs[1].0, "b");
        assert_eq!(pairs[1].1.as_number(), Some(2.0));
    }

    #[test]
    fn struct_iterators_are_fused_and_size_exact() {
        let s = Struct::from_fields([("a", 1.0_f64)]);
        let mut iter = s.iter();
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert!(iter.next().is_some());
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
        assert!(iter.next().is_none());
        let mut owned = s.into_iter();
        assert!(owned.next().is_some());
        assert!(owned.next().is_none());
        assert!(owned.next().is_none());
    }

    #[test]
    fn struct_iterators_implement_exact_size_and_fused() {
        // `FusedIterator` is a marker the compiler does not tie to the inner
        // iterator, so check the claim on the inner types directly.
        fn assert_traits<I: ExactSizeIterator + FusedIterator>() {}
        assert_traits::<<&Fields as IntoIterator>::IntoIter>();
        assert_traits::<<&mut Fields as IntoIterator>::IntoIter>();
        assert_traits::<<Fields as IntoIterator>::IntoIter>();
        assert_traits::<StructIter<'_>>();
        assert_traits::<StructIterMut<'_>>();
        assert_traits::<StructIntoIter>();

        let mut s = Struct::from_fields([("a", 1.0_f64)]);
        let mut iter = s.iter_mut();
        assert_eq!(iter.len(), 1);
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn struct_iterators_forward_fold_and_count() {
        let s = Struct::from_fields([("a", 1.0_f64), ("b", 2.0_f64), ("c", 3.0_f64)]);
        assert_eq!(s.iter().count(), 3);
        let total = s
            .iter()
            .fold(0.0, |acc, (_, v)| acc + v.as_number().unwrap());
        assert_eq!(total, 6.0);
        let mut keys = Vec::new();
        s.clone().into_iter().for_each(|(k, _)| keys.push(k));
        keys.sort_unstable();
        assert_eq!(keys, ["a", "b", "c"]);
    }

    #[test]
    fn struct_extend_merges_another_struct_by_value() {
        let mut a = Struct::from_fields([("x", 1.0_f64), ("y", 2.0_f64)]);
        let b = Struct::from_fields([("y", 20.0_f64), ("z", 30.0_f64)]);
        a.extend(b);
        assert_eq!(a.len(), 3);
        assert_eq!(a.get("y").and_then(Value::as_number), Some(20.0));
        assert_eq!(a.get("z").and_then(Value::as_number), Some(30.0));
    }

    #[test]
    fn struct_extend_converts_and_overwrites_existing_keys() {
        let mut s = Struct::from_fields([("a", 1.0_f64), ("b", 2.0_f64)]);
        s.extend([("b", "two"), ("c", "three")]);
        s.extend(vec![(String::from("d"), Value::from(true))]);
        assert_eq!(s.len(), 4);
        assert_eq!(s.get("a").and_then(Value::as_number), Some(1.0));
        assert_eq!(s.get("b").and_then(Value::as_str), Some("two"));
        assert_eq!(s.get("c").and_then(Value::as_str), Some("three"));
        assert_eq!(s.get("d").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn generic_code_accepts_every_collection_type() {
        fn extend_and_count<C, T>(mut c: C, items: impl IntoIterator<Item = T>) -> usize
        where
            C: Extend<T> + IntoIterator,
        {
            c.extend(items);
            c.into_iter().count()
        }
        assert_eq!(extend_and_count(Struct::new(), [("a", 1.0_f64)]), 1);
        assert_eq!(extend_and_count(ListValue::default(), [1.0_f64, 2.0]), 2);
        assert_eq!(
            extend_and_count(
                crate::google::protobuf::FieldMask::default(),
                ["a", "b", "c"]
            ),
            3
        );
    }

    #[test]
    fn list_value_owned_into_iter() {
        let l = ListValue::from_values([true, false]);
        let bools: Vec<bool> = l.into_iter().map(|v| v.as_bool().unwrap()).collect();
        assert_eq!(bools, [true, false]);
    }

    // ---- From<integer> ----------------------------------------------------

    #[test]
    fn value_from_i32() {
        let v = Value::from(42_i32);
        assert_eq!(v.as_number(), Some(42.0));
    }

    #[test]
    fn value_from_u32() {
        let v = Value::from(u32::MAX);
        assert_eq!(v.as_number(), Some(u32::MAX as f64));
    }

    #[test]
    fn value_from_i64_small() {
        // Small i64 values are representable exactly as f64.
        let v = Value::from(-100_i64);
        assert_eq!(v.as_number(), Some(-100.0));
    }

    #[test]
    fn value_from_u64_small() {
        let v = Value::from(1_000_000_u64);
        assert_eq!(v.as_number(), Some(1_000_000.0));
    }

    // ---- mutable accessors ------------------------------------------------

    #[test]
    fn as_struct_mut_returns_some_for_struct_value() {
        let mut s = Struct::new();
        s.insert("a", 1.0_f64);
        let mut v = Value::from(s);
        let m = v.as_struct_mut().unwrap();
        m.insert("b", 2.0_f64);
        assert_eq!(v.as_struct().unwrap().fields.len(), 2);
    }

    #[test]
    fn as_struct_mut_returns_none_for_non_struct() {
        let mut v = Value::from(1.0_f64);
        assert!(v.as_struct_mut().is_none());
    }

    #[test]
    fn as_list_mut_returns_some_for_list_value() {
        let l = ListValue::from_values([1.0_f64]);
        let mut v = Value::from(l);
        let m = v.as_list_mut().unwrap();
        m.values.push(Value::from(2.0_f64));
        assert_eq!(v.as_list().unwrap().values.len(), 2);
    }

    #[test]
    fn as_list_mut_returns_none_for_non_list() {
        let mut v = Value::from(true);
        assert!(v.as_list_mut().is_none());
    }

    // ── serde ───────────────────────────────────────────────────────────────

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        #[test]
        fn value_unset_serialize_is_error() {
            let v = Value::default();
            assert!(serde_json::to_string(&v).is_err());
        }

        #[test]
        fn value_with_only_unknown_fields_serialize_is_error() {
            use buffa::Message;

            let v = Value::decode_from_slice(&[0x38, 0x01]).unwrap();
            assert!(v.kind.is_none());
            assert!(serde_json::to_string(&v).is_err());
        }

        #[test]
        fn unset_value_in_containers_serialize_is_error() {
            let st = Struct::from_fields([("unset", Value::default())]);
            assert!(serde_json::to_string(&st).is_err());
            assert!(serde_json::to_string(&Value::from(st)).is_err());

            let list = ListValue::from_values([Value::null(), Value::default()]);
            assert!(serde_json::to_string(&list).is_err());
            assert!(serde_json::to_string(&Value::from(list)).is_err());
        }

        #[test]
        fn value_default_variants_roundtrip() {
            for (v, expected) in [
                (Value::null(), "null"),
                (Value::from(0.0_f64), "0.0"),
                (Value::from(false), "false"),
                (Value::from(""), r#""""#),
                (Value::from(Struct::new()), "{}"),
                (Value::from(ListValue::default()), "[]"),
            ] {
                let json = serde_json::to_string(&v).unwrap();
                assert_eq!(json, expected);
                assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), v);
            }
        }

        #[test]
        fn value_null_roundtrip() {
            let v = Value::null();
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, "null");
            let back: Value = serde_json::from_str(&json).unwrap();
            assert!(back.is_null());
        }

        #[test]
        fn value_number_roundtrip() {
            let v = Value::from(2.5_f64);
            let json = serde_json::to_string(&v).unwrap();
            let back: Value = serde_json::from_str(&json).unwrap();
            assert!((back.as_number().unwrap() - 2.5).abs() < 1e-10);
        }

        #[test]
        fn value_string_roundtrip() {
            let v = Value::from("hello");
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, r#""hello""#);
            let back: Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.as_str(), Some("hello"));
        }

        #[test]
        fn value_bool_roundtrip() {
            let v = Value::from(true);
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, "true");
            let back: Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.as_bool(), Some(true));
        }

        #[test]
        fn struct_value_roundtrip() {
            let s = Struct::from_fields([("x", Value::from(1.0_f64))]);
            let v = Value::from(s);
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, r#"{"x":1.0}"#);
            let back: Value = serde_json::from_str(&json).unwrap();
            assert!(back.as_struct().is_some());
            assert_eq!(
                back.as_struct()
                    .unwrap()
                    .get("x")
                    .and_then(|v| v.as_number()),
                Some(1.0)
            );
        }

        #[test]
        fn list_value_roundtrip() {
            let l = ListValue::from_values([1.0_f64, 2.0]);
            let v = Value::from(l);
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, "[1.0,2.0]");
            let back: Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back.as_list().unwrap().values.len(), 2);
        }

        #[test]
        fn struct_roundtrip() {
            let s = Struct::from_fields([("a", Value::from("b"))]);
            let json = serde_json::to_string(&s).unwrap();
            let back: Struct = serde_json::from_str(&json).unwrap();
            assert_eq!(back.get("a").and_then(|v| v.as_str()), Some("b"));
        }

        #[test]
        fn value_nan_serialize_is_error() {
            let v = Value::from(f64::NAN);
            let result = serde_json::to_string(&v);
            assert!(result.is_err(), "NaN must fail serialization");
        }

        #[test]
        fn value_infinity_serialize_is_error() {
            let v = Value::from(f64::INFINITY);
            assert!(
                serde_json::to_string(&v).is_err(),
                "Infinity must fail serialization"
            );

            let v = Value::from(f64::NEG_INFINITY);
            assert!(
                serde_json::to_string(&v).is_err(),
                "-Infinity must fail serialization"
            );
        }

        #[test]
        fn list_value_deserializes_from_array() {
            let json = r#"[null, 1, "s", true]"#;
            let l: ListValue = serde_json::from_str(json).unwrap();
            assert_eq!(l.values.len(), 4);
            assert!(l.values[0].is_null());
        }

        #[test]
        fn value_deserializes_integer() {
            // JSON integer → visit_i64 / visit_u64 → NumberValue(f64)
            let v: Value = serde_json::from_str("42").unwrap();
            assert_eq!(v.as_number(), Some(42.0));
        }

        #[test]
        fn value_deserializes_negative_integer() {
            let v: Value = serde_json::from_str("-100").unwrap();
            assert_eq!(v.as_number(), Some(-100.0));
        }

        #[test]
        fn value_deserializes_large_integer() {
            // 2^53 is exactly representable in f64.
            let v: Value = serde_json::from_str("9007199254740992").unwrap();
            assert_eq!(v.as_number(), Some(9007199254740992.0));
        }

        #[test]
        fn value_deep_nesting_binary_respects_recursion_limit() {
            // Value → ListValue → Value is recursive. Binary decode must
            // hit our RECURSION_LIMIT, not stack-overflow.
            use buffa::{DecodeError, Message};
            // Build a deeply-nested ListValue chain via wire bytes.
            // Each level: Value{list:ListValue{values:[Value{list:...}]}}
            // Value.list (field 6, oneof, wire type 2 length-delimited):
            //   tag=0x32, len, <ListValue bytes>
            // ListValue.values (field 1, repeated Value, length-delimited):
            //   tag=0x0a, len, <Value bytes>
            // Innermost: empty Value (0 bytes).
            let mut payload = alloc::vec::Vec::new();
            for _ in 0..200 {
                // Wrap: ListValue { values: [current payload as Value] }
                let mut lv = alloc::vec::Vec::new();
                lv.push(0x0a); // tag: field 1 wire 2
                lv.push(payload.len() as u8); // assumes < 128 — fine for small depth
                lv.extend_from_slice(&payload);
                // Wrap: Value { list_value: lv }
                let mut v = alloc::vec::Vec::new();
                v.push(0x32); // tag: field 6 wire 2
                v.push(lv.len() as u8);
                v.extend_from_slice(&lv);
                payload = v;
                // Stop growing once payload length exceeds single-byte varint.
                if payload.len() >= 120 {
                    break;
                }
            }
            // At ~120 bytes we have ~30 levels. Need to go deeper. Use proper
            // varint encoding for larger lengths.
            use buffa::encoding::encode_varint;
            for _ in 0..200 {
                let mut lv = alloc::vec::Vec::new();
                lv.push(0x0a);
                encode_varint(payload.len() as u64, &mut lv);
                lv.extend_from_slice(&payload);
                let mut v = alloc::vec::Vec::new();
                v.push(0x32);
                encode_varint(lv.len() as u64, &mut v);
                v.extend_from_slice(&lv);
                payload = v;
            }
            // ~230 levels of Value/ListValue nesting, each level consumes 2
            // from the depth budget (Value + ListValue are each a merge).
            // Default RECURSION_LIMIT is 100, so this should be rejected.
            let result = Value::decode(&mut payload.as_slice());
            assert!(
                matches!(result, Err(DecodeError::RecursionLimitExceeded)),
                "deep nesting must hit recursion limit, got: {result:?}"
            );
        }

        #[test]
        fn value_deep_nesting_json_bounded() {
            // serde_json has its own recursion limit (default 128). A deeply-
            // nested JSON array deserialize into Value must error cleanly,
            // not stack-overflow. serde_json returns its own error type, not
            // our DecodeError, so just assert is_err().
            let deep = alloc::format!("{}null{}", "[".repeat(200), "]".repeat(200));
            let result: Result<Value, _> = serde_json::from_str(&deep);
            assert!(result.is_err(), "200-level JSON nesting must be rejected");
        }
    }
}

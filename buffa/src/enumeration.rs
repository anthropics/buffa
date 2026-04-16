//! Enum support: the [`Enumeration`] trait and [`EnumValue<E>`] open-enum wrapper.
//!
//! Generated enum types implement [`Enumeration`]. For **open** enum fields
//! (the editions default), generated message structs use [`EnumValue<E>`] to
//! hold either a `Known(E)` variant or an `Unknown(i32)` wire value.

use core::fmt;
use core::hash::Hash;

/// Trait implemented by all generated protobuf enum types.
pub trait Enumeration: Clone + Copy + PartialEq + Eq + Hash + fmt::Debug {
    /// Convert from an `i32` wire value to the enum.
    ///
    /// Returns `Some` for known variants, `None` for unknown values.
    fn from_i32(value: i32) -> Option<Self>;

    /// Convert the enum to its `i32` wire value.
    fn to_i32(&self) -> i32;

    /// The name of this enum variant as it appears in the `.proto` file.
    fn proto_name(&self) -> &'static str;

    /// Look up a variant by its protobuf name string.
    ///
    /// Returns `Some` for recognized names, `None` for unrecognized names.
    /// The default implementation always returns `None`; generated code
    /// overrides this with a match on all known variant names.
    fn from_proto_name(_name: &str) -> Option<Self> {
        None
    }

    /// All known variants of this enum, in proto declaration order.
    ///
    /// Generated `impl Enumeration` blocks override this with a static
    /// slice of every variant. The default implementation returns an
    /// empty slice so out-of-tree consumers implementing this trait
    /// against an older codegen version continue to compile — they
    /// should override the default once regenerated.
    ///
    /// # Example
    ///
    /// ```ignore
    /// for variant in MyEnum::values() {
    ///     println!("{:?} = {}", variant, variant.to_i32());
    /// }
    /// assert!(MyEnum::values().contains(&MyEnum::Active));
    /// ```
    fn values() -> &'static [Self] {
        &[]
    }
}

/// A protobuf enum field value that can hold either a known variant or an
/// unknown `i32` value.
///
/// Used for **open** enums (the default in editions). Open enums accept any
/// `i32` value on the wire, even if it doesn't correspond to a known variant.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnumValue<E: Enumeration> {
    /// A known enum variant.
    Known(E),
    /// An unknown value received on the wire.
    Unknown(i32),
}

impl<E: Enumeration> EnumValue<E> {
    /// Get the `i32` wire value.
    #[inline]
    pub fn to_i32(&self) -> i32 {
        match self {
            Self::Known(e) => e.to_i32(),
            Self::Unknown(v) => *v,
        }
    }

    /// Returns `true` if this is a known enum variant.
    ///
    /// This is a shorthand for `self.as_known().is_some()`, mirroring the
    /// [`MessageField::is_set`](crate::MessageField::is_set) pattern.
    #[inline]
    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }

    /// Returns `true` if the wire value was not recognized as a known variant.
    #[inline]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// Try to convert to a known enum variant.
    #[inline]
    pub fn as_known(&self) -> Option<E> {
        match self {
            Self::Known(e) => Some(*e),
            Self::Unknown(_) => None,
        }
    }
}

impl<E: Enumeration> From<i32> for EnumValue<E> {
    fn from(value: i32) -> Self {
        match E::from_i32(value) {
            Some(e) => Self::Known(e),
            None => Self::Unknown(value),
        }
    }
}

impl<E: Enumeration> From<E> for EnumValue<E> {
    fn from(value: E) -> Self {
        Self::Known(value)
    }
}

/// Compare an [`EnumValue`] directly with a known variant: `field == MyEnum::Foo`.
///
/// Returns `true` only when `self` is `Known(e)` and `e == *other`.
/// An `Unknown` value is never equal to any known variant.
///
/// # Asymmetry
///
/// Only the `EnumValue == E` direction is available; the reverse `E ==
/// EnumValue` is not implementable due to the orphan rule. If you need to
/// compare in that order, use `field.as_known() == Some(variant)` instead.
impl<E: Enumeration> PartialEq<E> for EnumValue<E> {
    fn eq(&self, other: &E) -> bool {
        match self {
            Self::Known(e) => e == other,
            Self::Unknown(_) => false,
        }
    }
}

impl<E: Enumeration> fmt::Debug for EnumValue<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(e) => write!(f, "{:?}", e),
            Self::Unknown(v) => write!(f, "Unknown({v})"),
        }
    }
}

impl<E: Enumeration> fmt::Display for EnumValue<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(e) => f.write_str(e.proto_name()),
            Self::Unknown(v) => write!(f, "{v}"),
        }
    }
}

#[cfg(feature = "json")]
impl<E: Enumeration> serde::Serialize for EnumValue<E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Known(e) => s.serialize_str(e.proto_name()),
            Self::Unknown(v) => s.serialize_i32(*v),
        }
    }
}

#[cfg(feature = "json")]
impl<'de, E: Enumeration> serde::Deserialize<'de> for EnumValue<E> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct EnumValueVisitor<E>(core::marker::PhantomData<E>);

        impl<'de, E: Enumeration> serde::de::Visitor<'de> for EnumValueVisitor<E> {
            type Value = EnumValue<E>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a protobuf enum name string or integer value")
            }

            /// Handles JSON `null` input, used for `google.protobuf.NullValue`
            /// whose JSON representation is the literal `null`.
            fn visit_unit<Err: serde::de::Error>(self) -> Result<EnumValue<E>, Err> {
                Ok(EnumValue::from(0))
            }

            fn visit_str<Err: serde::de::Error>(self, v: &str) -> Result<EnumValue<E>, Err> {
                match E::from_proto_name(v) {
                    Some(e) => Ok(EnumValue::Known(e)),
                    None => {
                        #[cfg(all(feature = "std", feature = "json"))]
                        if crate::json::ignore_unknown_enum_values() {
                            return Ok(EnumValue::from(0));
                        }
                        Err(serde::de::Error::unknown_variant(v, &[]))
                    }
                }
            }

            fn visit_i64<Err: serde::de::Error>(self, v: i64) -> Result<EnumValue<E>, Err> {
                let n = i32::try_from(v).map_err(|_| {
                    serde::de::Error::invalid_value(
                        serde::de::Unexpected::Signed(v),
                        &"an i32 enum value",
                    )
                })?;
                Ok(EnumValue::from(n))
            }

            fn visit_u64<Err: serde::de::Error>(self, v: u64) -> Result<EnumValue<E>, Err> {
                let n = i32::try_from(v).map_err(|_| {
                    serde::de::Error::invalid_value(
                        serde::de::Unexpected::Unsigned(v),
                        &"an i32 enum value",
                    )
                })?;
                Ok(EnumValue::from(n))
            }
        }

        d.deserialize_any(EnumValueVisitor(core::marker::PhantomData))
    }
}

impl<E: Enumeration> Default for EnumValue<E> {
    /// Returns the default value for this enum field.
    ///
    /// Per the protobuf specification, the default value for an enum field is
    /// the variant whose wire value is `0`.  If `0` is not a defined variant of
    /// `E`, this returns [`EnumValue::Unknown(0)`](EnumValue::Unknown), which is
    /// the correct protobuf default but may be surprising — the field's default
    /// is technically an unknown variant.
    fn default() -> Self {
        Self::from(0)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, E: Enumeration + arbitrary::Arbitrary<'a>> arbitrary::Arbitrary<'a> for EnumValue<E> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Bias towards known variants (75%) but also generate unknown values
        // (25%) to exercise unknown-value handling.
        if u.ratio(3, 4)? {
            Ok(EnumValue::Known(E::arbitrary(u)?))
        } else {
            Ok(EnumValue::Unknown(i32::arbitrary(u)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum Color {
        Red,
        Green,
        Blue,
    }

    impl Enumeration for Color {
        fn from_i32(v: i32) -> Option<Self> {
            match v {
                0 => Some(Color::Red),
                1 => Some(Color::Green),
                2 => Some(Color::Blue),
                _ => None,
            }
        }
        fn to_i32(&self) -> i32 {
            *self as i32
        }
        fn proto_name(&self) -> &'static str {
            match self {
                Color::Red => "RED",
                Color::Green => "GREEN",
                Color::Blue => "BLUE",
            }
        }
        fn from_proto_name(name: &str) -> Option<Self> {
            match name {
                "RED" => Some(Color::Red),
                "GREEN" => Some(Color::Green),
                "BLUE" => Some(Color::Blue),
                _ => None,
            }
        }
    }

    // ---- From<i32> -------------------------------------------------------

    #[test]
    fn from_i32_known_value_produces_known_variant() {
        let v: EnumValue<Color> = EnumValue::from(1);
        assert_eq!(v, EnumValue::Known(Color::Green));
    }

    #[test]
    fn from_i32_unknown_values_produce_unknown_variant() {
        // Values not in Color's variant set (0/1/2) round-trip as Unknown(v).
        for v in [99, -1, i32::MIN, i32::MAX] {
            let ev: EnumValue<Color> = EnumValue::from(v);
            assert_eq!(ev, EnumValue::Unknown(v), "from({v})");
            assert_eq!(ev.to_i32(), v, "to_i32 roundtrip for {v}");
        }
    }

    // ---- From<E> ---------------------------------------------------------

    #[test]
    fn from_enum_variant_produces_known() {
        let v = EnumValue::from(Color::Blue);
        assert_eq!(v, EnumValue::Known(Color::Blue));
    }

    // ---- to_i32 ----------------------------------------------------------

    #[test]
    fn to_i32_known_returns_variant_value() {
        let v = EnumValue::Known(Color::Green);
        assert_eq!(v.to_i32(), 1);
    }

    #[test]
    fn to_i32_unknown_returns_raw_value() {
        let v: EnumValue<Color> = EnumValue::Unknown(42);
        assert_eq!(v.to_i32(), 42);
    }

    #[test]
    fn to_i32_unknown_negative_returns_raw_value() {
        let v: EnumValue<Color> = EnumValue::Unknown(-1);
        assert_eq!(v.to_i32(), -1);
    }

    // ---- as_known --------------------------------------------------------

    #[test]
    fn as_known_returns_some_for_known_variant() {
        let v = EnumValue::Known(Color::Red);
        assert_eq!(v.as_known(), Some(Color::Red));
    }

    #[test]
    fn as_known_returns_none_for_unknown() {
        let v: EnumValue<Color> = EnumValue::Unknown(7);
        assert_eq!(v.as_known(), None);
    }

    // ---- Debug -----------------------------------------------------------

    #[test]
    fn debug_known_variant_uses_enum_debug() {
        let v = EnumValue::Known(Color::Blue);
        assert_eq!(format!("{v:?}"), "Blue");
    }

    #[test]
    fn debug_unknown_shows_raw_value() {
        let v: EnumValue<Color> = EnumValue::Unknown(99);
        assert_eq!(format!("{v:?}"), "Unknown(99)");
    }

    // ---- Display ---------------------------------------------------------

    #[test]
    fn display_known_variant_uses_proto_name() {
        let v = EnumValue::Known(Color::Green);
        assert_eq!(format!("{v}"), "GREEN");
    }

    #[test]
    fn display_unknown_shows_raw_integer() {
        let v: EnumValue<Color> = EnumValue::Unknown(99);
        assert_eq!(format!("{v}"), "99");
    }

    #[test]
    fn display_unknown_negative_shows_signed_integer() {
        let v: EnumValue<Color> = EnumValue::Unknown(-1);
        assert_eq!(format!("{v}"), "-1");
    }

    // ---- Default ---------------------------------------------------------

    #[test]
    fn default_produces_known_when_zero_is_valid() {
        // Color::Red = 0, so the default should be Known(Red).
        let v: EnumValue<Color> = EnumValue::default();
        assert_eq!(v, EnumValue::Known(Color::Red));
    }

    #[test]
    fn default_produces_unknown_when_zero_is_not_a_valid_variant() {
        // An enum whose first valid variant is non-zero.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        enum Status {
            Active,
            Inactive,
        }
        impl Enumeration for Status {
            fn from_i32(v: i32) -> Option<Self> {
                match v {
                    1 => Some(Status::Active),
                    2 => Some(Status::Inactive),
                    _ => None,
                }
            }
            fn to_i32(&self) -> i32 {
                match self {
                    Status::Active => 1,
                    Status::Inactive => 2,
                }
            }
            fn proto_name(&self) -> &'static str {
                match self {
                    Status::Active => "ACTIVE",
                    Status::Inactive => "INACTIVE",
                }
            }
        }

        // Default must be Unknown(0) when 0 is not a valid variant.
        let v: EnumValue<Status> = EnumValue::default();
        assert_eq!(v, EnumValue::Unknown(0));
    }

    // ---- is_known / is_unknown -------------------------------------------

    #[test]
    fn is_known_returns_true_for_known_variant() {
        let v = EnumValue::Known(Color::Red);
        assert!(v.is_known());
        assert!(!v.is_unknown());
    }

    #[test]
    fn is_unknown_returns_true_for_unknown_value() {
        let v: EnumValue<Color> = EnumValue::Unknown(99);
        assert!(v.is_unknown());
        assert!(!v.is_known());
    }

    // ---- PartialEq<E> ----------------------------------------------------

    #[test]
    fn known_variant_equals_same_variant() {
        let v = EnumValue::Known(Color::Green);
        assert!(v == Color::Green);
    }

    #[test]
    fn known_variant_not_equal_to_different_variant() {
        let v = EnumValue::Known(Color::Red);
        assert!(!(v == Color::Green));
    }

    #[test]
    fn unknown_value_never_equals_known_variant() {
        // Even if the numeric value matches, Unknown != any known variant.
        let v: EnumValue<Color> = EnumValue::Unknown(1); // same wire value as Green
        assert!(!(v == Color::Green));
    }

    #[test]
    fn unknown_value_with_out_of_range_int_not_equal() {
        let v: EnumValue<Color> = EnumValue::Unknown(99);
        assert!(!(v == Color::Red));
    }

    // ---- serde -------------------------------------------------------

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        #[test]
        fn known_variant_serializes_as_proto_name() {
            let v = EnumValue::Known(Color::Green);
            assert_eq!(serde_json::to_string(&v).unwrap(), r#""GREEN""#);
        }

        #[test]
        fn unknown_variant_serializes_as_integer() {
            let v: EnumValue<Color> = EnumValue::Unknown(99);
            assert_eq!(serde_json::to_string(&v).unwrap(), "99");
        }

        #[test]
        fn deserialize_table() {
            use EnumValue::{Known, Unknown};
            // Some(v) = succeeds with v, None = is_err().
            #[rustfmt::skip]
            let cases: &[(&str, Option<EnumValue<Color>>)] = &[
                (r#""RED""#,    Some(Known(Color::Red))),    // proto name string
                ("1",           Some(Known(Color::Green))),  // integer (known)
                ("99",          Some(Unknown(99))),          // integer (unknown)
                ("null",        Some(Known(Color::Red))),    // null → default (Red = 0)
                (r#""PURPLE""#, None),                       // unknown string → error
                ("9223372036854775807", None),               // i64::MAX > i32 → error
            ];
            for &(json, expected) in cases {
                let result = serde_json::from_str::<EnumValue<Color>>(json);
                assert_eq!(result.ok(), expected, "input: {json}");
            }
        }

        #[test]
        fn deserialize_unknown_string_returns_default_when_lenient() {
            use crate::json::{with_json_parse_options, JsonParseOptions};
            let opts = JsonParseOptions {
                ignore_unknown_enum_values: true,
                ..Default::default()
            };
            let v: EnumValue<Color> =
                with_json_parse_options(&opts, || serde_json::from_str(r#""PURPLE""#).unwrap());
            // Unknown string → default enum value (0 = Red).
            assert_eq!(v, EnumValue::Known(Color::Red));
        }

        #[test]
        fn deserialize_known_string_unaffected_by_lenient_mode() {
            use crate::json::{with_json_parse_options, JsonParseOptions};
            let opts = JsonParseOptions {
                ignore_unknown_enum_values: true,
                ..Default::default()
            };
            let v: EnumValue<Color> =
                with_json_parse_options(&opts, || serde_json::from_str(r#""BLUE""#).unwrap());
            assert_eq!(v, EnumValue::Known(Color::Blue));
        }

        #[test]
        fn round_trip_known_variant() {
            let original = EnumValue::Known(Color::Blue);
            let json = serde_json::to_string(&original).unwrap();
            let recovered: EnumValue<Color> = serde_json::from_str(&json).unwrap();
            assert_eq!(original, recovered);
        }
    }
}

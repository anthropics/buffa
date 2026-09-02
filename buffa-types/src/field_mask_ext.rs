//! Ergonomic helpers for [`google::protobuf::FieldMask`](crate::google::protobuf::FieldMask).

use alloc::string::String;

use crate::google::protobuf::FieldMask;

impl FieldMask {
    /// Create a [`FieldMask`] from an iterator of field paths.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::FieldMask;
    ///
    /// let mask = FieldMask::from_paths(["user.name", "user.email"]);
    /// assert!(mask.contains("user.name"));
    /// ```
    pub fn from_paths(paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            paths: paths.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }

    /// Returns `true` if `path` is present in this field mask.
    ///
    /// Comparison is exact (case-sensitive, no wildcard expansion).
    /// Runs in O(n) time where n is the number of paths.
    pub fn contains(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    /// Returns the number of paths in the field mask.
    #[inline]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Returns `true` if the field mask contains no paths.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Returns an iterator over the paths in the field mask.
    #[inline]
    pub fn iter(&self) -> core::slice::Iter<'_, String> {
        self.paths.iter()
    }
}

impl<'a> IntoIterator for &'a FieldMask {
    type Item = &'a String;
    type IntoIter = core::slice::Iter<'a, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.paths.iter()
    }
}

impl IntoIterator for FieldMask {
    type Item = String;
    type IntoIter = alloc::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.paths.into_iter()
    }
}

// ── proto JSON camelCase ↔ snake_case conversion ──────────────────────────────
//
// The shared conversion primitives live in `buffa::json_helpers::wkt`. Both
// this typed serde impl and `buffa-descriptor`'s reflective JSON codec call
// into the same code, so the two paths can't drift on edge cases the
// conformance suite exercises.

#[cfg(feature = "json")]
use alloc::vec::Vec;
#[cfg(feature = "json")]
use buffa::json_helpers::wkt::{camel_to_snake, field_mask_path_round_trips, snake_to_camel};

// ── serde impls ──────────────────────────────────────────────────────────────

#[cfg(feature = "json")]
impl serde::Serialize for FieldMask {
    /// Serializes as a comma-separated string of lowerCamelCase field paths.
    ///
    /// # Errors
    ///
    /// Returns an error if any path is not a valid proto3 JSON field mask
    /// path: an empty component, a character outside `[a-z0-9_.]`, or a path
    /// that cannot round-trip through camelCase (already camelCase,
    /// consecutive underscores, a digit immediately after an underscore).
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let camel_paths: Vec<String> = self
            .paths
            .iter()
            .map(|p| {
                if !field_mask_path_round_trips(p) {
                    return Err(serde::ser::Error::custom(alloc::format!(
                        "FieldMask path '{p}' is not a valid field mask path"
                    )));
                }
                Ok(snake_to_camel(p))
            })
            .collect::<Result<_, _>>()?;
        s.serialize_str(&camel_paths.join(","))
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for FieldMask {
    /// Deserializes from a comma-separated string of lowerCamelCase field paths.
    ///
    /// # Errors
    ///
    /// Returns an error if any path is not a valid lowerCamelCase path in the
    /// JSON representation.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        let paths = if s.is_empty() {
            Vec::new()
        } else {
            s.split(',')
                .map(|component| {
                    if component.contains('_') {
                        return Err(serde::de::Error::custom(alloc::format!(
                            "FieldMask path '{component}' contains underscore, \
                             which is invalid in JSON (lowerCamelCase) representation"
                        )));
                    }
                    let snake = camel_to_snake(component);
                    if !field_mask_path_round_trips(&snake) || snake_to_camel(&snake) != component {
                        return Err(serde::de::Error::custom(alloc::format!(
                            "FieldMask path '{component}' is not a valid lowerCamelCase path"
                        )));
                    }
                    Ok(snake)
                })
                .collect::<Result<_, _>>()?
        };
        Ok(Self {
            paths,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_paths_empty() {
        let mask = FieldMask::from_paths(core::iter::empty::<&str>());
        assert!(mask.paths.is_empty());
        assert!(mask.is_empty());
        assert_eq!(mask.len(), 0);
    }

    #[test]
    fn len_and_is_empty() {
        let mask = FieldMask::from_paths(["a", "b", "c"]);
        assert_eq!(mask.len(), 3);
        assert!(!mask.is_empty());
    }

    #[test]
    fn iter_yields_all_paths() {
        let mask = FieldMask::from_paths(["x.y", "z"]);
        let collected: Vec<_> = mask.iter().collect();
        assert_eq!(collected, [&"x.y".to_string(), &"z".to_string()]);
    }

    #[test]
    fn from_paths_string_slices() {
        let mask = FieldMask::from_paths(["a.b", "c.d"]);
        assert_eq!(mask.paths, vec!["a.b", "c.d"]);
    }

    #[test]
    fn from_paths_owned_strings() {
        let paths = vec!["x".to_string(), "y.z".to_string()];
        let mask = FieldMask::from_paths(paths);
        assert_eq!(mask.paths, vec!["x", "y.z"]);
    }

    #[test]
    fn contains_returns_true_for_present_path() {
        let mask = FieldMask::from_paths(["user.name", "user.email"]);
        assert!(mask.contains("user.name"));
        assert!(mask.contains("user.email"));
    }

    #[test]
    fn contains_returns_false_for_absent_path() {
        let mask = FieldMask::from_paths(["user.name"]);
        assert!(!mask.contains("user.age"));
    }

    #[test]
    fn contains_is_exact_match_not_prefix() {
        let mask = FieldMask::from_paths(["user"]);
        assert!(!mask.contains("user.name"));
    }

    #[test]
    fn contains_is_case_sensitive() {
        let mask = FieldMask::from_paths(["user.Name"]);
        assert!(!mask.contains("user.name"));
    }

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        // ---- camelCase conversion unit tests ------------------------------

        #[test]
        fn snake_to_camel_simple() {
            assert_eq!(snake_to_camel("foo_bar"), "fooBar");
            assert_eq!(snake_to_camel("foo"), "foo");
            assert_eq!(snake_to_camel("foo_bar_baz"), "fooBarBaz");
        }

        #[test]
        fn snake_to_camel_dotted() {
            assert_eq!(snake_to_camel("user.first_name"), "user.firstName");
        }

        #[test]
        fn camel_to_snake_simple() {
            assert_eq!(camel_to_snake("fooBar"), "foo_bar");
            assert_eq!(camel_to_snake("foo"), "foo");
            assert_eq!(camel_to_snake("fooBarBaz"), "foo_bar_baz");
        }

        #[test]
        fn camel_to_snake_preserves_leading_underscore() {
            assert_eq!(camel_to_snake("FooBar"), "_foo_bar");
            assert_eq!(camel_to_snake("Foo"), "_foo");
            assert_eq!(camel_to_snake("A.B"), "_a._b");
        }

        #[test]
        fn camel_to_snake_dotted() {
            assert_eq!(camel_to_snake("user.firstName"), "user.first_name");
        }

        #[test]
        fn snake_to_camel_camel_to_snake_roundtrip() {
            let original = "user.first_name";
            assert_eq!(camel_to_snake(&snake_to_camel(original)), original);
        }

        // ---- serde roundtrips ---------------------------------------------

        #[test]
        fn field_mask_empty_roundtrip() {
            let m = FieldMask::from_paths(core::iter::empty::<&str>());
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, r#""""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert!(back.paths.is_empty());
        }

        #[test]
        fn field_mask_single_path_roundtrip() {
            let m = FieldMask::from_paths(["foo_bar"]);
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, r#""fooBar""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert_eq!(back.paths, ["foo_bar"]);
        }

        #[test]
        fn field_mask_multiple_paths_roundtrip() {
            let m = FieldMask::from_paths(["user_id", "display_name"]);
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, r#""userId,displayName""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert_eq!(back.paths, ["user_id", "display_name"]);
        }

        #[test]
        fn field_mask_dotted_path_roundtrip() {
            let m = FieldMask::from_paths(["user.email_address"]);
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, r#""user.emailAddress""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert_eq!(back.paths, ["user.email_address"]);
        }

        #[test]
        fn field_mask_leading_underscore_roundtrip() {
            let m = FieldMask::from_paths(["_foo", "foo._bar", "foo._b_bar"]);
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, r#""Foo,foo.Bar,foo.BBar""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert_eq!(back.paths, ["_foo", "foo._bar", "foo._b_bar"]);
        }

        // ---- serialize validation -------------------------------------------

        #[test]
        fn serialize_rejects_already_camel_case_path() {
            let m = FieldMask::from_paths(["fooBar"]);
            assert!(serde_json::to_string(&m).is_err());
        }

        #[test]
        fn serialize_rejects_digit_after_underscore() {
            let m = FieldMask::from_paths(["foo_3_bar"]);
            assert!(serde_json::to_string(&m).is_err());
        }

        #[test]
        fn serialize_rejects_consecutive_underscores() {
            let m = FieldMask::from_paths(["foo__bar"]);
            assert!(serde_json::to_string(&m).is_err());
        }

        // ---- deserialize validation -----------------------------------------

        #[test]
        fn deserialize_rejects_underscore_in_json() {
            let result: Result<FieldMask, _> = serde_json::from_str(r#""foo_bar""#);
            assert!(result.is_err());
        }

        #[test]
        fn deserialize_rejects_underscore_in_multi_path() {
            let result: Result<FieldMask, _> = serde_json::from_str(r#""fooBar,baz_qux""#);
            assert!(result.is_err());
        }

        #[test]
        fn serialize_accepts_path_with_digit_not_after_underscore() {
            let m = FieldMask::from_paths(["foo3_bar"]);
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, r#""foo3Bar""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert_eq!(back.paths, ["foo3_bar"]);
        }

        #[test]
        fn serialize_rejects_trailing_underscore() {
            let m = FieldMask::from_paths(["foo_"]);
            assert!(serde_json::to_string(&m).is_err());
        }

        #[test]
        fn serialize_rejects_invalid_path_characters() {
            for path in [
                " ", "foo bar", "foo-bar", "foo/bar", "3d", "", ".foo", "foo.", "foo..bar",
            ] {
                let m = FieldMask::from_paths([path]);
                assert!(
                    serde_json::to_string(&m).is_err(),
                    "path {path:?} must be rejected"
                );
            }
        }

        #[test]
        fn wildcard_roundtrip() {
            let mask = FieldMask::from_paths(["*"]);
            let json = serde_json::to_string(&mask).unwrap();
            assert_eq!(json, r#""*""#);
            let back: FieldMask = serde_json::from_str(&json).unwrap();
            assert_eq!(back.paths, ["*"]);
        }

        #[test]
        fn deserialize_rejects_invalid_path_characters() {
            for json in [
                r#"" ""#,
                r#""foo, barBaz""#,
                r#""foo,bar-baz""#,
                r#""foo/bar""#,
                r#""3d""#,
                r#""foo,""#,
                "\".foo\"",
                "\"foo.\"",
                "\"foo..bar\"",
            ] {
                let result: Result<FieldMask, _> = serde_json::from_str(json);
                assert!(result.is_err(), "JSON {json} must be rejected");
            }
        }
    }
}

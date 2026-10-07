//! Ergonomic helpers for [`google::protobuf::FieldMask`](crate::google::protobuf::FieldMask).

use alloc::string::String;
use alloc::vec::Vec;

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
    /// Comparison is exact (case-sensitive, no wildcard expansion), so a mask
    /// of `user` does not contain `user.name`. [`covers`](Self::covers) also
    /// returns `true` for a path whose ancestor is in the mask.
    /// Runs in O(n) time where n is the number of paths.
    pub fn contains(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    /// Returns `true` if this mask contains `path` or an ancestor of `path`.
    ///
    /// For example, `user` covers `user.name`, but does not cover `username`.
    /// Comparison is case-sensitive and uses dots as component boundaries.
    /// Runs in O(n) path comparisons where n is the number of mask paths.
    ///
    /// The mask is read as a plain list of paths:
    ///
    /// - Coverage runs from ancestor to descendant only. A mask that contains
    ///   `settings.theme` does not cover `settings`, so a handler that updates
    ///   `settings` as a unit must call `covers` for each field of `settings`
    ///   that it writes.
    /// - An empty mask does not cover any path, and an unset mask field reads
    ///   as an empty mask. `google.protobuf.FieldMask` defines an absent
    ///   update mask as every field. A handler that follows that rule must
    ///   check [`is_empty`](Self::is_empty) before it calls `covers`.
    /// - `*` is an ordinary path: a mask that contains `*` does not cover
    ///   `user`.
    /// - Paths are compared byte for byte and are not validated against a
    ///   message descriptor. A mask deserialized from JSON contains proto
    ///   field names (`user.display_name`), not JSON names
    ///   (`user.displayName`), so `path` must use proto field names.
    ///   [`from_paths`](Self::from_paths) stores its input unchanged.
    /// - Backtick quoting is not parsed. Where an API writes a map key that
    ///   contains a dot as `` labels.`a.b` ``, the dot inside the backticks is
    ///   a component boundary here: a mask that contains `` labels.`a ``
    ///   covers that path.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::FieldMask;
    ///
    /// let mask = FieldMask::from_paths(["user", "settings.theme"]);
    /// assert!(mask.covers("user"));
    /// assert!(mask.covers("user.name"));
    /// assert!(!mask.covers("username"));
    /// assert!(!mask.covers("settings"));
    /// assert!(!FieldMask::default().covers("user"));
    /// ```
    pub fn covers(&self, path: &str) -> bool {
        self.paths
            .iter()
            .any(|ancestor| path_is_covered(path, ancestor))
    }

    /// Sorts the paths component by component, comparing each component as a
    /// `str`, and removes duplicates and paths that an ancestor in the mask
    /// already covers.
    ///
    /// The normalized mask [`covers`](Self::covers) exactly the paths it
    /// covered before, and paths are compared as `covers` compares them. For
    /// paths made of field names, the order is the same as sorting the paths
    /// as strings. Runs in O(n log n) path comparisons and operates in place
    /// without allocating new path strings.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::google::protobuf::FieldMask;
    ///
    /// let mut mask = FieldMask::from_paths(["user.name", "settings.theme", "user", "user"]);
    /// mask.normalize();
    /// assert_eq!(mask.paths, ["settings.theme", "user"]);
    /// assert!(mask.covers("user.name"));
    /// ```
    pub fn normalize(&mut self) {
        // Component ordering keeps every parent's descendants adjacent even
        // when a caller supplies paths outside the usual proto identifier syntax.
        self.paths
            .sort_unstable_by(|a, b| a.split('.').cmp(b.split('.')));
        self.paths
            .dedup_by(|path, ancestor| path_is_covered(path, ancestor));
    }

    /// Returns a normalized mask covering every path covered by either mask.
    ///
    /// Uses the same path rules as [`covers`](Self::covers): an empty mask
    /// covers nothing, `*` is literal, and paths are not validated against a
    /// descriptor. Neither input is modified. The result contains only the
    /// combined paths, without unknown protobuf fields from either input.
    /// Runs in O((n + m) log(n + m)) path comparisons.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::FieldMask;
    ///
    /// let left = FieldMask::from_paths(["user.name", "settings.theme"]);
    /// let right = FieldMask::from_paths(["user", "settings.locale"]);
    /// assert_eq!(left.union(&right).paths, ["settings.locale", "settings.theme", "user"]);
    /// ```
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut mask = Self::from_paths(self.paths.iter().chain(&other.paths).cloned());
        mask.normalize();
        mask
    }

    /// Returns a normalized mask covering only paths covered by both masks.
    ///
    /// When one mask selects an ancestor of a path in the other, the result
    /// selects the more specific path: `user` intersected with `user.name`
    /// is `user.name`.
    ///
    /// Uses the same path rules as [`covers`](Self::covers): an empty mask
    /// covers nothing, `*` is literal, and paths are not validated against a
    /// descriptor. Neither input is modified. The result contains only the
    /// shared paths, without unknown protobuf fields from either input.
    /// Runs in O(n log n + m log m) path comparisons.
    ///
    /// # Example
    ///
    /// ```rust
    /// use buffa_types::FieldMask;
    ///
    /// let left = FieldMask::from_paths(["user", "settings.theme"]);
    /// let right = FieldMask::from_paths(["user.name", "settings"]);
    /// assert_eq!(left.intersection(&right).paths, ["settings.theme", "user.name"]);
    /// ```
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        if self.is_empty() || other.is_empty() {
            return Self::default();
        }
        let mut left = normalized_paths(&self.paths).into_iter().peekable();
        let mut right = normalized_paths(&other.paths).into_iter().peekable();
        let mut mask = Self::default();
        while let (Some(&a), Some(&b)) = (left.peek(), right.peek()) {
            if path_is_covered(a, b) {
                mask.paths.push(a.into());
                left.next();
            } else if path_is_covered(b, a) {
                mask.paths.push(b.into());
                right.next();
            } else if a.split('.').lt(b.split('.')) {
                left.next();
            } else {
                right.next();
            }
        }
        mask
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

/// Whether `ancestor` is `path` or one of its ancestors.
fn path_is_covered(path: &str, ancestor: &str) -> bool {
    path.strip_prefix(ancestor)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

fn normalized_paths(paths: &[String]) -> Vec<&str> {
    let mut paths: Vec<_> = paths.iter().map(String::as_str).collect();
    paths.sort_unstable_by(|a, b| a.split('.').cmp(b.split('.')));
    paths.dedup_by(|path, ancestor| path_is_covered(path, ancestor));
    paths
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
    use alloc::string::ToString;
    use alloc::{vec, vec::Vec};

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

    #[test]
    fn covers_exact_paths_and_descendants_only() {
        let mask = FieldMask::from_paths(["user", "settings.theme", "User.Name"]);
        for path in [
            "user",
            "user.name",
            "user.name.first",
            "settings.theme.color",
            "User.Name",
        ] {
            assert!(mask.covers(path), "{path}");
        }
        for path in [
            "username",
            "user_name",
            "settings",
            "settings.themes",
            "User.name",
            "",
        ] {
            assert!(!mask.covers(path), "{path}");
        }
        assert!(!FieldMask::default().covers("user"));
        assert!(!FieldMask::from_paths(["*"]).covers("user"));
    }

    #[test]
    fn covers_treats_every_dot_as_a_component_boundary() {
        // An empty path, a trailing dot and a backtick-quoted map key get no
        // special handling: a path is its text, split at each dot.
        let mask = FieldMask::from_paths(["user", "labels.`a.b`"]);
        assert!(mask.covers("user."));
        assert!(mask.covers("user..name"));
        assert!(!mask.covers("user`"));
        assert!(mask.covers("labels.`a.b`"));
        assert!(mask.covers("labels.`a.b`.c"));
        assert!(!mask.covers("labels.`a"));
        assert!(!mask.covers("labels.`a.b"));
        assert!(FieldMask::from_paths(["labels.`a"]).covers("labels.`a.b`"));

        let trailing = FieldMask::from_paths(["user."]);
        assert!(trailing.covers("user."));
        assert!(trailing.covers("user..name"));
        assert!(!trailing.covers("user"));
        assert!(!trailing.covers("user.name"));

        let empty_path = FieldMask::from_paths([""]);
        assert!(empty_path.covers(""));
        assert!(empty_path.covers(".user"));
        assert!(!empty_path.covers("user"));
    }

    #[test]
    fn normalize_reads_empty_components_and_backticks_as_plain_text() {
        let mut mask =
            FieldMask::from_paths(["user.", "user", "", "labels.`a.b`.c", "labels.`a.b`", "*"]);
        mask.normalize();
        assert_eq!(mask.paths, ["", "*", "labels.`a.b`", "user"]);

        // `user.` is a child of `user` with an empty name, not a parent of
        // `user.name`.
        let mut mask = FieldMask::from_paths(["user.name", "user."]);
        mask.normalize();
        assert_eq!(mask.paths, ["user.", "user.name"]);
    }

    #[test]
    fn normalize_removes_duplicates_and_redundant_descendants() {
        let mut mask = FieldMask::from_paths([
            "user.name.first",
            "settings.theme",
            "user.name",
            "user",
            "user",
            "settings.theme.color",
            "settings.locale",
            "username",
            "user_name",
        ]);
        mask.normalize();
        assert_eq!(
            mask.paths,
            [
                "settings.locale",
                "settings.theme",
                "user",
                "user_name",
                "username"
            ]
        );
        let once = mask.clone();
        mask.normalize();
        assert_eq!(mask, once);
    }

    #[test]
    fn normalize_handles_empty_masks_and_component_ordering() {
        let mut empty = FieldMask::default();
        empty.normalize();
        assert!(empty.is_empty());

        // A bytewise sort would put `a-b` between `a` and `a.b`, hiding
        // the parent from adjacent deduplication. Paths are not validated.
        let mut mask = FieldMask::from_paths(["a.b", "a-b", "a", "é.child", "é"]);
        mask.normalize();
        assert_eq!(mask.paths, ["a", "a-b", "é"]);
    }

    #[test]
    fn normalize_preserves_unknown_fields() {
        use buffa::Message;

        // Unknown field 2, varint 42, alongside two paths covered by `a`.
        let mut mask =
            FieldMask::decode(&mut &[0x0a, 3, b'a', b'.', b'b', 0x10, 42, 0x0a, 1, b'a'][..])
                .unwrap();
        let unknown = mask.__buffa_unknown_fields.clone();
        mask.normalize();
        assert_eq!(mask.paths, ["a"]);
        assert_eq!(mask.__buffa_unknown_fields, unknown);
        assert_eq!(
            FieldMask::decode(&mut mask.encode_to_vec().as_slice()).unwrap(),
            mask
        );
    }

    #[test]
    fn normalize_preserves_coverage_and_leaves_no_covered_path_for_every_subset() {
        // In reverse component order, so that every subset needs the sort.
        let paths = [
            "b.c", "b", "a_b", "a-b", "a.bc", "a.b.c", "a.b", "a.", "a", "",
        ];
        for subset in 0..(1 << paths.len()) {
            let original = FieldMask::from_paths(
                paths
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| subset & (1 << i) != 0)
                    .map(|(_, path)| *path),
            );
            let mut normalized = original.clone();
            normalized.normalize();
            for query in paths
                .into_iter()
                .chain(["a.b.c.d", "a.bcd", "b.c.d", "c", ".a"])
            {
                assert_eq!(
                    normalized.covers(query),
                    original.covers(query),
                    "subset {subset}, query {query}"
                );
            }
            for (i, path) in normalized.paths.iter().enumerate() {
                for (j, other) in normalized.paths.iter().enumerate() {
                    assert!(
                        i == j || !path_is_covered(path, other),
                        "subset {subset}: {other} covers {path}"
                    );
                }
            }
            assert!(
                normalized
                    .paths
                    .windows(2)
                    .all(|w| w[0].split('.').lt(w[1].split('.'))),
                "subset {subset}: {:?} is not in component order",
                normalized.paths
            );
        }
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

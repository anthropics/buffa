use buffa::Message;
use buffa_types::FieldMask;

#[test]
fn union_normalizes_unsorted_paths_and_removes_redundant_descendants() {
    let left = FieldMask::from_paths(["user.name", "settings.theme", "user.name", "username"]);
    let right = FieldMask::from_paths(["user", "settings.locale", "settings.theme.color"]);
    assert_eq!(
        left.union(&right).paths,
        ["settings.locale", "settings.theme", "user", "username"]
    );
}

#[test]
fn intersection_selects_the_more_specific_path_from_either_mask() {
    let left = FieldMask::from_paths(["user", "settings.theme", "user", "user.name"]);
    let right = FieldMask::from_paths(["settings", "user.name.first", "user.email", "user.email"]);
    assert_eq!(
        left.intersection(&right).paths,
        ["settings.theme", "user.email", "user.name.first"]
    );
    assert_eq!(left.intersection(&right), right.intersection(&left));
}

#[test]
fn intersection_rejects_partial_component_matches_and_case_differences() {
    let left = FieldMask::from_paths(["user", "settings.theme", "User.Name"]);
    let right = FieldMask::from_paths(["username", "user_name", "settings.themes", "User.name"]);
    assert!(left.intersection(&right).is_empty());
}

#[test]
fn intersection_keeps_multiple_descendants_of_a_shared_ancestor() {
    let left = FieldMask::from_paths(["a", "b.x"]);
    let right = FieldMask::from_paths(["a.z", "a.b.c", "a.b", "a.y", "b", "c"]);
    assert_eq!(
        left.intersection(&right).paths,
        ["a.b", "a.y", "a.z", "b.x"]
    );
}

#[test]
fn intersection_uses_component_order_for_nonidentifier_paths() {
    let left = FieldMask::from_paths(["a-b", "a.child"]);
    let right = FieldMask::from_paths(["a-b.sub", "a"]);
    assert_eq!(left.intersection(&right).paths, ["a.child", "a-b.sub"]);
    assert_eq!(left.intersection(&right), right.intersection(&left));
}

#[test]
fn empty_masks_select_no_paths_in_set_operations() {
    let empty = FieldMask::default();
    let mask = FieldMask::from_paths(["b.c", "a.b", "a"]);
    let normalized = FieldMask::from_paths(["a", "b.c"]);
    assert_eq!(empty.union(&mask), normalized);
    assert_eq!(mask.union(&empty), normalized);
    assert!(empty.union(&empty).is_empty());
    assert!(empty.intersection(&mask).is_empty());
    assert!(mask.intersection(&empty).is_empty());
    assert!(empty.intersection(&empty).is_empty());
}

#[test]
fn set_operations_keep_literal_path_semantics() {
    for (left, right, shared) in [
        ("", "", Some("")),
        ("", ".user", Some(".user")),
        ("", "user", None),
        ("user.", "user..name", Some("user..name")),
        ("user.", "user.name", None),
        ("*", "*", Some("*")),
        ("*", "user", None),
        ("*", "*.name", Some("*.name")),
        ("labels.`a", "labels.`a.b`", Some("labels.`a.b`")),
        ("é", "é.child", Some("é.child")),
        ("é", "éx", None),
        ("é.child", "É", None),
    ] {
        let left = FieldMask::from_paths([left]);
        let right = FieldMask::from_paths([right]);
        let expected = FieldMask::from_paths(shared);
        assert_eq!(left.intersection(&right), expected, "{left:?}, {right:?}");
        assert_eq!(right.intersection(&left), expected, "{left:?}, {right:?}");
        let union = left.union(&right);
        assert_eq!(union, right.union(&left));
        for query in left
            .paths
            .iter()
            .chain(&right.paths)
            .map(String::as_str)
            .chain([
                "",
                ".user",
                "user",
                "user.name",
                "*.name",
                "labels.`a.b`",
                "é.child",
                "éx",
            ])
        {
            assert_eq!(
                union.covers(query),
                left.covers(query) || right.covers(query)
            );
        }
    }
}

#[test]
fn set_operations_leave_inputs_and_their_unknown_fields_unchanged() {
    let left = FieldMask::decode_from_slice(b"\x0a\x03a.b\x10\x2a\x0a\x01a").unwrap();
    let right = FieldMask::decode_from_slice(b"\x0a\x03a.c\x18\x2b").unwrap();
    let left_before = left.encode_to_vec();
    let right_before = right.encode_to_vec();
    assert_eq!(left.union(&right), FieldMask::from_paths(["a"]));
    assert_eq!(left.intersection(&right), FieldMask::from_paths(["a.c"]));
    assert_eq!(left.encode_to_vec(), left_before);
    assert_eq!(right.encode_to_vec(), right_before);
}

#[test]
fn empty_masks_with_unknown_fields_still_select_no_paths() {
    let empty = FieldMask::decode_from_slice(b"\x10\x2a").unwrap();
    let mask = FieldMask::from_paths(["user"]);
    assert_eq!(empty.union(&mask), mask);
    assert_eq!(mask.union(&empty), mask);
    assert_eq!(empty.intersection(&mask), FieldMask::default());
    assert_eq!(mask.intersection(&empty), FieldMask::default());
    assert_eq!(empty.encode_to_vec(), b"\x10\x2a");
}

#[test]
fn set_operations_on_the_same_mask_return_its_normalized_paths() {
    let mask = FieldMask::from_paths(["b.c", "a.b.c", "a", "a", "b"]);
    let normalized = FieldMask::from_paths(["a", "b"]);
    assert_eq!(mask.union(&mask), normalized);
    assert_eq!(mask.intersection(&mask), normalized);
}

#[test]
fn set_operation_results_roundtrip_through_the_wire_format() {
    let left = FieldMask::from_paths(["user", "settings.theme"]);
    let right = FieldMask::from_paths(["user.name", "settings"]);
    for mask in [left.union(&right), left.intersection(&right)] {
        assert_eq!(
            FieldMask::decode_from_slice(&mask.encode_to_vec()).unwrap(),
            mask
        );
    }
}

#[cfg(feature = "json")]
#[test]
fn set_operation_results_use_proto_field_names_in_json() {
    let left = FieldMask::from_paths(["user", "settings.display_name"]);
    let right = FieldMask::from_paths(["user.first_name", "settings"]);
    let union = left.union(&right);
    let intersection = left.intersection(&right);
    assert_eq!(serde_json::to_string(&union).unwrap(), r#""settings,user""#);
    assert_eq!(
        serde_json::to_string(&intersection).unwrap(),
        r#""settings.displayName,user.firstName""#
    );
    for mask in [union, intersection] {
        assert_eq!(
            serde_json::from_str::<FieldMask>(&serde_json::to_string(&mask).unwrap()).unwrap(),
            mask
        );
    }
}

#[test]
fn set_operations_preserve_coverage_for_every_pair_of_subsets() {
    let paths = ["b.c", "b", "a_b", "a-b", "a.b.c", "a.b", "a"];
    let masks: Vec<_> = (0..(1 << paths.len()))
        .map(|subset| {
            FieldMask::from_paths(
                paths
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| subset & (1 << i) != 0)
                    .map(|(_, path)| *path),
            )
        })
        .collect();
    for left in &masks {
        for right in &masks {
            let union = left.union(right);
            let intersection = left.intersection(right);
            assert_eq!(union, right.union(left));
            assert_eq!(intersection, right.intersection(left));
            for query in paths
                .into_iter()
                .chain(["a.b.c.d", "a.bc", "a-b.c", "b.c.d", "c", ""])
            {
                assert_eq!(
                    union.covers(query),
                    left.covers(query) || right.covers(query),
                    "union of {:?} and {:?}, query {query}",
                    left.paths,
                    right.paths
                );
                assert_eq!(
                    intersection.covers(query),
                    left.covers(query) && right.covers(query),
                    "intersection of {:?} and {:?}, query {query}",
                    left.paths,
                    right.paths
                );
            }
            for result in [&union, &intersection] {
                assert!(result
                    .paths
                    .windows(2)
                    .all(|w| w[0].split('.').lt(w[1].split('.'))));
                for (i, path) in result.paths.iter().enumerate() {
                    for (j, other) in result.paths.iter().enumerate() {
                        assert!(i == j || !FieldMask::from_paths([other.clone()]).covers(path));
                    }
                }
            }
        }
    }
}

#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::{
    descriptor_proto::ReservedRange, enum_descriptor_proto::EnumReservedRange, DescriptorProto,
    Edition, EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

/// A reserved range as `(start, end)`.
type Range = (i32, i32);

fn message(name: &str, ranges: &[(i32, i32)]) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.into()),
        reserved_range: ranges
            .iter()
            .map(|&(start, end)| ReservedRange {
                start: Some(start),
                end: Some(end),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn enumeration(name: &str, ranges: &[(i32, i32)]) -> EnumDescriptorProto {
    EnumDescriptorProto {
        name: Some(name.into()),
        value: vec![EnumValueDescriptorProto {
            name: Some(format!("{name}_ZERO")),
            number: Some(0),
            ..Default::default()
        }],
        reserved_range: ranges
            .iter()
            .map(|&(start, end)| EnumReservedRange {
                start: Some(start),
                end: Some(end),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn set(
    syntax: &str,
    messages: Vec<DescriptorProto>,
    enums: Vec<EnumDescriptorProto>,
) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("reserved_overlap.proto".into()),
            package: Some("overlap.test".into()),
            syntax: Some(syntax.into()),
            edition: (syntax == "editions").then_some(Edition::EDITION_2023),
            message_type: messages,
            enum_type: enums,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Asserts that `set` is rejected for the overlapping message ranges
/// `[earlier, later]`, named in declaration order.
fn assert_message_error(set: FileDescriptorSet, name: &str, [earlier, later]: [Range; 2]) {
    let unsigned = |(start, end): Range| {
        (
            u32::try_from(start).expect("message range bounds are positive"),
            u32::try_from(end).expect("message range bounds are positive"),
        )
    };
    let ((start, end), (other_start, other_end)) = (unsigned(later), unsigned(earlier));
    let err = DescriptorPool::new(set).expect_err("overlapping message ranges must be rejected");
    assert!(
        matches!(
            &err,
            PoolError::OverlappingMessageReservedRange {
                message,
                start: actual_start,
                end: actual_end,
                other_start: actual_other_start,
                other_end: actual_other_end,
            } if message == name
                && (*actual_start, *actual_end, *actual_other_start, *actual_other_end)
                    == (start, end, other_start, other_end)
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        format!(
            "message {name} reserved range {start}..{end} overlaps reserved range {other_start}..{other_end}"
        )
    );
}

/// Asserts that `set` is rejected for the overlapping enum ranges
/// `[earlier, later]`, named in declaration order with every bound set.
fn assert_enum_error(set: FileDescriptorSet, name: &str, declared: [Range; 2]) {
    let [(other_start, other_end), (start, end)] = declared;
    assert_enum_error_as_declared(
        set,
        name,
        declared.map(|(start, end)| (Some(start), Some(end))),
        &format!(
            "enum {name} reserved range {start} to {end} overlaps reserved range {other_start} to {other_end}"
        ),
    );
}

/// As [`assert_enum_error`], for ranges that may leave a bound unset.
fn assert_enum_error_as_declared(
    set: FileDescriptorSet,
    name: &str,
    [earlier, later]: [(Option<i32>, Option<i32>); 2],
    display: &str,
) {
    let err = DescriptorPool::new(set).expect_err("overlapping enum ranges must be rejected");
    assert!(
        matches!(
            &err,
            PoolError::OverlappingEnumReservedRange {
                enum_name,
                start,
                end,
                other_start,
                other_end,
            } if enum_name == name
                && (*start, *end) == later
                && (*other_start, *other_end) == earlier
        ),
        "{err:?}"
    );
    assert_eq!(err.to_string(), display);
}

#[test]
fn rejects_overlapping_message_ranges_in_either_declaration_order() {
    for syntax in ["proto2", "proto3", "editions"] {
        for mut ranges in [
            [(1, 5), (3, 7)],
            [(1, 5), (1, 5)],
            [(1, 10), (3, 5)],
            [(1, 5), (1, 10)],
            [(1, 10), (5, 10)],
            [(536_870_910, 536_870_912), (536_870_911, 536_870_912)],
        ] {
            for _ in 0..2 {
                assert_message_error(
                    set(syntax, vec![message("M", &ranges)], vec![]),
                    "overlap.test.M",
                    ranges,
                );
                ranges.reverse();
            }
        }
    }
}

#[test]
fn rejects_overlapping_enum_ranges_in_either_declaration_order() {
    for syntax in ["proto2", "proto3", "editions"] {
        for mut ranges in [
            [(1, 5), (3, 7)],
            [(1, 5), (1, 5)],
            [(1, 10), (3, 5)],
            [(1, 5), (1, 10)],
            [(1, 10), (5, 10)],
            [(1, 5), (5, 7)],
            [(9, 9), (9, 9)],
            [(-8, -3), (-5, -1)],
            [(i32::MIN, i32::MIN + 2), (i32::MIN, i32::MIN)],
            [(i32::MAX - 2, i32::MAX), (i32::MAX, i32::MAX)],
        ] {
            for _ in 0..2 {
                assert_enum_error(
                    set(syntax, vec![], vec![enumeration("E", &ranges)]),
                    "overlap.test.E",
                    ranges,
                );
                ranges.reverse();
            }
        }
    }
}

#[test]
fn message_error_names_the_later_declared_range_first() {
    let err = DescriptorPool::new(set("proto3", vec![message("M", &[(1, 5), (3, 7)])], vec![]))
        .unwrap_err();
    assert!(
        matches!(
            err,
            PoolError::OverlappingMessageReservedRange {
                start: 3,
                end: 7,
                other_start: 1,
                other_end: 5,
                ..
            }
        ),
        "{err:?}"
    );
    let err = DescriptorPool::new(set("proto3", vec![message("M", &[(3, 7), (1, 5)])], vec![]))
        .unwrap_err();
    assert!(
        matches!(
            err,
            PoolError::OverlappingMessageReservedRange {
                start: 1,
                end: 5,
                other_start: 3,
                other_end: 7,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn enum_error_names_the_later_declared_range_first() {
    let err = DescriptorPool::new(set(
        "proto3",
        vec![],
        vec![enumeration("E", &[(1, 5), (3, 7)])],
    ))
    .unwrap_err();
    assert!(
        matches!(
            err,
            PoolError::OverlappingEnumReservedRange {
                start: Some(3),
                end: Some(7),
                other_start: Some(1),
                other_end: Some(5),
                ..
            }
        ),
        "{err:?}"
    );
    let err = DescriptorPool::new(set(
        "proto3",
        vec![],
        vec![enumeration("E", &[(3, 7), (1, 5)])],
    ))
    .unwrap_err();
    assert!(
        matches!(
            err,
            PoolError::OverlappingEnumReservedRange {
                start: Some(1),
                end: Some(5),
                other_start: Some(3),
                other_end: Some(7),
                ..
            }
        ),
        "{err:?}"
    );
}

/// `1..10`, `5..20` and `8..15` overlap pairwise under both the half-open
/// message reading and the inclusive enum reading. Ordered by `start`, the
/// first two neighbors that overlap are `1..10` and `5..20`, so every
/// declaration order reports that pair, and the roles follow the order.
/// Each row is the declared ranges, then the reported `[earlier, later]`.
const THREE_OVERLAPPING: [([Range; 3], [Range; 2]); 4] = [
    ([(1, 10), (5, 20), (8, 15)], [(1, 10), (5, 20)]),
    ([(1, 10), (8, 15), (5, 20)], [(1, 10), (5, 20)]),
    ([(8, 15), (5, 20), (1, 10)], [(5, 20), (1, 10)]),
    ([(5, 20), (8, 15), (1, 10)], [(5, 20), (1, 10)]),
];

#[test]
fn three_overlapping_message_ranges_report_the_lowest_pair() {
    for (declared, reported) in THREE_OVERLAPPING {
        assert_message_error(
            set("proto3", vec![message("M", &declared)], vec![]),
            "overlap.test.M",
            reported,
        );
    }
}

#[test]
fn three_overlapping_enum_ranges_report_the_lowest_pair() {
    for (declared, reported) in THREE_OVERLAPPING {
        assert_enum_error(
            set("proto3", vec![], vec![enumeration("E", &declared)]),
            "overlap.test.E",
            reported,
        );
    }
}

#[test]
fn accepts_adjacent_and_disjoint_message_ranges() {
    for syntax in ["proto2", "proto3", "editions"] {
        for ranges in [
            vec![],
            vec![(1, 2)],
            vec![(5, 7), (1, 5)],
            vec![(20, 30), (12, 15), (5, 6), (9, 12)],
            vec![(536_870_911, 536_870_912), (1, 2)],
        ] {
            let pool = DescriptorPool::new(set(syntax, vec![message("M", &ranges)], vec![]))
                .expect("non-overlapping message ranges are valid");
            assert!(pool.message_by_name("overlap.test.M").is_some());
        }
    }
}

#[test]
fn accepts_adjacent_and_disjoint_enum_ranges() {
    for syntax in ["proto2", "proto3", "editions"] {
        for ranges in [
            vec![],
            vec![(1, 1)],
            vec![(6, 7), (1, 5)],
            vec![(-8, -4), (-3, -1), (1, 5), (6, 9)],
            vec![(i32::MAX, i32::MAX), (i32::MIN, i32::MIN)],
        ] {
            let pool = DescriptorPool::new(set(syntax, vec![], vec![enumeration("E", &ranges)]))
                .expect("non-overlapping enum ranges are valid");
            assert!(pool.enum_by_name("overlap.test.E").is_some());
        }
    }
}

#[test]
fn nested_message_error_reports_the_full_name() {
    let ranges = [(4, 8), (6, 10)];
    let mut parent = message("Parent", &[]);
    parent.nested_type.push(message("M", &ranges));
    assert_message_error(
        set("proto3", vec![parent], vec![]),
        "overlap.test.Parent.M",
        ranges,
    );
}

#[test]
fn nested_enum_error_reports_the_full_name() {
    let ranges = [(4, 8), (6, 10)];
    let mut parent = message("Parent", &[]);
    parent.enum_type.push(enumeration("E", &ranges));
    assert_enum_error(
        set("proto3", vec![parent], vec![]),
        "overlap.test.Parent.E",
        ranges,
    );
}

#[test]
fn ranges_are_validated_within_each_declaration() {
    let ranges = [(4, 8)];
    let mut parent = message("Parent", &ranges);
    parent.nested_type.push(message("Child", &ranges));
    parent.enum_type.push(enumeration("Nested", &ranges));
    let pool = DescriptorPool::new(set(
        "proto3",
        vec![parent, message("Sibling", &ranges)],
        vec![enumeration("E", &ranges), enumeration("Other", &ranges)],
    ))
    .expect("different declarations may reserve the same numbers");
    assert_eq!(pool.messages().len(), 3);
    assert_eq!(pool.enums().len(), 3);
}

#[test]
fn missing_enum_bounds_still_participate_in_overlap_checks() {
    for (declared, display) in [
        (
            [(None, None), (Some(0), Some(0))],
            "reserved range 0 to 0 overlaps reserved range unset to unset",
        ),
        (
            [(None, Some(5)), (Some(5), Some(8))],
            "reserved range 5 to 8 overlaps reserved range unset to 5",
        ),
        (
            [(Some(-5), None), (None, Some(0))],
            "reserved range unset to 0 overlaps reserved range -5 to unset",
        ),
    ] {
        let mut e = enumeration("E", &[]);
        e.value[0].number = Some(100);
        e.reserved_range = declared
            .into_iter()
            .map(|(start, end)| EnumReservedRange {
                start,
                end,
                ..Default::default()
            })
            .collect();
        assert_enum_error_as_declared(
            set("proto2", vec![], vec![e]),
            "overlap.test.E",
            declared,
            &format!("enum overlap.test.E {display}"),
        );
    }
}

#[test]
fn encoded_descriptor_sets_reject_overlapping_ranges() {
    let ranges = [(4, 8), (6, 10)];
    for set in [
        set("proto3", vec![message("M", &ranges)], vec![]),
        set("proto3", vec![], vec![enumeration("E", &ranges)]),
    ] {
        let err = DescriptorPool::decode(&set.encode_to_vec())
            .expect_err("wire descriptors receive the same overlap validation");
        assert!(err.to_string().contains("overlaps reserved range"));
    }
}

#[test]
fn overlapping_ranges_leave_an_existing_pool_unchanged() {
    let ranges = [(4, 8), (6, 10)];
    for invalid in [
        set("proto3", vec![message("M", &ranges)], vec![]),
        set("proto3", vec![], vec![enumeration("E", &ranges)]),
    ] {
        let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
        let counts = (
            pool.files().len(),
            pool.messages().len(),
            pool.enums().len(),
            pool.extensions().len(),
            pool.services().len(),
        );
        let mut batch = set("proto3", vec![message("Valid", &[])], vec![]);
        batch.file[0].name = Some("valid.proto".into());
        batch.file.extend(invalid.file);
        let err = pool.add_file_descriptor_set(batch).unwrap_err();
        assert!(err.to_string().contains("overlaps reserved range"));
        assert_eq!(
            counts,
            (
                pool.files().len(),
                pool.messages().len(),
                pool.enums().len(),
                pool.extensions().len(),
                pool.services().len(),
            )
        );
        assert!(pool.file_by_name("valid.proto").is_none());
        assert!(pool.file_by_name("reserved_overlap.proto").is_none());
        assert!(pool.message_by_name("overlap.test.Valid").is_none());
        assert!(pool.message_by_name("overlap.test.M").is_none());
        assert!(pool.enum_by_name("overlap.test.E").is_none());
        assert_eq!(
            pool.message_by_name("reflect.test.Scalars")
                .unwrap()
                .field(3)
                .unwrap()
                .name(),
            "f_int32"
        );
        let mut valid = set("proto3", vec![message("Valid", &[])], vec![]);
        valid.file[0].name = Some("valid.proto".into());
        pool.add_file_descriptor_set(valid).unwrap();
        assert!(pool.message_by_name("overlap.test.Valid").is_some());
    }
}

#[test]
fn checks_unsorted_ranges_after_a_long_disjoint_prefix() {
    let mut ranges: Vec<_> = (1..1000).map(|n| (n * 3, n * 3 + 2)).collect();
    ranges.reverse();
    DescriptorPool::new(set("proto3", vec![message("M", &ranges)], vec![])).unwrap();
    DescriptorPool::new(set("proto3", vec![], vec![enumeration("E", &ranges)])).unwrap();
    ranges.push((2998, 3000));
    assert_message_error(
        set("proto3", vec![message("M", &ranges)], vec![]),
        "overlap.test.M",
        [(2997, 2999), (2998, 3000)],
    );
    assert_enum_error(
        set("proto3", vec![], vec![enumeration("E", &ranges)]),
        "overlap.test.E",
        [(2997, 2999), (2998, 3000)],
    );
}

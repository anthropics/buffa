#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::{
    descriptor_proto::ReservedRange, enum_descriptor_proto::EnumReservedRange, DescriptorProto,
    Edition, EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

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

fn assert_message_error(set: FileDescriptorSet, name: &str, mut ranges: [(i32, i32); 2]) {
    ranges.sort_unstable();
    let [(other_start, other_end), (start, end)] = ranges;
    let err = DescriptorPool::new(set).expect_err("overlapping message ranges must be rejected");
    assert!(matches!(
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
    ));
    assert_eq!(
        err.to_string(),
        format!(
            "message {name} reserved range {start}..{end} overlaps reserved range {other_start}..{other_end}"
        )
    );
}

fn assert_enum_error(set: FileDescriptorSet, name: &str, mut ranges: [(i32, i32); 2]) {
    ranges.sort_unstable();
    let [(other_start, other_end), (start, end)] = ranges;
    let err = DescriptorPool::new(set).expect_err("overlapping enum ranges must be rejected");
    assert!(matches!(
        &err,
        PoolError::OverlappingEnumReservedRange {
            enum_name,
            start: actual_start,
            end: actual_end,
            other_start: actual_other_start,
            other_end: actual_other_end,
        } if enum_name == name
            && (*actual_start, *actual_end, *actual_other_start, *actual_other_end)
                == (start, end, other_start, other_end)
    ));
    assert_eq!(
        err.to_string(),
        format!(
            "enum {name} reserved range {start} to {end} overlaps reserved range {other_start} to {other_end}"
        )
    );
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
    for (ranges, expected) in [
        (vec![(None, None), (Some(0), Some(0))], [(0, 0), (0, 0)]),
        (vec![(None, Some(5)), (Some(5), Some(8))], [(0, 5), (5, 8)]),
        (vec![(Some(-5), None), (None, Some(0))], [(-5, 0), (0, 0)]),
    ] {
        let mut e = enumeration("E", &[]);
        e.value[0].number = Some(100);
        e.reserved_range = ranges
            .into_iter()
            .map(|(start, end)| EnumReservedRange {
                start,
                end,
                ..Default::default()
            })
            .collect();
        assert_enum_error(set("proto2", vec![], vec![e]), "overlap.test.E", expected);
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

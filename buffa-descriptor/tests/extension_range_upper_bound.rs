#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FileDescriptorProto, FileDescriptorSet, MessageOptions,
};
use buffa_descriptor::{DescriptorPool, PoolError};

const MAX: i32 = buffa::encoding::MAX_FIELD_NUMBER as i32;
const SYNTAXES: &[(Option<&str>, Option<Edition>)] = &[
    (None, None),
    (Some("proto2"), None),
    (Some("editions"), Some(Edition::EDITION_2023)),
    (Some("editions"), Some(Edition::EDITION_2024)),
];

fn descriptor_set(
    range: (i32, i32),
    syntax: Option<&str>,
    edition: Option<Edition>,
    nested: bool,
    message_set: Option<bool>,
) -> FileDescriptorSet {
    let message = DescriptorProto {
        name: Some("RangeMessage".into()),
        extension_range: vec![ExtensionRange {
            start: Some(range.0),
            end: Some(range.1),
            ..Default::default()
        }],
        options: message_set
            .map(|enabled| MessageOptions {
                message_set_wire_format: Some(enabled),
                ..Default::default()
            })
            .into(),
        ..Default::default()
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("extension-upper-bound.proto".into()),
            package: Some("range.test".into()),
            syntax: syntax.map(Into::into),
            edition,
            message_type: vec![if nested {
                DescriptorProto {
                    name: Some("Container".into()),
                    nested_type: vec![message],
                    ..Default::default()
                }
            } else {
                message
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn message_name(nested: bool) -> &'static str {
    if nested {
        "range.test.Container.RangeMessage"
    } else {
        "range.test.RangeMessage"
    }
}

fn assert_oversized_range(error: &PoolError, name: &str, range: (i32, i32)) {
    assert!(
        matches!(
            error,
            PoolError::ExtensionRangeEndTooLarge { message, start, end }
                if message == name && *start == range.0 as u32 && *end == range.1 as u32
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn ordinary_extension_ranges_above_the_limit_are_rejected() {
    for &(syntax, edition) in SYNTAXES {
        for nested in [false, true] {
            for message_set in [None, Some(false)] {
                for range in [
                    (1, MAX + 2),
                    (MAX, MAX + 2),
                    (MAX + 1, MAX + 2),
                    (1, i32::MAX),
                    (i32::MAX - 1, i32::MAX),
                ] {
                    let error = DescriptorPool::new(descriptor_set(
                        range,
                        syntax,
                        edition,
                        nested,
                        message_set,
                    ))
                    .expect_err("ordinary extension ranges cannot exceed the field number limit");
                    assert_oversized_range(&error, message_name(nested), range);
                }
            }
        }
    }
}

#[test]
fn ordinary_extension_ranges_at_or_below_the_limit_are_preserved() {
    for &(syntax, edition) in SYNTAXES {
        for nested in [false, true] {
            for message_set in [None, Some(false)] {
                for range in [(1, MAX + 1), (MAX, MAX + 1), (100, 101), (MAX - 1, MAX)] {
                    let pool = DescriptorPool::new(descriptor_set(
                        range,
                        syntax,
                        edition,
                        nested,
                        message_set,
                    ))
                    .unwrap();
                    let message = pool.message_by_name(message_name(nested)).unwrap();
                    assert_eq!(
                        message.extension_ranges(),
                        &[(range.0 as u32, range.1 as u32)]
                    );
                    assert!(message.in_extension_range((range.1 - 1) as u32));
                    assert!(!message.in_extension_range(range.1 as u32));
                }
            }
        }
    }
}

#[test]
fn message_set_extension_ranges_can_exceed_the_field_number_limit() {
    for syntax in [None, Some("proto2")] {
        for nested in [false, true] {
            for range in [(100, MAX + 2), (MAX + 1, MAX + 2), (100, i32::MAX)] {
                let set = descriptor_set(range, syntax, None, nested, Some(true));
                for pool in [
                    DescriptorPool::new(set.clone()).unwrap(),
                    DescriptorPool::decode(&set.encode_to_vec()).unwrap(),
                ] {
                    assert_eq!(
                        pool.message_by_name(message_name(nested))
                            .unwrap()
                            .extension_ranges(),
                        &[(range.0 as u32, range.1 as u32)]
                    );
                }
            }
        }
    }
}

#[test]
fn encoded_ordinary_extension_ranges_above_the_limit_are_rejected() {
    for &(syntax, edition) in SYNTAXES {
        for nested in [false, true] {
            let set = descriptor_set((1, MAX + 2), syntax, edition, nested, None);
            let error = DescriptorPool::decode(&set.encode_to_vec())
                .expect_err("encoded descriptors must also enforce the upper bound");
            assert_oversized_range(&error, message_name(nested), (1, MAX + 2));
        }
    }
}

#[test]
fn rejecting_an_oversized_range_leaves_the_pool_unchanged() {
    let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
    let counts = |pool: &DescriptorPool| {
        (
            pool.messages().len(),
            pool.enums().len(),
            pool.services().len(),
            pool.extensions().len(),
            pool.files().len(),
        )
    };
    let before = counts(&pool);
    let field_name = pool
        .message_by_name("reflect.test.Scalars")
        .unwrap()
        .field(3)
        .unwrap()
        .name()
        .to_owned();
    let mut set = descriptor_set((100, MAX + 2), Some("proto2"), None, true, None);
    set.file.insert(
        0,
        FileDescriptorProto {
            name: Some("before-invalid-range.proto".into()),
            package: Some("before.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("Before".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let error = pool
        .add_file_descriptor_set(set)
        .expect_err("an oversized range cannot be added");
    assert_oversized_range(&error, message_name(true), (100, MAX + 2));
    assert_eq!(counts(&pool), before);
    for file in ["before-invalid-range.proto", "extension-upper-bound.proto"] {
        assert!(pool.file_by_name(file).is_none());
    }
    for symbol in [
        "before.test.Before",
        "range.test.Container",
        "range.test.Container.RangeMessage",
    ] {
        assert!(pool.message_by_name(symbol).is_none());
        assert!(pool.file_containing_symbol(symbol).is_none());
    }
    assert_eq!(
        pool.message_by_name("reflect.test.Scalars")
            .unwrap()
            .field(3)
            .unwrap()
            .name(),
        field_name
    );
}

#[test]
fn message_set_still_rejects_invalid_range_order_and_nonpositive_starts() {
    for range in [(0, 100), (-1, 100), (100, 100), (101, 100)] {
        let error = DescriptorPool::new(descriptor_set(
            range,
            Some("proto2"),
            None,
            false,
            Some(true),
        ))
        .unwrap_err();
        assert!(matches!(error, PoolError::InvalidExtensionRange { .. }));
    }
}

#[test]
fn oversized_range_errors_name_messages_without_a_package() {
    for nested in [false, true] {
        let mut set = descriptor_set((100, MAX + 2), Some("proto2"), None, nested, None);
        set.file[0].package = None;
        let error = DescriptorPool::new(set).unwrap_err();
        let name = if nested {
            "Container.RangeMessage"
        } else {
            "RangeMessage"
        };
        assert_oversized_range(&error, name, (100, MAX + 2));
        assert_eq!(
            error.to_string(),
            format!(
                "message {name} extension range 100..{} exceeds maximum end {}",
                MAX + 2,
                MAX + 1
            )
        );
    }
}

#[test]
fn empty_message_options_do_not_relax_the_upper_bound() {
    let mut set = descriptor_set((100, MAX + 2), Some("proto2"), None, false, None);
    set.file[0].message_type[0].options = MessageOptions::default().into();
    let error = DescriptorPool::new(set).unwrap_err();
    assert_oversized_range(&error, message_name(false), (100, MAX + 2));
}

#[test]
fn message_set_option_applies_only_to_the_declaring_message() {
    let mut set = descriptor_set((100, MAX + 2), Some("proto2"), None, true, Some(false));
    set.file[0].message_type[0].options = MessageOptions {
        message_set_wire_format: Some(true),
        ..Default::default()
    }
    .into();
    let error = DescriptorPool::new(set).unwrap_err();
    assert_oversized_range(&error, message_name(true), (100, MAX + 2));
}

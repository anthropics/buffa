#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet, MessageOptions,
};
use buffa_descriptor::{DescriptorPool, PoolError};

const MAX: i32 = buffa::encoding::MAX_FIELD_NUMBER as i32;

fn descriptor_set(number: i32, message_set: bool, nested: bool) -> FileDescriptorSet {
    let extension = FieldDescriptorProto {
        name: Some("item".into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(".test.Payload".into()),
        extendee: Some(".test.Container".into()),
        ..Default::default()
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("bounds.proto".into()),
            package: Some("test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![
                DescriptorProto {
                    name: Some("Container".into()),
                    options: MessageOptions {
                        message_set_wire_format: Some(message_set),
                        ..Default::default()
                    }
                    .into(),
                    extension_range: vec![ExtensionRange {
                        start: Some(4),
                        end: Some(if message_set { i32::MAX } else { MAX + 1 }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                DescriptorProto {
                    name: Some("Payload".into()),
                    extension: if nested {
                        vec![extension.clone()]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
            ],
            extension: if nested { vec![] } else { vec![extension] },
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn builds(set: FileDescriptorSet) -> [Result<DescriptorPool, PoolError>; 2] {
    [
        DescriptorPool::new(set.clone()),
        DescriptorPool::decode(&set.encode_to_vec()),
    ]
}

#[test]
fn message_set_extension_numbers_above_ordinary_limit_link() {
    for nested in [false, true] {
        for number in [MAX, MAX + 1, 1_000_000_000, i32::MAX - 1] {
            for result in builds(descriptor_set(number, true, nested)) {
                let pool = result.unwrap();
                let name = if nested {
                    "test.Payload.item"
                } else {
                    "test.item"
                };
                assert_eq!(
                    pool.extension_by_name(name).unwrap().field().number(),
                    number as u32
                );
            }
        }
    }
}

#[test]
fn invalid_numbers_and_exclusive_range_end_still_fail() {
    for nested in [false, true] {
        for (message_set, numbers) in [
            (true, vec![0, -1, i32::MIN, i32::MAX]),
            (false, vec![MAX + 1, i32::MAX]),
        ] {
            for number in numbers {
                for result in builds(descriptor_set(number, message_set, nested)) {
                    assert!(
                        matches!(result, Err(PoolError::InvalidFieldNumber { number: n, .. }) if n == number)
                    );
                }
            }
        }
        let mut set = descriptor_set(1_000_000_000, true, nested);
        set.file[0].message_type[0].extension_range[0].end = Some(1_000_000_000);
        for result in builds(set) {
            assert!(matches!(result, Err(PoolError::InvalidFieldNumber { .. })));
        }
    }
}

#[test]
fn message_set_extensions_keep_reserved_kind_and_duplicate_validation() {
    for number in [19_000, 19_999] {
        for result in builds(descriptor_set(number, true, false)) {
            assert!(matches!(result, Err(PoolError::ReservedFieldNumber { .. })));
        }
    }
    for (label, ty) in [
        (Label::LABEL_REPEATED, Type::TYPE_MESSAGE),
        (Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        (Label::LABEL_OPTIONAL, Type::TYPE_GROUP),
    ] {
        let mut set = descriptor_set(1_000_000_000, true, false);
        set.file[0].extension[0].label = Some(label);
        set.file[0].extension[0].r#type = Some(ty);
        if ty == Type::TYPE_INT32 {
            set.file[0].extension[0].type_name = None;
        }
        for result in builds(set) {
            let error = result.unwrap_err();
            assert!(
                matches!(error, PoolError::InvalidMessageSetExtension { .. }),
                "{error}"
            );
        }
    }
    let mut set = descriptor_set(1_000_000_000, true, false);
    let mut duplicate = set.file[0].extension[0].clone();
    duplicate.name = Some("other".into());
    set.file[0].extension.push(duplicate);
    for result in builds(set) {
        assert!(matches!(
            result,
            Err(PoolError::DuplicateExtensionNumber {
                number: 1_000_000_000,
                ..
            })
        ));
    }
}

#[test]
fn unrelated_message_set_does_not_relax_ordinary_fields() {
    let mut set = descriptor_set(1000, true, false);
    set.file[0].message_type[1]
        .field
        .push(FieldDescriptorProto {
            name: Some("too_large".into()),
            number: Some(MAX + 1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_INT32),
            ..Default::default()
        });
    for result in builds(set) {
        assert!(matches!(result, Err(PoolError::InvalidFieldNumber { .. })));
    }
}

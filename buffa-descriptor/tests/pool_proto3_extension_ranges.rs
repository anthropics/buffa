#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn range_set(
    syntax: Option<&str>,
    edition: Option<Edition>,
    nested: bool,
    ranges: &[(i32, i32)],
) -> FileDescriptorSet {
    let host = DescriptorProto {
        name: Some("Host".into()),
        extension_range: ranges
            .iter()
            .map(|&(start, end)| ExtensionRange {
                start: Some(start),
                end: Some(end),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("ranges.proto".into()),
            package: Some("range.test".into()),
            syntax: syntax.map(Into::into),
            edition,
            message_type: vec![if nested {
                DescriptorProto {
                    name: Some("Outer".into()),
                    nested_type: vec![host],
                    ..Default::default()
                }
            } else {
                host
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn assert_proto3_error(error: &PoolError, nested: bool) {
    let expected = if nested {
        "range.test.Outer.Host"
    } else {
        "range.test.Host"
    };
    assert!(
        matches!(error, PoolError::ExtensionRangeInProto3 { message } if message == expected),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!("message {expected} declares an extension range in proto3")
    );
}

#[test]
fn proto3_extension_ranges_are_rejected() {
    for nested in [false, true] {
        for ranges in [&[(100, 200)][..], &[(300, 400), (100, 200)][..]] {
            let error = DescriptorPool::new(range_set(Some("proto3"), None, nested, ranges))
                .expect_err("proto3 extension ranges must be rejected");
            assert_proto3_error(&error, nested);
        }
    }
}

#[test]
fn proto3_messages_without_extension_ranges_are_accepted() {
    for nested in [false, true] {
        DescriptorPool::new(range_set(Some("proto3"), None, nested, &[])).unwrap();
    }
}

#[test]
fn proto2_and_editions_extension_ranges_are_preserved() {
    let ranges = [(300, 400), (100, 200)];
    for (syntax, edition) in [
        (None, None),
        (Some(""), None),
        (Some("proto2"), None),
        (Some("editions"), Some(Edition::EDITION_2023)),
        (Some("editions"), Some(Edition::EDITION_2024)),
    ] {
        for nested in [false, true] {
            let pool = DescriptorPool::new(range_set(syntax, edition, nested, &ranges)).unwrap();
            let name = if nested {
                "range.test.Outer.Host"
            } else {
                "range.test.Host"
            };
            assert_eq!(
                pool.message_by_name(name).unwrap().extension_ranges(),
                &[(300, 400), (100, 200)],
                "syntax={syntax:?}, edition={edition:?}"
            );
        }
    }
}

#[test]
fn encoded_proto3_extension_ranges_are_rejected() {
    for nested in [false, true] {
        let bytes = range_set(Some("proto3"), None, nested, &[(100, 200)]).encode_to_vec();
        let error = DescriptorPool::decode(&bytes).unwrap_err();
        assert_proto3_error(&error, nested);
    }
}

#[test]
fn proto3_range_rejection_does_not_mutate_an_existing_pool() {
    let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
    let before = pool.clone();
    let mut valid = range_set(Some("proto2"), None, false, &[(100, 200)]).file;
    valid[0].name = Some("valid.proto".into());
    valid[0].package = Some("valid.test".into());
    valid.extend(range_set(Some("proto3"), None, true, &[(100, 200)]).file);
    let error = pool
        .add_file_descriptor_set(FileDescriptorSet {
            file: valid,
            ..Default::default()
        })
        .expect_err("the proto3 file must abort the entire addition");
    assert_proto3_error(&error, true);
    assert_eq!(pool.files(), before.files());
    assert_eq!(pool.messages().len(), before.messages().len());
    assert_eq!(pool.enums().len(), before.enums().len());
    assert_eq!(pool.extensions().len(), before.extensions().len());
    assert!(pool.message_by_name("range.test.Outer.Host").is_none());
    assert!(pool.message_by_name("valid.test.Host").is_none());
    assert!(pool.file_containing_symbol("valid.test.Host").is_none());
    assert_eq!(
        pool.message_by_name("reflect.test.Scalars")
            .unwrap()
            .field(3)
            .unwrap()
            .name(),
        before
            .message_by_name("reflect.test.Scalars")
            .unwrap()
            .field(3)
            .unwrap()
            .name()
    );
}

#[test]
fn proto3_extension_range_errors_name_messages_without_a_package() {
    let mut set = range_set(Some("proto3"), None, false, &[(100, 200)]);
    set.file[0].package = None;
    let error = DescriptorPool::new(set).unwrap_err();
    assert!(matches!(&error, PoolError::ExtensionRangeInProto3 { message } if message == "Host"));
    assert_eq!(
        error.to_string(),
        "message Host declares an extension range in proto3"
    );
}

#[test]
fn proto3_custom_option_extensions_are_accepted() {
    for nested in [false, true] {
        let option = FieldDescriptorProto {
            name: Some("label".into()),
            extendee: Some(".google.protobuf.MessageOptions".into()),
            number: Some(50001),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_STRING),
            ..Default::default()
        };
        let mut options = FileDescriptorProto {
            name: Some("options.proto".into()),
            package: Some("range.test".into()),
            syntax: Some("proto3".into()),
            dependency: vec!["google/protobuf/descriptor.proto".into()],
            ..Default::default()
        };
        if nested {
            options.message_type.push(DescriptorProto {
                name: Some("Scope".into()),
                extension: vec![option],
                ..Default::default()
            });
        } else {
            options.extension.push(option);
        }
        let mut set = range_set(Some("proto2"), None, false, &[(1000, 536_870_912)]);
        set.file[0].name = Some("google/protobuf/descriptor.proto".into());
        set.file[0].package = Some("google.protobuf".into());
        set.file[0].message_type[0].name = Some("MessageOptions".into());
        set.file.push(options);
        let pool = DescriptorPool::new(set).unwrap();
        let name = if nested {
            "range.test.Scope.label"
        } else {
            "range.test.label"
        };
        assert!(pool.extension_by_name(name).is_some());
    }
}

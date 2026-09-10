//! End-to-end tests for [`DescriptorPool`] linking and editions feature
//! resolution against a `protoc`-compiled `FileDescriptorSet`.
//!
//! Uses `tests/protos/reflect_test.{proto,fds}` and
//! `tests/protos/editions_test.proto`. Regenerate the `.fds` with:
//!
//! ```sh
//! protoc --include_imports --descriptor_set_out=reflect_test.fds \
//!     reflect_test.proto reflect_test_ext.proto editions_test.proto \
//!     closed_enum_test.proto
//! ```

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::editions::{EnumType, FieldPresence};
use buffa_descriptor::{DescriptorPool, FieldKind, PoolError, ScalarType, SingularKind};

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

fn scalar_field(
    name: &str,
    number: i32,
    ty: buffa_descriptor::generated::descriptor::field_descriptor_proto::Type,
) -> buffa_descriptor::generated::descriptor::FieldDescriptorProto {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Label;
    use buffa_descriptor::generated::descriptor::FieldDescriptorProto;

    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(ty),
        ..Default::default()
    }
}

fn enum_value(
    name: &str,
    number: i32,
) -> buffa_descriptor::generated::descriptor::EnumValueDescriptorProto {
    buffa_descriptor::generated::descriptor::EnumValueDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        ..Default::default()
    }
}

fn assert_rejected_without_mutating_pool(
    file_name: &str,
    full_message_name: &str,
    message: buffa_descriptor::generated::descriptor::DescriptorProto,
    assert_error: impl FnOnce(&PoolError),
) {
    use buffa_descriptor::generated::descriptor::{FileDescriptorProto, FileDescriptorSet};

    let mut p = DescriptorPool::decode(FDS_BYTES).unwrap();
    let baseline_message_count = p.messages().len();
    let baseline_file_count = p.files().len();
    let baseline_field_count = p
        .message_by_name("reflect.test.Scalars")
        .unwrap()
        .fields()
        .len();
    let baseline_field_name = p
        .message_by_name("reflect.test.Scalars")
        .unwrap()
        .field(3)
        .unwrap()
        .name()
        .to_owned();

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some(file_name.into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![message],
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = p.add_file_descriptor_set(set).unwrap_err();
    assert_error(&err);

    assert_eq!(p.messages().len(), baseline_message_count);
    assert_eq!(p.files().len(), baseline_file_count);
    assert!(p.file_by_name(file_name).is_none());
    assert!(p.message_by_name(full_message_name).is_none());
    let existing = p.message_by_name("reflect.test.Scalars").unwrap();
    assert_eq!(existing.fields().len(), baseline_field_count);
    assert_eq!(existing.field(3).unwrap().name(), baseline_field_name);
}

fn assert_set_rejected_without_mutating_pool(
    file_name: &str,
    symbol_name: &str,
    set: buffa_descriptor::generated::descriptor::FileDescriptorSet,
    assert_error: impl FnOnce(&PoolError),
) {
    let mut p = DescriptorPool::decode(FDS_BYTES).unwrap();
    let baseline_message_count = p.messages().len();
    let baseline_enum_count = p.enums().len();
    let baseline_service_count = p.services().len();
    let baseline_extension_count = p.extensions().len();
    let baseline_file_count = p.files().len();
    let baseline_field_name = p
        .message_by_name("reflect.test.Scalars")
        .unwrap()
        .field(3)
        .unwrap()
        .name()
        .to_owned();

    let err = p.add_file_descriptor_set(set).unwrap_err();
    assert_error(&err);

    assert_eq!(p.messages().len(), baseline_message_count);
    assert_eq!(p.enums().len(), baseline_enum_count);
    assert_eq!(p.services().len(), baseline_service_count);
    assert_eq!(p.extensions().len(), baseline_extension_count);
    assert_eq!(p.files().len(), baseline_file_count);
    assert!(p.file_by_name(file_name).is_none());
    assert!(p.file_containing_symbol(symbol_name).is_none());
    assert_eq!(
        p.message_by_name("reflect.test.Scalars")
            .unwrap()
            .field(3)
            .unwrap()
            .name(),
        baseline_field_name
    );
}

#[test]
fn pool_registers_all_types() {
    let p = pool();
    assert!(p.message_by_name("reflect.test.Scalars").is_some());
    assert!(p.message_by_name("reflect.test.Containers").is_some());
    assert!(p.message_by_name("reflect.test.Inner").is_some());
    assert!(p.message_by_name("reflect.test.OneOf").is_some());
    assert!(p.enum_by_name("reflect.test.Color").is_some());
    assert!(p.message_by_name("reflect.editions.Editions").is_some());
    assert!(p.enum_by_name("reflect.editions.Status").is_some());
    // Wrong-kind lookups return None.
    assert!(p.enum_by_name("reflect.test.Scalars").is_none());
    assert!(p.message_by_name("reflect.test.Color").is_none());
    // Unregistered names return None.
    assert!(p.message_by_name("reflect.test.NoSuchType").is_none());
}

#[test]
fn scalar_fields_link_with_proto3_presence() {
    let p = pool();
    let scalars = p.message_by_name("reflect.test.Scalars").unwrap();
    // 17 fields: 15 scalars, f_opt, and f_field_mask.
    assert_eq!(scalars.fields().len(), 17);

    // Lookup by number.
    let f_int32 = scalars.field(3).unwrap();
    assert_eq!(f_int32.name(), "f_int32");
    assert_eq!(f_int32.json_name(), "fInt32");
    assert_eq!(
        f_int32.kind(),
        FieldKind::Singular(SingularKind::Scalar(ScalarType::Int32))
    );
    // proto3 implicit presence.
    assert_eq!(f_int32.presence(), FieldPresence::Implicit);

    // proto3 `optional` → explicit presence + synthetic oneof.
    let f_opt = scalars.field(16).unwrap();
    assert_eq!(f_opt.presence(), FieldPresence::Explicit);
    assert!(f_opt.oneof_index().is_some());
    let oneof_idx = f_opt.oneof_index().unwrap() as usize;
    assert!(scalars.oneofs()[oneof_idx].is_synthetic());
}

#[test]
fn container_fields_link_correctly() {
    let p = pool();
    let containers = p.message_by_name("reflect.test.Containers").unwrap();

    // packed_ints: repeated int32, packed by default (proto3).
    let packed = containers.field(1).unwrap();
    assert_eq!(
        packed.kind(),
        FieldKind::List(SingularKind::Scalar(ScalarType::Int32))
    );
    assert!(packed.is_packed());

    // strings: repeated string, never packed.
    let strings = containers.field(2).unwrap();
    assert_eq!(
        strings.kind(),
        FieldKind::List(SingularKind::Scalar(ScalarType::String))
    );
    assert!(!strings.is_packed());

    // tags: map<string, int32>.
    let tags = containers.field(3).unwrap();
    assert_eq!(
        tags.kind(),
        FieldKind::Map {
            key: ScalarType::String,
            value: SingularKind::Scalar(ScalarType::Int32),
        }
    );

    // children: map<int32, Inner>.
    let children = containers.field(4).unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    assert_eq!(
        children.kind(),
        FieldKind::Map {
            key: ScalarType::Int32,
            value: SingularKind::Message(inner_idx),
        }
    );

    // nested: Inner — singular message, explicit presence.
    let nested = containers.field(5).unwrap();
    assert_eq!(
        nested.kind(),
        FieldKind::Singular(SingularKind::Message(inner_idx))
    );
    assert_eq!(nested.presence(), FieldPresence::Explicit);

    // color: enum.
    let color = containers.field(6).unwrap();
    let color_idx = p.enum_index("reflect.test.Color").unwrap();
    assert_eq!(
        color.kind(),
        FieldKind::Singular(SingularKind::Enum(color_idx))
    );

    // colors: repeated enum, packed by default.
    let colors = containers.field(7).unwrap();
    assert_eq!(
        colors.kind(),
        FieldKind::List(SingularKind::Enum(color_idx))
    );
    assert!(colors.is_packed());
}

#[test]
fn enum_links_with_proto3_open() {
    let p = pool();
    let color = p.enum_by_name("reflect.test.Color").unwrap();
    assert_eq!(color.enum_type(), EnumType::Open);
    assert_eq!(color.values().len(), 4);
    assert_eq!(color.value(1).unwrap().name(), "RED");
    assert_eq!(color.value_by_name("BLUE").unwrap().number(), 3);
}

#[test]
fn proto3_open_enum_first_value_must_be_zero() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("proto3-open-enum-nonzero.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("ACTIVE".into()),
                        number: Some(1),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("UNSPECIFIED".into()),
                        number: Some(0),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "proto3-open-enum-nonzero.proto",
        "invalid.test.Status",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::OpenEnumFirstValueNotZero {
                    enum_name,
                    name,
                    number,
                } if enum_name == "invalid.test.Status"
                    && name == "ACTIVE"
                    && *number == 1
            ));
        },
    );
}

#[test]
fn editions_open_enum_first_value_must_be_zero() {
    use buffa_descriptor::generated::descriptor::feature_set::EnumType as FeatureEnumType;
    use buffa_descriptor::generated::descriptor::{
        Edition, EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto, FeatureSet,
        FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("editions-open-enum-nonzero.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_2023),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("ACTIVE".into()),
                    number: Some(1),
                    ..Default::default()
                }],
                options: EnumOptions {
                    features: buffa::MessageField::some(FeatureSet {
                        enum_type: Some(FeatureEnumType::OPEN),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "editions-open-enum-nonzero.proto",
        "invalid.test.Status",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::OpenEnumFirstValueNotZero {
                    enum_name,
                    name,
                    number,
                } if enum_name == "invalid.test.Status"
                    && name == "ACTIVE"
                    && *number == 1
            ));
        },
    );
}

#[test]
fn editions_closed_enum_first_value_can_be_nonzero() {
    use buffa_descriptor::generated::descriptor::feature_set::EnumType as FeatureEnumType;
    use buffa_descriptor::generated::descriptor::{
        Edition, EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto, FeatureSet,
        FileDescriptorProto, FileDescriptorSet,
    };

    let pool = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("editions-closed-enum-nonzero.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_2023),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("ACTIVE".into()),
                        number: Some(1),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("UNSPECIFIED".into()),
                        number: Some(0),
                        ..Default::default()
                    },
                ],
                options: EnumOptions {
                    features: buffa::MessageField::some(FeatureSet {
                        enum_type: Some(FeatureEnumType::CLOSED),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .expect("closed editions enums may start with a non-zero value");

    let status = pool.enum_by_name("valid.test.Status").unwrap();
    assert_eq!(status.enum_type(), EnumType::Closed);
    assert_eq!(status.values()[0].number(), 1);
}

#[test]
fn proto2_enum_first_value_can_be_nonzero() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    // proto2 enums are closed, so the open-enum rule does not apply; a
    // proto2 enum starting at 1 is the common real-world shape.
    let pool = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("proto2-enum-nonzero.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto2".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("ACTIVE".into()),
                    number: Some(1),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .expect("proto2 enums are closed and may start at any number");
    let status = pool.enum_by_name("valid.test.Status").unwrap();
    assert_eq!(status.values()[0].number(), 1);
}

#[test]
fn open_enum_with_no_values_is_not_rejected_by_the_first_value_rule() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    // protoc rejects an empty enum for a different reason; this rule must
    // not panic or misfire on `value.first()` being `None`.
    let result = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("proto3-empty-enum.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Empty".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(
        !matches!(
            result,
            Err(buffa_descriptor::PoolError::OpenEnumFirstValueNotZero { .. })
        ),
        "{result:?}"
    );
}

#[test]
fn oneof_links() {
    let p = pool();
    let oneof = p.message_by_name("reflect.test.OneOf").unwrap();
    assert_eq!(oneof.oneofs().len(), 1);
    let o = &oneof.oneofs()[0];
    assert_eq!(o.name(), "variant");
    assert!(!o.is_synthetic());
    assert_eq!(o.field_indices(), vec![0, 1, 2]);
}

#[test]
fn editions_feature_resolution() {
    let p = pool();
    let editions = p.message_by_name("reflect.editions.Editions").unwrap();

    // editions 2023 defaults to explicit presence.
    let explicit = editions.field(2).unwrap();
    assert_eq!(
        explicit.presence(),
        FieldPresence::Explicit,
        "editions 2023 default"
    );

    // explicit IMPLICIT override.
    let implicit = editions.field(1).unwrap();
    assert_eq!(
        implicit.presence(),
        FieldPresence::Implicit,
        "explicit field-level override"
    );

    // editions 2023 defaults to packed.
    let packed_default = editions.field(3).unwrap();
    assert!(
        packed_default.is_packed(),
        "editions 2023 packs by default — this is the case buffa-reflect gets wrong"
    );

    // explicit EXPANDED override.
    let unpacked = editions.field(4).unwrap();
    assert!(!unpacked.is_packed(), "explicit EXPANDED override");

    // Closed enum from editions feature.
    let status = p.enum_by_name("reflect.editions.Status").unwrap();
    assert_eq!(status.enum_type(), EnumType::Closed);
}

#[test]
fn idempotent_re_add() {
    let mut p = DescriptorPool::decode(FDS_BYTES).unwrap();
    let count = p.messages().len();
    use buffa::Message;
    let set =
        buffa_descriptor::generated::descriptor::FileDescriptorSet::decode_from_slice(FDS_BYTES)
            .unwrap();
    p.add_file_descriptor_set(set).unwrap();
    assert_eq!(
        p.messages().len(),
        count,
        "re-adding the same files is a no-op"
    );
}

#[test]
fn failed_add_does_not_mutate_pool_and_retry_succeeds() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let mut p = DescriptorPool::decode(FDS_BYTES).unwrap();
    let baseline_message_count = p.messages().len();
    let baseline_file_count = p.files().len();

    let broken = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("poison.proto".into()),
            package: Some("poison.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("RetryMe".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("broken".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_MESSAGE),
                    type_name: Some(".poison.test.Missing".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let err = p.add_file_descriptor_set(broken).unwrap_err();
    assert!(
        err.to_string().contains("unresolved type name"),
        "unexpected error: {err}"
    );
    assert!(
        p.message_by_name("poison.test.RetryMe").is_none(),
        "failed add must not register placeholder descriptors"
    );
    assert!(
        p.file_by_name("poison.proto").is_none(),
        "failed add must not record the file as loaded"
    );
    assert_eq!(p.messages().len(), baseline_message_count);
    assert_eq!(p.files().len(), baseline_file_count);

    let fixed = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("poison.proto".into()),
            package: Some("poison.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("RetryMe".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("ok".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_STRING),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    p.add_file_descriptor_set(fixed)
        .expect("retry with a valid descriptor set succeeds");

    let retry = p
        .message_by_name("poison.test.RetryMe")
        .expect("message loads after the corrected retry");
    assert_eq!(retry.field(1).unwrap().name(), "ok");
    assert_eq!(p.messages().len(), baseline_message_count + 1);
    assert_eq!(p.files().len(), baseline_file_count + 1);
}

#[test]
fn duplicate_field_numbers_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    assert_rejected_without_mutating_pool(
        "duplicate-number.proto",
        "invalid.test.BadNumber",
        DescriptorProto {
            name: Some("BadNumber".into()),
            field: vec![
                scalar_field("count", 1, Type::TYPE_INT32),
                scalar_field("label", 1, Type::TYPE_STRING),
            ],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateFieldNumber { message, number }
                    if message == "invalid.test.BadNumber" && *number == 1
            ));
        },
    );
}

#[test]
fn implementation_reserved_field_numbers_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    for number in [
        (buffa::encoding::FIRST_RESERVED_FIELD_NUMBER - 1) as i32,
        (buffa::encoding::LAST_RESERVED_FIELD_NUMBER + 1) as i32,
    ] {
        let name = format!("Allowed{number}");
        assert!(
            add_message_with_syntax(
                "proto3",
                DescriptorProto {
                    name: Some(name),
                    field: vec![scalar_field("value", number, Type::TYPE_INT32)],
                    ..Default::default()
                },
            )
            .is_ok(),
            "field number {number} should be accepted"
        );
    }

    for number in [
        buffa::encoding::FIRST_RESERVED_FIELD_NUMBER as i32,
        buffa::encoding::LAST_RESERVED_FIELD_NUMBER as i32,
    ] {
        let message_name = format!("Reserved{number}");
        let full_name = format!("invalid.test.{message_name}");
        let field_name = format!("{full_name}.value");
        let file_name = format!("implementation-reserved-{number}.proto");

        assert_rejected_without_mutating_pool(
            &file_name,
            &full_name,
            DescriptorProto {
                name: Some(message_name),
                field: vec![scalar_field("value", number, Type::TYPE_INT32)],
                ..Default::default()
            },
            move |err| {
                assert!(
                    matches!(
                        err,
                        PoolError::ReservedFieldNumber {
                            field,
                            number: actual
                        } if field == &field_name && *actual == number
                    ),
                    "unexpected error: {err}"
                );
            },
        );
    }
}

#[test]
fn reserved_message_field_names_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    assert_rejected_without_mutating_pool(
        "reserved-field-name.proto",
        "invalid.test.ReservedName",
        DescriptorProto {
            name: Some("ReservedName".into()),
            field: vec![scalar_field("old_name", 1, Type::TYPE_STRING)],
            reserved_name: vec!["old_name".into()],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::ReservedMessageFieldName { message, name }
                    if message == "invalid.test.ReservedName" && name == "old_name"
            ));
        },
    );
}

#[test]
fn duplicate_message_reserved_names_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("duplicate-message-reserved-name.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![
                DescriptorProto {
                    name: Some("Valid".into()),
                    reserved_name: vec!["legacy".into()],
                    ..Default::default()
                },
                DescriptorProto {
                    name: Some("Invalid".into()),
                    reserved_name: vec!["legacy".into(), "legacy".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "duplicate-message-reserved-name.proto",
        "invalid.test.Invalid",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateMessageReservedName { message, name }
                    if message == "invalid.test.Invalid" && name == "legacy"
            ));
        },
    );
}

#[test]
fn reserved_message_field_numbers_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::ReservedRange;
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let max = buffa::encoding::MAX_FIELD_NUMBER as i32;
    for (suffix, start, end, number) in [("start", 7, 8, 7), ("max", max, max + 1, max)] {
        let message_name = format!("ReservedNumber{suffix}");
        let full_name = format!("invalid.test.{message_name}");
        let file_name = format!("reserved-field-number-{suffix}.proto");
        let expected_message = full_name.clone();

        assert_rejected_without_mutating_pool(
            &file_name,
            &full_name,
            DescriptorProto {
                name: Some(message_name),
                field: vec![scalar_field("value", number, Type::TYPE_INT32)],
                reserved_range: vec![ReservedRange {
                    start: Some(start),
                    end: Some(end),
                    ..Default::default()
                }],
                ..Default::default()
            },
            move |err| {
                assert!(matches!(
                    err,
                    PoolError::ReservedMessageFieldNumber {
                        message,
                        name,
                        number: actual,
                    } if message == &expected_message
                        && name == "value"
                        && *actual == number as u32
                ));
            },
        );
    }
}

#[test]
fn reserved_message_field_range_end_is_exclusive() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::ReservedRange;
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let pool = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("reserved-field-end.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Message".into()),
                field: vec![scalar_field("value", 8, Type::TYPE_INT32)],
                reserved_range: vec![ReservedRange {
                    start: Some(7),
                    end: Some(8),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .expect("the exclusive range end is available");

    assert_eq!(
        pool.message_by_name("valid.test.Message")
            .unwrap()
            .field(8)
            .unwrap()
            .number(),
        8
    );
}

#[test]
fn fields_inside_extension_ranges_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let make_set = |message_name: String, number: i32| FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some(format!("field-in-extension-range-{number}.proto")),
            package: Some("invalid.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some(message_name),
                field: vec![scalar_field("value", number, Type::TYPE_INT32)],
                extension_range: vec![
                    ExtensionRange {
                        start: Some(20),
                        end: Some(30),
                        ..Default::default()
                    },
                    ExtensionRange {
                        start: Some(7),
                        end: Some(10),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    for (suffix, number) in [("start", 7), ("end-minus-one", 9)] {
        let message_name = format!("FieldInRange{suffix}");
        let full_name = format!("invalid.test.{message_name}");
        let file_name = format!("field-in-extension-range-{number}.proto");
        let expected_message = full_name.clone();

        assert_rejected_without_mutating_pool(
            &file_name,
            &full_name,
            make_set(message_name, number).file[0].message_type[0].clone(),
            move |err| {
                assert!(matches!(
                    err,
                    PoolError::FieldInExtensionRange {
                        message,
                        name,
                        number: actual,
                    } if message == &expected_message
                        && name == "value"
                        && *actual == number as u32
                ));
            },
        );
    }

    let accepted = DescriptorPool::new(make_set("AdjacentFields".into(), 6)).unwrap();
    let message = accepted
        .message_by_name("invalid.test.AdjacentFields")
        .unwrap();
    assert!(message.field(6).is_some(), "start - 1 remains available");
    assert_eq!(
        message.extension_ranges(),
        &[(20, 30), (7, 10)],
        "stored extension ranges keep declaration order"
    );

    let accepted = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("field-at-extension-end.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("AtExtensionEnd".into()),
                field: vec![scalar_field("value", 10, Type::TYPE_INT32)],
                extension_range: vec![ExtensionRange {
                    start: Some(7),
                    end: Some(10),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    assert!(
        accepted
            .message_by_name("valid.test.AtExtensionEnd")
            .unwrap()
            .field(10)
            .is_some(),
        "the extension range end is exclusive"
    );
}

#[test]
fn reserved_and_extension_ranges_must_not_overlap() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::{
        ExtensionRange, ReservedRange,
    };
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("reserved-extension-overlap.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("RangeMessage".into()),
                reserved_range: vec![ReservedRange {
                    start: Some(7),
                    end: Some(8),
                    ..Default::default()
                }],
                extension_range: vec![ExtensionRange {
                    start: Some(6),
                    end: Some(9),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "reserved-extension-overlap.proto",
        "invalid.test.RangeMessage",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::ReservedExtensionRange {
                    message,
                    start: 6,
                    end: 9,
                } if message == "invalid.test.RangeMessage"
            ));
        },
    );
}

#[test]
fn adjacent_reserved_and_extension_ranges_are_accepted() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::{
        ExtensionRange, ReservedRange,
    };
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let pool = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("reserved-extension-adjacent.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("RangeMessage".into()),
                reserved_range: vec![ReservedRange {
                    start: Some(7),
                    end: Some(8),
                    ..Default::default()
                }],
                extension_range: vec![ExtensionRange {
                    start: Some(8),
                    end: Some(9),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .expect("adjacent ranges do not overlap");

    assert_eq!(
        pool.message_by_name("valid.test.RangeMessage")
            .unwrap()
            .extension_ranges(),
        &[(8, 9)]
    );
}

#[test]
fn extension_range_bounds_are_half_open_at_the_field_number_limit() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let max = buffa::encoding::MAX_FIELD_NUMBER as i32;
    let pool = DescriptorPool::new(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("valid-extension-bounds.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("RangeMessage".into()),
                extension_range: vec![
                    ExtensionRange {
                        start: Some(7),
                        end: Some(8),
                        ..Default::default()
                    },
                    ExtensionRange {
                        start: Some(max),
                        end: Some(max + 1),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .expect("one-element ranges include the maximum field number");

    let message = pool.message_by_name("valid.test.RangeMessage").unwrap();
    assert_eq!(
        message.extension_ranges(),
        &[(7, 8), (max as u32, (max + 1) as u32)]
    );
    assert!(message.in_extension_range(7));
    assert!(!message.in_extension_range(8));
    assert!(message.in_extension_range(max as u32));
    assert!(!message.in_extension_range((max + 1) as u32));
}

#[test]
fn invalid_extension_range_bounds_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let max = buffa::encoding::MAX_FIELD_NUMBER as i32;
    // protoc reads an unset bound as 0, then requires `0 < start < end`.
    for (suffix, start, end) in [
        ("equal", Some(7), Some(7)),
        ("reversed", Some(8), Some(7)),
        ("max-equal", Some(max + 1), Some(max + 1)),
        ("max-reversed", Some(max + 1), Some(max)),
        ("zero-start", Some(0), Some(5)),
        ("negative-start", Some(-3), Some(5)),
        ("negative-end", Some(3), Some(-5)),
        ("unset-start", None, Some(5)),
        ("unset-end", Some(5), None),
        ("unset-both", None, None),
    ] {
        let message_name = format!("InvalidRange{suffix}");
        let full_name = format!("invalid.test.{message_name}");
        let file_name = format!("invalid-extension-range-{suffix}.proto");
        let expected_message = full_name.clone();

        assert_set_rejected_without_mutating_pool(
            &file_name,
            &full_name,
            FileDescriptorSet {
                file: vec![FileDescriptorProto {
                    name: Some(file_name.clone()),
                    package: Some("invalid.test".into()),
                    syntax: Some("proto2".into()),
                    message_type: vec![DescriptorProto {
                        name: Some(message_name),
                        extension_range: vec![ExtensionRange {
                            start,
                            end,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            move |err| {
                assert!(
                    matches!(
                        err,
                        PoolError::InvalidExtensionRange {
                            message,
                            start: actual_start,
                            end: actual_end,
                        } if message == &expected_message
                            && *actual_start == start
                            && *actual_end == end
                    ),
                    "unexpected error: {err}"
                );
                if (start, end) == (Some(5), None) {
                    assert_eq!(
                        err.to_string(),
                        format!(
                            "message {expected_message} extension range 5..unset is invalid; \
                             bounds must satisfy 0 < start < end"
                        )
                    );
                }
            },
        );
    }
}

#[test]
fn duplicate_proto_field_names_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    assert_rejected_without_mutating_pool(
        "duplicate-proto-name.proto",
        "invalid.test.BadProtoName",
        DescriptorProto {
            name: Some("BadProtoName".into()),
            field: vec![
                scalar_field("same", 1, Type::TYPE_INT32),
                scalar_field("same", 2, Type::TYPE_STRING),
            ],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateFieldName { message, name }
                    if message == "invalid.test.BadProtoName" && name == "same"
            ));
        },
    );
}

#[test]
fn duplicate_json_field_names_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let mut first = scalar_field("first", 1, Type::TYPE_INT32);
    first.json_name = Some("sameName".into());
    let mut second = scalar_field("second", 2, Type::TYPE_STRING);
    second.json_name = Some("sameName".into());

    assert_rejected_without_mutating_pool(
        "duplicate-json-name.proto",
        "invalid.test.BadJsonName",
        DescriptorProto {
            name: Some("BadJsonName".into()),
            field: vec![first, second],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateFieldName { message, name }
                    if message == "invalid.test.BadJsonName" && name == "sameName"
            ));
        },
    );
}

#[test]
fn proto_and_json_field_name_collisions_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let mut first = scalar_field("first", 1, Type::TYPE_INT32);
    first.json_name = Some("sharedName".into());
    let second = scalar_field("sharedName", 2, Type::TYPE_STRING);

    assert_rejected_without_mutating_pool(
        "cross-name-collision.proto",
        "invalid.test.BadCrossName",
        DescriptorProto {
            name: Some("BadCrossName".into()),
            field: vec![first, second],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateFieldName { message, name }
                    if message == "invalid.test.BadCrossName" && name == "sharedName"
            ));
        },
    );
}

#[test]
fn positive_out_of_range_oneof_indices_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let mut field = scalar_field("member", 1, Type::TYPE_INT32);
    field.oneof_index = Some(1);

    assert_rejected_without_mutating_pool(
        "positive-oneof-index.proto",
        "invalid.test.BadPositiveOneof",
        DescriptorProto {
            name: Some("BadPositiveOneof".into()),
            field: vec![field],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::InvalidOneofIndex {
                    message,
                    field,
                    index,
                } if message == "invalid.test.BadPositiveOneof"
                    && field == "invalid.test.BadPositiveOneof.member"
                    && *index == 1
            ));
        },
    );
}

#[test]
fn negative_oneof_indices_are_rejected_without_mutating_pool() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let mut field = scalar_field("member", 1, Type::TYPE_INT32);
    field.oneof_index = Some(-1);

    assert_rejected_without_mutating_pool(
        "negative-oneof-index.proto",
        "invalid.test.BadNegativeOneof",
        DescriptorProto {
            name: Some("BadNegativeOneof".into()),
            field: vec![field],
            ..Default::default()
        },
        |err| {
            assert!(matches!(
                err,
                PoolError::InvalidOneofIndex {
                    message,
                    field,
                    index,
                } if message == "invalid.test.BadNegativeOneof"
                    && field == "invalid.test.BadNegativeOneof.member"
                    && *index == -1
            ));
        },
    );
}

#[test]
fn message_and_service_symbol_collisions_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet, ServiceDescriptorProto,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("message-service-collision.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Foo".into()),
                ..Default::default()
            }],
            service: vec![ServiceDescriptorProto {
                name: Some("Foo".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "message-service-collision.proto",
        "invalid.test.Foo",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateName(name) if name == "invalid.test.Foo"
            ));
        },
    );
}

#[test]
fn enum_and_service_symbol_collisions_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
        ServiceDescriptorProto,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("enum-service-collision.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Foo".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("FOO_UNSPECIFIED".into()),
                    number: Some(0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            service: vec![ServiceDescriptorProto {
                name: Some("Foo".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "enum-service-collision.proto",
        "invalid.test.Foo",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateName(name) if name == "invalid.test.Foo"
            ));
        },
    );
}

#[test]
fn duplicate_rpc_method_names_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto,
    };

    let method = || MethodDescriptorProto {
        name: Some("Run".into()),
        input_type: Some(".reflect.test.Inner".into()),
        output_type: Some(".reflect.test.Inner".into()),
        ..Default::default()
    };
    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("duplicate-method.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            dependency: vec!["reflect_test.proto".into()],
            service: vec![ServiceDescriptorProto {
                name: Some("Gateway".into()),
                method: vec![method(), method()],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "duplicate-method.proto",
        "invalid.test.Gateway.Run",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateMethodName { service, name }
                    if service == "invalid.test.Gateway" && name == "Run"
            ));
        },
    );
}

#[test]
fn duplicate_enum_value_names_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("duplicate-enum-value.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("UNKNOWN".into()),
                        number: Some(0),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("UNKNOWN".into()),
                        number: Some(1),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "duplicate-enum-value.proto",
        "invalid.test.Status",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateEnumValueName { enum_name, name }
                    if enum_name == "invalid.test.Status" && name == "UNKNOWN"
            ));
        },
    );
}

#[test]
fn reserved_enum_value_numbers_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::enum_descriptor_proto::EnumReservedRange;
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    for number in [7, 9] {
        let set = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some(format!("reserved-enum-number-{number}.proto")),
                package: Some("invalid.test".into()),
                syntax: Some("proto3".into()),
                enum_type: vec![
                    EnumDescriptorProto {
                        name: Some("Valid".into()),
                        value: vec![
                            EnumValueDescriptorProto {
                                name: Some("VALID_UNSPECIFIED".into()),
                                number: Some(0),
                                ..Default::default()
                            },
                            EnumValueDescriptorProto {
                                name: Some("ACTIVE".into()),
                                number: Some(1),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    },
                    EnumDescriptorProto {
                        name: Some("Status".into()),
                        value: vec![
                            EnumValueDescriptorProto {
                                name: Some("STATUS_UNSPECIFIED".into()),
                                number: Some(0),
                                ..Default::default()
                            },
                            EnumValueDescriptorProto {
                                name: Some("VALUE".into()),
                                number: Some(number),
                                ..Default::default()
                            },
                        ],
                        reserved_range: vec![EnumReservedRange {
                            start: Some(7),
                            end: Some(9),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_set_rejected_without_mutating_pool(
            &format!("reserved-enum-number-{number}.proto"),
            "invalid.test.Status",
            set,
            |err| {
                assert!(matches!(
                    err,
                    PoolError::ReservedEnumValueNumber {
                        enum_name,
                        name,
                        number: actual,
                    } if enum_name == "invalid.test.Status"
                        && name == "VALUE"
                        && *actual == number
                ));
            },
        );
    }
}

#[test]
fn reserved_enum_value_names_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("reserved-enum-name.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![
                EnumDescriptorProto {
                    name: Some("Valid".into()),
                    value: vec![
                        EnumValueDescriptorProto {
                            name: Some("VALID_UNSPECIFIED".into()),
                            number: Some(0),
                            ..Default::default()
                        },
                        EnumValueDescriptorProto {
                            name: Some("ACTIVE".into()),
                            number: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                EnumDescriptorProto {
                    name: Some("Status".into()),
                    value: vec![
                        EnumValueDescriptorProto {
                            name: Some("STATUS_UNSPECIFIED".into()),
                            number: Some(0),
                            ..Default::default()
                        },
                        EnumValueDescriptorProto {
                            name: Some("DEPRECATED".into()),
                            number: Some(2),
                            ..Default::default()
                        },
                    ],
                    reserved_name: vec!["DEPRECATED".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "reserved-enum-name.proto",
        "invalid.test.Status",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::ReservedEnumValueName { enum_name, name }
                    if enum_name == "invalid.test.Status" && name == "DEPRECATED"
            ));
        },
    );
}

#[test]
fn duplicate_enum_reserved_names_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("duplicate-enum-reserved-name.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![
                EnumDescriptorProto {
                    name: Some("Valid".into()),
                    reserved_name: vec!["LEGACY".into()],
                    value: vec![enum_value("UNSPECIFIED", 0), enum_value("ACTIVE", 1)],
                    ..Default::default()
                },
                EnumDescriptorProto {
                    name: Some("Invalid".into()),
                    reserved_name: vec!["LEGACY".into(), "LEGACY".into()],
                    value: vec![enum_value("INVALID_UNSPECIFIED", 0)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "duplicate-enum-reserved-name.proto",
        "invalid.test.Invalid",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateEnumReservedName { enum_name, name }
                    if enum_name == "invalid.test.Invalid" && name == "LEGACY"
            ));
        },
    );
}

#[test]
fn non_reserved_enum_values_are_accepted() {
    use buffa_descriptor::generated::descriptor::enum_descriptor_proto::EnumReservedRange;
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("valid-enum-value.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("STATUS_UNSPECIFIED".into()),
                        number: Some(0),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("ACTIVE".into()),
                        number: Some(6),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("AFTER".into()),
                        number: Some(10), // one past the reserved end
                        ..Default::default()
                    },
                ],
                reserved_range: vec![EnumReservedRange {
                    start: Some(7),
                    end: Some(9),
                    ..Default::default()
                }],
                reserved_name: vec!["DEPRECATED".into()],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    DescriptorPool::new(set).expect("non-reserved enum values are valid");
}

#[test]
fn distinct_reserved_names_and_names_in_different_owners_are_accepted() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, EnumDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("valid-reserved-names.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![
                DescriptorProto {
                    name: Some("First".into()),
                    field: vec![scalar_field("current", 1, Type::TYPE_STRING)],
                    reserved_name: vec!["legacy".into(), "old_name".into()],
                    ..Default::default()
                },
                DescriptorProto {
                    name: Some("Second".into()),
                    field: vec![scalar_field("current", 1, Type::TYPE_STRING)],
                    reserved_name: vec!["legacy".into()],
                    ..Default::default()
                },
            ],
            enum_type: vec![
                EnumDescriptorProto {
                    name: Some("FirstStatus".into()),
                    value: vec![enum_value("FIRST_UNSPECIFIED", 0), enum_value("ACTIVE", 1)],
                    reserved_name: vec!["LEGACY".into(), "OLD_STATUS".into()],
                    ..Default::default()
                },
                EnumDescriptorProto {
                    name: Some("SecondStatus".into()),
                    value: vec![
                        enum_value("SECOND_UNSPECIFIED", 0),
                        enum_value("STARTED", 1),
                    ],
                    reserved_name: vec!["LEGACY".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    DescriptorPool::new(set).expect("distinct reserved names should be accepted");
}

#[test]
fn duplicate_enum_value_numbers_are_rejected_without_allow_alias() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto, FileDescriptorProto,
        FileDescriptorSet,
    };

    for (suffix, allow_alias) in [("unset", None), ("false", Some(false))] {
        let file_name = format!("duplicate-enum-number-{suffix}.proto");
        let set = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some(file_name.clone()),
                package: Some("invalid.test".into()),
                syntax: Some("proto3".into()),
                enum_type: vec![EnumDescriptorProto {
                    name: Some("Status".into()),
                    options: allow_alias
                        .map(|value| EnumOptions {
                            allow_alias: Some(value),
                            ..Default::default()
                        })
                        .into(),
                    value: vec![
                        EnumValueDescriptorProto {
                            name: Some("UNSPECIFIED".into()),
                            number: Some(0),
                            ..Default::default()
                        },
                        EnumValueDescriptorProto {
                            name: Some("ACTIVE".into()),
                            number: Some(1),
                            ..Default::default()
                        },
                        EnumValueDescriptorProto {
                            name: Some("STARTED".into()),
                            number: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_set_rejected_without_mutating_pool(&file_name, "invalid.test.Status", set, |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateEnumValueNumber {
                    enum_name,
                    name,
                    number,
                } if enum_name == "invalid.test.Status"
                    && name == "STARTED"
                    && *number == 1
            ));
        });
    }
}

#[test]
fn duplicate_enum_value_numbers_are_accepted_when_allow_alias_is_true() {
    use buffa_descriptor::generated::descriptor::{
        EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto, FileDescriptorProto,
        FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("duplicate-enum-number-allowed.proto".into()),
            package: Some("valid.test".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Status".into()),
                options: EnumOptions {
                    allow_alias: Some(true),
                    ..Default::default()
                }
                .into(),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("UNSPECIFIED".into()),
                        number: Some(0),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("ACTIVE".into()),
                        number: Some(1),
                        ..Default::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("STARTED".into()),
                        number: Some(1),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let pool = DescriptorPool::new(set).expect("enum aliases should be accepted");
    let status = pool.enum_by_name("valid.test.Status").unwrap();
    assert_eq!(
        status
            .values()
            .iter()
            .map(|value| (value.name(), value.number()))
            .collect::<Vec<_>>(),
        [("UNSPECIFIED", 0), ("ACTIVE", 1), ("STARTED", 1)]
    );
    assert_eq!(status.value(1).unwrap().name(), "ACTIVE");
}

#[test]
fn method_fqn_collisions_with_registered_symbols_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto,
        ServiceDescriptorProto,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("method-symbol-collision.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Gateway".into()),
                nested_type: vec![DescriptorProto {
                    name: Some("Run".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            service: vec![ServiceDescriptorProto {
                name: Some("Gateway".into()),
                method: vec![MethodDescriptorProto {
                    name: Some("Run".into()),
                    input_type: Some(".reflect.test.Inner".into()),
                    output_type: Some(".reflect.test.Inner".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "method-symbol-collision.proto",
        "invalid.test.Gateway.Run",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::DuplicateName(name) if name == "invalid.test.Gateway.Run"
            ));
        },
    );
}

#[test]
fn out_of_range_public_dependency_indices_are_rejected_transactionally() {
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("bad-public-dependency.proto".into()),
            package: Some("invalid.test".into()),
            syntax: Some("proto3".into()),
            // No imports at all, so any index names nothing.
            public_dependency: vec![7],
            message_type: vec![DescriptorProto {
                name: Some("Solo".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_set_rejected_without_mutating_pool(
        "bad-public-dependency.proto",
        "invalid.test.Solo",
        set,
        |err| {
            assert!(matches!(
                err,
                PoolError::InvalidPublicDependencyIndex {
                    index: 7,
                    dependency_count: 0,
                    ..
                }
            ));
            assert_eq!(
                err.to_string(),
                "file bad-public-dependency.proto public_dependency index 7 \
                 is out of range (0 dependencies declared)"
            );
        },
    );
}

#[test]
fn import_public_indices_that_protoc_emits_are_accepted() {
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    // What `import public "b.proto";` compiles to: a `dependency` entry plus
    // its position in `public_dependency`. No .proto in the test corpus uses
    // `import public`, so this is the only cover for a non-empty
    // `public_dependency` reaching the pool.
    let set = FileDescriptorSet {
        file: vec![
            FileDescriptorProto {
                name: Some("b.proto".into()),
                package: Some("pubdep.test".into()),
                syntax: Some("proto3".into()),
                message_type: vec![DescriptorProto {
                    name: Some("Base".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            FileDescriptorProto {
                name: Some("a.proto".into()),
                package: Some("pubdep.test".into()),
                syntax: Some("proto3".into()),
                dependency: vec!["b.proto".into()],
                public_dependency: vec![0],
                message_type: vec![DescriptorProto {
                    name: Some("Front".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let mut p = DescriptorPool::decode(FDS_BYTES).unwrap();
    p.add_file_descriptor_set(set)
        .expect("a re-exporting import links");

    assert!(p.file_by_name("a.proto").is_some());
    assert!(p.message_by_name("pubdep.test.Front").is_some());
    assert!(p.message_by_name("pubdep.test.Base").is_some());
}

#[test]
fn service_descriptor_links() {
    let p = pool();
    let svc = p
        .service_by_name("reflect.test.Demo")
        .expect("Demo service registered");
    assert_eq!(svc.full_name(), "reflect.test.Demo");
    assert_eq!(svc.methods().len(), 4);

    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();

    let get = svc.method("Get").expect("Get method");
    assert_eq!(get.name(), "Get");
    assert_eq!(get.input(), inner_idx);
    assert_eq!(get.output(), containers_idx);
    assert!(!get.is_client_streaming());
    assert!(!get.is_server_streaming());

    let push = svc.method("Push").expect("Push method");
    assert!(push.is_client_streaming());
    assert!(!push.is_server_streaming());

    let pull = svc.method("Pull").expect("Pull method");
    assert!(!pull.is_client_streaming());
    assert!(pull.is_server_streaming());

    let sync = svc.method("Sync").expect("Sync method");
    assert!(sync.is_client_streaming());
    assert!(sync.is_server_streaming());

    assert!(svc.method("Nonexistent").is_none());
    assert!(p.service_by_name("reflect.test.Other").is_none());
    // service_index round-trips.
    let idx = p.service_index("reflect.test.Demo").expect("indexed");
    assert_eq!(p.service(idx).full_name(), "reflect.test.Demo");
}

#[test]
fn extensions_link() {
    let p = pool();
    let extendable = p.message_index("reflect.ext.Extendable").unwrap();

    // File-level extension, registered under the package.
    let ext = p
        .extension_by_name("reflect.ext.ext_int32")
        .expect("file-level extension registered");
    assert_eq!(ext.full_name(), "reflect.ext.ext_int32");
    assert_eq!(ext.extendee(), extendable);
    assert_eq!(ext.field().name(), "ext_int32");
    assert_eq!(ext.field().number(), 100);
    assert_eq!(
        ext.field().kind(),
        FieldKind::Singular(SingularKind::Scalar(ScalarType::Int32))
    );
    // proto2 optional → explicit presence.
    assert_eq!(ext.field().presence(), FieldPresence::Explicit);

    // Repeated extension.
    let rep = p.extension_by_name("reflect.ext.ext_repeated").unwrap();
    assert_eq!(
        rep.field().kind(),
        FieldKind::List(SingularKind::Scalar(ScalarType::Int32))
    );

    // Message-typed extension resolves its value type.
    let payload = p.message_index("reflect.ext.Payload").unwrap();
    let msg_ext = p.extension_by_name("reflect.ext.ext_message").unwrap();
    assert_eq!(
        msg_ext.field().kind(),
        FieldKind::Singular(SingularKind::Message(payload))
    );

    // Message-scoped extension is registered under the declaring message.
    let nested = p
        .extension_by_name("reflect.ext.Scope.ext_nested")
        .expect("message-scoped extension registered under its scope");
    assert_eq!(nested.extendee(), extendable);
    assert_eq!(nested.field().number(), 110);
    assert!(p.extension_by_name("reflect.ext.ext_nested").is_none());

    // (extendee, number) lookup and range iteration.
    assert!(p.extension_for(extendable, 100).is_some());
    assert!(p.extension_for(extendable, 99).is_none());
    let all: Vec<u32> = p
        .extensions_of(extendable)
        .map(|e| e.field().number())
        .collect();
    assert_eq!(all, vec![100, 101, 102, 103, 110, 120]);
    // A message with no extensions yields nothing.
    let inner = p.message_index("reflect.test.Inner").unwrap();
    assert_eq!(p.extensions_of(inner).count(), 0);
}

/// Build a one-message file with the given syntax and add it to a fresh pool.
fn add_message_with_syntax(
    syntax: &str,
    message: buffa_descriptor::generated::descriptor::DescriptorProto,
) -> Result<(), PoolError> {
    use buffa_descriptor::generated::descriptor::{FileDescriptorProto, FileDescriptorSet};

    let mut p = DescriptorPool::decode(FDS_BYTES).unwrap();
    p.add_file_descriptor_set(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("json-name-leniency.proto".into()),
            package: Some("lenient.test".into()),
            syntax: Some(syntax.into()),
            message_type: vec![message],
            ..Default::default()
        }],
        ..Default::default()
    })
}

/// Two fields resolving to one JSON name is ambiguous, but protobuf permits it
/// where JSON is best-effort, and protoc emits such a descriptor set with only
/// a warning. Rejecting it here would refuse input protoc produced, so proto2
/// keeps the leniency its `json_format` feature already resolves to.
#[test]
fn proto2_json_name_conflicts_are_accepted_as_protoc_emits_them() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let mut first = scalar_field("first", 1, Type::TYPE_INT32);
    first.json_name = Some("sameName".into());
    let mut second = scalar_field("second", 2, Type::TYPE_STRING);
    second.json_name = Some("sameName".into());

    add_message_with_syntax(
        "proto2",
        DescriptorProto {
            name: Some("LegacyJsonName".into()),
            field: vec![first, second],
            ..Default::default()
        },
    )
    .expect("proto2 JSON-name conflicts are best-effort, and protoc emits them");
}

/// `deprecated_legacy_json_field_conflicts` is protoc's own opt-out: it
/// downgrades the conflict to a warning and emits the set. Honour it rather
/// than reject a descriptor set the author explicitly asked protoc to produce.
#[test]
fn deprecated_legacy_json_field_conflicts_opts_a_proto3_message_out() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::{DescriptorProto, MessageOptions};

    let mut first = scalar_field("first", 1, Type::TYPE_INT32);
    first.json_name = Some("sameName".into());
    let mut second = scalar_field("second", 2, Type::TYPE_STRING);
    second.json_name = Some("sameName".into());

    add_message_with_syntax(
        "proto3",
        DescriptorProto {
            name: Some("OptedOut".into()),
            field: vec![first, second],
            options: MessageOptions {
                deprecated_legacy_json_field_conflicts: Some(true),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
    )
    .expect("the message opted out of JSON-name conflict checking");
}

/// The leniency is scoped to JSON names: a duplicate *proto* field name is
/// invalid in every syntax and protoc never emits one.
#[test]
fn proto2_still_rejects_duplicate_proto_field_names() {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::Type;
    use buffa_descriptor::generated::descriptor::DescriptorProto;

    let err = add_message_with_syntax(
        "proto2",
        DescriptorProto {
            name: Some("DupProtoName".into()),
            field: vec![
                scalar_field("same", 1, Type::TYPE_INT32),
                scalar_field("same", 2, Type::TYPE_STRING),
            ],
            ..Default::default()
        },
    )
    .expect_err("a duplicate proto field name is invalid whatever the syntax");
    assert!(matches!(
        err,
        PoolError::DuplicateFieldName { ref name, .. } if name == "same"
    ));
}

// ── import visibility (#423) ────────────────────────────────────────────────

mod import_visibility {
    use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
        MethodDescriptorProto, ServiceDescriptorProto,
    };
    use buffa_descriptor::{DescriptorPool, LinkOptions, PoolError};

    /// `<name>.proto` in package `<name>` declaring `message Thing {}`.
    fn leaf(name: &str) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(format!("{name}.proto")),
            package: Some(name.into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Thing".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// A singular message field referencing `type_name`.
    fn msg_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.into()),
            number: Some(number),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(type_name.into()),
            ..Default::default()
        }
    }

    /// `<name>.proto` in package `<name>` with `message Holder { <target> thing = 1; }`
    /// and the given import lists.
    fn referrer(name: &str, deps: &[&str], public: &[i32], target: &str) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(format!("{name}.proto")),
            package: Some(name.into()),
            syntax: Some("proto3".into()),
            dependency: deps.iter().map(|d| (*d).to_string()).collect(),
            public_dependency: public.to_vec(),
            message_type: vec![DescriptorProto {
                name: Some("Holder".into()),
                field: vec![msg_field("thing", 1, target)],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn set(files: Vec<FileDescriptorProto>) -> FileDescriptorSet {
        FileDescriptorSet {
            file: files,
            ..Default::default()
        }
    }

    fn assert_not_imported(err: &PoolError, file: &str, type_name: &str, defined_in: &str) {
        assert!(
            matches!(
                err,
                PoolError::TypeNotImported { file: f, type_name: t, defined_in: d, .. }
                    if f == file && t == type_name && d == defined_in
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reference_without_import_is_rejected() {
        let err = DescriptorPool::new(set(vec![leaf("a"), referrer("b", &[], &[], ".a.Thing")]))
            .unwrap_err();
        assert_not_imported(&err, "b.proto", ".a.Thing", "a.proto");
        assert_eq!(
            err.to_string(),
            "field b.Holder.thing references \".a.Thing\", which is defined in a.proto \
             and not imported by b.proto; add a.proto to its dependency list"
        );
    }

    #[test]
    fn reference_with_import_links() {
        let pool = DescriptorPool::new(set(vec![
            leaf("a"),
            referrer("b", &["a.proto"], &[], ".a.Thing"),
        ]))
        .expect("imported type links");
        assert!(pool.message_by_name("b.Holder").is_some());
    }

    #[test]
    fn import_order_within_a_set_does_not_matter() {
        // The referrer precedes the file it imports; protoc emits sets in
        // dependency order, but hand-built and merged sets need not be.
        DescriptorPool::new(set(vec![
            referrer("b", &["a.proto"], &[], ".a.Thing"),
            leaf("a"),
        ]))
        .expect("links regardless of file order");
    }

    #[test]
    fn same_file_and_nested_references_need_no_import() {
        let file = FileDescriptorProto {
            name: Some("self.proto".into()),
            package: Some("selfref".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Outer".into()),
                field: vec![
                    msg_field("inner", 1, ".selfref.Outer.Inner"),
                    msg_field("me", 2, ".selfref.Outer"),
                ],
                nested_type: vec![DescriptorProto {
                    name: Some("Inner".into()),
                    field: vec![msg_field("up", 1, ".selfref.Outer")],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        DescriptorPool::new(set(vec![file])).expect("same-file references link");
    }

    #[test]
    fn public_import_chain_is_visible_transitively() {
        // c imports b; b `import public "a.proto"`; c may use a.Thing.
        let b = FileDescriptorProto {
            name: Some("b.proto".into()),
            package: Some("b".into()),
            dependency: vec!["a.proto".into()],
            public_dependency: vec![0],
            ..Default::default()
        };
        DescriptorPool::new(set(vec![
            leaf("a"),
            b,
            referrer("c", &["b.proto"], &[], ".a.Thing"),
        ]))
        .expect("public re-export is visible");
    }

    #[test]
    fn two_hop_public_import_chain_is_visible() {
        // d imports c; c `import public` b; b `import public` a.
        let reexport = |name: &str, dep: &str| FileDescriptorProto {
            name: Some(format!("{name}.proto")),
            package: Some(name.into()),
            dependency: vec![dep.into()],
            public_dependency: vec![0],
            ..Default::default()
        };
        DescriptorPool::new(set(vec![
            leaf("a"),
            reexport("b", "a.proto"),
            reexport("c", "b.proto"),
            referrer("d", &["c.proto"], &[], ".a.Thing"),
        ]))
        .expect("transitive public re-export is visible");
    }

    #[test]
    fn ordinary_transitive_import_is_not_visible() {
        // c imports b; b imports a (not public); c may NOT use a.Thing.
        let b = FileDescriptorProto {
            name: Some("b.proto".into()),
            package: Some("b".into()),
            dependency: vec!["a.proto".into()],
            ..Default::default()
        };
        let err = DescriptorPool::new(set(vec![
            leaf("a"),
            b,
            referrer("c", &["b.proto"], &[], ".a.Thing"),
        ]))
        .unwrap_err();
        assert_not_imported(&err, "c.proto", ".a.Thing", "a.proto");
    }

    #[test]
    fn public_import_cycle_terminates() {
        // a and b publicly import each other; c imports a and uses b.Thing.
        // protoc rejects the cycle itself ("File recursively imports
        // itself"); the pool does not detect import cycles yet, so this pins
        // only that the visibility walk terminates on one.
        let cyc = |name: &str, dep: &str| FileDescriptorProto {
            dependency: vec![dep.into()],
            public_dependency: vec![0],
            ..leaf(name)
        };
        DescriptorPool::new(set(vec![
            cyc("a", "b.proto"),
            cyc("b", "a.proto"),
            referrer("c", &["a.proto"], &[], ".b.Thing"),
        ]))
        .expect("cycle in public imports does not loop and b is visible through a");
    }

    #[test]
    fn weak_dependency_is_visible() {
        let c = FileDescriptorProto {
            weak_dependency: vec![0],
            ..referrer("c", &["a.proto"], &[], ".a.Thing")
        };
        DescriptorPool::new(set(vec![leaf("a"), c])).expect("weak imports resolve like imports");
    }

    #[test]
    fn extendee_and_extension_type_must_be_imported() {
        let extendable = FileDescriptorProto {
            name: Some("base.proto".into()),
            package: Some("base".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("Ext".into()),
                extension_range: vec![
                    buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange {
                        start: Some(100),
                        end: Some(200),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let extender = |deps: &[&str], value_type: &str| FileDescriptorProto {
            name: Some("extender.proto".into()),
            package: Some("extender".into()),
            syntax: Some("proto2".into()),
            dependency: deps.iter().map(|d| (*d).to_string()).collect(),
            extension: vec![FieldDescriptorProto {
                extendee: Some(".base.Ext".into()),
                ..msg_field("payload", 100, value_type)
            }],
            ..Default::default()
        };

        // Extendee not imported.
        let err = DescriptorPool::new(set(vec![
            extendable.clone(),
            leaf("a"),
            extender(&["a.proto"], ".a.Thing"),
        ]))
        .unwrap_err();
        assert_not_imported(&err, "extender.proto", ".base.Ext", "base.proto");

        // Extendee imported, value type not.
        let err = DescriptorPool::new(set(vec![
            extendable.clone(),
            leaf("a"),
            extender(&["base.proto"], ".a.Thing"),
        ]))
        .unwrap_err();
        assert_not_imported(&err, "extender.proto", ".a.Thing", "a.proto");

        // Both imported.
        DescriptorPool::new(set(vec![
            extendable,
            leaf("a"),
            extender(&["base.proto", "a.proto"], ".a.Thing"),
        ]))
        .expect("extension links when extendee and value type are imported");
    }

    #[test]
    fn method_types_must_be_imported() {
        let svc = |deps: &[&str]| FileDescriptorProto {
            name: Some("svc.proto".into()),
            package: Some("svc".into()),
            syntax: Some("proto3".into()),
            dependency: deps.iter().map(|d| (*d).to_string()).collect(),
            service: vec![ServiceDescriptorProto {
                name: Some("S".into()),
                method: vec![MethodDescriptorProto {
                    name: Some("Call".into()),
                    input_type: Some(".a.Thing".into()),
                    output_type: Some(".a.Thing".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = DescriptorPool::new(set(vec![leaf("a"), svc(&[])])).unwrap_err();
        assert!(
            matches!(&err, PoolError::TypeNotImported { field, .. } if field == "svc.S.Call"),
            "unexpected error: {err}"
        );
        DescriptorPool::new(set(vec![leaf("a"), svc(&["a.proto"])]))
            .expect("method types link when imported");
    }

    #[test]
    fn map_value_type_must_be_imported() {
        let entry = DescriptorProto {
            name: Some("ByIdEntry".into()),
            field: vec![
                FieldDescriptorProto {
                    name: Some("key".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_OPTIONAL),
                    r#type: Some(Type::TYPE_INT32),
                    ..Default::default()
                },
                msg_field("value", 2, ".a.Thing"),
            ],
            options: buffa::MessageField::some(
                buffa_descriptor::generated::descriptor::MessageOptions {
                    map_entry: Some(true),
                    ..Default::default()
                },
            ),
            ..Default::default()
        };
        let holder = |deps: &[&str]| FileDescriptorProto {
            name: Some("m.proto".into()),
            package: Some("m".into()),
            syntax: Some("proto3".into()),
            dependency: deps.iter().map(|d| (*d).to_string()).collect(),
            message_type: vec![DescriptorProto {
                name: Some("Holder".into()),
                field: vec![FieldDescriptorProto {
                    label: Some(Label::LABEL_REPEATED),
                    ..msg_field("by_id", 1, ".m.Holder.ByIdEntry")
                }],
                nested_type: vec![entry.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = DescriptorPool::new(set(vec![leaf("a"), holder(&[])])).unwrap_err();
        assert_not_imported(&err, "m.proto", ".a.Thing", "a.proto");
        DescriptorPool::new(set(vec![leaf("a"), holder(&["a.proto"])]))
            .expect("map value type links when imported");
    }

    #[test]
    fn later_set_may_import_files_from_an_earlier_set() {
        let mut pool = DescriptorPool::new(set(vec![leaf("a")])).unwrap();
        pool.add_file_descriptor_set(set(vec![referrer("b", &["a.proto"], &[], ".a.Thing")]))
            .expect("import of an already-pooled file resolves");
        assert!(pool.message_by_name("b.Holder").is_some());

        // And visibility is still enforced across the boundary.
        let err = pool
            .add_file_descriptor_set(set(vec![referrer("c", &[], &[], ".a.Thing")]))
            .unwrap_err();
        assert_not_imported(&err, "c.proto", ".a.Thing", "a.proto");
        assert!(
            pool.file_by_name("c.proto").is_none(),
            "rejected set left the pool unchanged"
        );
        assert!(pool.message_by_name("c.Holder").is_none());
    }

    #[test]
    fn public_reexport_through_an_earlier_set_is_visible() {
        // Set 1: a, and b which `import public` a. Set 2: c imports b, uses a.
        let b = FileDescriptorProto {
            name: Some("b.proto".into()),
            package: Some("b".into()),
            dependency: vec!["a.proto".into()],
            public_dependency: vec![0],
            ..Default::default()
        };
        let mut pool = DescriptorPool::new(set(vec![leaf("a"), b])).unwrap();
        pool.add_file_descriptor_set(set(vec![referrer("c", &["b.proto"], &[], ".a.Thing")]))
            .expect("public re-export recorded in an earlier set is honoured");
    }

    #[test]
    fn absent_dependency_is_tolerated_by_default_and_rejected_when_required() {
        // `b` lists an import that is nowhere in the set, but references
        // nothing from it (the option-only-import shape).
        let b = FileDescriptorProto {
            dependency: vec!["a.proto".into(), "google/api/annotations.proto".into()],
            ..referrer("b", &[], &[], ".a.Thing")
        };
        DescriptorPool::new(set(vec![leaf("a"), b.clone()]))
            .expect("an absent, unreferenced import is tolerated by default");

        // A reference *into* the absent file is still an unresolved name, not
        // a visibility error: the file's symbols were never registered.
        let dangling = FileDescriptorProto {
            dependency: vec!["absent.proto".into()],
            ..referrer("d", &[], &[], ".absent.Thing")
        };
        let err = DescriptorPool::new(set(vec![dangling])).unwrap_err();
        assert!(
            matches!(&err, PoolError::UnresolvedTypeName { type_name, .. } if type_name == ".absent.Thing"),
            "unexpected error: {err}"
        );

        let mut strict =
            DescriptorPool::with_link_options(LinkOptions::new().with_required_dependencies(true));
        let err = strict
            .add_file_descriptor_set(set(vec![leaf("a"), b]))
            .unwrap_err();
        assert!(
            matches!(
                &err,
                PoolError::DependencyNotFound { file, dependency }
                    if file == "b.proto" && dependency == "google/api/annotations.proto"
            ),
            "unexpected error: {err}"
        );
        assert_eq!(
            err.to_string(),
            "file b.proto imports google/api/annotations.proto, which is not in the pool"
        );
        assert!(
            strict.file_by_name("a.proto").is_none(),
            "rejected set left the pool unchanged"
        );

        // A missing *weak* dependency is exempt, as under protoc.
        let weak_only = FileDescriptorProto {
            dependency: vec!["a.proto".into(), "gone.proto".into()],
            weak_dependency: vec![1],
            ..referrer("w", &[], &[], ".a.Thing")
        };
        strict
            .add_file_descriptor_set(set(vec![leaf("a"), weak_only]))
            .expect("a missing weak dependency is tolerated even when dependencies are required");
    }

    #[test]
    fn decode_with_link_options_applies_them() {
        use buffa::Message as _;
        let bytes = set(vec![leaf("a"), referrer("b", &[], &[], ".a.Thing")]).encode_to_vec();
        assert!(matches!(
            DescriptorPool::decode(&bytes),
            Err(PoolError::TypeNotImported { .. })
        ));
        let pool = DescriptorPool::decode_with_link_options(
            &bytes,
            &buffa::DecodeOptions::new(),
            LinkOptions::new().with_import_visibility(false),
        )
        .expect("flat resolution when visibility is off");
        assert!(!pool.link_options().import_visibility());
        assert!(pool.message_by_name("b.Holder").is_some());
    }

    #[test]
    fn enum_reference_follows_the_same_rule() {
        use buffa_descriptor::generated::descriptor::{
            EnumDescriptorProto, EnumValueDescriptorProto,
        };
        let enums = FileDescriptorProto {
            name: Some("e.proto".into()),
            package: Some("e".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Colour".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("COLOUR_UNSPECIFIED".into()),
                    number: Some(0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let user = |deps: &[&str]| FileDescriptorProto {
            name: Some("u.proto".into()),
            package: Some("u".into()),
            syntax: Some("proto3".into()),
            dependency: deps.iter().map(|d| (*d).to_string()).collect(),
            message_type: vec![DescriptorProto {
                name: Some("Paint".into()),
                field: vec![FieldDescriptorProto {
                    r#type: Some(Type::TYPE_ENUM),
                    ..msg_field("colour", 1, ".e.Colour")
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = DescriptorPool::new(set(vec![enums.clone(), user(&[])])).unwrap_err();
        assert_not_imported(&err, "u.proto", ".e.Colour", "e.proto");
        DescriptorPool::new(set(vec![enums, user(&["e.proto"])]))
            .expect("enum reference links when imported");
    }

    #[test]
    fn duplicate_file_names_in_one_set_are_rejected() {
        let mut pool = DescriptorPool::new(set(vec![leaf("z")])).unwrap();
        let err = pool
            .add_file_descriptor_set(set(vec![leaf("a"), leaf("a")]))
            .unwrap_err();
        assert!(
            matches!(&err, PoolError::DuplicateFileName { file } if file == "a.proto"),
            "unexpected error: {err}"
        );
        assert_eq!(
            err.to_string(),
            "file a.proto appears more than once in the set"
        );
        assert!(pool.file_by_name("a.proto").is_none());
        // A name already in the pool is an idempotent re-add, not a duplicate.
        pool.add_file_descriptor_set(set(vec![leaf("z"), leaf("z")]))
            .expect("re-adding a pooled file is a no-op");
    }

    #[test]
    fn enforcement_can_be_switched_off() {
        let mut pool =
            DescriptorPool::with_link_options(LinkOptions::new().with_import_visibility(false));
        assert!(!pool.link_options().import_visibility());
        pool.add_file_descriptor_set(set(vec![leaf("a"), referrer("b", &[], &[], ".a.Thing")]))
            .expect("flat resolution when visibility is not enforced");
        assert!(pool.message_by_name("b.Holder").is_some());
    }

    #[test]
    fn defaults_enforce_visibility_and_tolerate_absent_imports() {
        let opts = LinkOptions::default();
        assert!(opts.import_visibility());
        assert!(!opts.required_dependencies());
        assert_eq!(
            DescriptorPool::new(set(vec![])).unwrap().link_options(),
            opts
        );
    }

    #[test]
    fn protoc_output_links_under_the_defaults() {
        // The checked-in `reflect_test.fds` is real `protoc --include_imports`
        // output spanning several files with cross-file references
        // (extensions of messages in another file, WKT imports).
        DescriptorPool::decode(super::FDS_BYTES)
            .expect("protoc output satisfies import visibility");
    }
}

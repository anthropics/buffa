//! End-to-end tests for [`DescriptorPool`] linking and editions feature
//! resolution against a `protoc`-compiled `FileDescriptorSet`.
//!
//! Uses `tests/protos/reflect_test.{proto,fds}` and
//! `tests/protos/editions_test.proto`. Regenerate the `.fds` with:
//!
//! ```sh
//! protoc --include_imports --descriptor_set_out=reflect_test.fds \
//!     reflect_test.proto editions_test.proto
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
    // 16 fields: 15 scalars + f_opt.
    assert_eq!(scalars.fields().len(), 16);

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

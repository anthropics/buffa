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
use buffa_descriptor::{DescriptorPool, FieldKind, ScalarType, SingularKind};

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
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

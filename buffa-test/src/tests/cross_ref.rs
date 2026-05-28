//! Cross-package and cross-syntax type references via extern_path.

use super::round_trip;
use crate::basic::*;
use buffa::Message;

#[test]
fn test_cross_package_round_trip() {
    use crate::cross::Composite;
    let msg = Composite {
        person: buffa::MessageField::some(Person {
            id: 1,
            name: "Alice".into(),
            ..Default::default()
        }),
        address: buffa::MessageField::some(Address {
            street: "123 Main".into(),
            ..Default::default()
        }),
        status: buffa::EnumValue::Known(Status::ACTIVE),
        tree: buffa::MessageField::some(crate::nested::TreeNode {
            name: "root".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.person.name, "Alice");
    assert_eq!(decoded.address.street, "123 Main");
    assert_eq!(decoded.tree.name, "root");
}

#[test]
fn test_per_type_extern_path_round_trip() {
    // Per-type `extern_path` (issue #111): the build maps `.basic.Person` and
    // `.basic.Status` by exact FQN to crate::basic. If those mappings were
    // ignored (the old behavior) the generated references would not compile;
    // that this builds and round-trips proves per-type resolution works.
    use crate::cross_pertype::PerTypeComposite;
    // The field types below are crate::basic::Person / crate::basic::Status:
    // this would not compile if the per-type mappings resolved to a local
    // (non-existent) module instead.
    let msg = PerTypeComposite {
        person: buffa::MessageField::some(Person {
            id: 7,
            name: "Bob".into(),
            status: buffa::EnumValue::Known(Status::ACTIVE),
            ..Default::default()
        }),
        status: buffa::EnumValue::Known(Status::INACTIVE),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.person.id, 7);
    assert_eq!(decoded.person.name, "Bob");
    assert_eq!(
        decoded.person.status,
        buffa::EnumValue::Known(Status::ACTIVE)
    );
    assert_eq!(decoded.status, buffa::EnumValue::Known(Status::INACTIVE));
}

#[test]
fn test_cross_syntax_proto3_enum_in_proto2_is_open() {
    // Spec (protobuf.dev/programming-guides/enum): enum closedness
    // follows the DECLARING file's syntax. basic.Status is declared
    // in proto3, so it's open even when used from a proto2 message.
    // C++/Java/Kotlin are out-of-conformance here (they treat it as
    // closed); buffa follows the spec.
    //
    // Type-level assertion: field is Option<EnumValue<Status>>, not
    // Option<Status> (which would be closed-enum).
    use crate::cross_syntax::UsesProto3Enum;
    let _: Option<buffa::EnumValue<Status>> = UsesProto3Enum::default().status;
    let _: Vec<buffa::EnumValue<Status>> = UsesProto3Enum::default().statuses;

    // Runtime: unknown value (99) is preserved IN THE FIELD
    // (open-enum), not routed to unknown_fields (closed-enum).
    use buffa::encoding::{encode_varint, Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(1, WireType::Varint).encode(&mut wire);
    encode_varint(99, &mut wire);
    let msg = UsesProto3Enum::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(msg.status, Some(buffa::EnumValue::Unknown(99)));
    assert!(msg.__buffa_unknown_fields.is_empty());
}

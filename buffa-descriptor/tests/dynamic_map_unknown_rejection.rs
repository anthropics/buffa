#![cfg(feature = "reflect")]

//! `DecodeOptions::reject_unknown_fields()` reaches unknown fields inside the
//! map entries a `DynamicMessage` decodes, as it does for generated messages.

use std::sync::Arc;

use buffa::{DecodeError, DecodeOptions};
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet, MessageOptions,
};
use buffa_descriptor::reflect::DynamicMessage;
use buffa_descriptor::DescriptorPool;

fn field(
    name: &str,
    number: i32,
    label: Label,
    ty: Type,
    type_name: Option<&str>,
) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(label),
        r#type: Some(ty),
        type_name: type_name.map(Into::into),
        ..Default::default()
    }
}

fn entry(name: &str, value_ty: Type, value_type_name: Option<&str>) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.into()),
        field: vec![
            field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING, None),
            field("value", 2, Label::LABEL_OPTIONAL, value_ty, value_type_name),
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

/// proto2 `message Holder { map<string, int32> ints = 1; map<string, E> enums = 2; }`
/// with the closed `enum E { E0 = 0; }`.
fn pool() -> Arc<DescriptorPool> {
    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("dynamic-map-unknown.proto".into()),
            package: Some("dyn.map".into()),
            syntax: Some("proto2".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("E".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("E0".into()),
                    number: Some(0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            message_type: vec![DescriptorProto {
                name: Some("Holder".into()),
                field: vec![
                    field(
                        "ints",
                        1,
                        Label::LABEL_REPEATED,
                        Type::TYPE_MESSAGE,
                        Some(".dyn.map.Holder.IntsEntry"),
                    ),
                    field(
                        "enums",
                        2,
                        Label::LABEL_REPEATED,
                        Type::TYPE_MESSAGE,
                        Some(".dyn.map.Holder.EnumsEntry"),
                    ),
                ],
                nested_type: vec![
                    entry("IntsEntry", Type::TYPE_INT32, None),
                    entry("EnumsEntry", Type::TYPE_ENUM, Some(".dyn.map.E")),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    Arc::new(DescriptorPool::new(set).expect("valid pool"))
}

fn decode(wire: &[u8], opts: &DecodeOptions) -> Result<(), DecodeError> {
    let mut msg = DynamicMessage::new_by_name(pool(), "dyn.map.Holder").expect("known message");
    msg.merge_with_options(wire, opts)
}

const CASES: &[(&str, &[u8])] = &[
    // ints { key: "a" value: 1 <field 3 varint 9> }
    (
        "unknown varint in map entry",
        &[0x0a, 0x07, 0x0a, 0x01, b'a', 0x10, 0x01, 0x18, 0x09],
    ),
    // ints { value: 1 <field 3 empty group> }
    (
        "unknown group in map entry",
        &[0x0a, 0x04, 0x10, 0x01, 0x1b, 0x1c],
    ),
    // enums { value: 7 value: 0 } — the unknown 7 is overwritten by known 0
    (
        "overwritten unknown closed-enum map value",
        &[0x12, 0x04, 0x10, 0x07, 0x10, 0x00],
    ),
    // enums { value: 7 } — the entry ends with an unknown value
    ("unknown closed-enum map value", &[0x12, 0x02, 0x10, 0x07]),
    // enums { value: 0 <field 3 varint 9> }
    (
        "unknown varint in closed-enum map entry",
        &[0x12, 0x04, 0x10, 0x00, 0x18, 0x09],
    ),
];

#[test]
fn reject_unknown_fields_covers_dynamic_map_entries() {
    let reject = DecodeOptions::new().reject_unknown_fields();
    let accepted: Vec<_> = CASES
        .iter()
        .filter(|(_, wire)| decode(wire, &reject) != Err(DecodeError::UnknownFieldLimitExceeded))
        .map(|(name, _)| *name)
        .collect();
    assert!(accepted.is_empty(), "not rejected: {accepted:?}");
}

#[test]
fn default_options_accept_unknown_map_entry_fields() {
    let default = DecodeOptions::new();
    for (name, wire) in CASES {
        assert_eq!(decode(wire, &default), Ok(()), "{name}: expected success");
    }
}

#[test]
fn reject_unknown_fields_accepts_known_map_entries() {
    let reject = DecodeOptions::new().reject_unknown_fields();
    // ints { key: "a" value: 1 } enums { key: "b" value: 0 }
    let wire = [
        0x0a, 0x05, 0x0a, 0x01, b'a', 0x10, 0x01, 0x12, 0x05, 0x0a, 0x01, b'b', 0x10, 0x00,
    ];
    assert_eq!(decode(&wire, &reject), Ok(()));
}

#[test]
fn positive_allowance_counts_only_preserved_entries() {
    // enums { value: 7 value: 8 } ints { value: 1 <field 3 varint 9> }: one
    // preserved closed-enum entry, and a skipped entry field that costs nothing.
    let wire = [
        0x12, 0x04, 0x10, 0x07, 0x10, 0x08, 0x0a, 0x04, 0x10, 0x01, 0x18, 0x09,
    ];
    let opts = DecodeOptions::new().with_unknown_field_limit(1);
    let mut msg = DynamicMessage::new_by_name(pool(), "dyn.map.Holder").expect("known message");
    assert_eq!(msg.merge_with_options(&wire[..], &opts), Ok(()));
    assert_eq!(msg.unknown_fields().len(), 1);
}

#[test]
fn exhausted_positive_allowances_still_discard_entry_metadata_and_validate_groups() {
    // The first closed-enum entry spends the only preservation slot. The
    // following group is discarded entry metadata and must not spend another.
    let wire = [0x12, 2, 0x10, 7, 0x0a, 4, 0x10, 1, 0x1b, 0x1c];
    let positive = DecodeOptions::new().with_unknown_field_limit(1);
    assert_eq!(decode(&wire, &positive), Ok(()));
    assert_eq!(
        decode(&wire, &DecodeOptions::new().reject_unknown_fields()),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
    for (wire, error) in [
        (&[0x0a, 2, 0x1b, 0x24][..], DecodeError::InvalidEndGroup(4)),
        (&[0x0a, 1, 0x1b][..], DecodeError::UnexpectedEof),
    ] {
        assert_eq!(decode(wire, &positive), Err(error));
    }
    assert_eq!(
        decode(
            &[0x0a, 2, 0x1b, 0x1c],
            &positive.clone().with_recursion_limit(0)
        ),
        Err(DecodeError::RecursionLimitExceeded)
    );
    // A second preserved entry still exceeds the positive storage allowance.
    assert_eq!(
        decode(&[0x12, 2, 0x10, 7, 0x12, 2, 0x10, 8], &positive),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
}

#[test]
fn overwritten_unknown_enum_values_do_not_consume_a_positive_storage_allowance() {
    use buffa_descriptor::reflect::Value;
    // The first entry ends with a known value, so its earlier unknown value
    // costs no preservation slot. The second entry needs the one allowed slot.
    let wire = [0x12, 4, 0x10, 7, 0x10, 0, 0x12, 2, 0x10, 8];
    let mut msg = DynamicMessage::new_by_name(pool(), "dyn.map.Holder").unwrap();
    msg.merge_with_options(&wire, &DecodeOptions::new().with_unknown_field_limit(1))
        .unwrap();
    let Some(Value::Map(map)) = msg.field_by_number(2) else {
        panic!("expected enum map");
    };
    assert_eq!(map.get_str(""), Some(&Value::EnumNumber(0)));
    assert_eq!(msg.unknown_fields().len(), 1);
}

#[test]
fn rejecting_an_entry_preserves_prior_values_and_restores_map_order() {
    use buffa_descriptor::reflect::Value;
    let mut msg = DynamicMessage::new_by_name(pool(), "dyn.map.Holder").unwrap();
    msg.merge(&[0x0a, 5, 0x0a, 1, b'a', 0x10, 1]).unwrap();
    // Commit key z first, then attempt an overwrite of a containing an
    // unknown field. Failure must leave a unchanged and z discoverable.
    let wire = [
        0x0a, 5, 0x0a, 1, b'z', 0x10, 2, 0x0a, 7, 0x0a, 1, b'a', 0x10, 9, 0x18, 1,
    ];
    assert_eq!(
        msg.merge_with_options(&wire, &DecodeOptions::new().reject_unknown_fields()),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
    let Some(Value::Map(map)) = msg.field_by_number(1) else {
        panic!("expected int map");
    };
    assert_eq!(map.get_str("a"), Some(&Value::I32(1)));
    assert_eq!(map.get_str("z"), Some(&Value::I32(2)));
    assert_eq!(map.len(), 2);
    assert!(msg.unknown_fields().is_empty());
    assert_eq!(decode(&msg.encode_to_vec(), &DecodeOptions::new()), Ok(()));
}

#[test]
fn discarded_entry_metadata_does_not_consume_the_element_budget() {
    use buffa_descriptor::reflect::{MapKey, Value};
    let footprint = core::mem::size_of::<MapKey>()
        .checked_add(core::mem::size_of::<Value>())
        .unwrap();
    let exact = DecodeOptions::new().with_element_memory_limit(footprint);
    for wire in [&[0x0a, 0][..], &[0x0a, 2, 0x18, 1][..]] {
        assert_eq!(decode(wire, &exact), Ok(()));
        assert_eq!(
            decode(
                wire,
                &DecodeOptions::new().with_element_memory_limit(footprint.checked_sub(1).unwrap())
            ),
            Err(DecodeError::ElementMemoryLimitExceeded)
        );
    }
}

//! The same failure policy must work with generated unrolled and table codecs.

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::{DecodeError, DecodeOptions, Message};
use core::fmt::Debug;

fn varint(number: u32, value: u64) -> Vec<u8> {
    let mut wire = Vec::new();
    Tag::new(number, WireType::Varint).encode(&mut wire);
    encode_varint(value, &mut wire);
    wire
}

fn framed(number: u32, payload: &[u8]) -> Vec<u8> {
    let mut wire = Vec::new();
    Tag::new(number, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(u64::try_from(payload.len()).unwrap(), &mut wire);
    wire.extend_from_slice(payload);
    wire
}

fn failure_policy<M: Message + Debug>(
    initial: &M,
    wire: &[u8],
    options: DecodeOptions,
    expected: DecodeError,
) {
    let unchanged_wire = wire.to_vec();
    for clear in [false, true] {
        let options = options.clone().with_clear_on_merge_error(clear);
        for from_slice in [false, true] {
            let mut message = initial.clone();
            let result = if from_slice {
                options.merge_from_slice(&mut message, wire)
            } else {
                let mut input = wire;
                options.merge(&mut message, &mut input)
            };
            assert_eq!(result, Err(expected.clone()));
            if clear {
                assert_eq!(message, M::default());
            } else {
                assert_ne!(message, M::default());
                if expected == DecodeError::MessageTooLarge {
                    assert_eq!(message, *initial);
                }
            }
        }
    }
    assert_eq!(wire, unchanged_wire);
}

#[test]
fn generated_nested_and_budget_failures_clear_all_old_fields() {
    use buffa_test::lazyviews::Node;
    let initial = Node::decode_from_slice(&framed(1, b"old-name")).unwrap();
    let mut invalid_child = framed(1, b"new-name");
    invalid_child.push(0);
    let nested = framed(2, &invalid_child);
    failure_policy(
        &initial,
        &nested,
        DecodeOptions::new(),
        DecodeError::InvalidFieldNumber,
    );
    failure_policy(
        &initial,
        &nested,
        DecodeOptions::new().with_recursion_limit(0),
        DecodeError::RecursionLimitExceeded,
    );
    failure_policy(
        &initial,
        &framed(3, &[]),
        DecodeOptions::new().with_element_memory_limit(0),
        DecodeError::ElementMemoryLimitExceeded,
    );
    failure_policy(
        &initial,
        &varint(99, 7),
        DecodeOptions::new().with_unknown_field_limit(0),
        DecodeError::UnknownFieldLimitExceeded,
    );
    failure_policy(
        &initial,
        &nested,
        DecodeOptions::new().with_max_message_size(0),
        DecodeError::MessageTooLarge,
    );
}

#[cfg(has_table_codec)]
#[test]
fn generated_table_and_unrolled_oneofs_follow_the_same_failure_policy() {
    fn check<M: Message + Debug>() {
        let mut seed = framed(4, b"old-name");
        seed.extend(varint(99, 7));
        let initial = M::decode_from_slice(&seed).unwrap();
        let mut nested = varint(1, 42);
        nested.push(0);
        let wire = framed(2, &nested);
        failure_policy(
            &initial,
            &wire,
            DecodeOptions::new(),
            DecodeError::InvalidFieldNumber,
        );
        failure_policy(
            &initial,
            &wire,
            DecodeOptions::new().with_recursion_limit(0),
            DecodeError::RecursionLimitExceeded,
        );
        failure_policy(
            &initial,
            &wire,
            DecodeOptions::new().with_max_message_size(0),
            DecodeError::MessageTooLarge,
        );
        let enabled = DecodeOptions::new().clear_on_merge_error();
        let mut message = initial.clone();
        enabled.merge_from_slice(&mut message, &[]).unwrap();
        assert_eq!(message, initial);
        let valid = varint(1, 42);
        let mut expected = initial.clone();
        expected.merge_from_slice(&valid).unwrap();
        enabled.merge_from_slice(&mut message, &valid).unwrap();
        assert_eq!(message, expected);
        assert_eq!(
            enabled.merge_from_slice(&mut message, &[0]),
            Err(DecodeError::InvalidFieldNumber)
        );
        enabled.merge_from_slice(&mut message, &valid).unwrap();
        assert_eq!(message, M::decode_from_slice(&valid).unwrap());
    }
    check::<buffa_test::tc4u::Tree>();
    check::<buffa_test::tc4t::Tree>();
}

#[test]
fn message_set_failure_clears_old_and_partially_merged_extensions() {
    use buffa::ExtensionSet;
    use buffa_test::msgset::__buffa::ext::{MARKER_EXT, PAYLOAD_EXT};
    use buffa_test::msgset::{Container, Marker, Payload};

    let mut initial = Container::default();
    initial.set_extension(
        &PAYLOAD_EXT,
        Payload {
            name: Some("old-payload".into()),
            ..Default::default()
        },
    );
    initial.set_extension(
        &MARKER_EXT,
        Marker {
            flag: Some(true),
            ..Default::default()
        },
    );
    let mut replacement = Container::default();
    replacement.set_extension(
        &PAYLOAD_EXT,
        Payload {
            name: Some("new-payload".into()),
            ..Default::default()
        },
    );
    let mut wire = replacement.encode_to_vec();
    // An Item with an empty message but no required type_id. The preceding
    // Item must already have merged, so cleanup must remove both old and new
    // extension payloads stored in the MessageSet's unknown-field container.
    wire.extend_from_slice(&[0x0b, 0x1a, 0x00, 0x0c]);
    failure_policy(
        &initial,
        &wire,
        DecodeOptions::new(),
        DecodeError::InvalidMessageSet("missing type_id"),
    );

    let mut retained = initial;
    assert_eq!(
        DecodeOptions::new().merge_from_slice(&mut retained, &wire),
        Err(DecodeError::InvalidMessageSet("missing type_id"))
    );
    assert_eq!(
        retained.extension(&PAYLOAD_EXT).unwrap().name.as_deref(),
        Some("new-payload")
    );
    assert_eq!(retained.extension(&MARKER_EXT).unwrap().flag, Some(true));
    assert_eq!(
        DecodeOptions::new()
            .clear_on_merge_error()
            .merge_from_slice(&mut retained, &wire),
        Err(DecodeError::InvalidMessageSet("missing type_id"))
    );
    assert!(retained.extension(&PAYLOAD_EXT).is_none());
    assert!(retained.extension(&MARKER_EXT).is_none());
}

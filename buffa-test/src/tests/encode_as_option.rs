//! Generated encoders bind singular message fields through `as_option()`.
//!
//! An unset field must encode to nothing and add nothing to `encoded_len`; a
//! field set to the default message must still encode as an empty
//! length-delimited record (or an empty group). The assertions use hand-written
//! wire bytes so a change in either direction shows up as a byte diff.

use crate::basic::{Address, Person};
use crate::box_type::{Inner as BoxInner, Outer as BoxOuter};
use crate::inline_field::Outer as InlineOuter;
use crate::proto2::with_groups::MyGroup;
use crate::proto2::WithGroups;
use buffa::{Message, MessageField};

/// Assert that `encode_to_vec` matches `expected` and that the size pass
/// agrees with the bytes written.
#[track_caller]
fn assert_wire<M: Message>(msg: &M, expected: &[u8]) {
    let bytes = msg.encode_to_vec();
    assert_eq!(bytes, expected);
    assert_eq!(msg.encoded_len() as usize, expected.len());
}

#[test]
fn unset_singular_message_encodes_to_nothing() {
    assert_wire(&Person::default(), &[]);
    // A neighbouring scalar is unaffected by the unset message field.
    let msg = Person {
        id: 1,
        ..Default::default()
    };
    assert_wire(&msg, &[0x08, 0x01]);
}

#[test]
fn set_default_message_encodes_as_empty_record() {
    // Field 7, wire type 2, length 0.
    let msg = Person {
        address: MessageField::some(Address::default()),
        ..Default::default()
    };
    assert_wire(&msg, &[0x3a, 0x00]);
    assert!(msg.address.is_set());
}

#[test]
fn set_message_with_content_round_trips() {
    let msg = Person {
        address: MessageField::some(Address {
            zip_code: 5,
            ..Default::default()
        }),
        ..Default::default()
    };
    // Field 7, length 2, then Address.zip_code (field 3) = 5.
    assert_wire(&msg, &[0x3a, 0x02, 0x18, 0x05]);
    assert_eq!(super::round_trip(&msg), msg);
}

#[test]
fn oneof_message_variant_is_always_written() {
    use crate::basic::__buffa::oneof::person::Contact;
    // Field 15, wire type 2, length 0: the variant is always written.
    let msg = Person {
        contact: Some(Contact::HomeAddress(Box::default())),
        ..Default::default()
    };
    assert_wire(&msg, &[0x7a, 0x00]);
    assert_eq!(super::round_trip(&msg), msg);
}

#[test]
fn repeated_default_message_element_is_an_empty_record() {
    // Field 10, wire type 2; a default element is an empty record.
    let msg = Person {
        addresses: vec![
            Address::default(),
            Address {
                zip_code: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert_wire(&msg, &[0x52, 0x00, 0x52, 0x02, 0x18, 0x01]);
    assert_eq!(super::round_trip(&msg), msg);
}

#[test]
fn inline_pointer_repr() {
    // `inner` and `maybe` are `MessageField<_, Inline<_>>`.
    assert_wire(&InlineOuter::default(), &[]);
    let msg = InlineOuter {
        maybe: MessageField::some(Default::default()),
        ..Default::default()
    };
    assert_wire(&msg, &[0x12, 0x00]);
    assert_eq!(super::round_trip(&msg), msg);
}

#[test]
fn box_pointer_repr() {
    // `self_ref` is recursive, so it stays on `Box`.
    let msg = InlineOuter {
        self_ref: MessageField::some(InlineOuter::default()),
        ..Default::default()
    };
    assert_wire(&msg, &[0x22, 0x00]);
    assert_eq!(super::round_trip(&msg), msg);
}

#[test]
fn custom_pointer_repr() {
    assert_wire(&BoxOuter::default(), &[]);
    let msg = BoxOuter {
        inner: MessageField::some(BoxInner::default()),
        ..Default::default()
    };
    assert_wire(&msg, &[0x0a, 0x00]);
    assert_eq!(super::round_trip(&msg), msg);
}

#[test]
fn group_field_encodes_only_when_set() {
    assert_wire(&WithGroups::default(), &[]);
    // Field 1 start-group tag (0x0b) and end-group tag (0x0c) around no body.
    let msg = WithGroups {
        mygroup: MessageField::some(MyGroup::default()),
        ..Default::default()
    };
    assert_wire(&msg, &[0x0b, 0x0c]);
    assert_eq!(super::round_trip(&msg), msg);
}

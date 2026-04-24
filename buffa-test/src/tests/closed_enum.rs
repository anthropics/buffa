//! Closed-enum unknown-value routing to unknown_fields (proto spec).
//! Covers owned decoder (optional/repeated/oneof) and view decoder parity.

use super::varint_field;
use buffa::Message;

#[test]
fn test_closed_enum_optional_unknown_to_unknown_fields() {
    use crate::proto2::ClosedEnumContexts;
    // Field 1 (optional Priority) with value 99 (not in Priority).
    let wire = varint_field(1, 99);
    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(msg.opt, None, "field must report unset for unknown value");
    let unknowns: Vec<_> = msg.__buffa_unknown_fields.iter().collect();
    assert_eq!(unknowns.len(), 1, "unknown value must be in unknown_fields");
    assert_eq!(unknowns[0].number, 1);
    assert!(matches!(
        unknowns[0].data,
        buffa::UnknownFieldData::Varint(99)
    ));
    // Round-trip: re-encode must preserve the unknown value.
    let re = msg.encode_to_vec();
    assert_eq!(re, wire, "round-trip bytes must match");
}

#[test]
fn test_closed_enum_repeated_unknown_to_unknown_fields() {
    use crate::proto2::{ClosedEnumContexts, Priority};
    // Field 2 (repeated Priority, unpacked): [LOW=0, 99, HIGH=2, 42]
    let mut wire = Vec::new();
    wire.extend(varint_field(2, 0));
    wire.extend(varint_field(2, 99));
    wire.extend(varint_field(2, 2));
    wire.extend(varint_field(2, 42));
    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    // Known values stay in the list.
    assert_eq!(msg.rep, vec![Priority::LOW, Priority::HIGH]);
    // Unknown values go to unknown_fields (order preserved).
    let unknowns: Vec<_> = msg
        .__buffa_unknown_fields
        .iter()
        .filter(|u| u.number == 2)
        .collect();
    assert_eq!(unknowns.len(), 2);
    assert!(matches!(
        unknowns[0].data,
        buffa::UnknownFieldData::Varint(99)
    ));
    assert!(matches!(
        unknowns[1].data,
        buffa::UnknownFieldData::Varint(42)
    ));
    // Round-trip: bytes differ (known fields serialize before unknowns per
    // spec — "not in their original place"), but a second decode must yield
    // equivalent state.
    let re = msg.encode_to_vec();
    let msg2 = ClosedEnumContexts::decode(&mut re.as_slice()).unwrap();
    assert_eq!(msg2.rep, msg.rep);
    assert_eq!(
        msg2.__buffa_unknown_fields.iter().count(),
        msg.__buffa_unknown_fields.iter().count()
    );
}

#[test]
fn test_closed_enum_repeated_all_unknown() {
    // Edge case: ALL values in a repeated closed enum are unknown.
    // List ends up empty; all values in unknown_fields.
    use crate::proto2::ClosedEnumContexts;
    let mut wire = Vec::new();
    wire.extend(varint_field(2, 99));
    wire.extend(varint_field(2, 100));
    wire.extend(varint_field(2, 101));
    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    assert!(msg.rep.is_empty(), "no known values → empty list");
    let unknowns: Vec<_> = msg
        .__buffa_unknown_fields
        .iter()
        .filter(|u| u.number == 2)
        .collect();
    assert_eq!(unknowns.len(), 3);
    // Round-trip: re-encode has only the unknowns (no repeated-field bytes).
    let re = msg.encode_to_vec();
    let msg2 = ClosedEnumContexts::decode(&mut re.as_slice()).unwrap();
    assert!(msg2.rep.is_empty());
    assert_eq!(
        msg2.__buffa_unknown_fields
            .iter()
            .filter(|u| u.number == 2)
            .count(),
        3
    );
}

#[test]
fn test_closed_enum_repeated_packed_unknown_to_unknown_fields() {
    use crate::proto2::{ClosedEnumContexts, Priority};
    use buffa::encoding::{encode_varint, Tag, WireType};
    // Field 3 (repeated Priority, packed): [LOW=0, 99, HIGH=2]
    // Packed encoding: length-delimited, varints concatenated.
    let mut payload = Vec::new();
    encode_varint(0, &mut payload);
    encode_varint(99, &mut payload);
    encode_varint(2, &mut payload);
    let mut wire = Vec::new();
    Tag::new(3, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(payload.len() as u64, &mut wire);
    wire.extend_from_slice(&payload);

    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(msg.rep_packed, vec![Priority::LOW, Priority::HIGH]);
    let unknowns: Vec<_> = msg
        .__buffa_unknown_fields
        .iter()
        .filter(|u| u.number == 3)
        .collect();
    assert_eq!(unknowns.len(), 1);
    assert!(matches!(
        unknowns[0].data,
        buffa::UnknownFieldData::Varint(99)
    ));
}

#[test]
fn test_closed_enum_oneof_unknown_to_unknown_fields() {
    use crate::proto2::ClosedEnumContexts;
    // Field 4 (oneof Priority) with value 99.
    let wire = varint_field(4, 99);
    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    assert!(
        msg.choice.is_none(),
        "oneof must stay unset for unknown value"
    );
    let unknowns: Vec<_> = msg.__buffa_unknown_fields.iter().collect();
    assert_eq!(unknowns.len(), 1);
    assert_eq!(unknowns[0].number, 4);
    assert!(matches!(
        unknowns[0].data,
        buffa::UnknownFieldData::Varint(99)
    ));
    // Round-trip.
    let re = msg.encode_to_vec();
    assert_eq!(re, wire);
}

#[test]
fn test_closed_enum_known_value_not_routed_to_unknown() {
    // Sanity: known values should NOT go to unknown_fields.
    use crate::proto2::{ClosedEnumContexts, Priority};
    let wire = varint_field(1, 2); // HIGH = 2
    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(msg.opt, Some(Priority::HIGH));
    assert!(msg.__buffa_unknown_fields.is_empty());
}

#[test]
fn test_closed_enum_negative_unknown_value_sign_extension() {
    // Negative int32 values encode as sign-extended 10-byte varints.
    // Routing to unknown_fields via `__raw as u64` (i32 → u64 cast is
    // sign-extending in Rust) must preserve that on re-encode.
    use crate::proto2::ClosedEnumContexts;
    let wire = varint_field(1, (-999i32) as u64); // sign-extended
    assert_eq!(wire.len(), 11, "1-byte tag + 10-byte varint");
    let msg = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(msg.opt, None);
    let unknowns: Vec<_> = msg.__buffa_unknown_fields.iter().collect();
    assert_eq!(unknowns.len(), 1);
    // The stored u64 is the sign-extended value.
    assert!(matches!(
        unknowns[0].data,
        buffa::UnknownFieldData::Varint(v) if v == (-999i32) as u64
    ));
    // Round-trip: re-encoded bytes must match exactly (single field).
    let re = msg.encode_to_vec();
    assert_eq!(re, wire);
}

// ── View decoder: same semantics ──────────────────────────────────────
//
// Views must preserve the same round-trip guarantee: decode_view().
// to_owned_message().encode_to_vec() must equal the owned path.

#[test]
fn test_view_closed_enum_optional_unknown_to_unknown_fields() {
    use crate::proto2::__buffa::view::ClosedEnumContextsView;
    use buffa::MessageView;
    let wire = varint_field(1, 99);
    let view = ClosedEnumContextsView::decode_view(&wire).unwrap();
    assert_eq!(view.opt, None, "field must stay unset");
    assert!(!view.__buffa_unknown_fields.is_empty());
    // View → owned → encode must match original.
    let owned = view.to_owned_message();
    assert_eq!(owned.encode_to_vec(), wire);
}

#[test]
fn test_view_closed_enum_repeated_unpacked_unknown_preserved() {
    use crate::proto2::__buffa::view::ClosedEnumContextsView;
    use crate::proto2::Priority;
    use buffa::MessageView;
    // Field 2 (unpacked): [LOW=0, 99, HIGH=2]
    let mut wire = Vec::new();
    wire.extend(varint_field(2, 0));
    wire.extend(varint_field(2, 99));
    wire.extend(varint_field(2, 2));
    let view = ClosedEnumContextsView::decode_view(&wire).unwrap();
    // Known values in the list.
    let vals: Vec<_> = view.rep.iter().copied().collect();
    assert_eq!(vals, vec![Priority::LOW, Priority::HIGH]);
    // Unknown value span in unknown_fields.
    assert!(!view.__buffa_unknown_fields.is_empty());
    // View → owned → decode again: same state.
    let owned = view.to_owned_message();
    let re = owned.encode_to_vec();
    let view2 = ClosedEnumContextsView::decode_view(&re).unwrap();
    let vals2: Vec<_> = view2.rep.iter().copied().collect();
    assert_eq!(vals2, vals);
    assert!(!view2.__buffa_unknown_fields.is_empty());
}

#[test]
fn test_view_closed_enum_oneof_unknown_to_unknown_fields() {
    use crate::proto2::__buffa::view::ClosedEnumContextsView;
    use buffa::MessageView;
    let wire = varint_field(4, 99);
    let view = ClosedEnumContextsView::decode_view(&wire).unwrap();
    assert!(view.choice.is_none(), "oneof must stay unset");
    assert!(!view.__buffa_unknown_fields.is_empty());
    let owned = view.to_owned_message();
    assert_eq!(owned.encode_to_vec(), wire);
}

#[test]
fn test_view_closed_enum_known_not_routed() {
    use crate::proto2::__buffa::view::ClosedEnumContextsView;
    use crate::proto2::Priority;
    use buffa::MessageView;
    let wire = varint_field(1, 2); // HIGH = 2
    let view = ClosedEnumContextsView::decode_view(&wire).unwrap();
    assert_eq!(view.opt, Some(Priority::HIGH));
    assert!(view.__buffa_unknown_fields.is_empty());
}

#[test]
fn test_view_owned_parity_for_closed_enum_unknowns() {
    // Whatever the owned decoder produces, the view path must produce
    // byte-identical output after to_owned_message().encode_to_vec().
    use crate::proto2::__buffa::view::ClosedEnumContextsView;
    use crate::proto2::ClosedEnumContexts;
    use buffa::{Message, MessageView};
    let mut wire = Vec::new();
    wire.extend(varint_field(1, 99)); // optional unknown
    wire.extend(varint_field(2, 1)); // repeated known (MEDIUM)
    wire.extend(varint_field(2, 42)); // repeated unknown
    wire.extend(varint_field(4, 77)); // oneof unknown
    let owned_direct = ClosedEnumContexts::decode(&mut wire.as_slice()).unwrap();
    let via_view = ClosedEnumContextsView::decode_view(&wire)
        .unwrap()
        .to_owned_message();
    assert_eq!(
        owned_direct.encode_to_vec(),
        via_view.encode_to_vec(),
        "owned and view-to-owned decode paths must produce identical output"
    );
}

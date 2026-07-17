//! The decode-time budget on memory materialized by repeated length-delimited
//! elements (`DecodeOptions::with_element_memory_limit`).
//!
//! These fields cost far more decoded than encoded: an empty element is two
//! wire bytes and materializes `size_of::<Element>()` in the `Vec` it lands in.
//! The tests below pin the ratio for each element kind, that the budget is
//! shared across the whole decode rather than per field, and that packed
//! scalars are deliberately not charged.

use crate::lazyviews::{Holder, Payload};
use crate::proto2::Proto2Message;
use crate::proto3sem::RepeatedPacking;
use buffa::{DecodeError, DecodeOptions, Message, DEFAULT_ELEMENT_MEMORY_LIMIT};

/// `n` empty elements of the length-delimited field `field_number`: two bytes
/// each, a tag and a zero length.
fn empty_elements(field_number: u8, n: usize) -> Vec<u8> {
    let tag = (field_number << 3) | 2; // LengthDelimited
    let mut wire = Vec::with_capacity(n * 2);
    for _ in 0..n {
        wire.push(tag);
        wire.push(0x00);
    }
    wire
}

/// The amplification the budget exists to bound: an empty repeated message
/// element is 2 wire bytes and a whole `Payload` in memory.
#[test]
fn empty_repeated_message_elements_cost_far_more_decoded_than_encoded() {
    let n = 1000;
    let wire = empty_elements(2, n); // Holder.items
    let decoded = DecodeOptions::new()
        .with_element_memory_limit(usize::MAX)
        .decode_from_slice::<Holder>(&wire)
        .unwrap();

    assert_eq!(decoded.items.len(), n);
    let owned = n * core::mem::size_of::<Payload>();
    assert!(
        owned > wire.len() * 50,
        "expected a large wire->owned ratio, got {owned} bytes from {} wire bytes",
        wire.len()
    );
}

/// The default budget rejects a payload that would materialize more than it
/// allows, rather than letting the allocation happen and reporting nothing.
#[test]
fn default_budget_rejects_an_amplifying_payload() {
    // Enough empty elements to exceed the default several times over.
    let n = 4 * DEFAULT_ELEMENT_MEMORY_LIMIT / core::mem::size_of::<Payload>();
    let wire = empty_elements(2, n);
    assert_eq!(
        Holder::decode(&mut wire.as_slice()).unwrap_err(),
        DecodeError::ElementMemoryLimitExceeded
    );
}

/// The budget is an upper bound, not a target: an ordinary message decodes
/// untouched under the default.
#[test]
fn default_budget_admits_an_ordinary_message() {
    let msg = Holder {
        items: vec![Payload::default(); 32],
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    assert_eq!(
        Holder::decode(&mut wire.as_slice()).unwrap().items.len(),
        32
    );
}

/// Raising the budget is the escape hatch for trusted input that legitimately
/// decodes into more.
#[test]
fn a_raised_budget_admits_what_the_default_rejects() {
    let n = 4 * DEFAULT_ELEMENT_MEMORY_LIMIT / core::mem::size_of::<Payload>();
    let wire = empty_elements(2, n);
    let decoded = DecodeOptions::new()
        .with_element_memory_limit(usize::MAX)
        .decode_from_slice::<Holder>(&wire)
        .unwrap();
    assert_eq!(decoded.items.len(), n);
}

/// The budget is one pool for the whole decode, not an allowance per field:
/// elements charged by one field leave less for the next. A per-field budget
/// would bound nothing in aggregate.
#[test]
fn the_budget_is_shared_across_fields_not_per_field() {
    // Two elements of Payload fit in a budget of exactly two.
    let two = 2 * core::mem::size_of::<Payload>();
    let wire = empty_elements(2, 2);
    assert!(DecodeOptions::new()
        .with_element_memory_limit(two)
        .decode_from_slice::<Holder>(&wire)
        .is_ok());

    // A third does not, on the same budget.
    let wire = empty_elements(2, 3);
    assert_eq!(
        DecodeOptions::new()
            .with_element_memory_limit(two)
            .decode_from_slice::<Holder>(&wire)
            .unwrap_err(),
        DecodeError::ElementMemoryLimitExceeded
    );
}

/// Repeated `string` amplifies by the same route — an empty element is two wire
/// bytes and a `String` — so it is charged too.
#[test]
fn repeated_string_elements_are_charged() {
    let wire = empty_elements(3, 64); // Proto2Message.items
    let budget = 2 * core::mem::size_of::<String>();
    assert_eq!(
        DecodeOptions::new()
            .with_element_memory_limit(budget)
            .decode_from_slice::<Proto2Message>(&wire)
            .unwrap_err(),
        DecodeError::ElementMemoryLimitExceeded
    );
    // The same payload decodes when the budget covers it.
    assert!(DecodeOptions::new()
        .with_element_memory_limit(usize::MAX)
        .decode_from_slice::<Proto2Message>(&wire)
        .is_ok());
}

/// Packed scalars are deliberately not charged. Their worst case is a 1-byte
/// varint becoming a 4-byte `i32`, which is not an amplification vector, and
/// charging them would reject the columnar payloads that carry millions of
/// elements by design.
#[test]
fn packed_scalars_are_not_charged() {
    use buffa::encoding::encode_varint;

    // 4096 packed int32 elements, one wire byte each.
    let payload = vec![0x01u8; 4096];
    let mut wire = Vec::new();
    wire.push(0x0a); // RepeatedPacking.ints, LengthDelimited
    encode_varint(payload.len() as u64, &mut wire);
    wire.extend_from_slice(&payload);

    // A budget of zero would reject any charged element; packed decodes anyway.
    let decoded = DecodeOptions::new()
        .with_element_memory_limit(0)
        .decode_from_slice::<RepeatedPacking>(&wire)
        .expect("packed scalars are not charged against the element budget");
    assert_eq!(decoded.ints.len(), 4096);
}

/// A context built without a budget attached — what `DecodeContext::new` gives
/// older generated code — charges nothing, so the addition cannot change the
/// behaviour of code that has not been regenerated.
#[test]
fn a_context_without_a_budget_charges_nothing() {
    use buffa::{DecodeContext, RECURSION_LIMIT};
    use core::cell::Cell;

    let wire = empty_elements(2, 1000);
    let limit = Cell::new(buffa::DEFAULT_UNKNOWN_FIELD_LIMIT);
    let mut msg = Holder::default();
    msg.merge(
        &mut wire.as_slice(),
        DecodeContext::new(RECURSION_LIMIT, &limit),
    )
    .expect("no budget attached means no charge");
    assert_eq!(msg.items.len(), 1000);
}

/// A map entry amplifies exactly as a repeated element does: a message value
/// omitted from the wire still materializes a whole value in the map, and a few
/// bytes of key buy a distinct slot. `map<string, Message>` is the most common
/// length-delimited container in protobuf, so leaving it uncharged would make
/// the budget a formality.
#[test]
fn map_entries_are_charged() {
    use crate::basic::{Address, Inventory};
    use buffa::encoding::{encode_varint, Tag, WireType};

    // Inventory.locations = map<string, Address>, field 2. Each entry carries a
    // distinct key and no value field at all, so the value is a default Address.
    let n = 2000;
    let mut wire = Vec::new();
    for i in 0..n {
        let mut entry = Vec::new();
        Tag::new(1, WireType::LengthDelimited).encode(&mut entry);
        buffa::types::encode_string(&format!("{i:05}"), &mut entry);
        Tag::new(2, WireType::LengthDelimited).encode(&mut wire);
        encode_varint(entry.len() as u64, &mut wire);
        wire.extend_from_slice(&entry);
    }

    let per_entry = core::mem::size_of::<String>() + core::mem::size_of::<Address>();
    assert_eq!(
        DecodeOptions::new()
            .with_element_memory_limit(4 * per_entry)
            .decode_from_slice::<Inventory>(&wire)
            .unwrap_err(),
        DecodeError::ElementMemoryLimitExceeded,
        "a map budget of four entries must reject {n}"
    );

    // And the same payload decodes when the budget covers it.
    let decoded = DecodeOptions::new()
        .with_element_memory_limit(usize::MAX)
        .decode_from_slice::<Inventory>(&wire)
        .unwrap();
    assert_eq!(decoded.locations.len(), n);
}

/// The budget is one pool for the whole decode *tree*, not per message: a
/// nested message's elements draw from the same allowance as its parent's.
/// Without this, nesting would multiply the ceiling without limit.
#[test]
fn the_budget_is_shared_across_nesting() {
    use crate::lazyviews::{Holder, Payload};

    // A Holder whose items each carry their own repeated field: the inner
    // elements must draw from the same pool as the outer ones.
    let inner = Payload {
        pairs: vec![Default::default(); 8],
        ..Default::default()
    };
    let msg = Holder {
        items: vec![inner; 8],
        ..Default::default()
    };
    let wire = msg.encode_to_vec();

    // Enough for the 8 outer elements alone, but not for the 64 inner ones too.
    let outer_only = 8 * core::mem::size_of::<Payload>();
    assert_eq!(
        DecodeOptions::new()
            .with_element_memory_limit(outer_only)
            .decode_from_slice::<Holder>(&wire)
            .unwrap_err(),
        DecodeError::ElementMemoryLimitExceeded,
        "nested elements must draw from the parent's budget"
    );

    assert!(DecodeOptions::new()
        .with_element_memory_limit(usize::MAX)
        .decode_from_slice::<Holder>(&wire)
        .is_ok());
}

/// Views materialize a `Vec` of borrowed elements just as owned decoding
/// materializes a `Vec` of owned ones, so the view path is charged too.
#[test]
fn view_repeated_elements_are_charged() {
    use crate::lazyviews::__buffa::view::HolderView;

    let wire = empty_elements(2, 1000);
    assert_eq!(
        DecodeOptions::new()
            .with_element_memory_limit(64)
            .decode_view::<HolderView<'_>>(&wire)
            .unwrap_err(),
        DecodeError::ElementMemoryLimitExceeded
    );
    assert!(DecodeOptions::new()
        .with_element_memory_limit(usize::MAX)
        .decode_view::<HolderView<'_>>(&wire)
        .is_ok());
}

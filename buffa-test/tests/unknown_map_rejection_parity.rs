//! Strictness is a wire-occurrence policy shared by generated and reflective
//! decoders. Reuse generated descriptors so this comparison cannot silently
//! drift to a different map schema or enum openness.

use std::sync::Arc;

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::view::LazyMessageView;
use buffa::{DecodeError, DecodeOptions, MessageView};
use buffa_descriptor::reflect::DynamicMessage;
use buffa_descriptor::DescriptorPool;
use buffa_test::basic::{self, Inventory, InventoryView};
use buffa_test::scopedunknown::{self, Drop, DropLazyView, DropView, Keep, KeepLazyView, KeepView};

fn map_field(number: u32, entry: &[u8]) -> Vec<u8> {
    let mut wire = Vec::new();
    Tag::new(number, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(u64::try_from(entry.len()).unwrap(), &mut wire);
    wire.extend_from_slice(entry);
    wire
}

fn dynamic_result(
    pool: &Arc<DescriptorPool>,
    name: &str,
    wire: &[u8],
    options: &DecodeOptions,
) -> Result<(), DecodeError> {
    DynamicMessage::new_by_name(Arc::clone(pool), name)
        .unwrap()
        .merge_with_options(wire, options)
}

fn check_ordinary(wire: &[u8], options: &DecodeOptions, expected: Result<(), DecodeError>) {
    assert_eq!(
        options.decode_from_slice::<Inventory>(wire).map(|_| ()),
        expected
    );
    assert_eq!(
        options
            .decode_view::<InventoryView<'_>>(wire)
            .and_then(|view| view.to_owned_message())
            .map(|_| ()),
        expected
    );
    assert_eq!(
        dynamic_result(basic::descriptor_pool(), "basic.Inventory", wire, options),
        expected
    );
    for split in 0..=wire.len() {
        use buffa::bytes::Buf;
        let (head, tail) = wire.split_at(split);
        assert_eq!(
            options
                .decode::<Inventory>(&mut head.chain(tail))
                .map(|_| ()),
            expected,
            "split {split}"
        );
    }
}

fn check_closed(wire: &[u8], options: &DecodeOptions, expected: Result<(), DecodeError>) {
    // Keep and Drop have the same fields but opposite preservation settings.
    // Rejection must not depend on whether either decoder stores unknowns.
    assert_eq!(
        options.decode_from_slice::<Keep>(wire).map(|_| ()),
        expected
    );
    assert_eq!(
        options.decode_from_slice::<Drop>(wire).map(|_| ()),
        expected
    );
    assert_eq!(
        options
            .decode_view::<KeepView<'_>>(wire)
            .and_then(|view| view.to_owned_message())
            .map(|_| ()),
        expected
    );
    assert_eq!(
        options
            .decode_view::<DropView<'_>>(wire)
            .and_then(|view| view.to_owned_message())
            .map(|_| ()),
        expected
    );
    assert_eq!(
        options
            .decode_lazy_view::<KeepLazyView<'_>>(wire)
            .and_then(|view| view.to_owned_message())
            .map(|_| ()),
        expected
    );
    assert_eq!(
        options
            .decode_lazy_view::<DropLazyView<'_>>(wire)
            .and_then(|view| view.to_owned_message())
            .map(|_| ()),
        expected
    );
    assert_eq!(
        dynamic_result(
            scopedunknown::descriptor_pool(),
            "test.scopedunknown.Keep",
            wire,
            options
        ),
        expected
    );
}

#[test]
fn unknown_map_entry_fields_have_the_same_policy_in_each_decoder() {
    let reject = DecodeOptions::new().reject_unknown_fields();
    for extra in [
        vec![0x18, 9],                      // Varint.
        vec![0x19, 0, 0, 0, 0, 0, 0, 0, 0], // Fixed64.
        vec![0x1a, 2, 0xff, 0],             // Length-delimited.
        vec![0x1b, 0x20, 1, 0x1c],          // Group with one child.
        vec![0x1d, 0, 0, 0, 0],             // Fixed32.
    ] {
        let mut entry = vec![0x0a, 1, b'a', 0x10, 0];
        entry.extend_from_slice(&extra);
        let ordinary = map_field(1, &entry);
        let closed = map_field(4, &entry);
        check_ordinary(&ordinary, &DecodeOptions::new(), Ok(()));
        check_closed(&closed, &DecodeOptions::new(), Ok(()));
        check_ordinary(
            &ordinary,
            &reject,
            Err(DecodeError::UnknownFieldLimitExceeded),
        );
        check_closed(
            &closed,
            &reject,
            Err(DecodeError::UnknownFieldLimitExceeded),
        );
    }
}

#[test]
fn unknown_closed_enum_occurrences_cannot_be_hidden_by_a_later_known_value() {
    let reject = DecodeOptions::new().reject_unknown_fields();
    for values in [
        vec![0x10, 7, 0x10, 0],
        vec![0x10, 0, 0x10, 7],
        vec![0x10, 7, 0x10, 8, 0x10, 1],
    ] {
        check_closed(&map_field(4, &values), &DecodeOptions::new(), Ok(()));
        check_closed(
            &map_field(4, &values),
            &reject,
            Err(DecodeError::UnknownFieldLimitExceeded),
        );
        // The same numbers are valid values of Inventory's open Status enum.
        check_ordinary(&map_field(3, &values), &reject, Ok(()));
    }
}

#[test]
fn exhausting_a_positive_storage_budget_does_not_turn_discarding_into_rejection() {
    let mut wire = map_field(4, &[0x10, 7]);
    wire.extend(map_field(4, &[0x10, 0, 0x1b, 0x1c]));
    check_closed(
        &wire,
        &DecodeOptions::new().with_unknown_field_limit(1),
        Ok(()),
    );
    check_closed(
        &wire,
        &DecodeOptions::new().reject_unknown_fields(),
        Err(DecodeError::UnknownFieldLimitExceeded),
    );
}

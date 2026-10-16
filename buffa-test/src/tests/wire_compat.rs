//! Known numbers with incompatible wire types use the unknown-field path.
use super::{length_delimited_field as ld, varint_field as varint};
use buffa::{DecodeError, DecodeOptions, LazyMessageView, Message, MessageView};

fn fixed(number: u32, value: u32) -> Vec<u8> {
    use buffa::encoding::{Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(number, WireType::Fixed32).encode(&mut wire);
    wire.extend(value.to_le_bytes());
    wire
}

fn group(number: u32) -> Vec<u8> {
    use buffa::encoding::{Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(number, WireType::StartGroup).encode(&mut wire);
    wire.extend(varint(20, 1));
    Tag::new(number, WireType::EndGroup).encode(&mut wire);
    wire
}

macro_rules! decoder_tests {
    ($name:ident, $decode:expr) => {
        mod $name {
            use super::*;
            fn decode(bytes: &[u8], options: DecodeOptions) -> Result<Probe, DecodeError> {
                ($decode)(bytes, options)
            }

            #[test]
            fn incompatible_records_preserve_earlier_values() {
                let valid = [varint(1, 7), ld(4, &varint(1, 8)), varint(7, 9)].concat();
                let control = decode(&valid, DecodeOptions::new())
                    .unwrap()
                    .encode_to_vec();
                let mut failures = Vec::new();
                for wrong in [
                    ld(1, &[1, 2]),
                    fixed(1, 3),
                    group(1),
                    varint(2, 1),
                    varint(3, 1),
                    varint(4, 1),
                    fixed(5, 1),
                    fixed(6, 1),
                    ld(7, &[]),
                    varint(8, 1),
                    varint(9, 1),
                    varint(10, 1),
                    varint(11, 1),
                    ld(12, &[]),
                    fixed(13, 1),
                    varint(14, 1),
                ] {
                    let wire = [valid.clone(), wrong.clone()].concat();
                    let value = match decode(&wire, DecodeOptions::new()) {
                        Ok(value) => value,
                        Err(error) => {
                            failures.push((wire, error));
                            continue;
                        }
                    };
                    assert_eq!(value.n, Some(7));
                    assert_eq!(value.child.as_option().unwrap().n, Some(8));
                    assert_eq!(value.pick, Some(probe::Pick::Selected(9)));
                    let expected = if PRESERVE {
                        [control.clone(), wrong].concat()
                    } else {
                        control.clone()
                    };
                    assert_eq!(value.encode_to_vec(), expected);
                }
                assert!(
                    failures.is_empty(),
                    "incompatible records rejected: {failures:02x?}"
                );
            }

            #[test]
            fn inner_map_records_retain_strict_wire_checks() {
                use buffa::encoding::WireType::{Fixed32, LengthDelimited, StartGroup, Varint};
                // Include prior valid values: incompatible inner records are errors,
                // whereas the outer map field uses unknown fallback above.
                for (field, entry, number, expected, actual) in [
                    (
                        9,
                        [varint(1, 3), ld(1, b"bad"), varint(2, 4)].concat(),
                        1,
                        Varint,
                        LengthDelimited,
                    ),
                    (
                        9,
                        [varint(1, 3), varint(2, 4), fixed(2, 99)].concat(),
                        2,
                        Varint,
                        Fixed32,
                    ),
                    (9, ld(1, b"bad"), 1, Varint, LengthDelimited),
                    (
                        10,
                        [varint(1, 2), ld(2, &varint(1, 8)), varint(2, 99)].concat(),
                        2,
                        LengthDelimited,
                        Varint,
                    ),
                    (
                        10,
                        [varint(1, 2), group(2)].concat(),
                        2,
                        LengthDelimited,
                        StartGroup,
                    ),
                    (
                        15,
                        [varint(1, 1), varint(2, 1), ld(2, &[])].concat(),
                        2,
                        Varint,
                        LengthDelimited,
                    ),
                ] {
                    assert_eq!(
                        decode(&ld(field, &entry), DecodeOptions::new()),
                        Err(DecodeError::WireTypeMismatch {
                            field_number: number,
                            expected: expected as u8,
                            actual: actual as u8
                        })
                    );
                }
            }

            #[test]
            fn packed_and_unpacked_remain_compatible() {
                let wire = [
                    varint(5, 1),
                    ld(5, &[2, 3]),
                    ld(6, &[4, 5]),
                    varint(6, 6),
                    varint(13, 1),
                    ld(13, &[0, 1]),
                    fixed(14, 9),
                    ld(14, &10u32.to_le_bytes()),
                ]
                .concat();
                let value = decode(&wire, DecodeOptions::new()).unwrap();
                assert_eq!(value.nums, [1, 2, 3]);
                assert_eq!(value.unpacked, [4, 5, 6]);
                assert_eq!(value.states, [State::ONE, State::ZERO, State::ONE]);
                assert_eq!(value.fixeds, [9, 10]);
            }

            #[test]
            fn malformed_incompatible_records_still_fail() {
                for wire in [
                    vec![0],
                    vec![0x0e],
                    vec![0x0a, 3, 0],
                    vec![0x0d, 0],
                    vec![0x0b],
                    vec![0x0b, 0x14],
                    vec![0x0c],
                    vec![0x10, 0x80],
                    ld(9, &[0x0a, 2, 0]),
                ] {
                    assert!(decode(&wire, DecodeOptions::new()).is_err(), "{wire:02x?}");
                }
            }

            #[test]
            fn unknown_limits_and_group_depth_still_apply() {
                let wrong = ld(1, &[1, 2]);
                let outcome = decode(&wrong, DecodeOptions::new().with_unknown_field_limit(0));
                assert_eq!(outcome.is_err(), PRESERVE);
                assert!(decode(&group(1), DecodeOptions::new().with_recursion_limit(0)).is_err());
                // Compare the existing unknown-number route under the same memory budget.
                let opts = || DecodeOptions::new().with_element_memory_limit(0);
                assert_eq!(
                    decode(&wrong, opts()).is_ok(),
                    decode(&ld(99, &[1, 2]), opts()).is_ok()
                );
            }
        }
    };
}

macro_rules! fixture_tests {
    ($module:ident, $preserve:expr) => {
        mod $module {
            use super::*;
            use crate::$module::*;
            const PRESERVE: bool = $preserve;
            decoder_tests!(owned, |b, o: DecodeOptions| o.decode_from_slice::<Probe>(b));
            decoder_tests!(eager, |b, o: DecodeOptions| o
                .decode_view::<ProbeView<'_>>(b)?
                .to_owned_message());
            decoder_tests!(lazy, |b, o: DecodeOptions| o
                .decode_lazy_view::<ProbeLazyView<'_>>(b)?
                .to_owned_message());

            #[test]
            fn groups_keep_their_earlier_value() {
                let wire = [vec![0x0b, 0x10, 7, 0x0c], varint(1, 99)].concat();
                let owned = Groups::decode_from_slice(&wire).unwrap();
                assert_eq!(owned.item.as_option().unwrap().n, Some(7));
                assert_eq!(
                    GroupsView::decode_view(&wire)
                        .unwrap()
                        .to_owned_message()
                        .unwrap(),
                    owned
                );
                assert_eq!(
                    GroupsLazyView::decode_lazy(&wire)
                        .unwrap()
                        .to_owned_message()
                        .unwrap(),
                    owned
                );
            }
        }
    };
}
fixture_tests!(wcu, true);
fixture_tests!(wct, true);
fixture_tests!(wdu, false);
fixture_tests!(wdt, false);

#[test]
fn dynamic_reference_already_preserves_incompatible_top_level() {
    use buffa_descriptor::{DynamicMessage, Reflectable};
    let seed = crate::wcu::Probe::default().reflect().to_dynamic();
    let wire = [varint(1, 7), ld(1, &[1, 2]), varint(7, 8), varint(8, 9)].concat();
    let value = DynamicMessage::decode(seed.pool().clone(), seed.message_index(), &wire).unwrap();
    assert_eq!(
        value.encode_to_vec(),
        [varint(1, 7), varint(7, 8), ld(1, &[1, 2]), varint(8, 9)].concat()
    );
}

#[test]
fn dynamic_inner_map_records_retain_strict_wire_checks() {
    use buffa_descriptor::{DynamicMessage, Reflectable};
    let seed = crate::wcu::Probe::default().reflect().to_dynamic();
    let cases = [
        (
            vec![
                0x4a, 0x0e, 8, 3, 0x0a, 3, b'b', b'a', b'd', 0x10, 4, 0x15, 99, 0, 0, 0,
            ],
            vec![0x4a, 4, 8, 3, 0x10, 4],
        ),
        (
            vec![
                0x52, 0x0e, 8, 2, 0x12, 2, 8, 8, 0x10, 99, 0x12, 4, 0x12, 2, b'o', b'k',
            ],
            vec![0x52, 10, 8, 2, 0x12, 6, 8, 8, 0x12, 2, b'o', b'k'],
        ),
        (
            vec![0x4a, 5, 0x0a, 3, b'b', b'a', b'd'],
            vec![0x4a, 4, 8, 0, 0x10, 0],
        ),
        (
            vec![0x7a, 6, 8, 1, 0x10, 1, 0x12, 0],
            vec![0x7a, 4, 8, 1, 0x10, 1],
        ),
        (
            vec![0x52, 10, 8, 2, 0x12, 2, 8, 8, 0x13, 8, 7, 0x14],
            vec![0x52, 6, 8, 2, 0x12, 2, 8, 8],
        ),
        (
            vec![0x52, 4, 8, 2, 0x13, 0x14],
            vec![0x52, 4, 8, 2, 0x12, 0],
        ),
    ];
    // These C++-accepted inputs deliberately retain buffa's existing strict
    // inner-map contract from #387; changing it is outside #717's outer fix.
    for (wire, _cpp_encoding) in cases {
        assert!(
            matches!(
                DynamicMessage::decode(seed.pool().clone(), seed.message_index(), &wire),
                Err(DecodeError::WireTypeMismatch { .. })
            ),
            "{wire:02x?}"
        );
    }
    for malformed in [vec![0x52, 3, 8, 2, 0x13], vec![0x52, 4, 8, 2, 0x13, 0x1c]] {
        assert!(
            DynamicMessage::decode(seed.pool().clone(), seed.message_index(), &malformed).is_err()
        );
    }
}

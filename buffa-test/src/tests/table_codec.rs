//! `codec_strategy = Table` against the default unrolled codec.
//!
//! `build.rs` compiles each schema under two package names, one unrolled and
//! one with the table codec, so every message exists in both forms; `lib.rs`
//! lists the modules. The tests build the same value in both, and check that
//! the two codecs produce the same bytes and sizes, decode the same values,
//! and reject the same malformed input with the same error.

use core::fmt::Debug;

use buffa::{DecodeError, Message};

use super::{length_delimited_field, varint_field};

/// The error, or the value's re-encoding and `Debug` text.
type Outcome = Result<(Vec<u8>, String), DecodeError>;

/// The `Debug` text has the `Rpc` prefix of the `tcx` types removed, so that
/// they compare with the unprefixed ones.
fn outcome<M: Message + Debug>(wire: &[u8]) -> Outcome {
    M::decode_from_slice(wire).map(|m| (m.encode_to_vec(), format!("{m:?}").replace("Rpc", "")))
}

/// Decode `wire` with both codecs from a buffer of two chunks, split at every
/// offset, and require the outcomes to equal the one from a single slice.
#[track_caller]
fn assert_same_chained<U: Message + Debug, T: Message + Debug>(wire: &[u8]) {
    use buffa::bytes::Buf;
    let (unrolled, table) = (outcome::<U>(wire), outcome::<T>(wire));
    assert_eq!(unrolled, table);
    for split in 0..=wire.len() {
        let (head, tail) = wire.split_at(split);
        let u = U::decode(&mut head.chain(tail)).map(|m| (m.encode_to_vec(), format!("{m:?}")));
        let t = T::decode(&mut head.chain(tail))
            .map(|m| (m.encode_to_vec(), format!("{m:?}").replace("Rpc", "")));
        assert_eq!(u, unrolled, "unrolled, split at {split}");
        assert_eq!(t, table, "table, split at {split}");
    }
}

/// Decode `wire` with both codecs and require the same outcome.
///
/// One difference is allowed, between two rejections, and only when
/// `may_overrun`, which is for a message that has sub-messages. The unrolled
/// codec reads a sub-message's fields from the whole buffer, so a string or
/// group that runs past the end of its sub-message is read, and possibly
/// rejected for its content, before the overrun is noticed. The table decodes a
/// sub-message from a slice of exactly its length, so it reports
/// `UnexpectedEof` first.
#[track_caller]
fn assert_same_decode<U: Message + Debug, T: Message + Debug>(wire: &[u8], may_overrun: bool) {
    let (unrolled, table) = (outcome::<U>(wire), outcome::<T>(wire));
    let both_rejected_and_table_hit_the_end =
        may_overrun && unrolled.is_err() && table == Err(DecodeError::UnexpectedEof);
    assert!(
        unrolled == table || both_rejected_and_table_hit_the_end,
        "codecs disagree on {wire:02x?}: unrolled {unrolled:?}, table {table:?}"
    );
}

/// Both codecs encode `unrolled` and `table` (the same value in each form) to
/// the same bytes and length, and both decode those bytes to the same value.
#[track_caller]
fn assert_same_codec<U: Message + Debug + PartialEq, T: Message + Debug + PartialEq>(
    unrolled: &U,
    table: &T,
) -> Vec<u8> {
    let wire = unrolled.encode_to_vec();
    assert_eq!(wire, table.encode_to_vec(), "encoded bytes differ");
    assert_eq!(unrolled.encoded_len(), table.encoded_len());
    assert_eq!(wire.len(), table.encoded_len() as usize);
    let (mut framed_u, mut framed_t) = (Vec::new(), Vec::new());
    unrolled.encode_length_delimited(&mut framed_u);
    table.encode_length_delimited(&mut framed_t);
    assert_eq!(framed_u, framed_t, "length-delimited bytes differ");
    assert_eq!(
        unrolled.encode_to_bytes(),
        table.encode_to_bytes(),
        "encode_to_bytes differs"
    );
    let decoded_t = T::decode_from_slice(&wire).expect("table decodes its own output");
    assert_eq!(&decoded_t, table);
    assert_eq!(decoded_t.encode_to_vec(), wire);
    let decoded_u = U::decode_from_slice(&wire).expect("unrolled decodes its own output");
    assert_eq!(&decoded_u, unrolled);
    assert_eq!(format!("{decoded_u:?}"), format!("{decoded_t:?}"));
    wire
}

/// Both codecs give the same outcome on every prefix of `wire`, on every input
/// that differs from it in one byte, and on a run of pseudo-random inputs.
#[track_caller]
fn assert_same_on_corrupt_input<U: Message + Debug, T: Message + Debug>(
    wire: &[u8],
    may_overrun: bool,
) {
    for end in 0..=wire.len() {
        assert_same_decode::<U, T>(&wire[..end], may_overrun);
    }
    let mut flipped = wire.to_vec();
    for i in 0..wire.len() {
        let original = flipped[i];
        for xor in [0x01, 0x07, 0x80, 0xff] {
            flipped[i] = original ^ xor;
            assert_same_decode::<U, T>(&flipped, may_overrun);
        }
        flipped[i] = original;
    }
    // xorshift64, so the run is the same every time.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ wire.len() as u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let len = (next() % 48) as usize;
        let mut noise: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        assert_same_decode::<U, T>(&noise, may_overrun);
        // Noise after a valid prefix reaches the later fields.
        let keep = (next() as usize) % (wire.len() + 1);
        noise.splice(0..0, wire[..keep].iter().copied());
        assert_same_decode::<U, T>(&noise, may_overrun);
    }
}

/// Builds the same values in a generated module, `$m`: `tcu` or `tct`.
macro_rules! samples {
    ($name:ident, $m:ident) => {
        mod $name {
            use crate::$m::{Color, Inner, Nested, Optionals, Repeateds, Scalars, Sparse};
            use buffa::{EnumValue, MessageField};

            pub fn inner(id: i32, label: &str, tags: &[i32]) -> Inner {
                Inner {
                    id,
                    label: label.into(),
                    tags: tags.to_vec(),
                    ..Default::default()
                }
            }

            pub fn scalars() -> Scalars {
                Scalars {
                    i32: -7,
                    i64: i64::MIN,
                    u32: u32::MAX,
                    u64: u64::MAX,
                    s32: -300,
                    s64: i64::MAX,
                    b: true,
                    f32: 0xdead_beef,
                    f64: 0x0123_4567_89ab_cdef,
                    sf32: i32::MIN,
                    sf64: -5,
                    fl: 1.5,
                    db: -2.25,
                    s: "héllo".into(),
                    by: vec![0, 255, 7],
                    color: EnumValue::from(Color::GREEN),
                    ..Default::default()
                }
            }

            /// Explicit presence: every field set, several to a zero value.
            pub fn optionals() -> Optionals {
                Optionals {
                    i32: Some(0),
                    i64: Some(-1),
                    u32: Some(0),
                    u64: Some(1 << 40),
                    s32: Some(0),
                    s64: Some(-1),
                    b: Some(false),
                    f32: Some(0),
                    f64: Some(9),
                    sf32: Some(0),
                    sf64: Some(-9),
                    fl: Some(0.0),
                    db: Some(f64::MAX),
                    s: Some(String::new()),
                    by: Some(Vec::new()),
                    color: Some(EnumValue::from(Color::COLOR_UNSPECIFIED)),
                    ..Default::default()
                }
            }

            pub fn repeateds() -> Repeateds {
                Repeateds {
                    i32: vec![1, -1, 300, i32::MIN],
                    i64: vec![0, i64::MAX],
                    u32: vec![u32::MAX, 0],
                    u64: vec![u64::MAX],
                    s32: vec![-1, 1, -300],
                    s64: vec![i64::MIN, 5],
                    b: vec![true, false, true],
                    f32: vec![1, 2, 3],
                    f64: vec![u64::MAX, 0],
                    sf32: vec![-1, 1],
                    sf64: vec![i64::MIN],
                    fl: vec![0.5, -0.5, f32::INFINITY],
                    db: vec![1.0e300, -0.0],
                    s: vec!["a".into(), String::new(), "ccc".into()],
                    by: vec![vec![1, 2], Vec::new(), vec![255]],
                    color: vec![
                        EnumValue::from(Color::RED),
                        EnumValue::from(Color::GREEN),
                        EnumValue::from(9),
                    ],
                    unpacked_i32: vec![4, 5, 6],
                    unpacked_color: vec![EnumValue::from(Color::GREEN), EnumValue::from(-3)],
                    ..Default::default()
                }
            }

            pub fn nested() -> Nested {
                let leaf = Nested {
                    tail: 3,
                    ..Default::default()
                };
                let mid = Nested {
                    tail: 2,
                    next: MessageField::some(leaf.clone()),
                    kids: vec![leaf.clone(), Nested::default()],
                    ..Default::default()
                };
                Nested {
                    inner: MessageField::some(inner(1, "one", &[1, 2, 3])),
                    inners: vec![inner(2, "two", &[]), Inner::default(), inner(3, "", &[9])],
                    scalars: MessageField::some(scalars()),
                    optionals: MessageField::some(optionals()),
                    repeateds: MessageField::some(repeateds()),
                    next: MessageField::some(mid.clone()),
                    kids: vec![mid, leaf],
                    tail: 1,
                    ..Default::default()
                }
            }

            pub fn sparse() -> Sparse {
                Sparse {
                    a: 1,
                    mid: -2,
                    far: "far".into(),
                    max: 3,
                    ..Default::default()
                }
            }
        }
    };
}

samples!(u, tcu);
samples!(t, tct);

#[test]
fn scalars_agree() {
    let wire = assert_same_codec(&u::scalars(), &t::scalars());
    assert_same_on_corrupt_input::<crate::tcu::Scalars, crate::tct::Scalars>(&wire, false);
}

#[test]
fn explicit_presence_is_kept_for_zero_values() {
    let wire = assert_same_codec(&u::optionals(), &t::optionals());
    // Every field is on the wire, though many hold the type's zero value.
    let decoded = <crate::tct::Optionals as Message>::decode_from_slice(&wire).unwrap();
    assert_eq!(decoded.i32, Some(0));
    assert_eq!(decoded.s, Some(String::new()));
    assert_same_on_corrupt_input::<crate::tcu::Optionals, crate::tct::Optionals>(&wire, false);
}

#[test]
fn repeated_fields_agree() {
    let wire = assert_same_codec(&u::repeateds(), &t::repeateds());
    assert_same_on_corrupt_input::<crate::tcu::Repeateds, crate::tct::Repeateds>(&wire, false);
}

#[test]
fn nested_messages_agree() {
    let wire = assert_same_codec(&u::nested(), &t::nested());
    assert_same_on_corrupt_input::<crate::tcu::Nested, crate::tct::Nested>(&wire, true);
}

#[test]
fn sparse_field_numbers_agree() {
    let wire = assert_same_codec(&u::sparse(), &t::sparse());
    assert_same_on_corrupt_input::<crate::tcu::Sparse, crate::tct::Sparse>(&wire, false);
}

#[test]
fn empty_and_default_messages_encode_to_nothing() {
    assert!(crate::tct::Nested::default().encode_to_vec().is_empty());
    assert!(crate::tct::Empty::default().encode_to_vec().is_empty());
    assert_eq!(crate::tct::Nested::default().encoded_len(), 0);
}

#[test]
fn unpacked_and_packed_input_are_both_accepted() {
    // Field 1 of `Repeateds` is packed; a sender may write it unpacked, and
    // field 17 is unpacked but a sender may pack it.
    let mut wire = Vec::new();
    wire.extend(varint_field(1, 5));
    wire.extend(varint_field(1, 6));
    wire.extend(length_delimited_field(17, &[7, 8]));
    wire.extend(varint_field(17, 9));
    assert_same_decode::<crate::tcu::Repeateds, crate::tct::Repeateds>(&wire, false);
    let decoded = <crate::tct::Repeateds as Message>::decode_from_slice(&wire).unwrap();
    assert_eq!(decoded.i32, [5, 6]);
    assert_eq!(decoded.unpacked_i32, [7, 8, 9]);
}

#[test]
fn unknown_fields_are_kept() {
    let mut wire = u::scalars().encode_to_vec();
    wire.extend(varint_field(999, 42));
    wire.extend(length_delimited_field(1000, b"xyz"));
    // A group with an unknown field inside.
    wire.extend([0xf3, 0x3e]);
    wire.extend(varint_field(1, 5));
    wire.extend([0xf4, 0x3e]);
    assert_same_decode::<crate::tcu::Scalars, crate::tct::Scalars>(&wire, false);
    let decoded = <crate::tct::Scalars as Message>::decode_from_slice(&wire).unwrap();
    assert!(!decoded.__buffa_unknown_fields.is_empty());
    assert_eq!(decoded.encode_to_vec().len(), wire.len());
    // Unknown fields come back out, and clear() drops them.
    let mut cleared = decoded;
    cleared.clear();
    assert_eq!(cleared, crate::tct::Scalars::default());
}

#[test]
fn a_known_field_with_the_wrong_wire_type_is_rejected_alike() {
    // `i32` (field 1) as a length-delimited record, `s` (field 14) as a varint.
    let mut wire = length_delimited_field(1, b"ab");
    wire.extend(varint_field(14, 3));
    assert_same_decode::<crate::tcu::Scalars, crate::tct::Scalars>(&wire, false);
}

#[test]
fn merging_appends_repeated_and_merges_singular_message_fields() {
    let first = t::nested().encode_to_vec();
    let second = {
        let mut m = t::nested();
        m.tail = 99;
        m.inner = buffa::MessageField::some(t::inner(50, "", &[4]));
        m.encode_to_vec()
    };
    let mut merged_t = crate::tct::Nested::default();
    merged_t.merge_from_slice(&first).unwrap();
    merged_t.merge_from_slice(&second).unwrap();
    let mut merged_u = crate::tcu::Nested::default();
    merged_u.merge_from_slice(&first).unwrap();
    merged_u.merge_from_slice(&second).unwrap();
    assert_eq!(merged_t.encode_to_vec(), merged_u.encode_to_vec());
    assert_eq!(format!("{merged_t:?}"), format!("{merged_u:?}"));
    // The repeated field doubled, and the label of the merged `inner` survived
    // the second message's empty label.
    assert_eq!(merged_t.kids.len(), 4);
    assert_eq!(merged_t.inner.label, "one");
    assert_eq!(merged_t.inner.id, 50);
    assert_eq!(merged_t.tail, 99);
}

#[test]
fn recursion_depth_is_limited_like_the_unrolled_codec() {
    // `next` nested a thousand deep.
    let mut wire = Vec::new();
    for _ in 0..1000 {
        let mut outer = length_delimited_field(6, &wire);
        std::mem::swap(&mut outer, &mut wire);
    }
    assert_same_decode::<crate::tcu::Nested, crate::tct::Nested>(&wire, true);
    assert!(<crate::tct::Nested as Message>::decode_from_slice(&wire).is_err());
}

#[test]
fn invalid_utf8_is_rejected() {
    let wire = length_delimited_field(14, &[0xff, 0xfe]);
    assert_eq!(
        <crate::tct::Scalars as Message>::decode_from_slice(&wire),
        Err(DecodeError::InvalidUtf8)
    );
    assert_same_decode::<crate::tcu::Scalars, crate::tct::Scalars>(&wire, false);
}

#[test]
fn messages_the_table_cannot_handle_still_work() {
    // A oneof, a map, and the messages that hold them are unrolled, and a
    // table message may sit next to them.
    let with_oneof = crate::tct::WithOneof {
        choice: Some(crate::tct::with_oneof::Choice::B("x".into())),
        c: 4,
        ..Default::default()
    };
    let decoded =
        <crate::tct::WithOneof as Message>::decode_from_slice(&with_oneof.encode_to_vec()).unwrap();
    assert_eq!(decoded, with_oneof);

    let mixed = crate::tct::Mixed {
        inner: buffa::MessageField::some(t::inner(1, "a", &[1])),
        holds: buffa::MessageField::some(crate::tct::HoldsOneof {
            o: buffa::MessageField::some(with_oneof),
            x: 2,
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded =
        <crate::tct::Mixed as Message>::decode_from_slice(&mixed.encode_to_vec()).unwrap();
    assert_eq!(decoded, mixed);
}

// ---------------------------------------------------------------------------
// proto2: required fields, defaults, closed enums
// ---------------------------------------------------------------------------

macro_rules! req_samples {
    ($name:ident, $m:ident) => {
        mod $name {
            use crate::$m::{Color, Inner, Req};
            use buffa::MessageField;

            pub fn req() -> Req {
                Req {
                    a: 0,
                    s: String::new(),
                    d: Some(7),
                    unpacked: vec![1, 2, 3],
                    packed: vec![-1, 4, 5],
                    c: Some(Color::BLUE),
                    cs: vec![Color::RED, Color::GREEN],
                    packed_cs: vec![Color::BLUE, Color::RED],
                    i: MessageField::some(Inner {
                        id: Some(4),
                        ..Default::default()
                    }),
                    ri: MessageField::some(Inner::default()),
                    by: Some(b"ab".to_vec()),
                    st: Some("xyz".into()),
                    flag: Some(false),
                    ..Default::default()
                }
            }
        }
    };
}

req_samples!(req_u, tc2u);
req_samples!(req_t, tc2t);

#[test]
fn proto2_required_and_defaulted_fields_agree() {
    let wire = assert_same_codec(&req_u::req(), &req_t::req());
    // `a` and `s` are required, so they are written though they hold zero and "".
    assert!(wire.starts_with(&[0x08, 0x00, 0x12, 0x00]));
    assert_same_on_corrupt_input::<crate::tc2u::Req, crate::tc2t::Req>(&wire, true);
}

#[test]
fn a_message_that_omits_its_required_fields_encodes_them_anyway() {
    let wire = crate::tc2t::Req::default().encode_to_vec();
    assert_eq!(wire, crate::tc2u::Req::default().encode_to_vec());
}

#[test]
fn closed_enum_values_without_a_variant_go_to_unknown_fields() {
    // `c` (field 6), `cs` (7, unpacked) and `packed_cs` (9) with the values 99
    // and 2 (GREEN).
    let mut wire = varint_field(6, 99);
    wire.extend(varint_field(7, 99));
    wire.extend(varint_field(7, 2));
    wire.extend(length_delimited_field(9, &[99, 2]));
    assert_same_decode::<crate::tc2u::Req, crate::tc2t::Req>(&wire, false);
    let decoded = <crate::tc2t::Req as Message>::decode_from_slice(&wire).unwrap();
    assert_eq!(decoded.c, None);
    assert_eq!(decoded.cs, [crate::tc2t::Color::GREEN]);
    assert_eq!(decoded.packed_cs, [crate::tc2t::Color::GREEN]);
    assert!(!decoded.__buffa_unknown_fields.is_empty());
}

#[test]
fn groups_stay_unrolled() {
    let grouped = crate::tc2t::Grouped {
        a: Some(1),
        g: buffa::MessageField::some(crate::tc2t::grouped::G {
            x: Some(2),
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded =
        <crate::tc2t::Grouped as Message>::decode_from_slice(&grouped.encode_to_vec()).unwrap();
    assert_eq!(decoded, grouped);
}

#[test]
fn a_table_generated_with_a_type_prefix_boxed_fields_and_no_unknown_slot_agrees() {
    // `tcx` has `RpcNested` and friends, boxes its message fields, and drops
    // unknown fields, so it agrees with `tcu` on everything it keeps.
    let wire = u::nested().encode_to_vec();
    let decoded = <crate::tcx::RpcNested as Message>::decode_from_slice(&wire).unwrap();
    assert_eq!(decoded.encode_to_vec(), wire);
    assert_eq!(decoded.encoded_len() as usize, wire.len());

    let mut with_unknown = wire.clone();
    with_unknown.extend(varint_field(999, 1));
    let decoded = <crate::tcx::RpcNested as Message>::decode_from_slice(&with_unknown).unwrap();
    assert_eq!(
        decoded.encode_to_vec(),
        wire,
        "the unknown field is dropped"
    );
}

// ---------------------------------------------------------------------------
// More schema shapes: nested declarations, shadowed names, every kind, editions
// features, and a message wider than the dense array
// ---------------------------------------------------------------------------

macro_rules! shape_samples {
    ($name:ident, $m:ident, $m2:ident, $m3:ident, $wide:ident) => {
        mod $name {
            use crate::$m::outer::{Deep, Inner};
            use crate::$m::{
                Aux, Color, Entry, Kind, Mixed, Option as Opt, Outer, Table, Vec as Vc, WithOneof,
            };
            use buffa::{EnumValue, MessageField};

            pub fn outer() -> Outer {
                let deep = Deep {
                    y: -5,
                    back: MessageField::some(Inner {
                        x: 1,
                        s: "in".into(),
                        deeps: vec![Deep {
                            y: 2,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                Outer {
                    a: 1,
                    inner: MessageField::some(Inner {
                        x: 2,
                        s: "s".into(),
                        deeps: vec![deep.clone(), Deep::default()],
                        ..Default::default()
                    }),
                    deep: MessageField::some(deep.clone()),
                    inners: vec![
                        Inner::default(),
                        Inner {
                            x: 9,
                            ..Default::default()
                        },
                    ],
                    qualified: MessageField::some(deep),
                    color: EnumValue::from(Color::RED),
                    f63: 63,
                    f64: 64,
                    ..Default::default()
                }
            }

            pub fn aux() -> Aux {
                let option = |v| Opt {
                    v,
                    ..Default::default()
                };
                let entry = Entry {
                    k: MessageField::some(Kind {
                        t: MessageField::some(Table {
                            v: MessageField::some(Vc {
                                o: MessageField::some(option(1)),
                                os: vec![option(2), option(3)],
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    s: "e".into(),
                    ..Default::default()
                };
                Aux {
                    e: MessageField::some(entry.clone()),
                    es: vec![entry, Entry::default()],
                    ..Default::default()
                }
            }

            pub fn mixed() -> Mixed {
                mixed_with_map(true)
            }

            /// Without the map, whose entries a corrupted input can multiply and
            /// whose `Debug` order then differs between two `HashMap`s.
            pub fn mixed_without_map() -> Mixed {
                mixed_with_map(false)
            }

            fn mixed_with_map(with_map: bool) -> Mixed {
                let nested = || crate::$m::Nested {
                    inner: MessageField::some(inner()),
                    inners: vec![inner(), crate::$m::Inner::default()],
                    next: MessageField::some(crate::$m::Nested {
                        tail: 7,
                        ..Default::default()
                    }),
                    tail: 1,
                    ..Default::default()
                };
                Mixed {
                    inner: MessageField::some(inner()),
                    holds: MessageField::some(crate::$m::HoldsOneof {
                        o: MessageField::some(WithOneof {
                            choice: Some(crate::$m::with_oneof::Choice::I(Box::new(inner()))),
                            c: 4,
                            ..Default::default()
                        }),
                        x: 2,
                        m: if with_map {
                            MessageField::some(crate::$m::WithMap {
                                inners: [("k".to_string(), inner())].into_iter().collect(),
                                ..Default::default()
                            })
                        } else {
                            MessageField::none()
                        },
                        ..Default::default()
                    }),
                    nested: MessageField::some(nested()),
                    many: vec![nested(), crate::$m::Nested::default()],
                    ..Default::default()
                }
            }

            pub fn inner() -> crate::$m::Inner {
                crate::$m::Inner {
                    id: 3,
                    label: "l".into(),
                    tags: vec![1, 2],
                    ..Default::default()
                }
            }

            pub fn all_required() -> crate::$m2::AllRequired {
                use crate::$m2::Color as C2;
                crate::$m2::AllRequired {
                    i32: -1,
                    i64: -2,
                    u32: 3,
                    u64: 4,
                    s32: -5,
                    s64: -6,
                    b: true,
                    f32: 7,
                    f64: 8,
                    sf32: -9,
                    sf64: -10,
                    fl: 1.25,
                    db: -2.5,
                    s: "req".into(),
                    by: vec![9, 8],
                    color: C2::BLUE,
                    ..Default::default()
                }
            }

            pub fn all_repeated() -> crate::$m2::AllRepeated {
                use crate::$m2::Color as C2;
                crate::$m2::AllRepeated {
                    i32: vec![1, -2],
                    i64: vec![3, -4],
                    u32: vec![5, 6],
                    u64: vec![7],
                    s32: vec![-8, 9],
                    s64: vec![-10],
                    b: vec![true, false],
                    f32: vec![11, 12],
                    f64: vec![13],
                    sf32: vec![-14],
                    sf64: vec![-15, 16],
                    fl: vec![0.5, 1.5],
                    db: vec![2.5],
                    s: vec!["a".into(), "".into()],
                    by: vec![vec![1], vec![]],
                    color: vec![C2::RED, C2::GREEN],
                    ..Default::default()
                }
            }

            pub fn editions() -> crate::$m3::E {
                use crate::$m3::{Child, Closed, OpenE};
                crate::$m3::E {
                    explicit: Some(0),
                    implicit: 5,
                    required: 0,
                    closed: Some(Closed::C_B),
                    expanded: vec![1, 2, 3],
                    packed: vec![4, 5, 6],
                    closed_rep: vec![Closed::C_A, Closed::C_B],
                    open: Some(EnumValue::from(OpenE::O_B)),
                    s: Some("s".into()),
                    b: Some(vec![1]),
                    raw: Some("raw".into()),
                    child: MessageField::some(Child {
                        z: Some(1),
                        ..Default::default()
                    }),
                    children: vec![
                        Child::default(),
                        Child {
                            z: Some(2),
                            ..Default::default()
                        },
                    ],
                    implicit_s: "i".into(),
                    ..Default::default()
                }
            }

            pub fn wide() -> crate::$wide::Wide {
                use crate::$wide::Leaf;
                let leaf = |x| {
                    MessageField::some(Leaf {
                        x,
                        ..Default::default()
                    })
                };
                crate::$wide::Wide {
                    f1: leaf(1),
                    f128: leaf(128),
                    f256: leaf(256),
                    f280: leaf(280),
                    f281: 281,
                    f300: -300,
                    ..Default::default()
                }
            }
        }
    };
}

shape_samples!(shapes_u, tcu, tc2u, tc3u, wideu);
shape_samples!(shapes_t, tct, tc2t, tc3t, widet);

#[test]
fn nested_declarations_and_mutual_recursion_agree() {
    let wire = assert_same_codec(&shapes_u::outer(), &shapes_t::outer());
    assert_same_on_corrupt_input::<crate::tcu::Outer, crate::tct::Outer>(&wire, true);
}

#[test]
fn messages_named_like_what_generated_code_uses_agree() {
    let wire = assert_same_codec(&shapes_u::aux(), &shapes_t::aux());
    assert_same_on_corrupt_input::<crate::tcu::Aux, crate::tct::Aux>(&wire, true);
}

#[test]
fn a_table_message_inside_an_unrolled_tree_agrees() {
    // `Mixed.inner` is a table in `tct`, and `Mixed` and `HoldsOneof` are not.
    let wire = assert_same_codec(&shapes_u::mixed(), &shapes_t::mixed());
    assert_same_chained::<crate::tcu::Mixed, crate::tct::Mixed>(&wire);
    let wire = assert_same_codec(
        &shapes_u::mixed_without_map(),
        &shapes_t::mixed_without_map(),
    );
    assert_same_on_corrupt_input::<crate::tcu::Mixed, crate::tct::Mixed>(&wire, true);
}

#[test]
fn required_fields_of_every_type_agree() {
    let wire = assert_same_codec(&shapes_u::all_required(), &shapes_t::all_required());
    assert_same_on_corrupt_input::<crate::tc2u::AllRequired, crate::tc2t::AllRequired>(
        &wire, false,
    );
    // And the defaults are written too.
    let defaults = crate::tc2t::AllRequired::default().encode_to_vec();
    assert_eq!(
        defaults,
        crate::tc2u::AllRequired::default().encode_to_vec()
    );
    assert!(!defaults.is_empty());
}

#[test]
fn unpacked_repeated_fields_of_every_type_agree() {
    let wire = assert_same_codec(&shapes_u::all_repeated(), &shapes_t::all_repeated());
    assert_same_on_corrupt_input::<crate::tc2u::AllRepeated, crate::tc2t::AllRepeated>(
        &wire, false,
    );
}

#[test]
fn editions_features_agree() {
    let wire = assert_same_codec(&shapes_u::editions(), &shapes_t::editions());
    assert_same_on_corrupt_input::<crate::tc3u::E, crate::tc3t::E>(&wire, true);
    // A closed enum value with no variant is unknown, as in unrolled code.
    let mut unknown = varint_field(4, 99);
    unknown.extend(varint_field(5, 99));
    assert_same_decode::<crate::tc3u::E, crate::tc3t::E>(&unknown, false);
}

#[test]
fn a_message_with_more_than_255_fields_agrees() {
    let wire = assert_same_codec(&shapes_u::wide(), &shapes_t::wide());
    assert_same_on_corrupt_input::<crate::wideu::Wide, crate::widet::Wide>(&wire, true);
}

#[test]
fn nested_messages_decode_the_same_from_a_buffer_of_two_chunks() {
    assert_same_chained::<crate::tcu::Nested, crate::tct::Nested>(&u::nested().encode_to_vec());
    let outer = shapes_u::outer().encode_to_vec();
    assert_same_chained::<crate::tcu::Outer, crate::tct::Outer>(&outer);
    let editions = shapes_u::editions().encode_to_vec();
    assert_same_chained::<crate::tc3u::E, crate::tc3t::E>(&editions);
}

macro_rules! edge_widths {
    ($($name:ident: $ty:ident, $last:ident;)*) => {$(
        #[test]
        fn $name() {
            let unrolled = crate::wideu::$ty { f1: 1, $last: 9, ..Default::default() };
            let table = crate::widet::$ty { f1: 1, $last: 9, ..Default::default() };
            let wire = assert_same_codec(&unrolled, &table);
            assert_same_on_corrupt_input::<crate::wideu::$ty, crate::widet::$ty>(&wire, false);
        }
    )*};
}

edge_widths! {
    a_message_with_254_fields_agrees: W254, f254;
    a_message_with_255_fields_agrees: W255, f255;
    a_message_with_256_fields_agrees: W256, f256;
}

#[test]
fn fields_named_like_rust_keywords_agree() {
    use buffa::MessageField;
    macro_rules! keywords {
        ($m:ident) => {
            crate::$m::Keywords {
                r#type: 1,
                r#match: "m".into(),
                r#fn: vec![2, 3],
                r#ref: MessageField::some(crate::$m::Inner {
                    id: 4,
                    ..Default::default()
                }),
                ..Default::default()
            }
        };
    }
    let wire = assert_same_codec(&keywords!(tcu), &keywords!(tct));
    assert_same_on_corrupt_input::<crate::tcu::Keywords, crate::tct::Keywords>(&wire, true);
}

#[test]
fn negative_zero_and_nan_are_written_by_both_codecs() {
    // Implicit-presence floats are skipped when their bits are zero, and
    // `-0.0` and NaN are not zero bits.
    macro_rules! floats {
        ($m:ident) => {
            crate::$m::Scalars {
                fl: -0.0,
                db: f64::NAN,
                ..Default::default()
            }
        };
    }
    let (unrolled, table) = (floats!(tcu), floats!(tct));
    assert_eq!(unrolled.encode_to_vec(), table.encode_to_vec());
    assert!(!table.encode_to_vec().is_empty());
    let zero = crate::tct::Scalars::default();
    assert!(zero.encode_to_vec().is_empty());
}

#[test]
fn clear_restores_proto2_defaults_like_unrolled_code() {
    let wire = {
        let mut wire = varint_field(1, 5);
        wire.extend(length_delimited_field(2, b"s"));
        wire.extend(varint_field(3, 9));
        wire.extend(length_delimited_field(10, &varint_field(1, 1)));
        wire
    };
    let mut unrolled = crate::tc2u::Req::decode_from_slice(&wire).unwrap();
    let mut table = crate::tc2t::Req::decode_from_slice(&wire).unwrap();
    unrolled.clear();
    table.clear();
    assert_eq!(format!("{unrolled:?}"), format!("{table:?}"));
    assert_eq!(unrolled.encode_to_vec(), table.encode_to_vec());
}

/// Referencing a message's table is a compile error when it fell back to
/// unrolled code, so these pin which messages the table really covers.
#[test]
fn the_messages_the_table_can_handle_use_it() {
    fn is_table<M: Message>(_: &'static buffa::table::Table<M>) {}
    macro_rules! tables {
        ($($path:path),* $(,)?) => { $( is_table(&$path); )* };
    }
    tables!(
        crate::tct::__BUFFA_TABLE_Scalars,
        crate::tct::__BUFFA_TABLE_Optionals,
        crate::tct::__BUFFA_TABLE_Repeateds,
        crate::tct::__BUFFA_TABLE_Nested,
        crate::tct::__BUFFA_TABLE_Sparse,
        crate::tct::__BUFFA_TABLE_Empty,
        crate::tct::__BUFFA_TABLE_Inner,
        crate::tct::__BUFFA_TABLE_Outer,
        crate::tct::outer::__BUFFA_TABLE_Inner,
        crate::tct::outer::__BUFFA_TABLE_Deep,
        crate::tct::__BUFFA_TABLE_Option,
        crate::tct::__BUFFA_TABLE_Vec,
        crate::tct::__BUFFA_TABLE_Table,
        crate::tct::__BUFFA_TABLE_Kind,
        crate::tct::__BUFFA_TABLE_Entry,
        crate::tct::__BUFFA_TABLE_Aux,
        crate::tct::__BUFFA_TABLE_Keywords,
        crate::tc2t::__BUFFA_TABLE_Req,
        crate::tc2t::__BUFFA_TABLE_AllRequired,
        crate::tc2t::__BUFFA_TABLE_AllRepeated,
        crate::tc3t::__BUFFA_TABLE_E,
        crate::tc3t::__BUFFA_TABLE_Child,
        crate::widet::__BUFFA_TABLE_Wide,
        crate::widet::__BUFFA_TABLE_W254,
        crate::widet::__BUFFA_TABLE_W255,
        crate::widet::__BUFFA_TABLE_W256,
        crate::tcx::__BUFFA_TABLE_RpcNested,
        crate::xat::__BUFFA_TABLE_Leaf,
        crate::xat::__BUFFA_TABLE_Wrap,
        crate::xbt::__BUFFA_TABLE_Holder,
        crate::xbt::holder::__BUFFA_TABLE_Sub,
        crate::xti::xati::__BUFFA_TABLE_Leaf,
        crate::xti::xati::__BUFFA_TABLE_Wrap,
        crate::xti::xbti::__BUFFA_TABLE_Holder,
        crate::xti::xbti::holder::__BUFFA_TABLE_Sub,
    );
}

#[test]
fn messages_held_across_packages_agree_in_every_layout() {
    use buffa::MessageField;
    macro_rules! sample {
        ($xa:ident, $xb:ident) => {{
            let leaf = |x| $xa::Leaf {
                x,
                s: "s".into(),
                ..Default::default()
            };
            $xb::Holder {
                leaf: MessageField::some(leaf(1)),
                leaves: vec![leaf(2), leaf(3)],
                wrap: MessageField::some($xa::Wrap {
                    leaf: MessageField::some(leaf(4)),
                    leaves: vec![leaf(5)],
                    ..Default::default()
                }),
                sub: MessageField::some($xb::holder::Sub {
                    l: MessageField::some(leaf(6)),
                    ..Default::default()
                }),
                ..Default::default()
            }
        }};
    }
    use crate::xti::{xati, xbti};
    use crate::{xat, xau, xbt, xbu};
    let unrolled = sample!(xau, xbu);
    let table = sample!(xat, xbt);
    let idiomatic = sample!(xati, xbti);
    let wire = assert_same_codec(&unrolled, &table);
    assert_eq!(idiomatic.encode_to_vec(), wire);
    assert_eq!(
        xbti::Holder::decode_from_slice(&wire)
            .unwrap()
            .encode_to_vec(),
        wire
    );
}

//! Map fields under the table codec, against the unrolled codec. The packages
//! (`tcbu`, `tcbt`, `tcmu`, `tcmt`) are described in
//! protos/table_codec_maps.proto.

use core::fmt::Debug;

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::{DecodeError, DecodeOptions, Message};

use super::{
    assert_same_chained, assert_same_codec, assert_same_decode, assert_same_on_corrupt_input,
    length_delimited_field, varint_field,
};

/// The same values in the generated module `$m`.
macro_rules! samples {
    ($name:ident, $m:ident) => {
        // The `HashMap` variants use only some of the samples.
        #[allow(dead_code)]
        mod $name {
            use crate::$m::{Color, Holder, Item, Keys, Nesting, Sparse, Values};
            use buffa::{EnumValue, MessageField};

            pub fn item(id: i32, label: &str) -> Item {
                Item {
                    id,
                    label: label.into(),
                    tags: vec![id, -id],
                    ..Default::default()
                }
            }

            pub fn keys() -> Keys {
                Keys {
                    i32: [(i32::MIN, 1), (0, 0), (i32::MAX, -1)]
                        .into_iter()
                        .collect(),
                    i64: [(i64::MIN, 1), (0, 2), (i64::MAX, 3)].into_iter().collect(),
                    u32: [(0, 1), (u32::MAX, 2)].into_iter().collect(),
                    u64: [(0, 1), (u64::MAX, 2)].into_iter().collect(),
                    s32: [(i32::MIN, 1), (-1, 2), (1, 3)].into_iter().collect(),
                    s64: [(i64::MIN, 1), (-1, 2), (1, 3)].into_iter().collect(),
                    b: [(false, 1), (true, 2)].into_iter().collect(),
                    f32: [(0, 1), (u32::MAX, 2)].into_iter().collect(),
                    f64: [(0, 1), (u64::MAX, 2)].into_iter().collect(),
                    sf32: [(i32::MIN, 1), (-1, 2), (i32::MAX, 3)]
                        .into_iter()
                        .collect(),
                    sf64: [(i64::MIN, 1), (0, 2), (i64::MAX, 3)].into_iter().collect(),
                    s: [
                        (String::new(), 1),
                        ("a".to_string(), 2),
                        ("b".repeat(200), 3),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                }
            }

            pub fn values() -> Values {
                let key = |n: &str| n.to_string();
                Values {
                    i32: [(key("a"), i32::MIN), (key("b"), 0), (key("c"), 7)]
                        .into_iter()
                        .collect(),
                    i64: [(key("a"), i64::MIN), (key("b"), i64::MAX)]
                        .into_iter()
                        .collect(),
                    u32: [(key("a"), 0), (key("b"), u32::MAX)].into_iter().collect(),
                    u64: [(key("a"), 0), (key("b"), u64::MAX)].into_iter().collect(),
                    s32: [(key("a"), i32::MIN), (key("b"), -1), (key("c"), 1)]
                        .into_iter()
                        .collect(),
                    s64: [(key("a"), i64::MIN), (key("b"), 1)].into_iter().collect(),
                    b: [(key("f"), false), (key("t"), true)].into_iter().collect(),
                    f32: [(key("a"), 0), (key("b"), u32::MAX)].into_iter().collect(),
                    f64: [(key("a"), 0), (key("b"), u64::MAX)].into_iter().collect(),
                    sf32: [(key("a"), i32::MIN), (key("b"), i32::MAX)]
                        .into_iter()
                        .collect(),
                    sf64: [(key("a"), i64::MIN), (key("b"), i64::MAX)]
                        .into_iter()
                        .collect(),
                    fl: [(key("a"), 0.0), (key("neg"), -0.0), (key("b"), 1.5)]
                        .into_iter()
                        .collect(),
                    db: [(key("a"), f64::MAX), (key("b"), f64::MIN_POSITIVE)]
                        .into_iter()
                        .collect(),
                    s: [
                        (key(""), key("")),
                        (key("k"), key("v")),
                        (key("wide"), key("h\u{e9}llo \u{1f600}")),
                    ]
                    .into_iter()
                    .collect(),
                    by: [(key("a"), vec![]), (key("b"), vec![0, 255, 7])]
                        .into_iter()
                        .collect(),
                    color: [
                        (key("red"), EnumValue::from(Color::RED)),
                        (key("zero"), EnumValue::from(Color::COLOR_UNSPECIFIED)),
                        (key("other"), EnumValue::from(99)),
                    ]
                    .into_iter()
                    .collect(),
                    item: [
                        (
                            key("a"),
                            Item {
                                child: MessageField::some(item(9, "nine")),
                                ..item(1, "one")
                            },
                        ),
                        (key("b"), Item::default()),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                }
            }

            pub fn nesting() -> Nesting {
                let leaf = |tail| Nesting {
                    tail,
                    ..Default::default()
                };
                let mid = Nesting {
                    children: [("leaf".to_string(), leaf(1)), ("empty".into(), leaf(0))]
                        .into_iter()
                        .collect(),
                    items: [(2, item(2, "two"))].into_iter().collect(),
                    ..Default::default()
                };
                Nesting {
                    children: [("mid".to_string(), mid), ("other".into(), leaf(5))]
                        .into_iter()
                        .collect(),
                    items: [
                        (-1, item(-1, "m")),
                        (0, Item::default()),
                        (70_000, item(3, "")),
                    ]
                    .into_iter()
                    .collect(),
                    item: MessageField::some(item(4, "four")),
                    many: vec![item(5, "five"), Item::default()],
                    tail: 9,
                    ..Default::default()
                }
            }

            pub fn sparse() -> Sparse {
                Sparse {
                    a: [("a".to_string(), "b".to_string())].into_iter().collect(),
                    x: 3,
                    far: [(1, "one".to_string()), (-1, String::new())]
                        .into_iter()
                        .collect(),
                    last: [("z".to_string(), "y".to_string())].into_iter().collect(),
                    ..Default::default()
                }
            }

            pub fn holder() -> Holder {
                Holder {
                    values: MessageField::some(values()),
                    keys: MessageField::some(keys()),
                    nesting: MessageField::some(nesting()),
                    sparse: vec![sparse(), Sparse::default()],
                    by_name: [
                        (
                            "self".to_string(),
                            Holder {
                                keys: MessageField::some(keys()),
                                ..Default::default()
                            },
                        ),
                        ("empty".into(), Holder::default()),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                }
            }
        }
    };
}

samples!(bu, tcbu);
samples!(bt, tcbt);
samples!(hu, tcmu);
samples!(ht, tcmt);

// ---------------------------------------------------------------------------
// Both codecs, on values
// ---------------------------------------------------------------------------

/// Both codecs give the same bytes for the same value, decode each other's
/// bytes, and give the same outcome on the wire cut, chained and corrupted.
macro_rules! agree {
    ($test:ident, $ty:ident, $sample:ident) => {
        #[test]
        fn $test() {
            let wire = assert_same_codec(&bu::$sample(), &bt::$sample());
            assert!(!wire.is_empty());
            assert_same_chained::<crate::tcbu::$ty, crate::tcbt::$ty>(&wire);
            assert_same_on_corrupt_input::<crate::tcbu::$ty, crate::tcbt::$ty>(&wire, true);
        }
    };
}

agree!(every_key_type_agrees, Keys, keys);
agree!(every_value_type_agrees, Values, values);
agree!(maps_of_messages_and_nested_messages_agree, Nesting, nesting);
agree!(field_numbers_outside_the_dense_lookup_agree, Sparse, sparse);
agree!(a_message_holding_maps_agrees, Holder, holder);

#[test]
fn empty_maps_encode_to_nothing() {
    assert!(crate::tcbt::Values::default().encode_to_vec().is_empty());
    assert!(crate::tcbt::Holder::default().encode_to_vec().is_empty());
    assert_eq!(crate::tcbt::Keys::default().encoded_len(), 0);
}

#[test]
fn clear_empties_every_map() {
    let mut table = bt::values();
    let mut unrolled = bu::values();
    table.clear();
    unrolled.clear();
    assert_eq!(table, crate::tcbt::Values::default());
    assert_eq!(unrolled, crate::tcbu::Values::default());
    assert!(table.encode_to_vec().is_empty());
}

#[test]
fn unknown_fields_next_to_maps_are_kept() {
    let mut wire = bu::values().encode_to_vec();
    wire.extend(varint_field(9999, 7));
    wire.extend(length_delimited_field(9998, b"x"));
    let table = crate::tcbt::Values::decode_from_slice(&wire).unwrap();
    let unrolled = crate::tcbu::Values::decode_from_slice(&wire).unwrap();
    assert_eq!(format!("{table:?}"), format!("{unrolled:?}"));
    assert_eq!(table.encode_to_vec(), unrolled.encode_to_vec());
}

/// The entries of a map, sorted, as text.
fn sorted<K: Ord + Debug, V: Debug>(map: &std::collections::HashMap<K, V, impl Sized>) -> String {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    format!("{entries:?}")
}

/// The maps of a message with `HashMap` fields, each as sorted text.
macro_rules! canonical {
    ($msg:expr, [$($field:ident),*]) => {
        vec![$(sorted(&$msg.$field)),*]
    };
}

#[test]
fn hash_maps_agree_through_their_sorted_entries() {
    let (unrolled, table) = (hu::keys(), ht::keys());
    assert_eq!(unrolled.encoded_len(), table.encoded_len());
    let from_unrolled = crate::tcmt::Keys::decode_from_slice(&unrolled.encode_to_vec()).unwrap();
    let from_table = crate::tcmu::Keys::decode_from_slice(&table.encode_to_vec()).unwrap();
    let fields = |k: &crate::tcmt::Keys| {
        canonical!(
            k,
            [i32, i64, u32, u64, s32, s64, b, f32, f64, sf32, sf64, s]
        )
    };
    assert_eq!(fields(&from_unrolled), fields(&table));
    assert_eq!(from_table, unrolled);

    let (unrolled, table) = (hu::values(), ht::values());
    assert_eq!(unrolled.encoded_len(), table.encoded_len());
    let from_unrolled = crate::tcmt::Values::decode_from_slice(&unrolled.encode_to_vec()).unwrap();
    let from_table = crate::tcmu::Values::decode_from_slice(&table.encode_to_vec()).unwrap();
    assert_eq!(from_unrolled, table);
    assert_eq!(from_table, unrolled);
}

/// The outcome of decoding `wire` into the `HashMap` message `M`, as the text of
/// the same message with `BTreeMap`s, which is independent of iteration order.
fn hash_outcome<M: Message, C: Message + Debug>(wire: &[u8]) -> Result<String, DecodeError> {
    M::decode_from_slice(wire)
        .map(|m| format!("{:?}", C::decode_from_slice(&m.encode_to_vec()).unwrap()))
}

#[test]
fn hash_maps_agree_on_corrupt_input() {
    macro_rules! check {
        ($ty:ident, $wire:expr) => {{
            let wire: Vec<u8> = $wire;
            // As in `assert_same_decode`, a rejection may differ when the table
            // reports the end of a message value's slice first.
            let same = |input: &[u8]| {
                let table = hash_outcome::<crate::tcmt::$ty, crate::tcbu::$ty>(input);
                let unrolled = hash_outcome::<crate::tcmu::$ty, crate::tcbu::$ty>(input);
                assert!(
                    table == unrolled
                        || (unrolled.is_err() && table == Err(DecodeError::UnexpectedEof)),
                    "input {input:02x?}: unrolled {unrolled:?}, table {table:?}"
                );
            };
            for end in 0..=wire.len() {
                same(&wire[..end]);
            }
            let mut flipped = wire.clone();
            for i in 0..wire.len() {
                for xor in [0x01, 0x80, 0xff] {
                    flipped[i] = wire[i] ^ xor;
                    same(&flipped);
                }
                flipped[i] = wire[i];
            }
        }};
    }
    check!(Keys, hu::keys().encode_to_vec());
    check!(Values, hu::values().encode_to_vec());
    for wire in value_entries() {
        check!(Values, wire);
    }
}

// ---------------------------------------------------------------------------
// Entries built by hand
// ---------------------------------------------------------------------------

fn field_tag(number: u32, wire_type: WireType) -> Vec<u8> {
    let mut out = Vec::new();
    Tag::new(number, wire_type).encode(&mut out);
    out
}

/// An entry of field 1 that declares `length` bytes, followed by `tail`.
fn entry_declaring(length: u64, tail: &[u8]) -> Vec<u8> {
    let mut out = field_tag(1, WireType::LengthDelimited);
    encode_varint(length, &mut out);
    out.extend_from_slice(tail);
    out
}

/// A map entry of field `number`, with the given contents.
fn entry(number: u32, contents: &[&[u8]]) -> Vec<u8> {
    length_delimited_field(number, &contents.concat())
}

/// Wire inputs for `Values`: entries of unusual shape in every map.
fn value_entries() -> Vec<Vec<u8>> {
    let mut cases: Vec<Vec<u8>> = Vec::new();
    // An empty entry in every map, and one with only a key or only a value.
    for number in 1..=17 {
        cases.push(entry(number, &[]));
        cases.push(entry(number, &[&length_delimited_field(1, b"key")]));
    }
    cases.push(entry(1, &[&varint_field(2, 5)]));
    cases.push(entry(14, &[&length_delimited_field(2, b"only a value")]));
    // Unknown fields of every wire type in an entry.
    cases.push(entry(
        1,
        &[
            &varint_field(1000, 3),
            &length_delimited_field(1, b"k"),
            &length_delimited_field(3, b"skipped"),
            &varint_field(2, 8),
            &[0x2d, 1, 2, 3, 4],
            &[0x31, 1, 2, 3, 4, 5, 6, 7, 8],
        ],
    ));
    // A group in an entry, and one nested too deep.
    cases.push(entry(
        1,
        &[
            &length_delimited_field(1, b"g"),
            &[0x1b, 0x08, 0x01, 0x1c],
            &varint_field(2, 1),
        ],
    ));
    cases.push(entry(1, &[&[0x1b].repeat(200), &[0x1c].repeat(200)]));
    cases.push(entry(1, &[&[0x1b, 0x08, 0x01]]));
    // A key or a value more than once, and in either order.
    cases.push(entry(
        1,
        &[
            &length_delimited_field(1, b"first"),
            &varint_field(2, 1),
            &length_delimited_field(1, b"second"),
            &varint_field(2, 2),
        ],
    ));
    cases.push(entry(
        1,
        &[&varint_field(2, 6), &length_delimited_field(1, b"k")],
    ));
    // The same key in two entries.
    cases.push(
        [
            entry(1, &[&length_delimited_field(1, b"k"), &varint_field(2, 1)]),
            entry(1, &[&length_delimited_field(1, b"k"), &varint_field(2, 2)]),
        ]
        .concat(),
    );
    // A message value that is repeated merges its parts, and one that fails
    // is not inserted.
    cases.push(entry(
        17,
        &[
            &length_delimited_field(1, b"k"),
            &length_delimited_field(2, &varint_field(1, 10)),
            &length_delimited_field(2, &length_delimited_field(2, b"label")),
            &length_delimited_field(2, &length_delimited_field(4, &varint_field(1, 3))),
        ],
    ));
    cases.push(entry(
        17,
        &[
            &length_delimited_field(1, b"k"),
            &length_delimited_field(2, &length_delimited_field(2, &[0xff])),
        ],
    ));
    cases.push(entry(
        17,
        &[
            &length_delimited_field(1, b"k"),
            &length_delimited_field(2, &[0x0a]),
        ],
    ));
    // An open enum keeps a number it does not know.
    cases.push(entry(
        16,
        &[&length_delimited_field(1, b"k"), &varint_field(2, 12345)],
    ));
    cases.push(entry(
        16,
        &[&length_delimited_field(1, b"k"), &varint_field(2, u64::MAX)],
    ));
    // Wrong wire types for the key and for the value of each kind.
    for number in 1..=17 {
        cases.push(entry(number, &[&varint_field(1, 1)]));
        cases.push(entry(number, &[&[0x0d, 1, 2, 3, 4]]));
        cases.push(entry(
            number,
            &[
                &length_delimited_field(1, b"k"),
                &length_delimited_field(2, b"x"),
            ],
        ));
        cases.push(entry(
            number,
            &[&length_delimited_field(1, b"k"), &[0x15, 1, 2, 3, 4]],
        ));
        cases.push(entry(
            number,
            &[
                &length_delimited_field(1, b"k"),
                &[0x19, 1, 2, 3, 4, 5, 6, 7, 8],
            ],
        ));
    }
    // Invalid UTF-8 in a key and in a value.
    cases.push(entry(1, &[&length_delimited_field(1, &[0xff, 0xfe])]));
    cases.push(entry(
        14,
        &[
            &length_delimited_field(1, b"k"),
            &length_delimited_field(2, &[0xc0, 0x80]),
        ],
    ));
    // The entry itself with the wrong wire type.
    cases.push(varint_field(1, 1));
    cases.push([field_tag(1, WireType::Fixed32), vec![0; 4]].concat());
    // Lengths that do not fit: an entry longer than the input, a string longer
    // than its entry, an entry of length 2^64 - 1.
    cases.push(vec![0x0a, 0x09, 0x0a]);
    cases.push([&[0x0a, 0x03, 0x0a, 0x05, b'a'][..], b"bcde"].concat());
    cases.push(entry_declaring(u64::MAX, &[]));
    cases.push(entry_declaring(1 << 31, b"x"));
    // An entry cut anywhere, with trailing bytes.
    let whole = entry(
        17,
        &[
            &length_delimited_field(1, b"key"),
            &length_delimited_field(2, &length_delimited_field(2, b"label")),
        ],
    );
    for end in 0..whole.len() {
        cases.push(whole[..end].to_vec());
    }
    cases.push([whole.clone(), vec![0x08]].concat());
    cases
}

#[test]
fn entries_of_unusual_shape_decode_the_same() {
    for wire in value_entries() {
        assert_same_decode::<crate::tcbu::Values, crate::tcbt::Values>(&wire, true);
        assert_same_chained::<crate::tcbu::Values, crate::tcbt::Values>(&wire);
    }
}

#[test]
fn key_entries_of_unusual_shape_decode_the_same() {
    let mut cases = Vec::new();
    for number in 1..=12 {
        cases.push(entry(number, &[]));
        cases.push(entry(number, &[&varint_field(2, 4)]));
        cases.push(entry(number, &[&varint_field(1, 1), &varint_field(2, 4)]));
        cases.push(entry(
            number,
            &[&length_delimited_field(1, b"x"), &varint_field(2, 4)],
        ));
        cases.push(entry(number, &[&[0x0d, 9, 9, 9, 9], &varint_field(2, 4)]));
        cases.push(entry(
            number,
            &[&[0x09, 1, 2, 3, 4, 5, 6, 7, 8], &varint_field(2, 4)],
        ));
        cases.push(entry(
            number,
            &[&varint_field(1, u64::MAX), &varint_field(2, 4)],
        ));
        cases.push(entry(
            number,
            &[&varint_field(1, 1 << 40), &varint_field(2, 4)],
        ));
        cases.push(entry(
            number,
            &[&varint_field(1, 1), &length_delimited_field(2, b"x")],
        ));
    }
    for wire in cases {
        assert_same_decode::<crate::tcbu::Keys, crate::tcbt::Keys>(&wire, true);
    }
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

fn outcome_with<M: Message + Debug>(
    options: &DecodeOptions,
    wire: &[u8],
) -> Result<String, DecodeError> {
    options
        .decode_from_slice::<M>(wire)
        .map(|m| format!("{m:?}"))
}

#[test]
fn the_element_memory_limit_counts_each_entry_in_both_codecs() {
    // A thousand empty entries of a scalar map, a string map, a bytes map and
    // a message map: each still creates a key and a value.
    for number in [1, 14, 15, 17] {
        let wire: Vec<u8> = (0..1000).flat_map(|_| entry(number, &[])).collect();
        for limit in [50, 100, 4000, 64_000, 1_000_000] {
            let options = DecodeOptions::new().with_element_memory_limit(limit);
            assert_eq!(
                outcome_with::<crate::tcbt::Values>(&options, &wire),
                outcome_with::<crate::tcbu::Values>(&options, &wire),
                "field {number}, limit {limit}"
            );
        }
        let options = DecodeOptions::new().with_element_memory_limit(100);
        assert_eq!(
            outcome_with::<crate::tcbt::Values>(&options, &wire),
            Err(DecodeError::ElementMemoryLimitExceeded)
        );
    }
}

#[test]
fn the_recursion_limit_counts_message_values_in_both_codecs() {
    // children (1): key "k", value: the same message, `depth` deep.
    fn nested(depth: usize) -> Vec<u8> {
        let mut wire = Vec::new();
        for _ in 0..depth {
            wire = entry(
                1,
                &[
                    &length_delimited_field(1, b"k"),
                    &length_delimited_field(2, &wire),
                ],
            );
        }
        wire
    }
    for depth in [1, 50, 90, 95, 98, 99, 100, 101, 102, 110, 200] {
        let wire = nested(depth);
        for limit in [5, 20, 100] {
            let options = DecodeOptions::new().with_recursion_limit(limit);
            assert_eq!(
                outcome_with::<crate::tcbt::Nesting>(&options, &wire),
                outcome_with::<crate::tcbu::Nesting>(&options, &wire),
                "depth {depth}, limit {limit}"
            );
        }
    }
    assert_eq!(
        crate::tcbt::Nesting::decode_from_slice(&nested(200)),
        Err(DecodeError::RecursionLimitExceeded)
    );
}

// ---------------------------------------------------------------------------
// Closed enums
// ---------------------------------------------------------------------------

#[test]
fn an_entry_with_an_unknown_closed_enum_number_is_kept_as_unknown() {
    // proto2: colors[k] = 7, which `Color` does not have.
    let unknown = entry(1, &[&length_delimited_field(1, b"k"), &varint_field(2, 7)]);
    let known = entry(1, &[&length_delimited_field(1, b"j"), &varint_field(2, 2)]);
    for wire in [
        unknown.clone(),
        [known.clone(), unknown.clone()].concat(),
        entry(
            1,
            &[
                &length_delimited_field(1, b"k"),
                &varint_field(2, 7),
                &varint_field(2, 2),
            ],
        ),
        entry(
            1,
            &[
                &length_delimited_field(1, b"k"),
                &varint_field(2, 2),
                &varint_field(2, 7),
            ],
        ),
        entry(1, &[&varint_field(2, 7)]),
        entry(
            1,
            &[&length_delimited_field(1, b"k"), &varint_field(2, u64::MAX)],
        ),
    ] {
        assert_same_decode::<crate::tc2u::ClosedMaps, crate::tc2t::ClosedMaps>(&wire, true);
        let decoded = crate::tc2t::ClosedMaps::decode_from_slice(&wire).unwrap();
        assert_eq!(
            decoded.encode_to_vec().len(),
            crate::tc2u::ClosedMaps::decode_from_slice(&wire)
                .unwrap()
                .encode_to_vec()
                .len()
        );
    }
    let decoded = crate::tc2t::ClosedMaps::decode_from_slice(&unknown).unwrap();
    assert!(decoded.colors.is_empty());
    assert_eq!(decoded.encode_to_vec(), unknown, "the whole entry is kept");
}

#[test]
fn an_entry_with_an_unknown_closed_enum_number_is_dropped_without_unknown_fields() {
    let unknown = entry(1, &[&length_delimited_field(1, b"k"), &varint_field(2, 7)]);
    let known = entry(1, &[&length_delimited_field(1, b"j"), &varint_field(2, 2)]);
    for wire in [
        unknown.clone(),
        [known.clone(), unknown.clone()].concat(),
        entry(
            1,
            &[
                &length_delimited_field(1, b"k"),
                &varint_field(2, 7),
                &varint_field(2, 2),
            ],
        ),
        entry(
            1,
            &[
                &length_delimited_field(1, b"k"),
                &varint_field(2, 2),
                &varint_field(2, 7),
            ],
        ),
    ] {
        assert_same_decode::<crate::tclu::ClosedMaps, crate::tclt::ClosedMaps>(&wire, true);
    }
    let decoded = crate::tclt::ClosedMaps::decode_from_slice(&unknown).unwrap();
    assert!(decoded.colors.is_empty());
    assert!(decoded.encode_to_vec().is_empty());
}

#[test]
fn the_unknown_field_limit_counts_kept_entries_in_both_codecs() {
    let wire: Vec<u8> = (0..100)
        .flat_map(|_| entry(1, &[&length_delimited_field(1, b"k"), &varint_field(2, 7)]))
        .collect();
    for limit in [1, 10, 99, 100, 101, 1000] {
        let options = DecodeOptions::new().with_unknown_field_limit(limit);
        assert_eq!(
            outcome_with::<crate::tc2t::ClosedMaps>(&options, &wire),
            outcome_with::<crate::tc2u::ClosedMaps>(&options, &wire),
            "limit {limit}"
        );
    }
    let options = DecodeOptions::new().with_unknown_field_limit(10);
    assert_eq!(
        outcome_with::<crate::tc2t::ClosedMaps>(&options, &wire),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
    // Dropped entries are not counted.
    assert!(outcome_with::<crate::tclt::ClosedMaps>(&options, &wire).is_ok());
}

#[test]
fn closed_enum_maps_agree_on_corrupt_input() {
    let mut wire = entry(1, &[&length_delimited_field(1, b"a"), &varint_field(2, 1)]);
    wire.extend(entry(
        1,
        &[&length_delimited_field(1, b"b"), &varint_field(2, 7)],
    ));
    wire.extend(entry(
        2,
        &[
            &varint_field(1, 3),
            &length_delimited_field(2, &varint_field(1, 4)),
        ],
    ));
    wire.extend(varint_field(3, 9));
    assert_same_on_corrupt_input::<crate::tc2u::ClosedMaps, crate::tc2t::ClosedMaps>(&wire, true);
    assert_same_on_corrupt_input::<crate::tclu::ClosedMaps, crate::tclt::ClosedMaps>(&wire, true);
}

#[test]
fn editions_enum_openness_applies_to_map_values() {
    use crate::{tc3t, tc3u};
    // closed = { "k": 7 } (C_A = 0, C_B = 1 only), open = { 1: 7 }.
    let wire = [
        entry(1, &[&length_delimited_field(1, b"k"), &varint_field(2, 7)]),
        entry(2, &[&varint_field(1, 1), &varint_field(2, 7)]),
        entry(1, &[&length_delimited_field(1, b"j"), &varint_field(2, 1)]),
        entry(
            3,
            &[
                &length_delimited_field(1, b"c"),
                &length_delimited_field(2, &varint_field(1, 5)),
            ],
        ),
    ]
    .concat();
    assert_same_decode::<tc3u::EMaps, tc3t::EMaps>(&wire, true);
    let table = tc3t::EMaps::decode_from_slice(&wire).unwrap();
    assert_eq!(table.closed.len(), 1);
    assert_eq!(table.open.len(), 1);
    assert_same_on_corrupt_input::<tc3u::EMaps, tc3t::EMaps>(&wire, true);
}

// ---------------------------------------------------------------------------
// Which messages use the table
// ---------------------------------------------------------------------------

/// Referencing a message's table is a compile error when it fell back to
/// unrolled code.
#[test]
fn messages_with_maps_and_the_messages_that_hold_them_use_the_table() {
    fn is_table<M: Message>(_: &'static buffa::table::Table<M>) {}
    macro_rules! tables {
        ($($path:path),* $(,)?) => { $( is_table(&$path); )* };
    }
    tables!(
        crate::tcbt::__BUFFA_TABLE_Keys,
        crate::tcbt::__BUFFA_TABLE_Values,
        crate::tcbt::__BUFFA_TABLE_Nesting,
        crate::tcbt::__BUFFA_TABLE_Sparse,
        crate::tcbt::__BUFFA_TABLE_Holder,
        crate::tcmt::__BUFFA_TABLE_Keys,
        crate::tcmt::__BUFFA_TABLE_Values,
        crate::tcmt::__BUFFA_TABLE_Holder,
        crate::tc2t::__BUFFA_TABLE_ClosedMaps,
        crate::tclt::__BUFFA_TABLE_ClosedMaps,
        crate::tc3t::__BUFFA_TABLE_EMaps,
        crate::tcrt::__BUFFA_TABLE_MixedMaps,
        crate::tcu8t::__BUFFA_TABLE_Utf8Maps,
        // Its values are a message from another crate, reached through its
        // `Message` impl.
        crate::tcu8t::__BUFFA_TABLE_WrapperValues,
    );
}

// ---------------------------------------------------------------------------
// Duplicate keys, footprint and registration order
// ---------------------------------------------------------------------------

#[test]
fn a_repeated_key_replaces_a_message_value_instead_of_merging_into_it() {
    let key = length_delimited_field(1, b"k");
    let item = |contents: &[u8]| length_delimited_field(2, contents);
    let first = [&varint_field(1, 1)[..], &length_delimited_field(2, b"a")].concat();
    let wire = [
        entry(17, &[&key, &item(&first)]),
        entry(17, &[&key, &item(&varint_field(3, 9))]),
    ]
    .concat();
    assert_same_decode::<crate::tcbu::Values, crate::tcbt::Values>(&wire, true);
    assert_same_chained::<crate::tcbu::Values, crate::tcbt::Values>(&wire);
    let table = crate::tcbt::Values::decode_from_slice(&wire).unwrap();
    let unrolled = crate::tcbu::Values::decode_from_slice(&wire).unwrap();
    assert_eq!(
        format!("{:?}", table.item["k"]),
        format!("{:?}", unrolled.item["k"])
    );
    let value = &table.item["k"];
    assert_eq!((value.id, value.label.as_str()), (0, ""));
    assert_eq!(value.tags, [9]);
}

/// Entries of field `number` with keys `k0`, `k1`, … and no value.
fn keyed_entries(number: u32, count: usize) -> Vec<u8> {
    (0..count)
        .flat_map(|i| {
            entry(
                number,
                &[&length_delimited_field(1, format!("k{i}").as_bytes())],
            )
        })
        .collect()
}

#[test]
fn an_entry_counts_the_size_of_its_key_and_value_against_the_element_memory_limit() {
    use core::mem::size_of;
    let string = size_of::<String>();
    // The fields of `Values` with a `string` key, and the size of the value.
    let fields = [
        (1, size_of::<i32>()),
        (4, size_of::<u64>()),
        (7, size_of::<bool>()),
        (13, size_of::<f64>()),
        (14, string),
        (15, size_of::<Vec<u8>>()),
    ];
    for (number, value) in fields {
        let wire = keyed_entries(number, 5);
        let limit = 5 * (string + value);
        for (limit, accepted) in [(limit, true), (limit - 1, false)] {
            let options = DecodeOptions::new().with_element_memory_limit(limit);
            let (table, unrolled) = (
                outcome_with::<crate::tcbt::Values>(&options, &wire),
                outcome_with::<crate::tcbu::Values>(&options, &wire),
            );
            assert_eq!(table, unrolled, "field {number}, limit {limit}");
            assert_eq!(table.is_ok(), accepted, "field {number}, limit {limit}");
            if !accepted {
                assert_eq!(table, Err(DecodeError::ElementMemoryLimitExceeded));
            }
        }
    }
}

#[test]
fn an_entry_is_counted_before_it_is_decoded() {
    // Under a limit too small for one entry, an entry that is also malformed
    // is rejected for the limit or for the malformation in the same order in
    // both codecs.
    for wire in value_entries() {
        for limit in [0, 1, 20, 28, 40, 100, 4000] {
            let options = DecodeOptions::new().with_element_memory_limit(limit);
            assert_eq!(
                outcome_with::<crate::tcbt::Values>(&options, &wire),
                outcome_with::<crate::tcbu::Values>(&options, &wire),
                "limit {limit}, input {wire:02x?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Collection types and string and bytes representations
// ---------------------------------------------------------------------------

#[test]
fn map_type_in_can_mix_collections_within_one_message() {
    use buffa::__private::HashMap;
    use std::collections::BTreeMap;
    macro_rules! sample {
        ($m:ident) => {
            crate::$m::MixedMaps {
                // One entry in each `HashMap`, so that the bytes do not depend
                // on iteration order.
                hashed: [("h".to_string(), 1)].into_iter().collect(),
                ordered: [
                    ("c".to_string(), 3),
                    ("a".to_string(), 1),
                    ("b".to_string(), 2),
                ]
                .into_iter()
                .collect(),
                more: [(5, "five".to_string())].into_iter().collect(),
                ..Default::default()
            }
        };
    }
    let (unrolled, table) = (sample!(tcru), sample!(tcrt));
    let _: &HashMap<String, i32> = &table.hashed;
    let _: &BTreeMap<String, i32> = &table.ordered;
    let _: &HashMap<i32, String> = &table.more;
    let wire = assert_same_codec(&unrolled, &table);
    assert_same_chained::<crate::tcru::MixedMaps, crate::tcrt::MixedMaps>(&wire);
    assert_same_on_corrupt_input::<crate::tcru::MixedMaps, crate::tcrt::MixedMaps>(&wire, true);
}

#[test]
fn strings_without_utf8_validation_are_byte_vectors_in_a_table_map() {
    use buffa::__private::HashMap;
    macro_rules! sample {
        ($m:ident) => {
            crate::$m::Utf8Maps {
                checked: [("a".to_string(), "h\u{e9}llo".to_string())]
                    .into_iter()
                    .collect(),
                raw: [(vec![0xff, 0xfe], vec![0x80])].into_iter().collect(),
                carve: [(vec![1, 2], vec![3])].into_iter().collect(),
                ..Default::default()
            }
        };
    }
    let (unrolled, table) = (sample!(tcu8u), sample!(tcu8t));
    let _: &HashMap<Vec<u8>, Vec<u8>> = &table.raw;
    let _: &HashMap<Vec<u8>, Vec<u8>> = &table.carve;
    let wire = assert_same_codec(&unrolled, &table);
    assert_same_chained::<crate::tcu8u::Utf8Maps, crate::tcu8t::Utf8Maps>(&wire);
    assert_same_on_corrupt_input::<crate::tcu8u::Utf8Maps, crate::tcu8t::Utf8Maps>(&wire, true);

    // Invalid UTF-8 is accepted in `raw` and rejected in `checked`.
    let invalid = |number| {
        entry(
            number,
            &[
                &length_delimited_field(1, &[0xff]),
                &length_delimited_field(2, &[0xff]),
            ],
        )
    };
    for number in [2, 3] {
        assert!(crate::tcu8t::Utf8Maps::decode_from_slice(&invalid(number)).is_ok());
    }
    assert_same_decode::<crate::tcu8u::Utf8Maps, crate::tcu8t::Utf8Maps>(&invalid(1), false);
    assert!(crate::tcu8t::Utf8Maps::decode_from_slice(&invalid(1)).is_err());
}

#[test]
fn maps_of_custom_string_or_bytes_types_agree_in_the_unrolled_codec() {
    // `Bytes` values are not representable in a table, so these messages fall
    // back, and must still round-trip through the same API.
    macro_rules! sample {
        ($m:ident) => {
            (
                crate::$m::BytesValues {
                    blobs: [(
                        "b".to_string(),
                        buffa::bytes::Bytes::from_static(b"\x00\xff"),
                    )]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
                crate::$m::RawValues {
                    raw_values: [(1, buffa::bytes::Bytes::from_static(b"\xff"))]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
            )
        };
    }
    let ((bu, ru), (bt, rt)) = (sample!(tcu8u), sample!(tcu8t));
    assert_same_codec(&bu, &bt);
    assert_same_codec(&ru, &rt);
}

/// A `WrapperValues` decoded from `wire` under `options`, as the sorted entries
/// of its map, which do not depend on the iteration order of a `HashMap`.
macro_rules! wrapper_outcome {
    ($m:ident, $options:expr, $wire:expr) => {
        $options
            .decode_from_slice::<crate::$m::WrapperValues>($wire)
            .map(|m| sorted(&m.counts))
    };
}

#[test]
fn a_map_of_messages_from_another_crate_round_trips_and_agrees_on_corrupt_input() {
    use buffa_types::google::protobuf::Int32Value;
    // One entry, so that the bytes do not depend on the iteration order of the
    // `HashMap`.
    macro_rules! sample {
        ($m:ident) => {
            crate::$m::WrapperValues {
                counts: [("c".to_string(), Int32Value::from(3))]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }
        };
    }
    assert_same_codec(&sample!(tcu8u), &sample!(tcu8t));

    let entries = |count: u64| -> Vec<u8> {
        (0..count)
            .flat_map(|i| {
                entry(
                    1,
                    &[
                        &length_delimited_field(1, format!("k{i}").as_bytes()),
                        &length_delimited_field(2, &varint_field(1, i + 1)),
                    ],
                )
            })
            .collect()
    };
    let wire = entries(4);
    let options = DecodeOptions::new();
    // As in `assert_same_decode`, a rejection may differ when the table
    // reports the end of a message value's slice first.
    let same = |input: &[u8]| {
        let unrolled = wrapper_outcome!(tcu8u, options, input);
        let table = wrapper_outcome!(tcu8t, options, input);
        assert!(
            table == unrolled || (unrolled.is_err() && table == Err(DecodeError::UnexpectedEof)),
            "input {input:02x?}: unrolled {unrolled:?}, table {table:?}"
        );
    };
    for end in 0..=wire.len() {
        same(&wire[..end]);
    }
    let mut flipped = wire.clone();
    for i in 0..wire.len() {
        for xor in [0x01, 0x80, 0xff] {
            flipped[i] = wire[i] ^ xor;
            same(&flipped);
        }
        flipped[i] = wire[i];
    }

    // The element-memory limit counts each entry, and the recursion limit the
    // message value.
    let wire = entries(200);
    for limit in [0, 1, 50, 100, 4000, 1_000_000] {
        let options = DecodeOptions::new().with_element_memory_limit(limit);
        assert_eq!(
            wrapper_outcome!(tcu8t, options, &wire),
            wrapper_outcome!(tcu8u, options, &wire),
            "element memory limit {limit}"
        );
    }
    for limit in [0, 1, 2, 100] {
        let options = DecodeOptions::new().with_recursion_limit(limit);
        assert_eq!(
            wrapper_outcome!(tcu8t, options, &wire),
            wrapper_outcome!(tcu8u, options, &wire),
            "recursion limit {limit}"
        );
    }
    let options = DecodeOptions::new().with_element_memory_limit(100);
    assert_eq!(
        wrapper_outcome!(tcu8t, options, &wire),
        Err(DecodeError::ElementMemoryLimitExceeded)
    );
}

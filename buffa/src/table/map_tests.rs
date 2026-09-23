// Tests of the map arms over hand-written table messages, against a reference
// message that reads and writes the same fields through `map_codec`, as
// generated unrolled code does.

use super::*;
use crate::alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use crate::bytes::Buf;
use crate::encoding::{
    check_wire_type, decode_unknown_field, encode_varint, skip_field_depth, Tag, WireType,
};
use crate::map_codec::{
    self, BytesVec, ClosedEnum, Double, Fixed32, Fixed64, Int32, Int64, OpenEnum, Sint32, Sint64,
    Str, Uint32, Uint64,
};
use crate::{
    DecodeOptions, EnumValue, Enumeration, Message, MessageField, Rope, UnknownFieldData, UnknownFields,
};

type Hash<K, V> = crate::__private::HashMap<K, V>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Color {
    #[default]
    Red = 0,
    Green = 1,
    Blue = 2,
}

impl Enumeration for Color {
    fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Red),
            1 => Some(Self::Green),
            2 => Some(Self::Blue),
            _ => None,
        }
    }

    fn to_i32(&self) -> i32 {
        *self as i32
    }

    fn proto_name(&self) -> &'static str {
        match self {
            Self::Red => "RED",
            Self::Green => "GREEN",
            Self::Blue => "BLUE",
        }
    }
}

/// `int32 id = 1; string tag = 2; Item child = 3;`, keeping unknown fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct Item {
    id: i32,
    tag: String,
    child: MessageField<Item>,
    unknown: UnknownFields,
}

/// One map of each shape, in field number order.
#[derive(Clone, Debug, Default, PartialEq)]
struct Maps {
    by_name: Hash<String, i32>,
    items: BTreeMap<i32, Item>,
    closed: BTreeMap<u64, Color>,
    open: BTreeMap<bool, EnumValue<Color>>,
    blobs: BTreeMap<String, Vec<u8>>,
    names: BTreeMap<i64, String>,
    floats: BTreeMap<u32, f64>,
    zigzag: BTreeMap<i32, i64>,
    fixed: BTreeMap<u64, u32>,
    high: BTreeMap<String, String>,
    unknown: UnknownFields,
}

/// The closed-enum map of `Maps`, in a message that drops unknown fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct Lossy {
    closed: BTreeMap<u64, Color>,
}

/// A map whose values are the message itself.
#[derive(Clone, Debug, Default, PartialEq)]
struct Tree {
    children: BTreeMap<String, Tree>,
}

static ITEM: Table<Item> = crate::__table!(
    Item,
    abi = ABI,
    entries = [
        crate::__table_entry!(Item, id, Int32Implicit, 1),
        crate::__table_entry!(Item, tag, StrImplicit, 2),
        crate::__table_entry!(
            Item,
            child,
            MsgSingular,
            3,
            aux = 0,
            slot = MessageField<Item>
        ),
    ],
    dense = &[0, 1, 2, 3],
    aux = [Aux::Msg(&MsgVt::new::<MessageField<Item>>(&ITEM))],
    unknown = unknown,
);

static MAPS: Table<Maps> = crate::__table!(
    Maps,
    abi = ABI,
    entries = [
        crate::__table_entry!(Maps, by_name, Map, 1, aux = 0, slot = Hash<String, i32>),
        crate::__table_entry!(Maps, items, Map, 2, aux = 1, slot = BTreeMap<i32, Item>),
        crate::__table_entry!(Maps, closed, Map, 3, aux = 2, slot = BTreeMap<u64, Color>),
        crate::__table_entry!(
            Maps,
            open,
            Map,
            4,
            aux = 3,
            slot = BTreeMap<bool, EnumValue<Color>>
        ),
        crate::__table_entry!(Maps, blobs, Map, 5, aux = 4, slot = BTreeMap<String, Vec<u8>>),
        crate::__table_entry!(Maps, names, Map, 6, aux = 5, slot = BTreeMap<i64, String>),
        crate::__table_entry!(Maps, floats, Map, 7, aux = 6, slot = BTreeMap<u32, f64>),
        crate::__table_entry!(Maps, zigzag, Map, 8, aux = 7, slot = BTreeMap<i32, i64>),
        crate::__table_entry!(Maps, fixed, Map, 9, aux = 8, slot = BTreeMap<u64, u32>),
        crate::__table_entry!(Maps, high, Map, 300, aux = 9, slot = BTreeMap<String, String>),
    ],
    dense = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    aux = [
        Aux::Map(&MapVt::new::<Hash<String, i32>, kinds::StrRequired, kinds::Int32Required>()),
        Aux::Map(&MapVt::with_msg::<BTreeMap<i32, Item>, kinds::Int32Required, Item>(
            &DirectMsgVt::new(&ITEM)
        )),
        Aux::Map(&MapVt::with_enum::<BTreeMap<u64, Color>, kinds::Uint64Required, ImplicitClosed<Color>>()),
        Aux::Map(&MapVt::with_enum::<
            BTreeMap<bool, EnumValue<Color>>,
            kinds::BoolRequired,
            ImplicitOpen<Color>,
        >()),
        Aux::Map(&MapVt::new::<BTreeMap<String, Vec<u8>>, kinds::StrRequired, kinds::BytesRequired>()),
        Aux::Map(&MapVt::new::<BTreeMap<i64, String>, kinds::Int64Required, kinds::StrRequired>()),
        Aux::Map(&MapVt::new::<BTreeMap<u32, f64>, kinds::Uint32Required, kinds::DoubleRequired>()),
        Aux::Map(&MapVt::new::<BTreeMap<i32, i64>, kinds::Sint32Required, kinds::Sint64Required>()),
        Aux::Map(&MapVt::new::<BTreeMap<u64, u32>, kinds::Fixed64Required, kinds::Fixed32Required>()),
        Aux::Map(&MapVt::new::<BTreeMap<String, String>, kinds::StrRequired, kinds::StrRequired>()),
    ],
    unknown = unknown,
);

static LOSSY: Table<Lossy> = crate::__table!(
    Lossy,
    abi = ABI,
    entries = [crate::__table_entry!(Lossy, closed, Map, 3, aux = 0, slot = BTreeMap<u64, Color>)],
    dense = &[0, 0, 0, 1],
    aux = [Aux::Map(
        &MapVt::with_enum::<BTreeMap<u64, Color>, kinds::Uint64Required, ImplicitClosed<Color>>()
    )],
    unknown = none,
);

static TREE: Table<Tree> = crate::__table!(
    Tree,
    abi = ABI,
    entries = [crate::__table_entry!(Tree, children, Map, 1, aux = 0, slot = BTreeMap<String, Tree>)],
    dense = &[0, 1],
    aux = [Aux::Map(&MapVt::with_msg::<BTreeMap<String, Tree>, kinds::StrRequired, Tree>(
        &DirectMsgVt::new(&TREE)
    ))],
    unknown = none,
);

macro_rules! table_message {
    ($ty:ty, $table:ident) => {
        crate::impl_default_instance!($ty);

        impl Message for $ty {
            fn compute_size(&self, cache: &mut SizeCache) -> u32 {
                $table.compute_size(self, cache)
            }

            fn write_to(&self, cache: &mut SizeCache, buf: &mut impl EncodeSink) {
                $table.write_to(self, cache, buf);
            }

            fn merge_field(
                &mut self,
                tag: Tag,
                buf: &mut impl Buf,
                ctx: DecodeContext<'_>,
            ) -> Result<(), DecodeError> {
                $table.merge_field(self, tag, buf, ctx)
            }

            fn merge_to_limit(
                &mut self,
                buf: &mut impl Buf,
                ctx: DecodeContext<'_>,
                limit: usize,
            ) -> Result<(), DecodeError> {
                $table.merge_to_limit(self, buf, ctx, limit)
            }

            fn merge_length_delimited(
                &mut self,
                buf: &mut impl Buf,
                ctx: DecodeContext<'_>,
            ) -> Result<(), DecodeError> {
                $table.merge_length_delimited(self, buf, ctx)
            }

            fn clear(&mut self) {
                *self = Self::default();
            }
        }
    };
}

table_message!(Item, ITEM);
table_message!(Maps, MAPS);
table_message!(Lossy, LOSSY);
table_message!(Tree, TREE);

/// `Maps` written and read as unrolled generated code does: through the
/// generic helpers of `map_codec`, one call per field.
#[derive(Clone, Debug, Default, PartialEq)]
struct Reference(Maps);
crate::impl_default_instance!(Reference);

impl Message for Reference {
    fn compute_size(&self, cache: &mut SizeCache) -> u32 {
        let m = &self.0;
        let mut size = 0u64;
        size += map_codec::field_len::<Str, Int32, _>(&m.by_name, 1);
        size += map_codec::message_field_len::<Int32, Item, _>(&m.items, 1, cache);
        size += map_codec::field_len::<Uint64, ClosedEnum<Color>, _>(&m.closed, 1);
        size += map_codec::field_len::<map_codec::Bool, OpenEnum<Color>, _>(&m.open, 1);
        size += map_codec::field_len::<Str, BytesVec, _>(&m.blobs, 1);
        size += map_codec::field_len::<Int64, Str, _>(&m.names, 1);
        size += map_codec::field_len::<Uint32, Double, _>(&m.floats, 1);
        size += map_codec::field_len::<Sint32, Sint64, _>(&m.zigzag, 1);
        size += map_codec::field_len::<Fixed64, Fixed32, _>(&m.fixed, 1);
        size += map_codec::field_len::<Str, Str, _>(&m.high, 2);
        size += m.unknown.encoded_len() as u64;
        crate::saturate_size(size)
    }

    fn write_to(&self, cache: &mut SizeCache, buf: &mut impl EncodeSink) {
        let m = &self.0;
        map_codec::write_field::<Str, Int32, _>(&m.by_name, 1, buf);
        map_codec::write_message_field::<Int32, Item, _>(&m.items, 2, cache, buf);
        map_codec::write_field::<Uint64, ClosedEnum<Color>, _>(&m.closed, 3, buf);
        map_codec::write_field::<map_codec::Bool, OpenEnum<Color>, _>(&m.open, 4, buf);
        map_codec::write_field::<Str, BytesVec, _>(&m.blobs, 5, buf);
        map_codec::write_field::<Int64, Str, _>(&m.names, 6, buf);
        map_codec::write_field::<Uint32, Double, _>(&m.floats, 7, buf);
        map_codec::write_field::<Sint32, Sint64, _>(&m.zigzag, 8, buf);
        map_codec::write_field::<Fixed64, Fixed32, _>(&m.fixed, 9, buf);
        map_codec::write_field::<Str, Str, _>(&m.high, 300, buf);
        m.unknown.write_to(buf);
    }

    fn merge_field(
        &mut self,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let m = &mut self.0;
        let number = tag.field_number();
        if (1..=9).contains(&number) || number == 300 {
            check_wire_type(tag, WireType::LengthDelimited)?;
        }
        match number {
            1 => map_codec::merge_entry::<Str, Int32, _>(&mut m.by_name, buf, ctx),
            2 => map_codec::merge_entry::<Int32, map_codec::Msg<Item>, _>(&mut m.items, buf, ctx),
            3 => map_codec::merge_entry_with_unknowns::<Uint64, ClosedEnum<Color>, _>(
                &mut m.closed,
                buf,
                ctx,
                Some((3, &mut m.unknown)),
            ),
            4 => map_codec::merge_entry::<map_codec::Bool, OpenEnum<Color>, _>(
                &mut m.open,
                buf,
                ctx,
            ),
            5 => map_codec::merge_entry::<Str, BytesVec, _>(&mut m.blobs, buf, ctx),
            6 => map_codec::merge_entry::<Int64, Str, _>(&mut m.names, buf, ctx),
            7 => map_codec::merge_entry::<Uint32, Double, _>(&mut m.floats, buf, ctx),
            8 => map_codec::merge_entry::<Sint32, Sint64, _>(&mut m.zigzag, buf, ctx),
            9 => map_codec::merge_entry::<Fixed64, Fixed32, _>(&mut m.fixed, buf, ctx),
            300 => map_codec::merge_entry::<Str, Str, _>(&mut m.high, buf, ctx),
            _ => {
                let field = decode_unknown_field(tag, buf, ctx)?;
                m.unknown.push(field);
                Ok(())
            }
        }
    }

    fn clear(&mut self) {
        self.0 = Maps::default();
    }
}

/// `Lossy` the same way, which drops an entry with an unknown enum value.
#[derive(Clone, Debug, Default, PartialEq)]
struct ReferenceLossy(Lossy);
crate::impl_default_instance!(ReferenceLossy);

impl Message for ReferenceLossy {
    fn compute_size(&self, _: &mut SizeCache) -> u32 {
        crate::saturate_size(map_codec::field_len::<Uint64, ClosedEnum<Color>, _>(
            &self.0.closed,
            1,
        ))
    }

    fn write_to(&self, _: &mut SizeCache, buf: &mut impl EncodeSink) {
        map_codec::write_field::<Uint64, ClosedEnum<Color>, _>(&self.0.closed, 3, buf);
    }

    fn merge_field(
        &mut self,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        if tag.field_number() == 3 {
            check_wire_type(tag, WireType::LengthDelimited)?;
            map_codec::merge_entry::<Uint64, ClosedEnum<Color>, _>(&mut self.0.closed, buf, ctx)
        } else {
            skip_field_depth(tag, buf, ctx.depth())
        }
    }

    fn clear(&mut self) {
        self.0 = Lossy::default();
    }
}

fn item(id: i32, tag: &str) -> Item {
    Item {
        id,
        tag: tag.into(),
        ..Item::default()
    }
}

fn populated() -> Maps {
    Maps {
        by_name: [("a".to_string(), 1), (String::new(), 0), ("ccc".into(), -3)]
            .into_iter()
            .collect(),
        items: BTreeMap::from([
            (
                -1,
                Item {
                    child: MessageField::some(item(9, "deep")),
                    ..item(1, "one")
                },
            ),
            (0, Item::default()),
            (70_000, item(2, "")),
        ]),
        closed: BTreeMap::from([(0, Color::Red), (u64::MAX, Color::Blue)]),
        open: BTreeMap::from([(false, EnumValue::from(9)), (true, EnumValue::from(0))]),
        blobs: BTreeMap::from([("k".to_string(), vec![1, 2, 3]), (String::new(), Vec::new())]),
        names: BTreeMap::from([(i64::MIN, "min".to_string()), (0, String::new())]),
        floats: BTreeMap::from([(0, 0.0), (7, -1.5), (u32::MAX, f64::MAX)]),
        zigzag: BTreeMap::from([(i32::MIN, i64::MIN), (-1, 1), (0, 0)]),
        fixed: BTreeMap::from([(0, 0), (u64::MAX, u32::MAX)]),
        high: BTreeMap::from([("h".to_string(), "i".to_string())]),
        unknown: UnknownFields::new(),
    }
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

fn varint(v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    encode_varint(v, &mut out);
    out
}

/// A length-delimited field `number`.
fn ld(number: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = varint(u64::from(number) << 3 | 2);
    out.extend(varint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

/// A varint field `number`.
fn vf(number: u32, v: u64) -> Vec<u8> {
    let mut out = varint(u64::from(number) << 3);
    out.extend(varint(v));
    out
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// ---------------------------------------------------------------------------
// Comparison with the reference
// ---------------------------------------------------------------------------

/// Decode `wire` with the table and with the reference, and require the same
/// value or, from two rejections, the same error, except that the table may
/// report `UnexpectedEof` where the reference read on past the end of an
/// entry and found another error.
#[track_caller]
fn assert_same(wire: &[u8]) -> Result<Maps, DecodeError> {
    let table = Maps::decode_from_slice(wire);
    let reference = Reference::decode_from_slice(wire).map(|r| r.0);
    match (&table, &reference) {
        (Ok(t), Ok(r)) => assert_eq!(t, r, "values differ on {wire:02x?}"),
        (Err(t), Err(r)) => assert!(
            t == r || *t == DecodeError::UnexpectedEof,
            "errors differ on {wire:02x?}: table {t:?}, reference {r:?}"
        ),
        _ => panic!("outcomes differ on {wire:02x?}: table {table:?}, reference {reference:?}"),
    }
    table
}

#[test]
fn encodes_the_same_bytes_as_the_reference() {
    let msg = populated();
    let reference = Reference(msg.clone());
    let wire = reference.encode_to_vec();
    assert_eq!(msg.encode_to_vec(), wire);
    assert_eq!(msg.encoded_len(), reference.encoded_len());
    assert_eq!(msg.encoded_len() as usize, wire.len());
    assert_eq!(Maps::decode_from_slice(&wire).unwrap(), msg);

    let mut framed = Vec::new();
    msg.encode_length_delimited(&mut framed);
    let mut reference_framed = Vec::new();
    reference.encode_length_delimited(&mut reference_framed);
    assert_eq!(framed, reference_framed);
}

#[test]
fn an_empty_map_encodes_to_nothing() {
    assert_eq!(Maps::default().encode_to_vec(), Vec::<u8>::new());
    assert_eq!(Maps::default().encoded_len(), 0);
    let empty = Maps::decode_from_slice(&[]).unwrap();
    assert_eq!(empty, Maps::default());
}

#[test]
fn every_sink_receives_the_same_bytes() {
    let msg = populated();
    let expected = msg.encode_to_vec();
    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &expected[..]);
    assert_eq!(&msg.encode_to_bytes()[..], &expected[..]);
    let mut vec = vec![0xee];
    msg.encode(&mut vec);
    assert_eq!(&vec[1..], &expected[..]);
}

#[test]
fn known_wire_bytes() {
    let mut msg = Maps::default();
    msg.names.insert(-1, "x".into());
    msg.zigzag.insert(-1, 1);
    // names (6): an entry of key = -1 (ten bytes) and value = "x"; zigzag (8):
    // key -1 is 1, value 1 is 2.
    let mut names = vec![0x08];
    names.extend([0xff; 9]);
    names.push(0x01);
    names.extend([0x12, 0x01, b'x']);
    let expected = cat(&[&ld(6, &names), &ld(8, &[0x08, 0x01, 0x10, 0x02])]);
    assert_eq!(msg.encode_to_vec(), expected);
    assert_eq!(Maps::decode_from_slice(&expected).unwrap(), msg);
}

#[test]
fn a_high_field_number_takes_a_two_byte_tag() {
    let mut msg = Maps::default();
    msg.high.insert("k".into(), "v".into());
    // 300 << 3 | 2 = 2402 = 0xe2 0x12.
    assert_eq!(
        msg.encode_to_vec(),
        [0xe2, 0x12, 0x06, 0x0a, 0x01, b'k', 0x12, 0x01, b'v']
    );
    assert_eq!(Maps::decode_from_slice(&msg.encode_to_vec()).unwrap(), msg);
}

#[test]
fn a_map_of_messages_records_and_consumes_sizes_in_order() {
    // Nested messages inside map values reserve slots in the same cache.
    let mut msg = Maps::default();
    for i in 0..5 {
        msg.items.insert(
            i,
            Item {
                child: MessageField::some(Item {
                    child: MessageField::some(item(i, "leaf")),
                    ..item(i * 10, "mid")
                }),
                ..item(i * 100, "top")
            },
        );
    }
    msg.high.insert("after".into(), "the maps".into());
    let reference = Reference(msg.clone()).encode_to_vec();
    assert_eq!(msg.encode_to_vec(), reference);
    assert_eq!(Maps::decode_from_slice(&reference).unwrap(), msg);
}

#[test]
fn a_map_of_the_message_itself_round_trips() {
    let mut tree = Tree::default();
    let mut child = Tree::default();
    child.children.insert("grandchild".into(), Tree::default());
    tree.children.insert("child".into(), child);
    tree.children.insert("empty".into(), Tree::default());
    let wire = tree.encode_to_vec();
    assert_eq!(Tree::decode_from_slice(&wire).unwrap(), tree);
}

// ---------------------------------------------------------------------------
// Entry semantics
// ---------------------------------------------------------------------------

#[test]
fn a_missing_key_or_value_takes_its_default() {
    // An empty entry of each map: the default key with the default value.
    for number in [1, 2, 3, 4, 5, 6, 7, 8, 9, 300] {
        let decoded = assert_same(&ld(number, &[])).unwrap();
        assert!(!decoded.encode_to_vec().is_empty(), "field {number}");
    }
    // Key only, and value only.
    let decoded = assert_same(&ld(1, &[0x0a, 0x01, b'k'])).unwrap();
    assert_eq!(decoded.by_name, Hash::from_iter([("k".to_string(), 0)]));
    let decoded = assert_same(&ld(1, &[0x10, 0x07])).unwrap();
    assert_eq!(decoded.by_name, Hash::from_iter([(String::new(), 7)]));
}

#[test]
fn the_last_occurrence_of_a_key_or_value_in_an_entry_wins() {
    let entry = cat(&[
        &ld(1, b"first"),
        &vf(2, 1),
        &ld(1, b"second"),
        &vf(2, 2),
    ]);
    let decoded = assert_same(&ld(1, &entry)).unwrap();
    assert_eq!(decoded.by_name, Hash::from_iter([("second".to_string(), 2)]));
}

#[test]
fn the_value_may_precede_the_key() {
    let entry = cat(&[&vf(2, 5), &ld(1, b"k")]);
    let decoded = assert_same(&ld(1, &entry)).unwrap();
    assert_eq!(decoded.by_name, Hash::from_iter([("k".to_string(), 5)]));
}

#[test]
fn unknown_fields_in_an_entry_are_skipped() {
    let entry = cat(&[&vf(1000, 3), &ld(1, b"k"), &ld(3, b"ignored"), &vf(2, 8)]);
    let decoded = assert_same(&ld(1, &entry)).unwrap();
    assert_eq!(decoded.by_name, Hash::from_iter([("k".to_string(), 8)]));
    assert!(decoded.unknown.is_empty());
}

#[test]
fn the_same_key_in_two_entries_keeps_the_later_value() {
    let wire = cat(&[
        &ld(1, &cat(&[&ld(1, b"k"), &vf(2, 1)])),
        &ld(1, &cat(&[&ld(1, b"k"), &vf(2, 2)])),
    ]);
    let decoded = assert_same(&wire).unwrap();
    assert_eq!(decoded.by_name, Hash::from_iter([("k".to_string(), 2)]));
}

#[test]
fn a_message_value_repeated_in_an_entry_merges() {
    let entry = cat(&[
        &vf(1, 4),
        &ld(2, &vf(1, 10)),
        &ld(2, &ld(2, b"tag")),
    ]);
    let decoded = assert_same(&ld(2, &entry)).unwrap();
    assert_eq!(decoded.items[&4], item(10, "tag"));
}

#[test]
fn a_message_value_with_nested_messages_decodes() {
    let value = cat(&[&vf(1, 3), &ld(3, &vf(1, 4))]);
    let decoded = assert_same(&ld(2, &cat(&[&vf(1, 1), &ld(2, &value)]))).unwrap();
    assert_eq!(decoded.items[&1].child.as_option().unwrap().id, 4);
}

#[test]
fn a_wrong_wire_type_inside_an_entry_is_an_error() {
    // Key of a string map as a varint; value of an int map as length-delimited.
    for entry in [vf(1, 1), ld(2, b"x"), cat(&[&ld(1, b"k"), &ld(2, b"x")])] {
        assert!(assert_same(&ld(1, &entry)).is_err());
    }
    // The entry itself as a varint.
    assert!(assert_same(&vf(1, 1)).is_err());
}

#[test]
fn an_entry_that_runs_past_its_length_is_an_error() {
    // A string of five bytes in an entry of length three.
    let wire = cat(&[&[0x0a, 0x03, 0x0a, 0x05, b'a'], b"bcde"]);
    assert!(assert_same(&wire).is_err());
    // An entry longer than the message.
    assert_eq!(
        Maps::decode_from_slice(&[0x0a, 0x09, 0x0a]),
        Err(DecodeError::UnexpectedEof)
    );
    // An entry of an impossible length.
    assert!(assert_same(&cat(&[&[0x0a], &varint(u64::MAX)])).is_err());
}

#[test]
fn an_invalid_utf8_key_or_value_is_an_error() {
    assert!(assert_same(&ld(1, &ld(1, &[0xff, 0xfe]))).is_err());
    assert!(assert_same(&ld(6, &ld(2, &[0xc0]))).is_err());
}

#[test]
fn a_closed_enum_value_that_is_not_known_makes_the_entry_unknown() {
    // closed (3): key 5, value 7, which Color does not have.
    let entry = cat(&[&vf(1, 5), &vf(2, 7)]);
    let wire = ld(3, &entry);
    let decoded = assert_same(&wire).unwrap();
    assert!(decoded.closed.is_empty());
    // The whole entry is kept, under the map's field number.
    let kept: Vec<_> = decoded.unknown.iter().collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].number, 3);
    assert_eq!(kept[0].data, UnknownFieldData::LengthDelimited(entry.clone()));
    assert_eq!(decoded.encode_to_vec(), wire);

    // A message that keeps no unknown fields drops it.
    let lossy = Lossy::decode_from_slice(&wire).unwrap();
    assert!(lossy.closed.is_empty());
    assert_eq!(
        ReferenceLossy::decode_from_slice(&wire).unwrap().0,
        lossy,
        "the reference drops it too"
    );
}

#[test]
fn only_the_last_value_in_an_entry_decides_whether_a_closed_enum_is_known() {
    let known_last = cat(&[&vf(1, 5), &vf(2, 7), &vf(2, 2)]);
    let decoded = assert_same(&ld(3, &known_last)).unwrap();
    assert_eq!(decoded.closed, BTreeMap::from([(5, Color::Blue)]));
    assert!(decoded.unknown.is_empty());

    let unknown_last = cat(&[&vf(1, 5), &vf(2, 2), &vf(2, 7)]);
    let decoded = assert_same(&ld(3, &unknown_last)).unwrap();
    assert!(decoded.closed.is_empty());
    assert_eq!(decoded.unknown.iter().count(), 1);
}

#[test]
fn an_open_enum_value_that_is_not_known_is_kept() {
    let decoded = assert_same(&ld(4, &cat(&[&vf(1, 1), &vf(2, 42)]))).unwrap();
    assert_eq!(decoded.open, BTreeMap::from([(true, EnumValue::from(42))]));
}

#[test]
fn every_prefix_and_bit_flip_and_noise_input_matches_the_reference() {
    let wire = Reference(populated()).encode_to_vec();
    // Miri is about a hundred times slower, so it takes every eleventh offset.
    let stride = if cfg!(miri) { 11 } else { 1 };
    for end in (0..=wire.len()).step_by(stride) {
        let _ = assert_same(&wire[..end]);
    }
    let mut flipped = wire.clone();
    for i in (0..wire.len()).step_by(stride) {
        let original = flipped[i];
        for xor in [0x01, 0x07, 0x80, 0xff] {
            flipped[i] = original ^ xor;
            let _ = assert_same(&flipped);
        }
        flipped[i] = original;
    }
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let rounds = if cfg!(miri) { 10 } else { 2000 };
    for _ in 0..rounds {
        let len = (next() % 48) as usize;
        let mut noise: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        let _ = assert_same(&noise);
        let keep = (next() as usize) % (wire.len() + 1);
        noise.splice(0..0, wire[..keep].iter().copied());
        let _ = assert_same(&noise);
    }
}

#[test]
fn a_non_contiguous_buffer_is_gathered() {
    let msg = populated();
    let wire = msg.encode_to_vec();
    let step = if cfg!(miri) { 17 } else { 1 };
    for split in (0..=wire.len()).step_by(step) {
        let (head, tail) = wire.split_at(split);
        let decoded = Maps::decode(&mut head.chain(tail)).unwrap();
        assert_eq!(decoded, msg, "split at {split}");
    }
}

#[test]
fn a_map_field_decodes_from_a_contiguous_buffer_through_merge_field() {
    let wire = ld(6, &cat(&[&vf(1, 3), &ld(2, b"x")]));
    let mut msg = Maps::default();
    let mut buf = &wire[..];
    let tag = Tag::decode(&mut buf).unwrap();
    let limit = core::cell::Cell::new(1000);
    let ctx = DecodeContext::new(crate::RECURSION_LIMIT, &limit);
    msg.merge_field(tag, &mut buf, ctx).unwrap();
    assert_eq!(msg.names, BTreeMap::from([(3, "x".to_string())]));
    assert!(buf.is_empty());
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[test]
fn the_element_memory_limit_counts_each_entry() {
    // A thousand entries (under Miri, a tenth of that) of a scalar map, of a
    // string map, and of a message map, each an empty entry that still
    // materialises a key and a value.
    let count = if cfg!(miri) { 100 } else { 1000 };
    for number in [1, 2, 6] {
        let wire: Vec<u8> = (0..count).flat_map(|_| ld(number, &[])).collect();
        assert!(Maps::decode_from_slice(&wire).is_ok());
        let limited = DecodeOptions::new().with_element_memory_limit(100);
        assert_eq!(
            limited.decode_from_slice::<Maps>(&wire),
            Err(DecodeError::ElementMemoryLimitExceeded),
            "field {number}"
        );
        assert_eq!(
            limited.decode_from_slice::<Reference>(&wire).map(|r| r.0),
            Err(DecodeError::ElementMemoryLimitExceeded),
            "the reference agrees on field {number}"
        );
    }
}

#[test]
fn the_unknown_field_limit_counts_preserved_entries() {
    let wire: Vec<u8> = (0..100).flat_map(|_| ld(3, &vf(2, 7))).collect();
    assert!(Maps::decode_from_slice(&wire).is_ok());
    let limited = DecodeOptions::new().with_unknown_field_limit(10);
    assert_eq!(
        limited.decode_from_slice::<Maps>(&wire),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
    assert_eq!(
        limited.decode_from_slice::<Reference>(&wire).map(|r| r.0),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
    // Dropped entries do not count.
    assert!(limited.decode_from_slice::<Lossy>(&wire).is_ok());
}

#[test]
fn the_recursion_limit_applies_to_message_values() {
    // children (1): key "k", value: the same, `depth` levels deep.
    fn nested(depth: usize) -> Vec<u8> {
        let mut wire = Vec::new();
        for _ in 0..depth {
            wire = ld(1, &cat(&[&ld(1, b"k"), &ld(2, &wire)]));
        }
        wire
    }
    assert!(Tree::decode_from_slice(&nested(50)).is_ok());
    assert_eq!(
        Tree::decode_from_slice(&nested(150)),
        Err(DecodeError::RecursionLimitExceeded)
    );
    assert_eq!(
        DecodeOptions::new()
            .with_recursion_limit(10)
            .decode_from_slice::<Tree>(&nested(50)),
        Err(DecodeError::RecursionLimitExceeded)
    );
}

#[test]
fn a_failed_message_value_is_not_inserted() {
    // The value's string is invalid UTF-8.
    let entry = cat(&[&vf(1, 1), &ld(2, &ld(2, &[0xff]))]);
    assert!(Maps::decode_from_slice(&ld(2, &entry)).is_err());
    let mut msg = Maps::default();
    let result = msg.merge_from_slice(&ld(2, &entry));
    assert!(result.is_err());
    assert!(msg.items.is_empty());
}

// ---------------------------------------------------------------------------
// Descriptors
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Values reached through their `Message` impl
// ---------------------------------------------------------------------------

/// A map whose values are `Reference`s, a hand-written `Message` with no
/// table, which the map reaches through its `Message` impl.
#[derive(Clone, Debug, Default, PartialEq)]
struct HoldsRefs {
    refs: BTreeMap<String, Reference>,
}

static HOLDS_REFS: Table<HoldsRefs> = crate::__table!(
    HoldsRefs,
    abi = ABI,
    entries = [crate::__table_entry!(
        HoldsRefs,
        refs,
        Map,
        1,
        aux = 0,
        slot = BTreeMap<String, Reference>
    )],
    dense = &[0, 1],
    aux = [Aux::Map(&MapVt::with_msg::<
        BTreeMap<String, Reference>,
        kinds::StrRequired,
        Reference,
    >(&DirectMsgVt::via_message()))],
    unknown = none,
);
table_message!(HoldsRefs, HOLDS_REFS);

/// Declares `size` bytes and writes `writes` of them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Liar {
    size: u32,
    writes: u8,
}
crate::impl_default_instance!(Liar);

impl Message for Liar {
    fn compute_size(&self, _: &mut SizeCache) -> u32 {
        self.size
    }

    fn write_to(&self, _: &mut SizeCache, buf: &mut impl EncodeSink) {
        for _ in 0..self.writes {
            buf.put_u8(1);
        }
    }

    fn merge_field(
        &mut self,
        _: Tag,
        _: &mut impl Buf,
        _: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        unreachable!()
    }

    fn clear(&mut self) {}
}

#[derive(Clone, Debug, Default, PartialEq)]
struct HoldsLiars {
    liars: BTreeMap<String, Liar>,
}

static HOLDS_LIARS: Table<HoldsLiars> = crate::__table!(
    HoldsLiars,
    abi = ABI,
    entries = [crate::__table_entry!(
        HoldsLiars,
        liars,
        Map,
        1,
        aux = 0,
        slot = BTreeMap<String, Liar>
    )],
    dense = &[0, 1],
    aux = [Aux::Map(&MapVt::with_msg::<
        BTreeMap<String, Liar>,
        kinds::StrRequired,
        Liar,
    >(&DirectMsgVt::via_message()))],
    unknown = none,
);
table_message!(HoldsLiars, HOLDS_LIARS);

fn holds_liars(size: u32, writes: u8) -> HoldsLiars {
    HoldsLiars {
        liars: BTreeMap::from([("k".to_string(), Liar { size, writes })]),
    }
}

fn refs() -> HoldsRefs {
    let nested = Maps {
        items: BTreeMap::from([(
            3,
            Item {
                child: MessageField::some(item(4, "nested")),
                ..item(3, "three")
            },
        )]),
        ..Maps::default()
    };
    HoldsRefs {
        refs: BTreeMap::from([
            ("a".to_string(), Reference(populated())),
            (String::new(), Reference::default()),
            ("z".to_string(), Reference(nested)),
        ]),
    }
}

#[test]
fn a_map_of_hand_written_messages_writes_the_same_bytes_to_every_sink() {
    let msg = refs();
    let expected: Vec<u8> = msg
        .refs
        .iter()
        .flat_map(|(key, value)| {
            ld(
                1,
                &cat(&[&ld(1, key.as_bytes()), &ld(2, &value.encode_to_vec())]),
            )
        })
        .collect();
    // Through the cursor that `encode_to_vec` writes, which each value is
    // written to directly.
    assert_eq!(msg.encode_to_vec(), expected);
    assert_eq!(msg.encoded_len() as usize, expected.len());
    // A `Rope` is not written through the cursor, so each value is staged in
    // a buffer of the size it declared.
    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &expected[..]);
    // Room for one byte at a time.
    let mut chunked = crate::bytes::BytesMut::with_capacity(1);
    msg.encode_length_delimited(&mut chunked);
    assert_eq!(&chunked[..], &cat(&[&varint(expected.len() as u64), &expected])[..]);
    // `write_to` on a `Vec` is not the cursor either.
    let mut cache = SizeCache::new();
    msg.compute_size(&mut cache);
    let mut out = Vec::new();
    msg.write_to(&mut cache, &mut out);
    assert_eq!(out, expected);
    // Decoding reads each value, and the maps and messages nested in it,
    // through its `Message` impl.
    assert_eq!(HoldsRefs::decode_from_slice(&expected).unwrap(), msg);
}

#[test]
#[should_panic(expected = "more bytes than compute_size declared")]
fn a_map_value_that_writes_more_than_it_sized_panics_in_the_scratch_buffer() {
    // The value is staged in a buffer of the size `compute_size` gave.
    holds_liars(0, 1).encode(&mut Rope::new());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "different byte count than compute_size declared")]
fn a_map_value_that_writes_less_than_it_sized_panics_in_debug_builds() {
    holds_liars(3, 1).encode(&mut Rope::new());
}

mod invalid_descriptors {
    use super::*;

    static ENUM_VT: EnumVt = EnumVt::new::<ImplicitOpen<Color>>();
    static ENUM_AUX: [Aux; 1] = [Aux::Enum(&ENUM_VT)];

    #[test]
    #[should_panic(expected = "a map key must be of a required scalar or string kind")]
    fn a_key_must_have_required_cardinality() {
        let _ =
            MapVt::new::<BTreeMap<Option<i32>, i32>, kinds::Int32Optional, kinds::Int32Required>();
    }

    #[test]
    #[should_panic(expected = "a map value must be of a required kind or a singular message")]
    fn a_value_must_have_required_cardinality() {
        let _ =
            MapVt::new::<BTreeMap<i32, Vec<i32>>, kinds::Int32Required, kinds::Int32Repeated>();
    }

    #[test]
    #[should_panic(expected = "a map's enum values must have an implicit shape")]
    fn an_enum_value_must_have_an_implicit_shape() {
        let _ = MapVt::with_enum::<
            BTreeMap<i32, Option<Color>>,
            kinds::Int32Required,
            OptionalClosed<Color>,
        >();
    }

    #[test]
    #[should_panic(expected = "wrong variant")]
    fn a_map_entry_needs_a_map_descriptor() {
        const E: Entry = Entry::new(Kind::Map, 1, 0, 0);
        // SAFETY: the table is dropped without being used.
        let _: Table<Lossy> = unsafe { Table::new(ABI, &[E], &[], &ENUM_AUX, None) };
    }
}

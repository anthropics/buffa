// Tests of the interpreters over hand-written table messages.

use super::*;
use crate::alloc::{string::String, vec, vec::Vec};
use crate::bytes::Buf;
use crate::{
    DecodeOptions, EnumValue, Enumeration, Inline, Message, MessageField, Rope, UnknownFieldData,
    UnknownFields,
};

// ---------------------------------------------------------------------------
// Test messages
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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

/// `int32 id = 1; string label = 2; Inner next = 3;`, keeping unknown fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct Inner {
    id: i32,
    label: String,
    next: MessageField<Inner>,
    unknown: UnknownFields,
}

/// Fields of most shapes the interpreters handle, plus a field number above
/// the dense lookup. `Wide` has the rest.
#[derive(Clone, Debug, Default, PartialEq)]
struct Outer {
    a: i32,
    b: Option<u64>,
    c: String,
    d: Vec<u8>,
    e: Vec<i32>,
    f: Vec<String>,
    g: MessageField<Inner>,
    h: Vec<Inner>,
    open: EnumValue<Color>,
    closed: Color,
    packed_closed: Vec<Color>,
    single: f32,
    opt_double: Option<f64>,
    flag: bool,
    unpacked_sfixed: Vec<i64>,
    zigzag: Vec<i32>,
    opt_open: Option<EnumValue<Color>>,
    opt_bytes: Option<Vec<u8>>,
    opt_str: Option<String>,
    rep_bytes: Vec<Vec<u8>>,
    required_int: i64,
    packed_fixed: Vec<u32>,
    inl: MessageField<Inner, Inline<Inner>>,
    high: u32,
    unknown: UnknownFields,
}

/// A message with only the first field of `Inner`, which drops unknown fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct Lossy {
    id: i32,
}

/// The field shapes that `Outer` does not use, in a message that drops
/// unknown fields.
#[derive(Clone, Debug, Default, PartialEq)]
struct Wide {
    sint64: i64,
    fixed64: Option<u64>,
    sfixed32: Vec<i32>,
    fixed64s: Vec<u64>,
    req_str: String,
    req_bytes: Vec<u8>,
    req_enum: EnumValue<Color>,
    rep_enum_open: Vec<EnumValue<Color>>,
    opt_closed: Option<Color>,
    req_bool: bool,
    opt_u32: Option<u32>,
    rep_double: Vec<f64>,
    rep_bool: Vec<bool>,
    rep_float: Vec<f32>,
    opt_i64: Option<i64>,
    opt_sint64: Option<i64>,
    packed_u64: Vec<u64>,
    opt_sint32: Option<i32>,
    packed_sint64: Vec<i64>,
    rep_enum_closed: Vec<Color>,
}

/// The `dense` lookup for entries numbered `numbers`, which are ascending.
const fn dense<const N: usize>(numbers: &[u32]) -> [u8; N] {
    let mut d = [0u8; N];
    let mut i = 0;
    while i < numbers.len() {
        if (numbers[i] as usize) < N {
            d[numbers[i] as usize] = (i + 1) as u8;
        }
        i += 1;
    }
    d
}

const OUTER_NUMBERS: [u32; 24] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 1000,
];
const OUTER_DENSE: [u8; 24] = dense(&OUTER_NUMBERS);
const WIDE_NUMBERS: [u32; 20] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
];
const WIDE_DENSE: [u8; 21] = dense(&WIDE_NUMBERS);

static INNER: Table<Inner> = crate::__table!(
    Inner,
    abi = ABI,
    entries = [
        crate::__table_entry!(Inner, id, Int32Implicit, 1),
        crate::__table_entry!(Inner, label, StrImplicit, 2),
        crate::__table_entry!(
            Inner,
            next,
            MsgSingular,
            3,
            aux = 0,
            slot = MessageField<Inner>
        ),
    ],
    dense = &dense::<4>(&[1, 2, 3]),
    aux = [Aux::Msg(&MsgVt::new::<MessageField<Inner>>(&INNER))],
    unknown = unknown,
);

static LOSSY: Table<Lossy> = crate::__table!(
    Lossy,
    abi = ABI,
    entries = [crate::__table_entry!(Lossy, id, Int32Implicit, 1)],
    dense = &dense::<2>(&[1]),
    aux = [],
    unknown = none,
);

static WIDE: Table<Wide> = crate::__table!(
    Wide,
    abi = ABI,
    entries = [
        crate::__table_entry!(Wide, sint64, Sint64Implicit, 1),
        crate::__table_entry!(Wide, fixed64, Fixed64Optional, 2),
        crate::__table_entry!(Wide, sfixed32, Sfixed32Packed, 3),
        crate::__table_entry!(Wide, fixed64s, Fixed64Repeated, 4),
        crate::__table_entry!(Wide, req_str, StrRequired, 5),
        crate::__table_entry!(Wide, req_bytes, BytesRequired, 6),
        crate::__table_entry!(
            Wide,
            req_enum,
            EnumRequired,
            7,
            aux = 0,
            slot = <ImplicitOpen<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(
            Wide,
            rep_enum_open,
            EnumRepeated,
            8,
            aux = 1,
            slot = <RepeatedOpen<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(
            Wide,
            opt_closed,
            EnumOptional,
            9,
            aux = 2,
            slot = <OptionalClosed<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(Wide, req_bool, BoolRequired, 10),
        crate::__table_entry!(Wide, opt_u32, Uint32Optional, 11),
        crate::__table_entry!(Wide, rep_double, DoublePacked, 12),
        crate::__table_entry!(Wide, rep_bool, BoolRepeated, 13),
        crate::__table_entry!(Wide, rep_float, FloatPacked, 14),
        crate::__table_entry!(Wide, opt_i64, Int64Optional, 15),
        crate::__table_entry!(Wide, opt_sint64, Sint64Optional, 16),
        crate::__table_entry!(Wide, packed_u64, Uint64Packed, 17),
        crate::__table_entry!(Wide, opt_sint32, Sint32Optional, 18),
        crate::__table_entry!(Wide, packed_sint64, Sint64Packed, 19),
        crate::__table_entry!(
            Wide,
            rep_enum_closed,
            EnumRepeated,
            20,
            aux = 3,
            slot = <RepeatedClosed<Color> as EnumShape>::Slot
        ),
    ],
    dense = &WIDE_DENSE,
    aux = [
        Aux::Enum(&EnumVt::new::<ImplicitOpen<Color>>()),
        Aux::Enum(&EnumVt::new::<RepeatedOpen<Color>>()),
        Aux::Enum(&EnumVt::new::<OptionalClosed<Color>>()),
        Aux::Enum(&EnumVt::new::<RepeatedClosed<Color>>()),
    ],
    unknown = none,
);

static OUTER: Table<Outer> = crate::__table!(
    Outer,
    abi = ABI,
    entries = [
        crate::__table_entry!(Outer, a, Int32Implicit, 1),
        crate::__table_entry!(Outer, b, Uint64Optional, 2),
        crate::__table_entry!(Outer, c, StrImplicit, 3),
        crate::__table_entry!(Outer, d, BytesImplicit, 4),
        crate::__table_entry!(Outer, e, Int32Packed, 5),
        crate::__table_entry!(Outer, f, StrRepeated, 6),
        crate::__table_entry!(
            Outer,
            g,
            MsgSingular,
            7,
            aux = 0,
            slot = MessageField<Inner>
        ),
        crate::__table_entry!(Outer, h, MsgRepeated, 8, aux = 1, slot = Vec<Inner>),
        crate::__table_entry!(
            Outer,
            open,
            EnumImplicit,
            9,
            aux = 2,
            slot = <ImplicitOpen<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(
            Outer,
            closed,
            EnumImplicit,
            10,
            aux = 3,
            slot = <ImplicitClosed<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(
            Outer,
            packed_closed,
            EnumPacked,
            11,
            aux = 4,
            slot = <RepeatedClosed<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(Outer, single, FloatImplicit, 12),
        crate::__table_entry!(Outer, opt_double, DoubleOptional, 13),
        crate::__table_entry!(Outer, flag, BoolImplicit, 14),
        crate::__table_entry!(Outer, unpacked_sfixed, Sfixed64Repeated, 15),
        crate::__table_entry!(Outer, zigzag, Sint32Packed, 16),
        crate::__table_entry!(
            Outer,
            opt_open,
            EnumOptional,
            17,
            aux = 5,
            slot = <OptionalOpen<Color> as EnumShape>::Slot
        ),
        crate::__table_entry!(Outer, opt_bytes, BytesOptional, 18),
        crate::__table_entry!(Outer, opt_str, StrOptional, 19),
        crate::__table_entry!(Outer, rep_bytes, BytesRepeated, 20),
        crate::__table_entry!(Outer, required_int, Int64Required, 21),
        crate::__table_entry!(Outer, packed_fixed, Fixed32Packed, 22),
        crate::__table_entry!(
            Outer,
            inl,
            MsgSingular,
            23,
            aux = 6,
            slot = MessageField<Inner, Inline<Inner>>
        ),
        crate::__table_entry!(Outer, high, Uint32Implicit, 1000),
    ],
    dense = &OUTER_DENSE,
    aux = [
        Aux::Msg(&MsgVt::new::<MessageField<Inner>>(&INNER)),
        Aux::Rep(&RepVt::new::<Inner>(&INNER)),
        Aux::Enum(&EnumVt::new::<ImplicitOpen<Color>>()),
        Aux::Enum(&EnumVt::new::<ImplicitClosed<Color>>()),
        Aux::Enum(&EnumVt::new::<RepeatedClosed<Color>>()),
        Aux::Enum(&EnumVt::new::<OptionalOpen<Color>>()),
        Aux::Msg(&MsgVt::new::<MessageField<Inner, Inline<Inner>>>(&INNER)),
    ],
    unknown = unknown,
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

table_message!(Inner, INNER);
table_message!(Outer, OUTER);
table_message!(Lossy, LOSSY);
table_message!(Wide, WIDE);

fn populated() -> Outer {
    Outer {
        a: -7,
        b: Some(0),
        c: "hello".into(),
        d: vec![0, 1, 2],
        e: vec![1, -1, 300],
        f: vec!["x".into(), String::new(), "yz".into()],
        g: MessageField::some(Inner {
            id: 5,
            label: "inner".into(),
            next: MessageField::some(Inner {
                id: 6,
                ..Inner::default()
            }),
            unknown: UnknownFields::new(),
        }),
        h: vec![
            Inner {
                id: 1,
                ..Inner::default()
            },
            Inner::default(),
        ],
        open: EnumValue::from(9),
        closed: Color::Blue,
        packed_closed: vec![Color::Green, Color::Blue, Color::Red],
        single: 1.5,
        opt_double: Some(-2.25),
        flag: true,
        unpacked_sfixed: vec![-1, 2],
        zigzag: vec![-3, 4],
        opt_open: Some(EnumValue::from(0)),
        opt_bytes: Some(Vec::new()),
        opt_str: Some("opt".into()),
        rep_bytes: vec![vec![9], Vec::new()],
        required_int: 0,
        packed_fixed: vec![7, 8],
        inl: MessageField::some(Inner {
            id: 11,
            next: MessageField::some(Inner {
                label: "deep".into(),
                ..Inner::default()
            }),
            ..Inner::default()
        }),
        high: 70_000,
        unknown: UnknownFields::new(),
    }
}

fn round_trip(msg: &Outer) -> Outer {
    let bytes = msg.encode_to_vec();
    assert_eq!(bytes.len() as u32, msg.encoded_len());
    Outer::decode_from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

#[test]
fn encodes_known_wire_bytes() {
    let msg = Outer {
        a: 150,
        c: "hi".into(),
        e: vec![3, 270],
        h: vec![Inner {
            id: 1,
            ..Inner::default()
        }],
        high: 1,
        ..Outer::default()
    };
    // a = 150; c = "hi"; e packed; h submessage {id = 1}; required_int (21) = 0
    // is always written; high (1000) has a two-byte tag.
    let expected: &[u8] = &[
        0x08, 0x96, 0x01, // 1: varint 150
        0x1a, 0x02, b'h', b'i', // 3: "hi"
        0x2a, 0x03, 0x03, 0x8e, 0x02, // 5: packed [3, 270]
        0x42, 0x02, 0x08, 0x01, // 8: {1: 1}
        0xa8, 0x01, 0x00, // 21: varint 0 (required)
        0xc0, 0x3e, 0x01, // 1000: varint 1
    ];
    assert_eq!(msg.encode_to_vec(), expected);
}

#[test]
fn an_empty_message_encodes_to_only_its_required_fields() {
    assert_eq!(Outer::default().encode_to_vec(), [0xa8, 0x01, 0x00]);
    assert_eq!(Inner::default().encode_to_vec(), Vec::<u8>::new());
}

#[test]
fn round_trips_every_shape() {
    let msg = populated();
    assert_eq!(round_trip(&msg), msg);
    assert_eq!(round_trip(&Outer::default()), Outer::default());
}

#[test]
fn every_sink_receives_the_same_bytes() {
    let msg = populated();
    let expected = msg.encode_to_vec();

    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &expected[..]);

    let mut bytes_mut = crate::bytes::BytesMut::new();
    msg.encode(&mut bytes_mut);
    assert_eq!(&bytes_mut[..], &expected[..]);

    let mut roomy = Vec::with_capacity(expected.len());
    msg.encode_length_delimited(&mut roomy);
    let mut framed = Vec::new();
    crate::encoding::encode_varint(expected.len() as u64, &mut framed);
    framed.extend_from_slice(&expected);
    assert_eq!(roomy, framed);
}

#[test]
fn a_large_bytes_field_encodes_into_a_rope() {
    let payload = vec![0xab; 64 * 1024];
    let msg = Outer {
        d: payload.clone(),
        ..Outer::default()
    };
    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &msg.encode_to_vec()[..]);
    assert!(rope.len() > payload.len());
}

#[test]
fn a_closed_enum_value_it_does_not_know_becomes_an_unknown_field() {
    // closed (10) = 7; packed_closed (11) = 7 unpacked, then packed [1, 7].
    let wire = [0x50, 0x07, 0x58, 0x07, 0x5a, 0x02, 0x01, 0x07];
    let msg = Outer::decode_from_slice(&wire).unwrap();
    assert_eq!(msg.closed, Color::Red);
    assert_eq!(msg.packed_closed, [Color::Green]);
    let unknown: Vec<_> = msg.unknown.iter().map(|u| u.number).collect();
    assert_eq!(unknown, [10, 11, 11]);
    assert!(msg
        .unknown
        .iter()
        .all(|u| matches!(u.data, UnknownFieldData::Varint(7))));
    // Known fields are written first, in field order, then the unknown ones.
    assert_eq!(
        msg.encode_to_vec(),
        [0x5a, 0x01, 0x01, 0xa8, 0x01, 0x00, 0x50, 0x07, 0x58, 0x07, 0x58, 0x07]
    );
}

#[test]
fn a_closed_enum_value_is_dropped_by_a_message_that_drops_unknown_fields() {
    // rep_enum_closed (20) = [1, 7, 2] unpacked; optional closed (9) = 7.
    let wire = [
        0xa0, 0x01, 0x01, 0xa0, 0x01, 0x07, 0xa0, 0x01, 0x02, 0x48, 0x07,
    ];
    let msg = Wide::decode_from_slice(&wire).unwrap();
    assert_eq!(msg.rep_enum_closed, [Color::Green, Color::Blue]);
    assert_eq!(msg.opt_closed, None);
}

#[test]
fn an_open_enum_keeps_a_value_it_does_not_know() {
    let msg = Outer::decode_from_slice(&[0x48, 0x09]).unwrap();
    assert_eq!(msg.open.to_i32(), 9);
    assert!(msg.unknown.is_empty());
}

#[test]
fn unknown_fields_are_preserved_or_skipped() {
    // field 500: varint 1; field 501: length-delimited "ab"
    let wire = [0xa0, 0x1f, 0x01, 0xaa, 0x1f, 0x02, b'a', b'b'];
    let msg = Inner::decode_from_slice(&wire).unwrap();
    assert_eq!(msg.unknown.len(), 2);
    assert_eq!(msg.encode_to_vec(), wire);
    assert!(matches!(
        &msg.unknown.iter().nth(1).unwrap().data,
        UnknownFieldData::LengthDelimited(v) if v == b"ab"
    ));

    let lossy = Lossy::decode_from_slice(&wire).unwrap();
    assert_eq!(lossy, Lossy::default());
    assert!(lossy.encode_to_vec().is_empty());
}

#[test]
fn repeated_scalars_accept_packed_and_unpacked_forms() {
    // e (5) is packed: [1] unpacked, [2, 3] packed, [4] unpacked.
    let wire = [0x28, 0x01, 0x2a, 0x02, 0x02, 0x03, 0x28, 0x04];
    assert_eq!(Outer::decode_from_slice(&wire).unwrap().e, [1, 2, 3, 4]);
    // unpacked_sfixed (15) is unpacked: a packed payload is also accepted.
    let mut wire = vec![0x7a, 16];
    wire.extend_from_slice(&5i64.to_le_bytes());
    wire.extend_from_slice(&(-6i64).to_le_bytes());
    assert_eq!(
        Outer::decode_from_slice(&wire).unwrap().unpacked_sfixed,
        [5, -6]
    );
}

#[test]
fn a_singular_field_takes_the_last_value_and_messages_merge() {
    // a = 1, a = 2; g = {id = 5}, g = {label = "l"}
    let wire = [
        0x08, 0x01, 0x08, 0x02, 0x3a, 0x02, 0x08, 0x05, 0x3a, 0x03, 0x12, 0x01, b'l',
    ];
    let msg = Outer::decode_from_slice(&wire).unwrap();
    assert_eq!(msg.a, 2);
    assert_eq!(msg.g.id, 5);
    assert_eq!(msg.g.label, "l");
}

#[test]
fn a_wire_type_mismatch_is_an_error() {
    // a (varint) sent as length-delimited; c (string) sent as varint.
    assert!(matches!(
        Outer::decode_from_slice(&[0x0a, 0x00]),
        Err(DecodeError::WireTypeMismatch { .. })
    ));
    assert!(matches!(
        Outer::decode_from_slice(&[0x18, 0x01]),
        Err(DecodeError::WireTypeMismatch { .. })
    ));
}

#[test]
fn invalid_utf8_in_a_string_is_an_error() {
    assert!(matches!(
        Outer::decode_from_slice(&[0x1a, 0x01, 0xff]),
        Err(DecodeError::InvalidUtf8)
    ));
}

#[test]
fn a_truncated_message_is_an_error_or_ends_on_a_field_boundary() {
    let bytes = populated().encode_to_vec();
    let mut ok = 0;
    for end in 0..bytes.len() {
        match Outer::decode_from_slice(&bytes[..end]) {
            Ok(_) => ok += 1,
            Err(e) => assert!(
                matches!(e, DecodeError::UnexpectedEof | DecodeError::VarintTooLong),
                "prefix {end}: {e}"
            ),
        }
    }
    // The empty prefix and each top-level field boundary decode; the rest fail.
    assert!(ok > 1 && ok < bytes.len() / 2);
    assert!(Outer::decode_from_slice(&bytes).is_ok());
}

#[test]
fn a_length_that_runs_past_its_message_is_an_error() {
    // g = a submessage of 2 bytes whose label declares 5.
    assert_eq!(
        Outer::decode_from_slice(&[0x3a, 0x02, 0x12, 0x05, b'a', b'b', b'c']),
        Err(DecodeError::UnexpectedEof)
    );
}

#[test]
fn a_non_contiguous_buffer_is_gathered() {
    let msg = populated();
    let bytes = msg.encode_to_vec();
    for split in [1, 2, bytes.len() / 2, bytes.len() - 1] {
        let (head, tail) = bytes.split_at(split);
        let mut chained = head.chain(tail);
        let mut decoded = Outer::default();
        with_ctx(|ctx| decoded.merge(&mut chained, ctx)).unwrap();
        assert_eq!(decoded, msg);
    }
}

#[test]
fn length_delimited_decode_leaves_the_rest_of_the_buffer() {
    let msg = populated();
    let mut framed = Vec::new();
    msg.encode_length_delimited(&mut framed);
    framed.extend_from_slice(b"tail");
    let mut buf = &framed[..];
    let decoded = Outer::decode_length_delimited(&mut buf).unwrap();
    assert_eq!(decoded, msg);
    assert_eq!(buf, b"tail");
}

fn with_ctx<R>(f: impl FnOnce(DecodeContext<'_>) -> R) -> R {
    let limit = core::cell::Cell::new(1000);
    f(DecodeContext::new(crate::RECURSION_LIMIT, &limit))
}

#[test]
fn merge_field_decodes_one_field_from_a_contiguous_buffer() {
    let wire = [0x08, 0x2a, 0x12, 0x02, b'o', b'k'];
    let mut msg = Inner::default();
    let mut buf = &wire[..];
    let tag = Tag::decode(&mut buf).unwrap();
    with_ctx(|ctx| msg.merge_field(tag, &mut buf, ctx)).unwrap();
    assert_eq!(msg.id, 42);
    assert_eq!(buf, &wire[2..]);
}

#[test]
fn merge_field_rejects_a_non_contiguous_buffer() {
    let wire = [0x2a, 0x01];
    let mut chained = (&wire[..1]).chain(&wire[1..]);
    let mut msg = Inner::default();
    assert!(matches!(
        with_ctx(|ctx| msg.merge_field(Tag::new(1, WireType::Varint), &mut chained, ctx)),
        Err(DecodeError::UnexpectedEof)
    ));
}

#[test]
fn every_remaining_shape_round_trips() {
    let msg = Wide {
        sint64: -5,
        fixed64: Some(0),
        sfixed32: vec![-1, 2],
        fixed64s: vec![3, 4],
        req_str: String::new(),
        req_bytes: vec![1],
        req_enum: EnumValue::from(4),
        rep_enum_open: vec![EnumValue::from(0), EnumValue::from(9)],
        opt_closed: Some(Color::Green),
        req_bool: false,
        opt_u32: Some(0),
        rep_double: vec![0.0, -1.5],
        rep_bool: vec![true, false],
        rep_float: vec![2.5],
        opt_i64: Some(-7),
        opt_sint64: Some(-8),
        packed_u64: vec![u64::MAX],
        opt_sint32: Some(i32::MIN),
        packed_sint64: vec![i64::MIN, 1],
        rep_enum_closed: vec![Color::Blue],
    };
    let bytes = msg.encode_to_vec();
    assert_eq!(bytes.len() as u32, msg.encoded_len());
    assert_eq!(Wide::decode_from_slice(&bytes).unwrap(), msg);
    // The required fields are written even when they hold the default.
    let empty = Wide::default().encode_to_vec();
    assert_eq!(empty, [0x2a, 0x00, 0x32, 0x00, 0x38, 0x00, 0x50, 0x00]);
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

fn nested(depth: usize) -> Vec<u8> {
    // next (3) nested `depth` levels deep, innermost empty.
    let mut wire = Vec::new();
    for _ in 0..depth {
        let mut outer = vec![0x1a];
        crate::encoding::encode_varint(wire.len() as u64, &mut outer);
        outer.extend_from_slice(&wire);
        wire = outer;
    }
    wire
}

#[test]
fn the_recursion_limit_applies() {
    let deep = nested(150);
    assert!(matches!(
        Inner::decode_from_slice(&deep),
        Err(DecodeError::RecursionLimitExceeded)
    ));
    assert!(Inner::decode_from_slice(&nested(50)).is_ok());
    assert!(matches!(
        DecodeOptions::new()
            .with_recursion_limit(10)
            .decode_from_slice::<Inner>(&nested(50)),
        Err(DecodeError::RecursionLimitExceeded)
    ));
}

#[test]
fn the_element_memory_limit_applies_to_repeated_messages_and_strings() {
    // 1000 empty elements of a repeated message field (8), then of strings (6).
    let messages: Vec<u8> = (0..1000).flat_map(|_| [0x42, 0x00]).collect();
    let strings: Vec<u8> = (0..1000).flat_map(|_| [0x32, 0x00]).collect();
    for wire in [&messages, &strings] {
        assert!(Outer::decode_from_slice(wire).is_ok());
        assert!(matches!(
            DecodeOptions::new()
                .with_element_memory_limit(100)
                .decode_from_slice::<Outer>(wire),
            Err(DecodeError::ElementMemoryLimitExceeded)
        ));
    }
}

#[test]
fn the_unknown_field_limit_applies() {
    let wire: Vec<u8> = (0..100).flat_map(|_| [0xa0, 0x1f, 0x01]).collect();
    assert!(Inner::decode_from_slice(&wire).is_ok());
    assert!(matches!(
        DecodeOptions::new()
            .with_unknown_field_limit(10)
            .decode_from_slice::<Inner>(&wire),
        Err(DecodeError::UnknownFieldLimitExceeded)
    ));
}

#[test]
fn a_closed_enum_value_counts_against_the_unknown_field_limit() {
    // closed (10) = 7, a hundred times.
    let wire: Vec<u8> = (0..100).flat_map(|_| [0x50, 0x07]).collect();
    assert!(Outer::decode_from_slice(&wire).is_ok());
    assert!(matches!(
        DecodeOptions::new()
            .with_unknown_field_limit(10)
            .decode_from_slice::<Outer>(&wire),
        Err(DecodeError::UnknownFieldLimitExceeded)
    ));
}

#[test]
fn a_sub_message_longer_than_the_size_limit_is_an_error() {
    // g (7) declares a length of 2^31.
    let wire = [0x3a, 0x80, 0x80, 0x80, 0x80, 0x08];
    assert_eq!(
        Outer::decode_from_slice(&wire),
        Err(DecodeError::MessageTooLarge)
    );
    // The same length as the prefix of a length-delimited message.
    let framed = [0x80, 0x80, 0x80, 0x80, 0x08];
    assert_eq!(
        Outer::decode_length_delimited(&mut &framed[..]),
        Err(DecodeError::MessageTooLarge)
    );
}

#[test]
fn a_repeated_message_element_that_fails_to_decode_is_not_kept() {
    // h (8): one valid element, then one whose string label is not UTF-8.
    let wire = [0x42, 0x02, 0x08, 0x01, 0x42, 0x03, 0x12, 0x01, 0xff];
    let mut msg = Outer::default();
    let result = with_ctx(|ctx| msg.merge(&mut &wire[..], ctx));
    assert_eq!(result, Err(DecodeError::InvalidUtf8));
    assert_eq!(msg.h.len(), 1);
    assert_eq!(msg.h[0].id, 1);
}

// ---------------------------------------------------------------------------
// Table construction
// ---------------------------------------------------------------------------

#[test]
fn an_entry_records_its_tag_and_the_tag_length() {
    let one = Entry::new(Kind::Int32Implicit, 15, 0, 0);
    assert_eq!((one.tag, one.tag_len), (15 << 3, 1));
    let two = Entry::new(Kind::Int32Implicit, 16, 0, 0);
    assert_eq!((two.tag, two.tag_len), (16 << 3, 2));
    let packed = Entry::new(Kind::Int32Packed, 1, 0, 0);
    assert_eq!(packed.tag, (1 << 3) | 2);
    let fixed = Entry::new(Kind::DoubleImplicit, 1, 0, 0);
    assert_eq!(fixed.tag, (1 << 3) | 1);
    let big = Entry::new(Kind::Int32Implicit, (1 << 29) - 1, 0, 0);
    assert_eq!(big.tag_len, 5);
}

#[test]
#[should_panic(expected = "field number out of range")]
fn an_entry_rejects_field_number_zero() {
    let _ = Entry::new(Kind::Int32Implicit, 0, 0, 0);
}

#[test]
fn find_uses_the_dense_array_and_falls_back_to_search() {
    let raw = &OUTER.raw;
    assert_eq!(raw.find(1).unwrap().kind, Kind::Int32Implicit);
    assert_eq!(raw.find(22).unwrap().kind, Kind::Fixed32Packed);
    assert_eq!(raw.find(23).unwrap().kind, Kind::MsgSingular);
    assert!(raw.find(24).is_none());
    assert!(raw.find(63).is_none());
    assert_eq!(raw.find(1000).unwrap().kind, Kind::Uint32Implicit);
    assert!(raw.find(1001).is_none());
}

/// Building a table that violates the checks in `Table::new` is a compile
/// error in a `static`, so these run the checks at run time.
mod invalid_tables {
    use super::*;

    const ENTRY_1: Entry = Entry::new(Kind::Int32Implicit, 1, 0, 0);
    const ENTRY_2: Entry = Entry::new(Kind::Int32Implicit, 2, 0, 0);

    /// `Table::new` on a `Lossy` (one `i32`), which is never used to access a
    /// message.
    fn lossy(
        abi: u32,
        entries: &'static [Entry],
        dense: &'static [u8],
        aux: &'static [Aux],
        unknown: Option<usize>,
    ) -> Table<Lossy> {
        // SAFETY: the table is dropped without being used.
        unsafe { Table::new(abi, entries, dense, aux, unknown) }
    }

    #[test]
    #[should_panic(expected = "different table ABI")]
    fn the_abi_must_match() {
        let _ = lossy(ABI + 1, &[ENTRY_1], &[], &[], None);
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn entries_must_be_sorted() {
        let _ = lossy(ABI, &[ENTRY_2, ENTRY_1], &[], &[], None);
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn field_numbers_must_be_distinct() {
        let _ = lossy(ABI, &[ENTRY_1, ENTRY_1], &[], &[], None);
    }

    #[test]
    #[should_panic(expected = "offset is outside the message struct")]
    fn an_offset_must_lie_inside_the_message() {
        const E: Entry = Entry::new(Kind::Int32Implicit, 1, 4, 0);
        let _ = lossy(ABI, &[E], &[], &[], None);
    }

    #[test]
    #[should_panic(expected = "aux index is out of range")]
    fn an_enum_entry_needs_an_aux() {
        const E: Entry = Entry::new(Kind::EnumImplicit, 1, 0, 0);
        let _ = lossy(ABI, &[E], &[], &[], None);
    }

    static CLOSED_VT: EnumVt = EnumVt::new::<ImplicitClosed<Color>>();
    static CLOSED_AUX: [Aux; 1] = [Aux::Enum(&CLOSED_VT)];

    #[test]
    #[should_panic(expected = "wrong variant")]
    fn an_aux_must_be_of_the_kinds_variant() {
        const E: Entry = Entry::new(Kind::MsgSingular, 1, 0, 0);
        let _ = lossy(ABI, &[E], &[], &CLOSED_AUX, None);
    }

    #[test]
    #[should_panic(expected = "wrong cardinality")]
    fn an_enum_shape_must_have_the_cardinality_of_its_kind() {
        const E: Entry = Entry::new(Kind::EnumPacked, 1, 0, 0);
        let _ = lossy(ABI, &[E], &[], &CLOSED_AUX, None);
    }

    #[test]
    fn an_enum_shape_of_the_right_cardinality_is_accepted() {
        const E: Entry = Entry::new(Kind::EnumRequired, 1, 0, 0);
        let _ = lossy(ABI, &[E], &[], &CLOSED_AUX, None);
    }

    #[test]
    #[should_panic(expected = "wrong field number")]
    fn a_dense_slot_must_name_its_entry() {
        let _ = lossy(ABI, &[ENTRY_1], &[0, 0, 1], &[], None);
    }

    #[test]
    #[should_panic(expected = "omits an entry")]
    fn the_dense_array_must_cover_every_entry_in_its_range() {
        let _ = lossy(ABI, &[ENTRY_1], &[0, 0], &[], None);
    }

    #[test]
    #[should_panic(expected = "unknown-fields offset")]
    fn the_unknown_fields_must_fit_in_the_message() {
        let _ = lossy(ABI, &[ENTRY_1], &[], &[], Some(0));
    }
}

// ---------------------------------------------------------------------------
// The `EncodeSink` hooks
// ---------------------------------------------------------------------------

#[test]
fn the_pre_sized_hook_runs_only_on_a_pre_sized_cursor() {
    use core::mem::MaybeUninit;

    let mut ran = false;
    let mut vec: Vec<u8> = Vec::new();
    assert!(!vec.__with_pre_sized(&mut |_| ran = true));
    let mut rope = Rope::new();
    assert!(!rope.__with_pre_sized(&mut |_| ran = true));
    assert!(!ran);

    let mut storage = [MaybeUninit::<u8>::uninit(); 8];
    let mut cursor = crate::encode_sink::PreSized::new(&mut storage);
    assert!(cursor.__with_pre_sized(&mut |c| c.put_u8(7)));
    assert_eq!(cursor.written(), 1);
}

#[test]
fn writing_through_a_nested_cursor_continues_after_the_bytes_already_written() {
    // A manual `write_to` that writes a prefix and then a table message must
    // append the message after the prefix, through the same cursor.
    #[derive(Clone, Default, PartialEq)]
    struct Prefixed(Inner);
    crate::impl_default_instance!(Prefixed);
    impl Message for Prefixed {
        fn compute_size(&self, cache: &mut SizeCache) -> u32 {
            2 + self.0.compute_size(cache)
        }
        fn write_to(&self, cache: &mut SizeCache, buf: &mut impl EncodeSink) {
            buf.put_slice(&[0xaa, 0xbb]);
            self.0.write_to(cache, buf);
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
    let msg = Prefixed(Inner {
        id: 3,
        label: "x".into(),
        ..Inner::default()
    });
    let mut expected = vec![0xaa, 0xbb];
    expected.extend(msg.0.encode_to_vec());
    assert_eq!(msg.encode_to_vec(), expected);
    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &expected[..]);
}

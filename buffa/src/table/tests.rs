// Tests of the interpreters over hand-written table messages.

use super::*;
use crate::alloc::{string::String, vec, vec::Vec};
use crate::bytes::Buf;
use crate::encoding::WireType;
use crate::{
    types, DecodeOptions, EnumValue, Enumeration, Inline, Message, MessageField, Rope,
    UnknownFieldData, UnknownFields,
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

// ---------------------------------------------------------------------------
// Children without a table
// ---------------------------------------------------------------------------

/// `int32 n = 1; string s = 2; Hand next = 3; Inner inner = 4;`, written by
/// hand as the unrolled codec writes a message, so that it has no table.
#[derive(Clone, Debug, Default, PartialEq)]
struct Hand {
    n: i32,
    s: String,
    next: MessageField<Hand>,
    inner: MessageField<Inner>,
    unknown: UnknownFields,
}

crate::impl_default_instance!(Hand);

impl Message for Hand {
    fn compute_size(&self, cache: &mut SizeCache) -> u32 {
        let mut size = 0u64;
        if self.n != 0 {
            size += 1 + types::int32_encoded_len(self.n) as u64;
        }
        if !self.s.is_empty() {
            size += 1 + types::string_encoded_len(&self.s) as u64;
        }
        if let Some(next) = self.next.as_option() {
            let slot = cache.reserve();
            let inner = next.compute_size(cache);
            cache.set(slot, inner);
            size += 1 + crate::encoding::varint_len(u64::from(inner)) as u64 + u64::from(inner);
        }
        if let Some(inner) = self.inner.as_option() {
            let slot = cache.reserve();
            let len = inner.compute_size(cache);
            cache.set(slot, len);
            size += 1 + crate::encoding::varint_len(u64::from(len)) as u64 + u64::from(len);
        }
        size += self.unknown.encoded_len() as u64;
        crate::saturate_size(size)
    }

    fn write_to(&self, cache: &mut SizeCache, buf: &mut impl EncodeSink) {
        if self.n != 0 {
            types::put_int32_field(1, self.n, buf);
        }
        if !self.s.is_empty() {
            types::put_string_field(2, &self.s, buf);
        }
        if let Some(next) = self.next.as_option() {
            buf.put_u8(0x1a);
            crate::encoding::encode_varint(u64::from(cache.consume_next()), buf);
            next.write_to(cache, buf);
        }
        if let Some(inner) = self.inner.as_option() {
            buf.put_u8(0x22);
            crate::encoding::encode_varint(u64::from(cache.consume_next()), buf);
            inner.write_to(cache, buf);
        }
        self.unknown.write_to(buf);
    }

    fn merge_field(
        &mut self,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        match tag.field_number() {
            1 => {
                crate::encoding::check_wire_type(tag, WireType::Varint)?;
                self.n = types::decode_int32(buf)?;
            }
            2 => {
                crate::encoding::check_wire_type(tag, WireType::LengthDelimited)?;
                self.s = types::decode_string(buf)?;
            }
            3 => {
                crate::encoding::check_wire_type(tag, WireType::LengthDelimited)?;
                self.next
                    .get_or_insert_default()
                    .merge_length_delimited(buf, ctx)?;
            }
            4 => {
                crate::encoding::check_wire_type(tag, WireType::LengthDelimited)?;
                self.inner
                    .get_or_insert_default()
                    .merge_length_delimited(buf, ctx)?;
            }
            _ => self
                .unknown
                .push(crate::encoding::decode_unknown_field(tag, buf, ctx)?),
        }
        Ok(())
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A table message whose children are reached through `Message`: a hand-written
/// message and a table message, singular, repeated and inline, next to one
/// child reached through its table.
#[derive(Clone, Debug, Default, PartialEq)]
struct Bridged {
    hand: MessageField<Hand>,
    hands: Vec<Hand>,
    tabled: MessageField<Inner>,
    tabled_list: Vec<Inner>,
    direct: MessageField<Inner>,
    hand_inline: MessageField<Hand, Inline<Hand>>,
    tail: i32,
    unknown: UnknownFields,
}

static BRIDGED: Table<Bridged> = crate::__table!(
    Bridged,
    abi = ABI,
    entries = [
        crate::__table_entry!(
            Bridged,
            hand,
            MsgSingular,
            1,
            aux = 0,
            slot = MessageField<Hand>
        ),
        crate::__table_entry!(Bridged, hands, MsgRepeated, 2, aux = 1, slot = Vec<Hand>),
        crate::__table_entry!(
            Bridged,
            tabled,
            MsgSingular,
            3,
            aux = 2,
            slot = MessageField<Inner>
        ),
        crate::__table_entry!(
            Bridged,
            tabled_list,
            MsgRepeated,
            4,
            aux = 3,
            slot = Vec<Inner>
        ),
        crate::__table_entry!(
            Bridged,
            direct,
            MsgSingular,
            5,
            aux = 4,
            slot = MessageField<Inner>
        ),
        crate::__table_entry!(
            Bridged,
            hand_inline,
            MsgSingular,
            6,
            aux = 5,
            slot = MessageField<Hand, Inline<Hand>>
        ),
        crate::__table_entry!(Bridged, tail, Int32Implicit, 7),
    ],
    dense = &dense::<8>(&[1, 2, 3, 4, 5, 6, 7]),
    aux = [
        Aux::Msg(&MsgVt::new_via_message::<MessageField<Hand>>()),
        Aux::Rep(&RepVt::new_via_message::<Hand>()),
        Aux::Msg(&MsgVt::new_via_message::<MessageField<Inner>>()),
        Aux::Rep(&RepVt::new_via_message::<Inner>()),
        Aux::Msg(&MsgVt::new::<MessageField<Inner>>(&INNER)),
        Aux::Msg(&MsgVt::new_via_message::<MessageField<Hand, Inline<Hand>>>()),
    ],
    unknown = unknown,
);

table_message!(Bridged, BRIDGED);

fn hand(n: i32, s: &str) -> Hand {
    Hand {
        n,
        s: s.into(),
        ..Hand::default()
    }
}

fn bridged() -> Bridged {
    let mut deep = hand(1, "deep");
    deep.next = MessageField::some(hand(2, ""));
    deep.inner = MessageField::some(Inner {
        id: 3,
        label: "tabled inside".into(),
        next: MessageField::some(Inner {
            id: 4,
            ..Inner::default()
        }),
        ..Inner::default()
    });
    let mut with_unknown = Inner {
        id: 9,
        ..Inner::default()
    };
    with_unknown
        .unknown
        .push(crate::UnknownField {
            number: 900,
            data: UnknownFieldData::Varint(5),
        });
    Bridged {
        hand: MessageField::some(deep),
        hands: vec![hand(7, "a"), Hand::default(), hand(0, "c")],
        tabled: MessageField::some(with_unknown),
        tabled_list: vec![
            Inner {
                id: 1,
                ..Inner::default()
            },
            Inner::default(),
        ],
        direct: MessageField::some(Inner {
            label: "direct".into(),
            ..Inner::default()
        }),
        hand_inline: MessageField::some(hand(-1, "inline")),
        tail: 70,
        unknown: UnknownFields::new(),
    }
}

#[test]
fn a_child_without_a_table_encodes_the_bytes_a_table_child_does() {
    let child = Inner {
        id: 5,
        label: "same".into(),
        ..Inner::default()
    };
    let through_message = Bridged {
        tabled: MessageField::some(child.clone()),
        ..Bridged::default()
    }
    .encode_to_vec();
    let through_table = Bridged {
        direct: MessageField::some(child.clone()),
        ..Bridged::default()
    }
    .encode_to_vec();
    // Field 3 against field 5: only the tag differs.
    assert_eq!(through_message[0], 0x1a);
    assert_eq!(through_table[0], 0x2a);
    assert_eq!(through_message[1..], through_table[1..]);

    let wire = Bridged {
        hand: MessageField::some(hand(1, "a")),
        hands: vec![hand(2, "")],
        tail: 3,
        ..Bridged::default()
    }
    .encode_to_vec();
    let expected: &[u8] = &[
        0x0a, 0x05, 0x08, 0x01, 0x12, 0x01, b'a', // 1: {n: 1, s: "a"}
        0x12, 0x02, 0x08, 0x02, // 2: {n: 2}
        0x38, 0x03, // 7: 3
    ];
    assert_eq!(wire, expected);
}

#[test]
fn children_without_a_table_round_trip() {
    let msg = bridged();
    let bytes = msg.encode_to_vec();
    assert_eq!(bytes.len() as u32, msg.encoded_len());
    assert_eq!(Bridged::decode_from_slice(&bytes).unwrap(), msg);
    assert_eq!(
        Bridged::decode_from_slice(&Bridged::default().encode_to_vec()).unwrap(),
        Bridged::default()
    );
}

#[test]
fn a_child_without_a_table_reaches_every_sink() {
    let msg = bridged();
    let expected = msg.encode_to_vec();

    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &expected[..]);

    let mut bytes_mut = crate::bytes::BytesMut::new();
    msg.encode(&mut bytes_mut);
    assert_eq!(&bytes_mut[..], &expected[..]);

    let mut roomy = Vec::with_capacity(expected.len());
    msg.encode(&mut roomy);
    assert_eq!(roomy, expected);

    // A sink whose chunk is shorter than the message receives the whole
    // message from a scratch buffer.
    let mut small = crate::bytes::BytesMut::with_capacity(1);
    msg.encode_length_delimited(&mut small);
    let mut framed = Vec::new();
    crate::encoding::encode_varint(expected.len() as u64, &mut framed);
    framed.extend_from_slice(&expected);
    assert_eq!(&small[..], &framed[..]);
}

#[test]
fn children_without_a_table_decode_from_a_non_contiguous_buffer() {
    let msg = bridged();
    let bytes = msg.encode_to_vec();
    for split in 1..bytes.len() {
        let (head, tail) = bytes.split_at(split);
        let mut chained = head.chain(tail);
        let mut decoded = Bridged::default();
        with_ctx(|ctx| decoded.merge(&mut chained, ctx)).unwrap();
        assert_eq!(decoded, msg, "split at {split}");
    }
}

#[test]
fn a_singular_child_without_a_table_merges() {
    let mut msg = Bridged::default();
    for wire in [
        // hand {n = 1, s = "x"}, then hand {s = "y", next = {n = 2}}.
        &[0x0a, 0x05, 0x08, 0x01, 0x12, 0x01, b'x'][..],
        &[0x0a, 0x07, 0x12, 0x01, b'y', 0x1a, 0x02, 0x08, 0x02][..],
    ] {
        with_ctx(|ctx| msg.merge(&mut &wire[..], ctx)).unwrap();
    }
    let hand = msg.hand.as_option().unwrap();
    assert_eq!((hand.n, hand.s.as_str()), (1, "y"));
    assert_eq!(hand.next.as_option().unwrap().n, 2);
}

#[test]
fn unknown_fields_in_a_child_without_a_table_are_kept() {
    let msg = bridged();
    let decoded = Bridged::decode_from_slice(&msg.encode_to_vec()).unwrap();
    let tabled = decoded.tabled.as_option().unwrap();
    assert_eq!(tabled.unknown.iter().count(), 1);
}

#[test]
fn the_recursion_limit_applies_through_a_child_without_a_table() {
    let wrap = |field: u8, inner: Vec<u8>| {
        let mut wire = vec![field];
        crate::encoding::encode_varint(inner.len() as u64, &mut wire);
        wire.extend(inner);
        wire
    };
    // The child is a hand-written message, so its own decode counts the depth.
    assert!(Bridged::decode_from_slice(&wrap(0x0a, nested(50))).is_ok());
    assert!(matches!(
        Bridged::decode_from_slice(&wrap(0x0a, nested(150))),
        Err(DecodeError::RecursionLimitExceeded)
    ));
    // And a table child reached through `Message` counts it too.
    assert!(Bridged::decode_from_slice(&wrap(0x1a, nested(50))).is_ok());
    assert!(matches!(
        Bridged::decode_from_slice(&wrap(0x1a, nested(150))),
        Err(DecodeError::RecursionLimitExceeded)
    ));
    assert!(matches!(
        DecodeOptions::new()
            .with_recursion_limit(10)
            .decode_from_slice::<Bridged>(&wrap(0x0a, nested(50))),
        Err(DecodeError::RecursionLimitExceeded)
    ));
}

#[test]
fn the_element_memory_limit_applies_to_repeated_children_without_a_table() {
    // 1000 empty elements of `hands` (2), then of `tabled_list` (4).
    let hands: Vec<u8> = (0..1000).flat_map(|_| [0x12, 0x00]).collect();
    let tabled: Vec<u8> = (0..1000).flat_map(|_| [0x22, 0x00]).collect();
    for wire in [&hands, &tabled] {
        assert!(Bridged::decode_from_slice(wire).is_ok());
        assert!(matches!(
            DecodeOptions::new()
                .with_element_memory_limit(100)
                .decode_from_slice::<Bridged>(wire),
            Err(DecodeError::ElementMemoryLimitExceeded)
        ));
    }
}

#[test]
fn a_repeated_child_without_a_table_that_fails_to_decode_is_not_kept() {
    // `hands`: one valid element, then one whose string is not UTF-8.
    let wire = [0x12, 0x02, 0x08, 0x01, 0x12, 0x03, 0x12, 0x01, 0xff];
    let mut msg = Bridged::default();
    let result = with_ctx(|ctx| msg.merge(&mut &wire[..], ctx));
    assert_eq!(result, Err(DecodeError::InvalidUtf8));
    assert_eq!(msg.hands.len(), 1);
    assert_eq!(msg.hands[0].n, 1);
}

#[test]
fn a_truncated_child_without_a_table_is_an_error_or_ends_on_a_field_boundary() {
    let bytes = bridged().encode_to_vec();
    let mut ok = 0;
    for end in 0..bytes.len() {
        match Bridged::decode_from_slice(&bytes[..end]) {
            Ok(_) => ok += 1,
            Err(e) => assert!(
                matches!(e, DecodeError::UnexpectedEof | DecodeError::VarintTooLong),
                "prefix {end}: {e}"
            ),
        }
    }
    assert!(ok > 1 && ok < bytes.len() / 2);
    assert!(Bridged::decode_from_slice(&bytes).is_ok());
}

#[test]
fn a_length_that_runs_past_a_child_without_a_table_is_an_error() {
    // `hand` is 2 bytes and its string declares 5.
    assert_eq!(
        Bridged::decode_from_slice(&[0x0a, 0x02, 0x12, 0x05, b'a', b'b', b'c']),
        Err(DecodeError::UnexpectedEof)
    );
    // A child longer than the size limit.
    assert_eq!(
        Bridged::decode_from_slice(&[0x0a, 0x80, 0x80, 0x80, 0x80, 0x08]),
        Err(DecodeError::MessageTooLarge)
    );
}

/// Declares `size` bytes and writes `writes` of them.
#[derive(Clone, Default, PartialEq)]
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

#[derive(Clone, Default, PartialEq)]
struct HoldsLiar {
    liar: MessageField<Liar>,
}

static HOLDS_LIAR: Table<HoldsLiar> = crate::__table!(
    HoldsLiar,
    abi = ABI,
    entries = [crate::__table_entry!(
        HoldsLiar,
        liar,
        MsgSingular,
        1,
        aux = 0,
        slot = MessageField<Liar>
    )],
    dense = &dense::<2>(&[1]),
    aux = [Aux::Msg(&MsgVt::new_via_message::<MessageField<Liar>>())],
    unknown = none,
);
table_message!(HoldsLiar, HOLDS_LIAR);

fn holds_liar(size: u32, writes: u8) -> HoldsLiar {
    HoldsLiar {
        liar: MessageField::some(Liar { size, writes }),
    }
}

#[test]
#[should_panic(expected = "more bytes than compute_size declared")]
fn a_child_that_writes_more_than_it_sized_panics_in_the_scratch_buffer() {
    // A Rope is not written through the cursor, so the child is staged in a
    // buffer of the size `compute_size` gave, which the write overruns.
    holds_liar(0, 1).encode(&mut Rope::new());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "different byte count than compute_size declared")]
fn a_child_that_writes_less_than_it_sized_panics_in_debug_builds() {
    holds_liar(3, 1).encode(&mut Rope::new());
}

#[test]
fn a_child_can_be_written_by_calling_write_to_on_a_buffer() {
    // `write_to` on a `Vec` is not the pre-sized path `encode` takes, so each
    // child is staged and copied. The bytes are the same.
    let msg = bridged();
    let mut cache = SizeCache::new();
    let size = msg.compute_size(&mut cache);
    let mut out = Vec::new();
    msg.write_to(&mut cache, &mut out);
    assert_eq!(out.len(), size as usize);
    assert_eq!(out, msg.encode_to_vec());
}

// ---------------------------------------------------------------------------
// Oneofs
// ---------------------------------------------------------------------------

/// `oneof pick { int32 num = 2; string text = 4; bytes blob = 5; Color open = 7;
/// Color strict = 8; Inner child = 9; }`, with the message boxed.
#[derive(Clone, Debug, PartialEq)]
enum Pick {
    Num(i32),
    Text(String),
    Blob(Vec<u8>),
    Open(EnumValue<Color>),
    Strict(Color),
    Child(crate::alloc::boxed::Box<Inner>),
}

impl OneofEnum for Pick {
    fn number(&self) -> u32 {
        match self {
            Self::Num(_) => 2,
            Self::Text(_) => 4,
            Self::Blob(_) => 5,
            Self::Open(_) => 7,
            Self::Strict(_) => 8,
            Self::Child(_) => 9,
        }
    }

    fn payload(&self) -> *const u8 {
        match self {
            Self::Num(v) => (v as *const i32).cast(),
            Self::Text(v) => (v as *const String).cast(),
            Self::Blob(v) => (v as *const Vec<u8>).cast(),
            Self::Open(v) => (v as *const EnumValue<Color>).cast(),
            Self::Strict(v) => (v as *const Color).cast(),
            Self::Child(v) => (&**v as *const Inner).cast(),
        }
    }

    fn payload_mut(&mut self) -> *mut u8 {
        match self {
            Self::Num(v) => (v as *mut i32).cast(),
            Self::Text(v) => (v as *mut String).cast(),
            Self::Blob(v) => (v as *mut Vec<u8>).cast(),
            Self::Open(v) => (v as *mut EnumValue<Color>).cast(),
            Self::Strict(v) => (v as *mut Color).cast(),
            Self::Child(v) => (&mut **v as *mut Inner).cast(),
        }
    }

    fn with_default(number: u32) -> Option<Self> {
        Some(match number {
            2 => Self::Num(Default::default()),
            4 => Self::Text(Default::default()),
            5 => Self::Blob(Default::default()),
            7 => Self::Open(Default::default()),
            8 => Self::Strict(Default::default()),
            9 => Self::Child(Default::default()),
            _ => return None,
        })
    }
}

/// `oneof other { float x = 6; Inner y = 10; }`, with the message inline.
#[derive(Clone, Debug, PartialEq)]
enum Alt {
    X(f32),
    Y(Inner),
}

impl OneofEnum for Alt {
    fn number(&self) -> u32 {
        match self {
            Self::X(_) => 6,
            Self::Y(_) => 10,
        }
    }

    fn payload(&self) -> *const u8 {
        match self {
            Self::X(v) => (v as *const f32).cast(),
            Self::Y(v) => (v as *const Inner).cast(),
        }
    }

    fn payload_mut(&mut self) -> *mut u8 {
        match self {
            Self::X(v) => (v as *mut f32).cast(),
            Self::Y(v) => (v as *mut Inner).cast(),
        }
    }

    fn with_default(number: u32) -> Option<Self> {
        Some(match number {
            6 => Self::X(Default::default()),
            10 => Self::Y(Default::default()),
            _ => return None,
        })
    }
}

/// `int32 a = 1; oneof pick {...}; int32 b = 3; oneof other {...}`, keeping
/// unknown fields. `pick` has members either side of `b`, so where its
/// members are written shows.
#[derive(Clone, Debug, Default, PartialEq)]
struct Holder {
    a: i32,
    pick: Option<Pick>,
    b: i32,
    other: Option<Alt>,
    unknown: UnknownFields,
}

static HOLDER: Table<Holder> = crate::__table!(
    Holder,
    abi = ABI,
    entries = [
        crate::__table_entry!(Holder, a, Int32Implicit, 1),
        crate::__table_entry!(
            Holder,
            pick,
            oneof(Int32Required, true),
            2,
            aux = 2,
            slot = Option<Pick>
        ),
        crate::__table_entry!(Holder, b, Int32Implicit, 3),
        crate::__table_entry!(
            Holder,
            pick,
            oneof(StrRequired, false),
            4,
            aux = 3,
            slot = Option<Pick>
        ),
        crate::__table_entry!(
            Holder,
            pick,
            oneof(BytesRequired, false),
            5,
            aux = 4,
            slot = Option<Pick>
        ),
        crate::__table_entry!(
            Holder,
            other,
            oneof(FloatRequired, true),
            6,
            aux = 5,
            slot = Option<Alt>
        ),
        crate::__table_entry!(
            Holder,
            pick,
            oneof(EnumRequired, false),
            7,
            aux = 6,
            slot = Option<Pick>
        ),
        crate::__table_entry!(
            Holder,
            pick,
            oneof(EnumRequired, false),
            8,
            aux = 7,
            slot = Option<Pick>
        ),
        crate::__table_entry!(
            Holder,
            pick,
            oneof(MsgSingular, false),
            9,
            aux = 10,
            slot = Option<Pick>
        ),
        crate::__table_entry!(
            Holder,
            other,
            oneof(MsgSingular, false),
            10,
            aux = 12,
            slot = Option<Alt>
        ),
    ],
    dense = &dense::<11>(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
    aux = [
        Aux::Group(&OneofVt::new::<Pick>(
            crate::table::offset_of!(Holder, pick),
            2
        )),
        Aux::Group(&OneofVt::new::<Alt>(
            crate::table::offset_of!(Holder, other),
            6
        )),
        Aux::Member(Member::new(0, Kind::Int32Required, 0)),
        Aux::Member(Member::new(0, Kind::StrRequired, 0)),
        Aux::Member(Member::new(0, Kind::BytesRequired, 0)),
        Aux::Member(Member::new(1, Kind::FloatRequired, 0)),
        Aux::Member(Member::new(0, Kind::EnumRequired, 8)),
        Aux::Member(Member::new(0, Kind::EnumRequired, 9)),
        Aux::Enum(&EnumVt::new::<ImplicitOpen<Color>>()),
        Aux::Enum(&EnumVt::new::<ImplicitClosed<Color>>()),
        Aux::Member(Member::new(0, Kind::MsgSingular, 11)),
        Aux::Msg(&MsgVt::direct(&INNER)),
        Aux::Member(Member::new(1, Kind::MsgSingular, 11)),
    ],
    unknown = unknown,
);

table_message!(Holder, HOLDER);

fn child(id: i32) -> Pick {
    Pick::Child(crate::alloc::boxed::Box::new(Inner {
        id,
        ..Inner::default()
    }))
}

fn holder(pick: Pick) -> Holder {
    Holder {
        pick: Some(pick),
        ..Holder::default()
    }
}

#[test]
fn a_oneof_is_written_where_its_lowest_member_would_be() {
    // a = 1; the child (9) of `pick`, whose lowest member is 2, comes before b
    // = 7 (3), not after it.
    let msg = Holder {
        a: 1,
        pick: Some(child(5)),
        b: 7,
        ..Holder::default()
    };
    let wire = [0x08, 0x01, 0x4a, 0x02, 0x08, 0x05, 0x18, 0x07];
    assert_eq!(msg.encode_to_vec(), wire);
    assert_eq!(msg.encoded_len() as usize, wire.len());
    assert_eq!(Holder::decode_from_slice(&wire).unwrap(), msg);

    // The other oneof is written at 6, after b, when it is the one that is set.
    let msg = Holder {
        b: 7,
        other: Some(Alt::X(1.0)),
        ..Holder::default()
    };
    assert_eq!(
        msg.encode_to_vec(),
        [0x18, 0x07, 0x35, 0x00, 0x00, 0x80, 0x3f]
    );
}

#[test]
fn every_member_round_trips_and_a_default_value_is_still_written() {
    let picks = [
        Pick::Num(0),
        Pick::Num(-1),
        Pick::Text(String::new()),
        Pick::Text("héllo".into()),
        Pick::Blob(Vec::new()),
        Pick::Blob(vec![1, 2, 3]),
        Pick::Open(EnumValue::from(0)),
        Pick::Open(EnumValue::from(9)),
        Pick::Strict(Color::Red),
        Pick::Strict(Color::Blue),
        Pick::Child(Default::default()),
        child(3),
    ];
    for pick in picks {
        let msg = holder(pick);
        let wire = msg.encode_to_vec();
        assert!(!wire.is_empty(), "{msg:?} wrote nothing");
        assert_eq!(wire.len() as u32, msg.encoded_len());
        assert_eq!(Holder::decode_from_slice(&wire).unwrap(), msg);
    }
    for other in [
        Alt::X(0.0),
        Alt::X(-2.5),
        Alt::Y(Inner::default()),
        Alt::Y(Inner {
            id: 4,
            label: "y".into(),
            ..Inner::default()
        }),
    ] {
        let msg = Holder {
            other: Some(other),
            ..Holder::default()
        };
        let wire = msg.encode_to_vec();
        assert!(!wire.is_empty());
        assert_eq!(Holder::decode_from_slice(&wire).unwrap(), msg);
    }
    assert!(Holder::default().encode_to_vec().is_empty());
    assert_eq!(holder(Pick::Num(0)).encode_to_vec(), [0x10, 0x00]);
}

#[test]
fn the_last_member_on_the_wire_wins() {
    // num = 5, then text = "hi".
    let wire = [0x10, 0x05, 0x22, 0x02, b'h', b'i'];
    assert_eq!(
        Holder::decode_from_slice(&wire).unwrap().pick,
        Some(Pick::Text("hi".into()))
    );
    // text, then num, then num again.
    let wire = [0x22, 0x02, b'h', b'i', 0x10, 0x05, 0x10, 0x06];
    assert_eq!(
        Holder::decode_from_slice(&wire).unwrap().pick,
        Some(Pick::Num(6))
    );
    // The two oneofs are independent.
    let wire = [0x10, 0x05, 0x35, 0x00, 0x00, 0x00, 0x40];
    let msg = Holder::decode_from_slice(&wire).unwrap();
    assert_eq!(msg.pick, Some(Pick::Num(5)));
    assert_eq!(msg.other, Some(Alt::X(2.0)));
}

#[test]
fn a_message_member_that_is_set_is_merged_into_and_a_different_one_replaces_it() {
    // child {id = 5}, child {label = "ab"}: merged.
    let wire = [0x4a, 0x02, 0x08, 0x05, 0x4a, 0x04, 0x12, 0x02, b'a', b'b'];
    let merged = Holder::decode_from_slice(&wire).unwrap();
    assert_eq!(
        merged.pick,
        Some(Pick::Child(crate::alloc::boxed::Box::new(Inner {
            id: 5,
            label: "ab".into(),
            ..Inner::default()
        })))
    );
    // Then num = 7 replaces the child, and a new child starts from nothing.
    let mut wire = wire.to_vec();
    wire.extend_from_slice(&[0x10, 0x07]);
    assert_eq!(
        Holder::decode_from_slice(&wire).unwrap().pick,
        Some(Pick::Num(7))
    );
    wire.extend_from_slice(&[0x4a, 0x02, 0x08, 0x01]);
    assert_eq!(Holder::decode_from_slice(&wire).unwrap().pick, Some(child(1)));

    // The same for the inline message of the other oneof.
    let wire = [0x52, 0x02, 0x08, 0x05, 0x52, 0x04, 0x12, 0x02, b'a', b'b'];
    assert_eq!(
        Holder::decode_from_slice(&wire).unwrap().other,
        Some(Alt::Y(Inner {
            id: 5,
            label: "ab".into(),
            ..Inner::default()
        }))
    );
}

#[test]
fn merging_into_a_message_keeps_a_member_that_the_input_does_not_set() {
    let mut msg = holder(Pick::Text("keep".into()));
    msg.merge_from_slice(&[0x08, 0x02]).unwrap();
    assert_eq!(msg.a, 2);
    assert_eq!(msg.pick, Some(Pick::Text("keep".into())));
}

#[test]
fn a_closed_enum_value_the_member_does_not_know_leaves_the_oneof_alone() {
    // text = "hi", then strict (8) = 7, which Color has no variant for.
    let wire = [0x22, 0x02, b'h', b'i', 0x40, 0x07];
    let msg = Holder::decode_from_slice(&wire).unwrap();
    assert_eq!(msg.pick, Some(Pick::Text("hi".into())));
    let unknown: Vec<_> = msg.unknown.iter().map(|u| u.number).collect();
    assert_eq!(unknown, [8]);
    assert!(matches!(
        msg.unknown.iter().next().unwrap().data,
        UnknownFieldData::Varint(7)
    ));
    // Nothing was set before, and nothing is now.
    assert_eq!(Holder::decode_from_slice(&[0x40, 0x07]).unwrap().pick, None);
    // A known value replaces the member, and an open enum keeps any value.
    assert_eq!(
        Holder::decode_from_slice(&[0x22, 0x00, 0x40, 0x02])
            .unwrap()
            .pick,
        Some(Pick::Strict(Color::Blue))
    );
    assert_eq!(
        Holder::decode_from_slice(&[0x22, 0x00, 0x38, 0x09])
            .unwrap()
            .pick,
        Some(Pick::Open(EnumValue::from(9)))
    );
}

#[test]
fn a_rejected_member_value_leaves_the_oneof_as_it_was() {
    // For every member: a value that is cut short, a string that is not UTF-8,
    // and a wire type the member does not have. Each fails without touching
    // the member that is set, whichever it is, or setting one.
    let failing: [&[u8]; 9] = [
        &[0x10, 0x80],
        &[0x12, 0x00],
        &[0x22, 0x01, 0xff],
        &[0x22, 0x05, b'a'],
        &[0x2a, 0x05, 0x01],
        &[0x38, 0x80],
        &[0x40, 0x80],
        &[0x4a, 0x05, 0x08],
        &[0x4a],
    ];
    let initial: [fn() -> Option<Pick>; 3] = [
        || Some(Pick::Num(5)),
        || Some(Pick::Text("abc".into())),
        || None,
    ];
    for initial in initial {
        for wire in failing {
            let mut msg = Holder {
                pick: initial(),
                ..Holder::default()
            };
            assert!(msg.merge_from_slice(wire).is_err(), "{wire:02x?}");
            assert_eq!(msg.pick, initial(), "{wire:02x?}");
        }
    }
    // The float member of the other oneof, in the same way.
    for wire in [&[0x35, 0x00][..], &[0x32, 0x00][..]] {
        let mut msg = Holder {
            other: Some(Alt::Y(Inner::default())),
            ..Holder::default()
        };
        assert!(msg.merge_from_slice(wire).is_err(), "{wire:02x?}");
        assert_eq!(msg.other, Some(Alt::Y(Inner::default())), "{wire:02x?}");
    }
    assert!(matches!(
        Holder::decode_from_slice(&[0x22, 0x01, 0xff]),
        Err(DecodeError::InvalidUtf8)
    ));
}

#[test]
fn a_member_with_the_wrong_wire_type_is_an_error() {
    // num (varint) sent as length-delimited; text sent as a varint; a message
    // sent as a fixed32.
    for wire in [[0x12, 0x00], [0x20, 0x01], [0x4d, 0x00]] {
        assert!(matches!(
            Holder::decode_from_slice(&wire),
            Err(DecodeError::WireTypeMismatch { .. })
        ));
    }
}

#[test]
fn a_message_member_that_is_cut_short_is_an_error() {
    let wire = holder(child(5)).encode_to_vec();
    for end in 0..wire.len() {
        // A prefix ends the message at a field boundary or is an error, and
        // never yields a member the input did not hold.
        if let Ok(msg) = Holder::decode_from_slice(&wire[..end]) {
            assert_eq!(msg, Holder::default(), "prefix of {end}");
        }
    }
}

#[test]
fn a_message_member_that_fails_leaves_a_different_member_as_it_was() {
    // The child (9) has no length, is longer than the input, or holds a field
    // that does not decode; the oneof keeps the text.
    for wire in [
        &[0x4a][..],
        &[0x4a, 0x05, 0x08][..],
        &[0x4a, 0x02, 0x08, 0x80][..],
        &[0x4a, 0x03, 0x08, 0x05, 0x10][..],
    ] {
        let mut msg = holder(Pick::Text("abc".into()));
        assert!(msg.merge_from_slice(wire).is_err(), "{wire:02x?}");
        assert_eq!(msg.pick, Some(Pick::Text("abc".into())), "{wire:02x?}");
        let mut none = Holder::default();
        assert!(none.merge_from_slice(wire).is_err(), "{wire:02x?}");
        assert_eq!(none.pick, None, "{wire:02x?}");
    }
}

#[test]
fn a_message_member_that_fails_keeps_what_it_merged_into_the_member_that_is_set() {
    // id = 5 merges into the child, and then the label has the wrong wire
    // type, as it would for a singular message field.
    let mut msg = holder(child(1));
    assert!(msg
        .merge_from_slice(&[0x4a, 0x03, 0x08, 0x05, 0x10])
        .is_err());
    assert_eq!(msg.pick, Some(child(5)));
}

#[test]
fn a_message_member_past_the_recursion_limit_is_an_error_that_changes_nothing() {
    let mut msg = holder(Pick::Text("abc".into()));
    let wire = [0x4a, 0x02, 0x08, 0x05];
    let err = crate::DecodeOptions::new()
        .with_recursion_limit(0)
        .merge_from_slice(&mut msg, &wire)
        .unwrap_err();
    assert_eq!(err, DecodeError::RecursionLimitExceeded);
    assert_eq!(msg.pick, Some(Pick::Text("abc".into())));
}

#[test]
fn a_message_member_larger_than_the_limit_is_an_error_that_changes_nothing() {
    let mut msg = holder(Pick::Text("abc".into()));
    // A length of 2 GiB.
    let wire = [0x4a, 0x80, 0x80, 0x80, 0x80, 0x08];
    assert_eq!(
        msg.merge_from_slice(&wire),
        Err(DecodeError::MessageTooLarge)
    );
    assert_eq!(msg.pick, Some(Pick::Text("abc".into())));
}

#[test]
fn oneof_members_decode_from_a_buffer_of_two_chunks() {
    let msg = Holder {
        a: 1,
        pick: Some(Pick::Text("split".into())),
        b: 2,
        other: Some(Alt::Y(Inner {
            id: 3,
            label: "in".into(),
            ..Inner::default()
        })),
        ..Holder::default()
    };
    let wire = msg.encode_to_vec();
    for split in 0..=wire.len() {
        let (head, tail) = wire.split_at(split);
        let mut chained = head.chain(tail);
        let mut decoded = Holder::default();
        with_ctx(|ctx| decoded.merge(&mut chained, ctx)).unwrap();
        assert_eq!(decoded, msg, "split at {split}");
    }
}

#[test]
fn oneof_members_encode_into_every_sink() {
    let msg = Holder {
        a: 1,
        pick: Some(Pick::Blob(vec![0xcd; 300])),
        other: Some(Alt::Y(Inner {
            id: 3,
            ..Inner::default()
        })),
        ..Holder::default()
    };
    let expected = msg.encode_to_vec();
    let mut rope = Rope::new();
    msg.encode(&mut rope);
    assert_eq!(&rope.to_contiguous_bytes()[..], &expected[..]);
    let mut bytes_mut = crate::bytes::BytesMut::new();
    msg.encode(&mut bytes_mut);
    assert_eq!(&bytes_mut[..], &expected[..]);
}

#[test]
fn find_reaches_every_oneof_member() {
    for (number, kind) in [(2, Kind::OneofLeader), (9, Kind::OneofFollower)] {
        assert_eq!(HOLDER.raw.find(number).unwrap().kind, kind);
    }
    let pe = HOLDER.raw.payload_entry(0, 4);
    assert_eq!(pe.kind, Kind::StrRequired);
    assert_eq!(pe.number(), 4);
    let pe = HOLDER.raw.payload_entry(0, 9);
    assert_eq!(pe.kind, Kind::MsgSingular);
    assert_eq!(pe.aux, 11);
}

#[test]
#[should_panic(expected = "does not match the oneof enum")]
fn a_member_of_another_oneof_is_not_a_member_of_this_one() {
    // 6 is a member of `other`, which is group 1.
    let _ = HOLDER.raw.payload_entry(0, 6);
}

#[test]
#[should_panic(expected = "does not match the oneof enum")]
fn a_field_that_is_not_in_a_oneof_is_not_a_member() {
    let _ = HOLDER.raw.payload_entry(0, 1);
}

#[test]
fn the_accessors_keep_a_member_that_is_set_and_replace_any_other() {
    let vt = OneofVt::new::<Pick>(0, 2);
    let mut slot = Some(Pick::Text("kept".into()));
    let slot_ptr = (&mut slot as *mut Option<Pick>).cast::<u8>();
    // SAFETY: `slot` is a live `Option<Pick>` and 4 and 2 are members of it.
    unsafe {
        let (number, payload) = (vt.get)(slot_ptr);
        assert_eq!(number, 4);
        assert_eq!(*payload.cast::<String>(), "kept");
        let payload = (vt.place)(slot_ptr, 4);
        assert_eq!(*payload.cast::<String>(), "kept");
        let payload = (vt.place)(slot_ptr, 2);
        assert_eq!(*payload.cast::<i32>(), 0);
        *payload.cast::<i32>() = 8;
    }
    assert_eq!(slot, Some(Pick::Num(8)));
    let mut none: Option<Pick> = None;
    // SAFETY: as above.
    let (number, payload) = unsafe { (vt.get)((&mut none as *mut Option<Pick>).cast()) };
    assert_eq!((number, payload.is_null()), (0, true));
}

#[test]
fn place_with_merges_into_the_member_that_is_set_and_replaces_another_only_on_success() {
    let vt = OneofVt::new::<Pick>(0, 2);
    let mut slot = Some(Pick::Text("kept".into()));
    let slot_ptr = (&mut slot as *mut Option<Pick>).cast::<u8>();
    // SAFETY: `slot` is a live `Option<Pick>` and 4, 2 and 5 are members.
    unsafe {
        // The member that is set is decoded into where it is, and stays set
        // when the decoding fails part of the way.
        let r = (vt.place_with)(slot_ptr, 4, &mut |p| {
            p.cast::<String>().as_mut().unwrap().push_str("+more");
            Err(DecodeError::UnexpectedEof)
        });
        assert_eq!(r, Err(DecodeError::UnexpectedEof));
        assert_eq!(slot, Some(Pick::Text("kept+more".into())));
        // Another member is decoded into a new default one, which a failure
        // discards.
        let r = (vt.place_with)(slot_ptr, 2, &mut |p| {
            assert_eq!(*p.cast::<i32>(), 0);
            *p.cast::<i32>() = 9;
            Err(DecodeError::UnexpectedEof)
        });
        assert_eq!(r, Err(DecodeError::UnexpectedEof));
        assert_eq!(slot, Some(Pick::Text("kept+more".into())));
        // Success replaces it.
        let r = (vt.place_with)(slot_ptr, 2, &mut |p| {
            *p.cast::<i32>() = 9;
            Ok(())
        });
        assert_eq!(r, Ok(()));
        assert_eq!(slot, Some(Pick::Num(9)));
    }
    // An oneof that is unset stays unset on failure.
    let mut none: Option<Pick> = None;
    // SAFETY: as above.
    let r = unsafe {
        (vt.place_with)((&mut none as *mut Option<Pick>).cast(), 5, &mut |_| {
            Err(DecodeError::UnexpectedEof)
        })
    };
    assert_eq!(r, Err(DecodeError::UnexpectedEof));
    assert_eq!(none, None);
}

#[test]
#[should_panic(expected = "does not match the oneof enum")]
fn place_with_a_member_the_enum_does_not_have_panics() {
    let vt = OneofVt::new::<Pick>(0, 2);
    let mut slot: Option<Pick> = None;
    // SAFETY: `slot` is a live `Option<Pick>`; 99 is not a member, which the
    // accessor reports by panicking.
    let _ = unsafe { (vt.place_with)((&mut slot as *mut Option<Pick>).cast(), 99, &mut |_| Ok(())) };
}

/// An enum whose `with_default` returns a member other than the one asked
/// for, which breaks the contract of [`OneofEnum`].
#[derive(Debug, PartialEq)]
struct LyingEnum;

impl OneofEnum for LyingEnum {
    fn number(&self) -> u32 {
        1
    }
    fn payload(&self) -> *const u8 {
        core::ptr::null()
    }
    fn payload_mut(&mut self) -> *mut u8 {
        core::ptr::null_mut()
    }
    fn with_default(_number: u32) -> Option<Self> {
        Some(LyingEnum)
    }
}

#[test]
#[should_panic(expected = "does not match the oneof enum")]
fn placing_a_member_whose_default_has_another_number_panics() {
    let vt = OneofVt::new::<LyingEnum>(0, 1);
    let mut slot: Option<LyingEnum> = None;
    // SAFETY: `slot` is a live `Option<LyingEnum>`; the accessor checks the
    // number of what `with_default` returns.
    unsafe { (vt.place)((&mut slot as *mut Option<LyingEnum>).cast(), 2) };
}

#[test]
#[should_panic(expected = "does not match the oneof enum")]
fn place_with_a_member_whose_default_has_another_number_panics() {
    let vt = OneofVt::new::<LyingEnum>(0, 1);
    let mut slot: Option<LyingEnum> = None;
    // SAFETY: as above.
    let _ = unsafe { (vt.place_with)((&mut slot as *mut Option<LyingEnum>).cast(), 2, &mut |_| Ok(())) };
}

#[test]
#[should_panic(expected = "does not match the oneof enum")]
fn placing_a_member_the_enum_does_not_have_panics() {
    let vt = OneofVt::new::<Pick>(0, 2);
    let mut slot: Option<Pick> = None;
    // SAFETY: `slot` is a live `Option<Pick>`; 99 is not a member, which the
    // accessor reports by panicking.
    unsafe { (vt.place)((&mut slot as *mut Option<Pick>).cast(), 99) };
}

/// Building a table that violates the checks on oneofs is a compile error in
/// a `static`, so these run the checks at run time.
mod invalid_oneof_tables {
    use super::*;

    static PICK: OneofVt = OneofVt::new::<Pick>(0, 2);
    static INT_MEMBERS: [Aux; 2] = [
        Aux::Group(&PICK),
        Aux::Member(Member::new(0, Kind::Int32Required, 0)),
    ];

    /// `Table::new` on a `Holder`, which is never used to access a message.
    fn holder_table(entries: &'static [Entry], aux: &'static [Aux]) -> Table<Holder> {
        // SAFETY: the table is dropped without being used.
        unsafe { Table::new(ABI, entries, &[], aux, None) }
    }

    /// A member of `pick` (whose lowest number is 2), at its offset, leading
    /// it if it is number 2.
    const fn member(kind: Kind, number: u32, aux: u16) -> Entry {
        Entry::oneof_member(kind, number == 2, number, 0, aux)
    }

    #[test]
    fn a_valid_table_is_accepted() {
        const E: Entry = member(Kind::Int32Required, 2, 1);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "payload must be a `Required`")]
    fn a_payload_must_be_a_kind_a_member_can_have() {
        let _ = Entry::oneof_member(Kind::Int32Optional, true, 2, 0, 1);
    }

    #[test]
    #[should_panic(expected = "wire type is its payload kind's")]
    fn an_entry_of_the_member_kind_needs_the_constructor_for_members() {
        let _ = Entry::new(Kind::OneofFollower, 2, 0, 1);
    }

    #[test]
    #[should_panic(expected = "wrong variant")]
    fn a_member_entry_needs_a_member_aux() {
        const E: Entry = member(Kind::Int32Required, 2, 0);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "group index is out of range")]
    fn a_member_needs_its_group() {
        static AUX: [Aux; 1] = [Aux::Member(Member::new(3, Kind::Int32Required, 0))];
        const E: Entry = member(Kind::Int32Required, 2, 0);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "not a oneof descriptor")]
    fn a_members_group_must_be_a_group() {
        static AUX: [Aux; 1] = [Aux::Member(Member::new(0, Kind::Int32Required, 0))];
        const E: Entry = member(Kind::Int32Required, 2, 0);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "payload kind is not one a payload can have")]
    fn a_members_kind_must_be_a_payload_kind() {
        static AUX: [Aux; 2] = [
            Aux::Group(&PICK),
            Aux::Member(Member::new(0, Kind::Int32Optional, 0)),
        ];
        const E: Entry = member(Kind::Int32Required, 2, 1);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "wire type")]
    fn a_members_tag_must_have_its_kinds_wire_type() {
        static AUX: [Aux; 2] = [
            Aux::Group(&PICK),
            Aux::Member(Member::new(0, Kind::FloatRequired, 0)),
        ];
        const E: Entry = member(Kind::Int32Required, 2, 1);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "payload aux index is out of range")]
    fn a_message_member_needs_its_payload_descriptor() {
        static AUX: [Aux; 2] = [
            Aux::Group(&PICK),
            Aux::Member(Member::new(0, Kind::MsgSingular, 5)),
        ];
        const E: Entry = member(Kind::MsgSingular, 2, 1);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "wrong variant for its kind")]
    fn a_members_payload_descriptor_must_be_of_its_kinds_variant() {
        static AUX: [Aux; 3] = [
            Aux::Group(&PICK),
            Aux::Member(Member::new(0, Kind::MsgSingular, 2)),
            Aux::Enum(&EnumVt::new::<ImplicitClosed<Color>>()),
        ];
        const E: Entry = member(Kind::MsgSingular, 2, 1);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "`MsgVt::direct` one")]
    fn a_message_member_needs_a_descriptor_that_reaches_the_message_directly() {
        static AUX: [Aux; 3] = [
            Aux::Group(&PICK),
            Aux::Member(Member::new(0, Kind::MsgSingular, 2)),
            Aux::Msg(&MsgVt::new::<MessageField<Inner>>(&INNER)),
        ];
        const E: Entry = member(Kind::MsgSingular, 2, 1);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "singular field")]
    fn a_members_enum_descriptor_must_be_singular() {
        static AUX: [Aux; 3] = [
            Aux::Group(&PICK),
            Aux::Member(Member::new(0, Kind::EnumRequired, 2)),
            Aux::Enum(&EnumVt::new::<RepeatedClosed<Color>>()),
        ];
        const E: Entry = member(Kind::EnumRequired, 2, 1);
        let _ = holder_table(&[E], &AUX);
    }

    #[test]
    #[should_panic(expected = "offset differs from its oneof's")]
    fn a_members_offset_must_be_its_oneofs() {
        const E: Entry = Entry::oneof_member(Kind::Int32Required, true, 2, 4, 1);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "numbered below its oneof's lowest")]
    fn a_member_cannot_be_numbered_below_its_oneofs_lowest() {
        const E: Entry = member(Kind::Int32Required, 1, 1);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "leader of a oneof must be exactly")]
    fn the_lowest_member_must_lead() {
        const E: Entry = Entry::oneof_member(Kind::Int32Required, false, 2, 0, 1);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "leader of a oneof must be exactly")]
    fn no_other_member_may_lead() {
        const E: Entry = Entry::oneof_member(Kind::Int32Required, true, 3, 0, 1);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "exactly one leading member")]
    fn every_oneof_needs_a_leader() {
        // The only member is numbered above the oneof's lowest, so nothing
        // leads.
        const E: Entry = member(Kind::Int32Required, 3, 1);
        let _ = holder_table(&[E], &INT_MEMBERS);
    }

    #[test]
    #[should_panic(expected = "exactly one leading member")]
    fn a_oneof_without_members_is_rejected() {
        static AUX: [Aux; 1] = [Aux::Group(&PICK)];
        let _ = holder_table(&[], &AUX);
    }
}

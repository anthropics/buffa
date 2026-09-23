//! The thirteen numeric and bool protobuf scalar types as zero-sized markers,
//! so the interpreters are written once per cardinality and instantiated per
//! type.

use crate::alloc::vec::Vec;
use crate::encoding::WireType;
use crate::{types, DecodeError, EncodeSink};

/// One protobuf scalar type: its wire type and the codec functions for a
/// single value.
pub trait Sc {
    /// The Rust type of a field of this scalar type.
    type V: Copy;
    const WIRE: WireType;
    fn read(buf: &mut &[u8]) -> Result<Self::V, DecodeError>;
    fn encode<K: EncodeSink>(v: Self::V, buf: &mut K);
    /// Encoded size of `v`, without the tag.
    fn len(v: Self::V) -> u64;
    /// Whether `v` is the proto3 implicit-presence default (not written).
    fn is_default(v: Self::V) -> bool;
    /// Append every element of a packed payload to `out`.
    fn extend(payload: &[u8], out: &mut Vec<Self::V>) -> Result<(), DecodeError>;
}

macro_rules! scalar {
    ($name:ident, $v:ty, $wire:expr, $read:path, $enc:path, $len:expr, $default:expr, $extend:expr) => {
        pub struct $name;
        impl Sc for $name {
            type V = $v;
            const WIRE: WireType = $wire;
            #[inline]
            fn read(buf: &mut &[u8]) -> Result<$v, DecodeError> {
                $read(buf)
            }
            #[inline]
            fn encode<K: EncodeSink>(v: $v, buf: &mut K) {
                $enc(v, buf);
            }
            #[inline]
            fn len(v: $v) -> u64 {
                ($len)(v)
            }
            #[inline]
            fn is_default(v: $v) -> bool {
                ($default)(v)
            }
            #[inline]
            fn extend(payload: &[u8], out: &mut Vec<$v>) -> Result<(), DecodeError> {
                ($extend)(payload, out)
            }
        }
    };
}

macro_rules! varint_scalar {
    ($name:ident, $v:ty, $read:path, $enc:path, $len:path, $extend:path) => {
        scalar!(
            $name,
            $v,
            WireType::Varint,
            $read,
            $enc,
            |v| $len(v) as u64,
            |v| v == <$v>::default(),
            |p: &[u8], out: &mut Vec<$v>| $extend(p, out, p.len())
        );
    };
}

/// `$bits` maps a value to its bit pattern, so that a float is the default
/// only when it is `+0.0`: `-0.0` is not the default and is written, as in
/// unrolled code.
macro_rules! fixed_scalar {
    ($name:ident, $v:ty, $wire:expr, $width:expr, $read:path, $enc:path, $extend:path, $bits:expr) => {
        scalar!(
            $name,
            $v,
            $wire,
            $read,
            $enc,
            |_| $width as u64,
            |v| ($bits)(v) == 0,
            |p: &[u8], out: &mut Vec<$v>| $extend(p, out)
        );
    };
}

varint_scalar!(
    Int32,
    i32,
    types::decode_int32,
    types::encode_int32,
    types::int32_encoded_len,
    types::extend_packed_int32
);
varint_scalar!(
    Int64,
    i64,
    types::decode_int64,
    types::encode_int64,
    types::int64_encoded_len,
    types::extend_packed_int64
);
varint_scalar!(
    Uint32,
    u32,
    types::decode_uint32,
    types::encode_uint32,
    types::uint32_encoded_len,
    types::extend_packed_uint32
);
varint_scalar!(
    Uint64,
    u64,
    types::decode_uint64,
    types::encode_uint64,
    types::uint64_encoded_len,
    types::extend_packed_uint64
);
varint_scalar!(
    Sint32,
    i32,
    types::decode_sint32,
    types::encode_sint32,
    types::sint32_encoded_len,
    types::extend_packed_sint32
);
varint_scalar!(
    Sint64,
    i64,
    types::decode_sint64,
    types::encode_sint64,
    types::sint64_encoded_len,
    types::extend_packed_sint64
);
scalar!(
    Bool,
    bool,
    WireType::Varint,
    types::decode_bool,
    types::encode_bool,
    |_| types::BOOL_ENCODED_LEN as u64,
    |v: bool| !v,
    |p: &[u8], out: &mut Vec<bool>| types::extend_packed_bool(p, out, p.len())
);
fixed_scalar!(
    Fixed32,
    u32,
    WireType::Fixed32,
    4,
    types::decode_fixed32,
    types::encode_fixed32,
    types::extend_packed_fixed32,
    |v: u32| v
);
fixed_scalar!(
    Sfixed32,
    i32,
    WireType::Fixed32,
    4,
    types::decode_sfixed32,
    types::encode_sfixed32,
    types::extend_packed_sfixed32,
    |v: i32| v
);
fixed_scalar!(
    Float,
    f32,
    WireType::Fixed32,
    4,
    types::decode_float,
    types::encode_float,
    types::extend_packed_float,
    |v: f32| v.to_bits()
);
fixed_scalar!(
    Fixed64,
    u64,
    WireType::Fixed64,
    8,
    types::decode_fixed64,
    types::encode_fixed64,
    types::extend_packed_fixed64,
    |v: u64| v
);
fixed_scalar!(
    Sfixed64,
    i64,
    WireType::Fixed64,
    8,
    types::decode_sfixed64,
    types::encode_sfixed64,
    types::extend_packed_sfixed64,
    |v: i64| v
);
fixed_scalar!(
    Double,
    f64,
    WireType::Fixed64,
    8,
    types::decode_double,
    types::encode_double,
    types::extend_packed_double,
    |v: f64| v.to_bits()
);

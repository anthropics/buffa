//! The decode pass: [`merge_to_limit`] and its per-kind arms.
//!
//! Decoding runs over one contiguous `&[u8]`, so every arm reads through a
//! non-generic function compiled in this crate.

use super::scalar::Sc;
use super::{
    Bool, Double, Entry, EnumVt, Fixed32, Fixed64, Float, Int32, Int64, Kind, MessageTable,
    Sfixed32, Sfixed64, Sint32, Sint64, Uint32, Uint64, IMPLICIT, NO_UNKNOWN, OPTIONAL, PACKED,
    REPEATED, REQUIRED,
};
use crate::alloc::{string::String, vec::Vec};
use crate::bytes::Buf;
use crate::encoding::{
    check_wire_type, decode_unknown_field, decode_varint, skip_field_depth, wire_type_mismatch,
    Tag, WireType,
};
use crate::message::MAX_MESSAGE_BYTES;
use crate::{types, DecodeContext, DecodeError, UnknownField, UnknownFieldData, UnknownFields};

/// Decode into the message at `base` until `buf` has `limit` bytes remaining.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
pub(super) unsafe fn merge_to_limit<B: Buf>(
    table: &MessageTable,
    base: *mut u8,
    buf: &mut B,
    ctx: DecodeContext<'_>,
    limit: usize,
) -> Result<(), DecodeError> {
    let n = buf.remaining().saturating_sub(limit);
    let chunk = buf.chunk();
    if chunk.len() >= n {
        let mut payload = &chunk[..n];
        // SAFETY: forwarded from the caller.
        unsafe { merge_slice(table, base, &mut payload, ctx)? };
        buf.advance(n);
        Ok(())
    } else {
        let gathered = buf.copy_to_bytes(n);
        let mut payload = &gathered[..];
        // SAFETY: forwarded from the caller.
        unsafe { merge_slice(table, base, &mut payload, ctx) }
    }
}

/// Decode a length-prefixed message into the message at `base`.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
pub(super) unsafe fn merge_length_delimited<B: Buf>(
    table: &MessageTable,
    base: *mut u8,
    buf: &mut B,
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    let ctx = ctx.descend()?;
    let len = decode_varint(buf)?;
    if len > u64::from(MAX_MESSAGE_BYTES) {
        return Err(DecodeError::MessageTooLarge);
    }
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let limit = buf.remaining() - len;
    // SAFETY: forwarded from the caller.
    unsafe { merge_to_limit(table, base, buf, ctx, limit) }
}

/// Decode one field, whose `tag` has been read, into the message at `base`.
/// The rest of `buf` must be one chunk.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
pub(super) unsafe fn merge_field<B: Buf>(
    table: &MessageTable,
    base: *mut u8,
    tag: Tag,
    buf: &mut B,
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    let remaining = buf.remaining();
    let chunk = buf.chunk();
    if chunk.len() < remaining {
        return Err(DecodeError::UnexpectedEof);
    }
    let mut payload = chunk;
    // SAFETY: forwarded from the caller.
    unsafe { merge_one(table, base, tag, &mut payload, ctx)? };
    let consumed = remaining - payload.len();
    buf.advance(consumed);
    Ok(())
}

/// Decode every field in `buf` into the message at `base`.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
unsafe fn merge_slice(
    table: &MessageTable,
    base: *mut u8,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    while !buf.is_empty() {
        let tag = Tag::decode(buf)?;
        // SAFETY: forwarded from the caller.
        unsafe { merge_one(table, base, tag, buf, ctx)? };
    }
    Ok(())
}

/// Decode a length-prefixed sub-message into the message at `base`.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
unsafe fn merge_sub(
    table: &MessageTable,
    base: *mut u8,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    let ctx = ctx.descend()?;
    let len = decode_varint(buf)?;
    if len > u64::from(MAX_MESSAGE_BYTES) {
        return Err(DecodeError::MessageTooLarge);
    }
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.len() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let (mut payload, rest) = buf.split_at(len);
    *buf = rest;
    // SAFETY: forwarded from the caller.
    unsafe { merge_slice(table, base, &mut payload, ctx) }
}

/// # Safety
///
/// `base` points to a live message of the type `table` describes.
#[inline]
unsafe fn merge_one(
    table: &MessageTable,
    base: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    let Some(e) = table.find(tag.field_number()) else {
        // SAFETY: forwarded from the caller.
        return unsafe { merge_unknown(table, base, tag, buf, ctx) };
    };
    // SAFETY: the offset is within the message, per the table's contract.
    unsafe { merge_kind(table, e, base, base.add(e.offset as usize), tag, buf, ctx) }
}

/// # Safety
///
/// `base` points to a live message of the type `table` describes.
#[cold]
unsafe fn merge_unknown(
    table: &MessageTable,
    base: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    if table.unknown == NO_UNKNOWN {
        return skip_field_depth(tag, buf, ctx.depth());
    }
    let field = decode_unknown_field(tag, buf, ctx)?;
    // SAFETY: `unknown` is the offset of the message's `UnknownFields`.
    unsafe { (*base.add(table.unknown as usize).cast::<UnknownFields>()).push(field) };
    Ok(())
}

macro_rules! merge_dispatch {
    ($($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        /// # Safety
        ///
        /// `slot` points to the field `e` describes, inside the live message
        /// at `base` of the type `table` describes.
        #[inline]
        unsafe fn merge_kind(
            table: &MessageTable,
            e: &Entry,
            base: *mut u8,
            slot: *mut u8,
            tag: Tag,
            buf: &mut &[u8],
            ctx: DecodeContext<'_>,
        ) -> Result<(), DecodeError> {
            // SAFETY: each arm writes the slot as the type its kind names.
            unsafe {
                match e.kind {
                    $(Kind::$name => merge_dispatch!(@arm $fam $ty $card table e base slot tag buf ctx),)*
                }
            }
        }
    };
    (@arm Scalar $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_scalar::<$ty, $card>($slot, $tag, $buf)
    };
    (@arm Str $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_str::<$card>($slot, $tag, $buf, $ctx)
    };
    (@arm Bytes $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_bytes::<$card>($slot, $tag, $buf, $ctx)
    };
    (@arm Enum $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_enum::<$card>($table, $e, $base, $slot, $tag, $buf, $ctx)
    };
    (@arm Msg $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_msg::<$card>($table, $e, $slot, $tag, $buf, $ctx)
    };
}

kind_table!(merge_dispatch);

/// # Safety
///
/// `slot` points to a field of scalar type `S` in the shape `C` names.
#[inline]
unsafe fn merge_scalar<S: Sc, const C: u8>(
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
) -> Result<(), DecodeError> {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT | REQUIRED => {
                check_wire_type(tag, S::WIRE)?;
                *slot.cast::<S::V>() = S::read(buf)?;
            }
            OPTIONAL => {
                check_wire_type(tag, S::WIRE)?;
                *slot.cast::<Option<S::V>>() = Some(S::read(buf)?);
            }
            _ => {
                let out = &mut *slot.cast::<Vec<S::V>>();
                let wire = tag.wire_type();
                if wire == WireType::LengthDelimited {
                    let payload = take_len_delimited(buf)?;
                    S::extend(payload, out)?;
                } else if wire == S::WIRE {
                    out.push(S::read(buf)?);
                } else {
                    return Err(wire_type_mismatch(tag, WireType::LengthDelimited));
                }
            }
        }
    }
    Ok(())
}

/// Split a length-prefixed payload off the front of `buf`.
#[inline]
fn take_len_delimited<'a>(buf: &mut &'a [u8]) -> Result<&'a [u8], DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.len() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let (payload, rest) = buf.split_at(len);
    *buf = rest;
    Ok(payload)
}

/// # Safety
///
/// `slot` points to a `String` field in the shape `C` names.
#[inline]
unsafe fn merge_str<const C: u8>(
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT | REQUIRED => types::merge_string(&mut *slot.cast::<String>(), buf),
            OPTIONAL => types::merge_string(
                (*slot.cast::<Option<String>>()).get_or_insert_with(String::new),
                buf,
            ),
            _ => {
                let elem = types::decode_string(buf)?;
                ctx.register_element_memory(core::mem::size_of::<String>())?;
                (*slot.cast::<Vec<String>>()).push(elem);
                Ok(())
            }
        }
    }
}

/// # Safety
///
/// `slot` points to a `Vec<u8>` field in the shape `C` names.
#[inline]
unsafe fn merge_bytes<const C: u8>(
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT | REQUIRED => types::merge_bytes(&mut *slot.cast::<Vec<u8>>(), buf),
            OPTIONAL => types::merge_bytes(
                (*slot.cast::<Option<Vec<u8>>>()).get_or_insert_with(Vec::new),
                buf,
            ),
            _ => {
                let elem = types::decode_bytes(buf)?;
                ctx.register_element_memory(core::mem::size_of::<Vec<u8>>())?;
                (*slot.cast::<Vec<Vec<u8>>>()).push(elem);
                Ok(())
            }
        }
    }
}

/// # Safety
///
/// `slot` points to the message field `e` describes, in the shape `C` names.
#[inline]
unsafe fn merge_msg<const C: u8>(
    table: &MessageTable,
    e: &Entry,
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    // SAFETY: the descriptor was built for the slot's shape and child type.
    unsafe {
        if C == REPEATED {
            let vt = table.rep_vt(e);
            ctx.register_element_memory(vt.size)?;
            let elem = (vt.push)(slot);
            let decoded = merge_sub(vt.table, elem, buf, ctx);
            if decoded.is_err() {
                // Like unrolled code, which decodes into a local and pushes
                // it only on success, leave no partial element behind.
                (vt.pop)(slot);
            }
            decoded
        } else {
            let vt = table.msg_vt(e);
            let child = (vt.place)(slot);
            merge_sub(vt.table, child, buf, ctx)
        }
    }
}

/// # Safety
///
/// `slot` points to the enum field `e` describes, in the shape `C` names,
/// inside the live message at `base`.
#[inline]
unsafe fn merge_enum<const C: u8>(
    table: &MessageTable,
    e: &Entry,
    base: *mut u8,
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    let vt = table.enum_vt(e);
    // SAFETY: `vt` was built for the slot's shape.
    unsafe {
        match C {
            REPEATED | PACKED => {
                let wire = tag.wire_type();
                if wire == WireType::LengthDelimited {
                    let mut payload = take_len_delimited(buf)?;
                    while !payload.is_empty() {
                        let raw = types::decode_int32_packed(&mut payload)?;
                        enum_store(table, e, base, vt, slot, raw, ctx)?;
                    }
                    Ok(())
                } else if wire == WireType::Varint {
                    let raw = types::decode_int32(buf)?;
                    enum_store(table, e, base, vt, slot, raw, ctx)
                } else {
                    Err(wire_type_mismatch(tag, WireType::LengthDelimited))
                }
            }
            _ => {
                check_wire_type(tag, WireType::Varint)?;
                let raw = types::decode_int32(buf)?;
                enum_store(table, e, base, vt, slot, raw, ctx)
            }
        }
    }
}

/// Store `raw` in the enum field at `slot`; a value a closed enum rejects goes
/// to the message's unknown fields, or is dropped if it keeps none.
///
/// # Safety
///
/// `slot` is a live slot of the shape `vt` was built for, inside the live
/// message at `base` of the type `table` describes.
#[inline]
unsafe fn enum_store(
    table: &MessageTable,
    e: &Entry,
    base: *mut u8,
    vt: &EnumVt,
    slot: *mut u8,
    raw: i32,
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    // SAFETY: `slot` matches the shape `vt` was built for.
    if unsafe { (vt.set)(slot, raw) } || table.unknown == NO_UNKNOWN {
        return Ok(());
    }
    ctx.register_unknown_field()?;
    // SAFETY: `unknown` is the offset of the message's `UnknownFields`.
    unsafe {
        (*base.add(table.unknown as usize).cast::<UnknownFields>()).push(UnknownField {
            number: e.tag >> 3,
            data: UnknownFieldData::Varint(raw as u64),
        });
    }
    Ok(())
}

//! The decode pass: [`merge_to_limit`] and its per-kind arms.
//!
//! Decoding runs over one contiguous `&[u8]`, so every arm reads through a
//! non-generic function compiled in this crate.

use super::map::merge_map;
use super::scalar::Sc;
use super::{
    Bool, Double, Entry, EnumVt, Fixed32, Fixed64, Float, Int32, Int64, Kind, MessageTable,
    OneofVt, Sfixed32, Sfixed64, Sint32, Sint64, Uint32, Uint64, IMPLICIT, NO_UNKNOWN, OPTIONAL,
    PACKED, REPEATED, REQUIRED,
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
pub(super) unsafe fn merge_sub(
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
    let slot = unsafe { base.add(e.offset as usize) };
    // SAFETY: `slot` is the field `e` describes.
    unsafe {
        if e.kind == Kind::Map {
            merge_map(table, e, base, slot, tag, buf, ctx)
        } else {
            merge_kind(table, e, base, slot, tag, buf, ctx)
        }
    }
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

// The arm for a map is unreachable: `merge_one`, which `merge_slice` calls for
// each field, tests for a map before it calls this. `map::merge_entry` also
// calls this function, for a key and a value, which are never maps. The
// reason is in the module documentation of `map.rs`.
macro_rules! merge_dispatch {
    ($($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        /// # Safety
        ///
        /// `slot` points to the field `e` describes, inside the live message
        /// at `base` of the type `table` describes. `base` may be null if
        /// `table` has no unknown fields, which is how a map entry, whose
        /// fields are not in a message, calls this.
        #[inline]
        pub(super) unsafe fn merge_kind(
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
    (@arm Oneof $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_oneof($table, $e, $base, $slot, $tag, $buf, $ctx)
    };
    (@arm Map $ty:ident $card:ident $table:ident $e:ident $base:ident $slot:ident $tag:ident $buf:ident $ctx:ident) => {
        unreachable!("a map field is merged by `merge_one`")
    };
}

kind_table!(merge_dispatch);

/// Decode the member of a oneof that the entry `e` describes into the oneof.
///
/// # Safety
///
/// `slot` points to the `Option` of the oneof enum that `e`'s group describes,
/// inside the live message at `base` of the type `table` describes.
#[inline(never)]
unsafe fn merge_oneof(
    table: &MessageTable,
    e: &Entry,
    base: *mut u8,
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    let m = table.member(e);
    let payload_entry = Entry {
        kind: m.kind,
        aux: m.aux,
        ..*e
    };
    let oneof = OneofSlot {
        group: table.group(m),
        slot,
    };
    // SAFETY: forwarded from the caller.
    unsafe { merge_payload(table, &payload_entry, &oneof, base, tag, buf, ctx) }
}

/// A oneof inside the message that is being decoded: the `Option` of its enum
/// and its descriptor.
struct OneofSlot<'a> {
    group: &'a OneofVt,
    slot: *mut u8,
}

impl OneofSlot<'_> {
    /// Make the member `number` the one that is set, and return a pointer to
    /// its value.
    ///
    /// # Safety
    ///
    /// `slot` points to a live `Option` of the enum that `group` describes,
    /// and `number` is a member of it.
    #[inline]
    unsafe fn place(&self, number: u32) -> *mut u8 {
        // SAFETY: forwarded from the caller.
        unsafe { (self.group.place)(self.slot, number) }
    }

    /// Decode into the member `number`: in place if it is the one that is set,
    /// and otherwise into a new default member, which becomes the one that is
    /// set only if `f` succeeds.
    ///
    /// # Safety
    ///
    /// As for [`place`](Self::place).
    #[inline]
    unsafe fn place_with(
        &self,
        number: u32,
        f: &mut dyn FnMut(*mut u8) -> Result<(), DecodeError>,
    ) -> Result<(), DecodeError> {
        // SAFETY: forwarded from the caller.
        unsafe { (self.group.place_with)(self.slot, number, f) }
    }
}

/// Defines `merge_payload`, which decodes a oneof member's value by its
/// payload kind. Every arm decodes the value before it replaces the member
/// that is set, so a value that is rejected, or that a closed enum does not
/// know, leaves the oneof as it was. The exception is a message member that
/// is the one that is set, which is merged into as it is decoded, so a failure
/// part of the way through keeps the fields that had been merged.
macro_rules! merge_payload_dispatch {
    ($fname:ident; $($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        /// # Safety
        ///
        /// `payload_entry` is the entry of a member of the oneof `oneof`
        /// describes, with its payload's kind and aux index, and the oneof is
        /// inside the live message at `base` of the type `table` describes.
        #[inline]
        unsafe fn $fname(
            table: &MessageTable,
            payload_entry: &Entry,
            oneof: &OneofSlot<'_>,
            base: *mut u8,
            tag: Tag,
            buf: &mut &[u8],
            ctx: DecodeContext<'_>,
        ) -> Result<(), DecodeError> {
            let number = payload_entry.number();
            // SAFETY: each arm stores into the member the entry describes.
            unsafe {
                match payload_entry.kind {
                    $(Kind::$name => merge_payload_dispatch!(
                        @arm $fam $ty table payload_entry oneof number base tag buf ctx
                    ),)*
                    #[allow(unreachable_patterns)]
                    _ => unreachable!("`Table::new` checked the payload kinds"),
                }
            }
        }
    };
    (@arm Scalar $ty:ident $table:ident $pe:ident $o:ident $n:ident $base:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_oneof_scalar::<$ty>($o, $n, $tag, $buf)
    };
    (@arm Str $ty:ident $table:ident $pe:ident $o:ident $n:ident $base:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_oneof_str($o, $n, $tag, $buf)
    };
    (@arm Bytes $ty:ident $table:ident $pe:ident $o:ident $n:ident $base:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_oneof_bytes($o, $n, $tag, $buf)
    };
    (@arm Enum $ty:ident $table:ident $pe:ident $o:ident $n:ident $base:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_oneof_enum($table, $pe, $o, $base, $tag, $buf, $ctx)
    };
    (@arm Msg $ty:ident $table:ident $pe:ident $o:ident $n:ident $base:ident $tag:ident $buf:ident $ctx:ident) => {
        merge_oneof_msg($table, $pe, $o, $tag, $buf, $ctx)
    };
}

payload_kind_table!(merge_payload_dispatch, merge_payload);

/// # Safety
///
/// `oneof` is a live oneof whose member `number` has the scalar type `S`.
#[inline]
unsafe fn merge_oneof_scalar<S: Sc>(
    oneof: &OneofSlot<'_>,
    number: u32,
    tag: Tag,
    buf: &mut &[u8],
) -> Result<(), DecodeError> {
    check_wire_type(tag, S::WIRE)?;
    let value = S::read(buf)?;
    // SAFETY: the caller's contract gives the member's type.
    unsafe { *oneof.place(number).cast::<S::V>() = value };
    Ok(())
}

/// # Safety
///
/// `oneof` is a live oneof whose member `number` is a `String`.
#[inline]
unsafe fn merge_oneof_str(
    oneof: &OneofSlot<'_>,
    number: u32,
    tag: Tag,
    buf: &mut &[u8],
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    let value = types::decode_string(buf)?;
    // SAFETY: the caller's contract gives the member's type.
    unsafe { *oneof.place(number).cast::<String>() = value };
    Ok(())
}

/// # Safety
///
/// `oneof` is a live oneof whose member `number` is a `Vec<u8>`.
#[inline]
unsafe fn merge_oneof_bytes(
    oneof: &OneofSlot<'_>,
    number: u32,
    tag: Tag,
    buf: &mut &[u8],
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    let value = types::decode_bytes(buf)?;
    // SAFETY: the caller's contract gives the member's type.
    unsafe { *oneof.place(number).cast::<Vec<u8>>() = value };
    Ok(())
}

/// A closed enum's value with no variant goes to the unknown fields, like an
/// ordinary field, and leaves the member that is set as it is.
///
/// # Safety
///
/// `payload_entry` is the entry of an enum member of the live oneof `oneof`,
/// with its payload's kind and aux index, which is inside the live message at
/// `base` of the type `table` describes.
#[inline]
unsafe fn merge_oneof_enum(
    table: &MessageTable,
    payload_entry: &Entry,
    oneof: &OneofSlot<'_>,
    base: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::Varint)?;
    let raw = types::decode_int32(buf)?;
    let vt = table.enum_vt(payload_entry);
    if !(vt.accepts)(raw) {
        // SAFETY: forwarded from the caller.
        return unsafe { enum_reject(table, payload_entry, base, raw, ctx) };
    }
    // SAFETY: the member is stored in the shape `vt` was built for.
    let stored = unsafe { (vt.set)(oneof.place(payload_entry.number()), raw) };
    debug_assert!(stored, "`accepts` said the enum stores {raw}");
    Ok(())
}

/// # Safety
///
/// `payload_entry` is the entry of a message member of the live oneof `oneof`,
/// with its payload's kind and aux index, whose descriptor is for the
/// message's type and reaches it through a pointer to the message.
#[inline]
unsafe fn merge_oneof_msg(
    table: &MessageTable,
    payload_entry: &Entry,
    oneof: &OneofSlot<'_>,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    let vt = table.msg_vt(payload_entry);
    // SAFETY: the member is a message of the type `vt.child` reaches, through
    // a pointer to it, and `place_with` hands the closure a pointer to a live
    // one. A member that is already set is merged into, as a singular message
    // field is. Any other member is replaced only by a message that decoded,
    // so a failure leaves the oneof as it was.
    unsafe {
        oneof.place_with(payload_entry.number(), &mut |child| {
            vt.child.merge_sub(child, buf, ctx)
        })
    }
}

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
pub(super) fn take_len_delimited<'a>(buf: &mut &'a [u8]) -> Result<&'a [u8], DecodeError> {
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
            let decoded = vt.child.merge_sub(elem, buf, ctx);
            if decoded.is_err() {
                // Like unrolled code, which decodes into a local and pushes
                // it only on success, leave no partial element behind.
                (vt.pop)(slot);
            }
            decoded
        } else {
            let vt = table.msg_vt(e);
            let child = (vt.place)(slot);
            vt.child.merge_sub(child, buf, ctx)
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
    if unsafe { (vt.set)(slot, raw) } {
        return Ok(());
    }
    // SAFETY: forwarded from the caller.
    unsafe { enum_reject(table, e, base, raw, ctx) }
}

/// Handle a value that a closed enum has no variant for: keep it as an unknown
/// field, or drop it if the message keeps none.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
#[inline]
unsafe fn enum_reject(
    table: &MessageTable,
    e: &Entry,
    base: *mut u8,
    raw: i32,
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    if table.unknown == NO_UNKNOWN {
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

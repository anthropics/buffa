//! The write pass: [`write_to`] and its per-kind arms.

use super::bridge::write_field_value;
use super::scalar::Sc;
use super::{
    Bool, Double, Entry, Fixed32, Fixed64, Float, Int32, Int64, Kind, MessageTable, Sfixed32,
    Sfixed64, Sint32, Sint64, Uint32, Uint64, IMPLICIT, NO_UNKNOWN, OPTIONAL, PACKED, REPEATED,
    REQUIRED,
};
use crate::alloc::{string::String, vec::Vec};
use crate::encode_sink::PreSized;
use crate::encoding::encode_varint;
use crate::{types, EncodeSink, SizeCache, UnknownFields};

/// Write the message at `base` to `buf`, consuming nested sizes from `cache`,
/// which [`compute_size`](super::size::compute_size) must have filled for the
/// message; a cache that does not match makes the write panic.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
pub(super) unsafe fn write_to<K: EncodeSink>(
    table: &MessageTable,
    base: *const u8,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    let mut through_cursor = |cursor: &mut PreSized<'_>| {
        // SAFETY: forwarded from the caller.
        unsafe { write_pre_sized(table, base, &mut *cache, cursor) }
    };
    if !buf.__with_pre_sized(&mut through_cursor) {
        // SAFETY: forwarded from the caller.
        unsafe { write_message(table, base, cache, buf) };
    }
}

/// [`write_message`] for a [`PreSized`] sink, which every `BufMut` is written
/// through. Not generic, so it is compiled once, in this crate, however many
/// sink types the caller's crate encodes into.
///
/// # Safety
///
/// As for [`write_to`].
#[inline(never)]
unsafe fn write_pre_sized(
    table: &MessageTable,
    base: *const u8,
    cache: &mut SizeCache,
    buf: &mut PreSized<'_>,
) {
    // SAFETY: forwarded from the caller.
    unsafe { write_message(table, base, cache, buf) }
}

/// # Safety
///
/// As for [`write_to`].
pub(super) unsafe fn write_message<K: EncodeSink>(
    table: &MessageTable,
    base: *const u8,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    for e in table.entries {
        // SAFETY: the offset is within the message, per the table's contract.
        unsafe { write_kind(table, e, base.add(e.offset as usize), cache, buf) };
    }
    if table.unknown != NO_UNKNOWN {
        // SAFETY: `unknown` is the offset of the message's `UnknownFields`.
        unsafe { (*base.add(table.unknown as usize).cast::<UnknownFields>()).write_to(buf) };
    }
}

#[inline(always)]
fn put_tag<K: EncodeSink>(e: &Entry, buf: &mut K) {
    if e.tag_len == 1 {
        buf.put_u8(e.tag as u8);
    } else {
        encode_varint(u64::from(e.tag), buf);
    }
}

/// Defines `$fname`, the write of one field by kind, for the kinds listed, as
/// `size_dispatch!` does.
macro_rules! write_dispatch {
    ($fname:ident; $($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        /// # Safety
        ///
        /// `slot` points to the field `e` describes, in a live message of the
        /// type `table` describes.
        #[inline]
        unsafe fn $fname<K: EncodeSink>(
            table: &MessageTable,
            e: &Entry,
            slot: *const u8,
            cache: &mut SizeCache,
            buf: &mut K,
        ) {
            // SAFETY: each arm reads the slot as the type its kind names.
            unsafe {
                match e.kind {
                    $(Kind::$name => write_dispatch!(@arm $fam $ty $card table e slot cache buf),)*
                    // The kinds a list leaves out are ruled out by `Table::new`.
                    #[allow(unreachable_patterns)]
                    _ => unreachable!("`Table::new` checked the kinds of the entries"),
                }
            }
        }
    };
    (@arm Scalar $ty:ident $card:ident $table:ident $e:ident $slot:ident $cache:ident $buf:ident) => {
        write_scalar::<$ty, $card, K>($e, $slot, $buf)
    };
    (@arm Str $ty:ident $card:ident $table:ident $e:ident $slot:ident $cache:ident $buf:ident) => {
        write_str::<$card, K>($e, $slot, $buf)
    };
    (@arm Bytes $ty:ident $card:ident $table:ident $e:ident $slot:ident $cache:ident $buf:ident) => {
        write_bytes::<$card, K>($e, $slot, $buf)
    };
    (@arm Enum $ty:ident $card:ident $table:ident $e:ident $slot:ident $cache:ident $buf:ident) => {
        write_enum::<$card, K>($table, $e, $slot, $buf)
    };
    (@arm Msg $ty:ident $card:ident $table:ident $e:ident $slot:ident $cache:ident $buf:ident) => {
        write_msg::<$card, K>($table, $e, $slot, $cache, $buf)
    };
    (@arm Oneof $ty:ident $card:ident $table:ident $e:ident $slot:ident $cache:ident $buf:ident) => {
        write_oneof::<K>($table, $e, $slot, $cache, $buf)
    };
}

kind_table!(write_dispatch, write_kind);
payload_kind_table!(write_dispatch, write_payload);

/// Write the oneof that the member `e` describes, if `e` is its leader, and
/// nothing for any other member.
///
/// # Safety
///
/// `slot` points to the `Option` of the oneof enum that `e`'s group describes,
/// in a live message of the type `table` describes.
#[inline(never)]
unsafe fn write_oneof<K: EncodeSink>(
    table: &MessageTable,
    e: &Entry,
    slot: *const u8,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    let m = table.member(e);
    if !m.leader {
        return;
    }
    // SAFETY: `slot` is a live `Option<E>` for the `E` the group was built for.
    let (number, payload) = unsafe { (table.group(m).get)(slot) };
    if number == 0 {
        return;
    }
    let payload_entry = table.payload_entry(number);
    // SAFETY: `payload` points to a value of the payload kind of member
    // `number`, per the oneof enum's `OneofEnum` implementation.
    unsafe { write_payload(table, &payload_entry, payload, cache, buf) };
}

/// # Safety
///
/// `slot` points to a field of scalar type `S` in the shape `C` names.
#[inline]
unsafe fn write_scalar<S: Sc, const C: u8, K: EncodeSink>(e: &Entry, slot: *const u8, buf: &mut K) {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT => {
                let v = *slot.cast::<S::V>();
                if !S::is_default(v) {
                    put_tag(e, buf);
                    S::encode(v, buf);
                }
            }
            REQUIRED => {
                put_tag(e, buf);
                S::encode(*slot.cast::<S::V>(), buf);
            }
            OPTIONAL => {
                if let Some(v) = *slot.cast::<Option<S::V>>() {
                    put_tag(e, buf);
                    S::encode(v, buf);
                }
            }
            REPEATED => {
                for &v in &*slot.cast::<Vec<S::V>>() {
                    put_tag(e, buf);
                    S::encode(v, buf);
                }
            }
            _ => {
                let v = &*slot.cast::<Vec<S::V>>();
                if !v.is_empty() {
                    let payload: u64 = v.iter().map(|&x| S::len(x)).sum();
                    put_tag(e, buf);
                    encode_varint(payload, buf);
                    for &x in v {
                        S::encode(x, buf);
                    }
                }
            }
        }
    }
}

/// # Safety
///
/// `slot` points to a `String` field in the shape `C` names.
#[inline]
unsafe fn write_str<const C: u8, K: EncodeSink>(e: &Entry, slot: *const u8, buf: &mut K) {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT => {
                let s = &*slot.cast::<String>();
                if !s.is_empty() {
                    put_tag(e, buf);
                    types::encode_string(s, buf);
                }
            }
            REQUIRED => {
                put_tag(e, buf);
                types::encode_string(&*slot.cast::<String>(), buf);
            }
            OPTIONAL => {
                if let Some(s) = &*slot.cast::<Option<String>>() {
                    put_tag(e, buf);
                    types::encode_string(s, buf);
                }
            }
            _ => {
                for s in &*slot.cast::<Vec<String>>() {
                    put_tag(e, buf);
                    types::encode_string(s, buf);
                }
            }
        }
    }
}

/// # Safety
///
/// `slot` points to a `Vec<u8>` field in the shape `C` names.
#[inline]
unsafe fn write_bytes<const C: u8, K: EncodeSink>(e: &Entry, slot: *const u8, buf: &mut K) {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT => {
                let s = &*slot.cast::<Vec<u8>>();
                if !s.is_empty() {
                    put_tag(e, buf);
                    types::encode_shared_bytes(s, buf);
                }
            }
            REQUIRED => {
                put_tag(e, buf);
                types::encode_shared_bytes(&*slot.cast::<Vec<u8>>(), buf);
            }
            OPTIONAL => {
                if let Some(s) = &*slot.cast::<Option<Vec<u8>>>() {
                    put_tag(e, buf);
                    types::encode_shared_bytes(s, buf);
                }
            }
            _ => {
                for s in &*slot.cast::<Vec<Vec<u8>>>() {
                    put_tag(e, buf);
                    types::encode_shared_bytes(s, buf);
                }
            }
        }
    }
}

/// # Safety
///
/// `slot` points to the enum field `e` describes, in the shape `C` names.
#[inline]
unsafe fn write_enum<const C: u8, K: EncodeSink>(
    table: &MessageTable,
    e: &Entry,
    slot: *const u8,
    buf: &mut K,
) {
    let vt = table.enum_vt(e);
    // SAFETY: `vt` was built for the slot's shape.
    unsafe {
        match C {
            IMPLICIT => {
                if let Some(v) = (vt.get)(slot, 0) {
                    if v != 0 {
                        put_tag(e, buf);
                        types::encode_int32(v, buf);
                    }
                }
            }
            REQUIRED | OPTIONAL => {
                if let Some(v) = (vt.get)(slot, 0) {
                    put_tag(e, buf);
                    types::encode_int32(v, buf);
                }
            }
            REPEATED => {
                for i in 0..(vt.len)(slot) {
                    if let Some(v) = (vt.get)(slot, i) {
                        put_tag(e, buf);
                        types::encode_int32(v, buf);
                    }
                }
            }
            _ => {
                let n = (vt.len)(slot);
                if n == 0 {
                    return;
                }
                let mut payload = 0;
                for i in 0..n {
                    if let Some(v) = (vt.get)(slot, i) {
                        payload += types::int32_encoded_len(v) as u64;
                    }
                }
                put_tag(e, buf);
                encode_varint(payload, buf);
                for i in 0..n {
                    if let Some(v) = (vt.get)(slot, i) {
                        types::encode_int32(v, buf);
                    }
                }
            }
        }
    }
}

/// # Safety
///
/// `slot` points to the message field `e` describes, in the shape `C` names.
#[inline]
unsafe fn write_msg<const C: u8, K: EncodeSink>(
    table: &MessageTable,
    e: &Entry,
    slot: *const u8,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    // SAFETY: the descriptor was built for the slot's shape and child type.
    unsafe {
        if C == REPEATED {
            let vt = table.rep_vt(e);
            let (ptr, len) = (vt.parts)(slot);
            for i in 0..len {
                put_tag(e, buf);
                write_field_value(vt.child, ptr.add(i * vt.size), cache, buf);
            }
        } else {
            let vt = table.msg_vt(e);
            let child = (vt.get)(slot);
            if !child.is_null() {
                put_tag(e, buf);
                write_field_value(vt.child, child, cache, buf);
            }
        }
    }
}

//! The size pass: [`compute_size`] and its per-kind arms.

use super::scalar::Sc;
use super::{
    Bool, Double, Entry, Fixed32, Fixed64, Float, Int32, Int64, Kind, MessageTable, Sfixed32,
    Sfixed64, Sint32, Sint64, Uint32, Uint64, IMPLICIT, NO_UNKNOWN, OPTIONAL, PACKED, REPEATED,
    REQUIRED,
};
use crate::alloc::{string::String, vec::Vec};
use crate::encoding::varint_len;
use crate::{types, SizeCache, UnknownFields};

/// The encoded size of the message at `base`, recording nested sizes in
/// `cache`.
///
/// # Safety
///
/// `base` points to a live message of the type `table` describes.
pub(super) unsafe fn compute_size(
    table: &MessageTable,
    base: *const u8,
    cache: &mut SizeCache,
) -> u32 {
    let mut size = 0u64;
    for e in table.entries {
        // SAFETY: the offset is within the message, per the table's contract.
        size += unsafe { size_kind(table, e, base.add(e.offset as usize), cache) };
    }
    if table.unknown != NO_UNKNOWN {
        // SAFETY: `unknown` is the offset of the message's `UnknownFields`.
        size += unsafe { (*base.add(table.unknown as usize).cast::<UnknownFields>()).encoded_len() }
            as u64;
    }
    crate::saturate_size(size)
}

macro_rules! size_dispatch {
    ($($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        /// # Safety
        ///
        /// `slot` points to the field `e` describes, in a live message of the
        /// type `table` describes.
        #[inline]
        unsafe fn size_kind(
            table: &MessageTable,
            e: &Entry,
            slot: *const u8,
            cache: &mut SizeCache,
        ) -> u64 {
            let tl = u64::from(e.tag_len);
            // SAFETY: each arm reads the slot as the type its kind names.
            unsafe {
                match e.kind {
                    $(Kind::$name => size_dispatch!(@arm $fam $ty $card table e tl slot cache),)*
                }
            }
        }
    };
    (@arm Scalar $ty:ident $card:ident $table:ident $e:ident $tl:ident $slot:ident $cache:ident) => {
        size_scalar::<$ty, $card>($tl, $slot)
    };
    (@arm Str $ty:ident $card:ident $table:ident $e:ident $tl:ident $slot:ident $cache:ident) => {
        size_str::<$card>($tl, $slot)
    };
    (@arm Bytes $ty:ident $card:ident $table:ident $e:ident $tl:ident $slot:ident $cache:ident) => {
        size_bytes::<$card>($tl, $slot)
    };
    (@arm Enum $ty:ident $card:ident $table:ident $e:ident $tl:ident $slot:ident $cache:ident) => {
        size_enum::<$card>($table, $e, $tl, $slot)
    };
    (@arm Msg $ty:ident $card:ident $table:ident $e:ident $tl:ident $slot:ident $cache:ident) => {
        size_msg::<$card>($table, $e, $tl, $slot, $cache)
    };
}

kind_table!(size_dispatch);

/// # Safety
///
/// `slot` points to a field of scalar type `S` in the shape `C` names.
#[inline]
unsafe fn size_scalar<S: Sc, const C: u8>(tl: u64, slot: *const u8) -> u64 {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT => {
                let v = *slot.cast::<S::V>();
                if S::is_default(v) {
                    0
                } else {
                    tl + S::len(v)
                }
            }
            REQUIRED => tl + S::len(*slot.cast::<S::V>()),
            OPTIONAL => match *slot.cast::<Option<S::V>>() {
                Some(v) => tl + S::len(v),
                None => 0,
            },
            REPEATED => (*slot.cast::<Vec<S::V>>())
                .iter()
                .map(|&v| tl + S::len(v))
                .sum(),
            _ => {
                let v = &*slot.cast::<Vec<S::V>>();
                if v.is_empty() {
                    0
                } else {
                    let payload: u64 = v.iter().map(|&x| S::len(x)).sum();
                    tl + varint_len(payload) as u64 + payload
                }
            }
        }
    }
}

/// # Safety
///
/// `slot` points to a `String` field in the shape `C` names.
#[inline]
unsafe fn size_str<const C: u8>(tl: u64, slot: *const u8) -> u64 {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT => {
                let s = &*slot.cast::<String>();
                if s.is_empty() {
                    0
                } else {
                    tl + types::string_encoded_len(s) as u64
                }
            }
            REQUIRED => tl + types::string_encoded_len(&*slot.cast::<String>()) as u64,
            OPTIONAL => match &*slot.cast::<Option<String>>() {
                Some(s) => tl + types::string_encoded_len(s) as u64,
                None => 0,
            },
            _ => (*slot.cast::<Vec<String>>())
                .iter()
                .map(|s| tl + types::string_encoded_len(s) as u64)
                .sum(),
        }
    }
}

/// # Safety
///
/// `slot` points to a `Vec<u8>` field in the shape `C` names.
#[inline]
unsafe fn size_bytes<const C: u8>(tl: u64, slot: *const u8) -> u64 {
    // SAFETY: the caller's contract gives the slot's type.
    unsafe {
        match C {
            IMPLICIT => {
                let s = &*slot.cast::<Vec<u8>>();
                if s.is_empty() {
                    0
                } else {
                    tl + types::bytes_encoded_len(s) as u64
                }
            }
            REQUIRED => tl + types::bytes_encoded_len(&*slot.cast::<Vec<u8>>()) as u64,
            OPTIONAL => match &*slot.cast::<Option<Vec<u8>>>() {
                Some(s) => tl + types::bytes_encoded_len(s) as u64,
                None => 0,
            },
            _ => (*slot.cast::<Vec<Vec<u8>>>())
                .iter()
                .map(|s| tl + types::bytes_encoded_len(s) as u64)
                .sum(),
        }
    }
}

/// # Safety
///
/// `slot` points to the enum field `e` describes, in the shape `C` names.
#[inline]
unsafe fn size_enum<const C: u8>(table: &MessageTable, e: &Entry, tl: u64, slot: *const u8) -> u64 {
    let vt = table.enum_vt(e);
    // SAFETY: `vt` was built for the slot's shape.
    unsafe {
        match C {
            IMPLICIT => match (vt.get)(slot, 0) {
                Some(v) if v != 0 => tl + types::int32_encoded_len(v) as u64,
                _ => 0,
            },
            REQUIRED | OPTIONAL => match (vt.get)(slot, 0) {
                Some(v) => tl + types::int32_encoded_len(v) as u64,
                None => 0,
            },
            REPEATED => {
                let mut size = 0;
                for i in 0..(vt.len)(slot) {
                    if let Some(v) = (vt.get)(slot, i) {
                        size += tl + types::int32_encoded_len(v) as u64;
                    }
                }
                size
            }
            _ => {
                let n = (vt.len)(slot);
                if n == 0 {
                    return 0;
                }
                let mut payload = 0;
                for i in 0..n {
                    if let Some(v) = (vt.get)(slot, i) {
                        payload += types::int32_encoded_len(v) as u64;
                    }
                }
                tl + varint_len(payload) as u64 + payload
            }
        }
    }
}

/// # Safety
///
/// `slot` points to the message field `e` describes, in the shape `C` names.
#[inline]
unsafe fn size_msg<const C: u8>(
    table: &MessageTable,
    e: &Entry,
    tl: u64,
    slot: *const u8,
    cache: &mut SizeCache,
) -> u64 {
    // SAFETY: the descriptor was built for the slot's shape and child type.
    unsafe {
        if C == REPEATED {
            let vt = table.rep_vt(e);
            let (ptr, len) = (vt.parts)(slot);
            let mut size = 0;
            for i in 0..len {
                let idx = cache.reserve();
                let inner = compute_size(vt.table, ptr.add(i * vt.size), cache);
                cache.set(idx, inner);
                size += tl + varint_len(u64::from(inner)) as u64 + u64::from(inner);
            }
            size
        } else {
            let vt = table.msg_vt(e);
            let child = (vt.get)(slot);
            if child.is_null() {
                return 0;
            }
            let idx = cache.reserve();
            let inner = compute_size(vt.table, child, cache);
            cache.set(idx, inner);
            tl + varint_len(u64::from(inner)) as u64 + u64::from(inner)
        }
    }
}

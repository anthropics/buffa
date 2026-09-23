//! Message-typed fields whose message is not a table message.
//!
//! A table records, for each message field, how to size, write and decode the
//! child. When the child has a table, the interpreters recurse into it. When
//! it has none (it was generated with the unrolled codec, by another crate, or
//! is a well-known type) they call the child's [`Message`] impl through the
//! function pointers of a [`DynVt`], one per operation, compiled once per
//! child type.
//!
//! A pointer cannot be generic over the write sink, so the write function
//! takes the [`PreSized`] cursor that every `BufMut` is written through. Any
//! other sink gets the child's bytes from a scratch buffer, which costs one
//! copy of the child and, for a segmented sink such as [`Rope`](crate::Rope),
//! the reference-counted segments of large `bytes` fields inside it.

use core::marker::PhantomData;

use super::{decode, encode, size, MessageTable};
use crate::encode_sink::{write_to_new_vec, PreSized};
use crate::encoding::encode_varint;
use crate::{DecodeContext, DecodeError, EncodeSink, Message, SizeCache};

/// The operations of one child message type, reached through its [`Message`]
/// impl. Built by [`MsgVt::new_dyn`](super::MsgVt::new_dyn) and
/// [`RepVt::new_dyn`](super::RepVt::new_dyn).
pub(super) struct DynVt {
    /// The encoded size of the child, recording nested sizes in the cache.
    ///
    /// # Safety
    ///
    /// The argument points to a live message of the type the vtable was built
    /// for.
    size: unsafe fn(*const u8, &mut SizeCache) -> u32,
    /// Write the child through a cursor, consuming sizes from the cache.
    ///
    /// # Safety
    ///
    /// As for `size`.
    write: unsafe fn(*const u8, &mut SizeCache, &mut PreSized<'_>),
    /// Merge a length-prefixed encoding of the child from the front of the
    /// buffer, as generated code does for a message field.
    ///
    /// # Safety
    ///
    /// As for `size`, and the argument is exclusive.
    merge: unsafe fn(*mut u8, &mut &[u8], DecodeContext<'_>) -> Result<(), DecodeError>,
}

/// # Safety
///
/// `msg` points to a live `T`.
unsafe fn size_thunk<T: Message>(msg: *const u8, cache: &mut SizeCache) -> u32 {
    // SAFETY: the caller passes a pointer to a live `T`.
    unsafe { (*msg.cast::<T>()).compute_size(cache) }
}

/// # Safety
///
/// `msg` points to a live `T`.
unsafe fn write_thunk<T: Message>(msg: *const u8, cache: &mut SizeCache, buf: &mut PreSized<'_>) {
    // SAFETY: the caller passes a pointer to a live `T`.
    unsafe { (*msg.cast::<T>()).write_to(cache, buf) }
}

/// # Safety
///
/// `msg` points to a live `T` that nothing else accesses.
unsafe fn merge_thunk<T: Message>(
    msg: *mut u8,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    // SAFETY: the caller passes an exclusive pointer to a live `T`.
    unsafe { (*msg.cast::<T>()).merge_length_delimited(buf, ctx) }
}

/// Holds the `'static` [`DynVt`] of `T`.
struct Thunks<T>(PhantomData<fn() -> T>);

impl<T: Message> Thunks<T> {
    const VT: &'static DynVt = &DynVt {
        size: size_thunk::<T>,
        write: write_thunk::<T>,
        merge: merge_thunk::<T>,
    };
}

/// How to reach the message of a message field: through its table, or through
/// its [`Message`] impl.
#[derive(Clone, Copy)]
pub(super) enum Child {
    Table(&'static MessageTable),
    Dyn(&'static DynVt),
}

impl Child {
    pub(super) const fn of<T: Message>() -> Self {
        Self::Dyn(Thunks::<T>::VT)
    }

    /// The encoded size of the message at `base`, recording nested sizes in
    /// `cache`.
    ///
    /// # Safety
    ///
    /// `base` points to a live message of the type this was built for.
    #[inline]
    pub(super) unsafe fn compute_size(self, base: *const u8, cache: &mut SizeCache) -> u32 {
        // SAFETY: forwarded from the caller.
        unsafe {
            match self {
                Self::Table(table) => size::compute_size(table, base, cache),
                Self::Dyn(vt) => (vt.size)(base, cache),
            }
        }
    }

    /// Write the message at `base`, whose encoded size is `len`, to `buf`.
    ///
    /// # Safety
    ///
    /// `base` points to a live message of the type this was built for.
    #[inline]
    pub(super) unsafe fn write_to<K: EncodeSink>(
        self,
        base: *const u8,
        len: u32,
        cache: &mut SizeCache,
        buf: &mut K,
    ) {
        // SAFETY: forwarded from the caller.
        unsafe {
            match self {
                Self::Table(table) => encode::write_message(table, base, cache, buf),
                Self::Dyn(vt) => write_dyn(vt, base, len, cache, buf),
            }
        }
    }

    /// Decode a length-prefixed message from the front of `buf` into the
    /// message at `base`.
    ///
    /// # Safety
    ///
    /// `base` points to a live message of the type this was built for, which
    /// nothing else accesses.
    #[inline]
    pub(super) unsafe fn merge_sub(
        self,
        base: *mut u8,
        buf: &mut &[u8],
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        // SAFETY: forwarded from the caller.
        unsafe {
            match self {
                Self::Table(table) => decode::merge_sub(table, base, buf, ctx),
                Self::Dyn(vt) => (vt.merge)(base, buf, ctx),
            }
        }
    }
}

/// Write the message at `base`, of encoded size `len`, to `buf` through its
/// [`DynVt`].
///
/// # Safety
///
/// `base` points to a live message of the type `vt` was built for.
#[inline(never)]
unsafe fn write_dyn<K: EncodeSink>(
    vt: &DynVt,
    base: *const u8,
    len: u32,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    let mut through_cursor = |cursor: &mut PreSized<'_>| {
        // SAFETY: forwarded from the caller.
        unsafe { (vt.write)(base, &mut *cache, cursor) }
    };
    if buf.__with_pre_sized(&mut through_cursor) {
        return;
    }
    let len = len as usize;
    let scratch = write_to_new_vec(len, |cursor| {
        // SAFETY: forwarded from the caller.
        unsafe { (vt.write)(base, &mut *cache, cursor) }
    });
    crate::message::debug_assert_two_pass(scratch.len(), len);
    buf.put_slice(&scratch);
}

/// Write `len` and then the message at `base` to `buf`, as the value of a
/// length-delimited field whose tag has been written.
///
/// # Safety
///
/// `base` points to a live message of the type `child` was built for.
#[inline]
pub(super) unsafe fn write_field_value<K: EncodeSink>(
    child: Child,
    base: *const u8,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    let len = cache.consume_next();
    encode_varint(u64::from(len), buf);
    // SAFETY: forwarded from the caller.
    unsafe { child.write_to(base, len, cache, buf) };
}

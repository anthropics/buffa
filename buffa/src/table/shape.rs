//! Accessors for the field shapes a table entry cannot reach by offset alone:
//! message-typed fields (whose storage is a pointer or an `Option`) and
//! enum-typed fields (whose storage is an `i32` newtype or a closed enum).

use core::marker::PhantomData;

use super::{Table, IMPLICIT, OPTIONAL, REPEATED};
use crate::alloc::vec::Vec;
use crate::{EnumValue, Enumeration, MessageField, ProtoBox};

// ---------------------------------------------------------------------------
// Singular message fields
// ---------------------------------------------------------------------------

/// Storage of a singular message field.
pub trait MsgSlot {
    /// The message type the field holds.
    type Msg;
    /// The message, creating the default if the field is unset.
    fn place(&mut self) -> &mut Self::Msg;
    /// The message, or `None` if the field is unset.
    fn get(&self) -> Option<&Self::Msg>;
}

impl<T: Default, P: ProtoBox<T>> MsgSlot for MessageField<T, P> {
    type Msg = T;

    #[inline]
    fn place(&mut self) -> &mut T {
        self.get_or_insert_default()
    }

    #[inline]
    fn get(&self) -> Option<&T> {
        self.as_option()
    }
}

/// Descriptor of a singular message field: the child's table and how to reach
/// the child through the field's storage.
pub struct MsgVt {
    pub(super) table: &'static super::MessageTable,
    /// The message in the field, created with its default if unset.
    ///
    /// # Safety
    ///
    /// The argument points to a live `F`.
    pub(super) place: unsafe fn(*mut u8) -> *mut u8,
    /// The message in the field, or null if unset.
    ///
    /// # Safety
    ///
    /// The argument points to a live `F`.
    pub(super) get: unsafe fn(*const u8) -> *const u8,
}

/// # Safety
///
/// `slot` points to a live `F`.
unsafe fn place_impl<F: MsgSlot>(slot: *mut u8) -> *mut u8 {
    // SAFETY: the caller passes a pointer to a live `F`.
    unsafe { (*slot.cast::<F>()).place() as *mut F::Msg as *mut u8 }
}

/// # Safety
///
/// `slot` points to a live `F`.
unsafe fn get_impl<F: MsgSlot>(slot: *const u8) -> *const u8 {
    // SAFETY: the caller passes a pointer to a live `F`.
    match unsafe { (*slot.cast::<F>()).get() } {
        Some(m) => (m as *const F::Msg).cast::<u8>(),
        None => core::ptr::null(),
    }
}

impl MsgVt {
    /// Describe a field of type `F`, whose messages `table` describes.
    #[must_use]
    pub const fn new<F: MsgSlot>(table: &'static Table<F::Msg>) -> Self {
        Self {
            table: &table.raw,
            place: place_impl::<F>,
            get: get_impl::<F>,
        }
    }
}

// ---------------------------------------------------------------------------
// Repeated message fields
// ---------------------------------------------------------------------------

/// Descriptor of a repeated message field, a `Vec<T>`.
pub struct RepVt {
    pub(super) table: &'static super::MessageTable,
    /// The size in bytes of one element.
    pub(super) size: usize,
    /// Append a default element and return a pointer to it.
    ///
    /// # Safety
    ///
    /// The argument points to a live `Vec<T>`.
    pub(super) push: unsafe fn(*mut u8) -> *mut u8,
    /// Remove the last element, which `push` added.
    ///
    /// # Safety
    ///
    /// The argument points to a live `Vec<T>`.
    pub(super) pop: unsafe fn(*mut u8),
    /// The element storage: a pointer to the first element and the count.
    ///
    /// # Safety
    ///
    /// The argument points to a live `Vec<T>`.
    pub(super) parts: unsafe fn(*const u8) -> (*const u8, usize),
}

/// # Safety
///
/// `slot` points to a live `Vec<T>`.
unsafe fn push_impl<T: Default>(slot: *mut u8) -> *mut u8 {
    // SAFETY: the caller passes a pointer to a live `Vec<T>`.
    let v = unsafe { &mut *slot.cast::<Vec<T>>() };
    v.push(T::default());
    let last = v.len() - 1;
    // SAFETY: `last` is in bounds.
    unsafe { v.as_mut_ptr().add(last).cast::<u8>() }
}

/// # Safety
///
/// `slot` points to a live `Vec<T>`.
unsafe fn pop_impl<T>(slot: *mut u8) {
    // SAFETY: the caller passes a pointer to a live `Vec<T>`.
    unsafe { (*slot.cast::<Vec<T>>()).pop() };
}

/// # Safety
///
/// `slot` points to a live `Vec<T>`.
unsafe fn parts_impl<T>(slot: *const u8) -> (*const u8, usize) {
    // SAFETY: the caller passes a pointer to a live `Vec<T>`.
    let v = unsafe { &*slot.cast::<Vec<T>>() };
    (v.as_ptr().cast::<u8>(), v.len())
}

impl RepVt {
    /// Describe a `Vec<T>` field, whose messages `table` describes.
    #[must_use]
    pub const fn new<T: Default>(table: &'static Table<T>) -> Self {
        Self {
            table: &table.raw,
            size: core::mem::size_of::<T>(),
            push: push_impl::<T>,
            pop: pop_impl::<T>,
            parts: parts_impl::<T>,
        }
    }
}

// ---------------------------------------------------------------------------
// Enum fields
// ---------------------------------------------------------------------------

/// Descriptor of an enum field: how to store and read its `i32` values.
pub struct EnumVt {
    /// The cardinality the shape stores: `IMPLICIT`, `OPTIONAL` or
    /// `REPEATED`.
    pub(super) card: u8,
    /// Store `raw` (append, for a repeated field). `false` if a closed enum
    /// has no variant with that number, in which case nothing is stored.
    ///
    /// # Safety
    ///
    /// The argument points to a live slot of the shape the descriptor was
    /// built for.
    pub(super) set: unsafe fn(*mut u8, i32) -> bool,
    /// The value at `idx` (ignored for singular shapes), or `None` if unset.
    ///
    /// # Safety
    ///
    /// As for `set`.
    pub(super) get: unsafe fn(*const u8, usize) -> Option<i32>,
    /// The element count of a repeated shape, `0` otherwise.
    ///
    /// # Safety
    ///
    /// As for `set`.
    pub(super) len: unsafe fn(*const u8) -> usize,
}

/// A way of storing an enum field, implemented by the marker types below.
///
/// # Safety
///
/// [`Slot`](Self::Slot) is the type of the field, and its functions read and
/// write a field of that type. [`CARD`](Self::CARD) is its cardinality:
/// `IMPLICIT` for a singular field with no presence, `OPTIONAL` for
/// `Option<_>`, and `REPEATED` for `Vec<_>`; the table checks it against the
/// entry's kind.
pub unsafe trait EnumShape {
    /// The type of the field.
    type Slot;

    /// The cardinality of the field.
    const CARD: u8;

    /// Store `raw`, appending it if the shape is repeated. Returns `false`,
    /// and stores nothing, if a closed enum has no variant numbered `raw`.
    ///
    /// # Safety
    ///
    /// `slot` points to a live [`Slot`](Self::Slot).
    unsafe fn set(slot: *mut u8, raw: i32) -> bool;

    /// The value at `idx` (ignored for singular shapes), or `None` if unset.
    ///
    /// # Safety
    ///
    /// `slot` points to a live [`Slot`](Self::Slot).
    unsafe fn get(slot: *const u8, idx: usize) -> Option<i32>;

    /// The element count of a repeated shape, `0` otherwise.
    ///
    /// # Safety
    ///
    /// `slot` points to a live [`Slot`](Self::Slot).
    unsafe fn len(_slot: *const u8) -> usize {
        0
    }
}

impl EnumVt {
    /// Describe a field stored as `S`.
    #[must_use]
    pub const fn new<S: EnumShape>() -> Self {
        Self {
            card: S::CARD,
            set: S::set,
            get: S::get,
            len: S::len,
        }
    }
}

macro_rules! enum_shape {
    ($(#[$m:meta])* $name:ident, $card:ident, $slot:ty, $set:expr, $get:expr, $len:expr) => {
        $(#[$m])*
        pub struct $name<E>(PhantomData<E>);

        // SAFETY: each function reads or writes the slot as `$slot`, which
        // the trait's contract says it is.
        unsafe impl<E: Enumeration> EnumShape for $name<E> {
            type Slot = $slot;
            const CARD: u8 = $card;

            #[inline]
            unsafe fn set(slot: *mut u8, raw: i32) -> bool {
                // SAFETY: the caller passes a pointer to a live slot of this shape.
                let s = unsafe { &mut *slot.cast::<$slot>() };
                ($set)(s, raw)
            }

            #[inline]
            unsafe fn get(slot: *const u8, idx: usize) -> Option<i32> {
                // SAFETY: as above.
                let s = unsafe { &*slot.cast::<$slot>() };
                ($get)(s, idx)
            }

            #[inline]
            unsafe fn len(slot: *const u8) -> usize {
                // SAFETY: as above.
                let s = unsafe { &*slot.cast::<$slot>() };
                ($len)(s)
            }
        }
    };
}

enum_shape!(
    /// An open enum with implicit presence: `EnumValue<E>`.
    ImplicitOpen, IMPLICIT, EnumValue<E>,
    |s: &mut EnumValue<E>, raw| { *s = EnumValue::from(raw); true },
    |s: &EnumValue<E>, _| Some(s.to_i32()),
    |_: &EnumValue<E>| 0
);
enum_shape!(
    /// A closed enum with implicit presence: `E`.
    ImplicitClosed, IMPLICIT, E,
    |s: &mut E, raw| match E::from_i32(raw) { Some(v) => { *s = v; true } None => false },
    |s: &E, _| Some(s.to_i32()),
    |_: &E| 0
);
enum_shape!(
    /// An open enum with explicit presence: `Option<EnumValue<E>>`.
    OptionalOpen, OPTIONAL, Option<EnumValue<E>>,
    |s: &mut Option<EnumValue<E>>, raw| { *s = Some(EnumValue::from(raw)); true },
    |s: &Option<EnumValue<E>>, _| s.as_ref().map(EnumValue::to_i32),
    |_: &Option<EnumValue<E>>| 0
);
enum_shape!(
    /// A closed enum with explicit presence: `Option<E>`.
    OptionalClosed, OPTIONAL, Option<E>,
    |s: &mut Option<E>, raw| match E::from_i32(raw) { Some(v) => { *s = Some(v); true } None => false },
    |s: &Option<E>, _| s.as_ref().map(Enumeration::to_i32),
    |_: &Option<E>| 0
);
enum_shape!(
    /// A repeated open enum: `Vec<EnumValue<E>>`.
    RepeatedOpen, REPEATED, Vec<EnumValue<E>>,
    |s: &mut Vec<EnumValue<E>>, raw| { s.push(EnumValue::from(raw)); true },
    |s: &Vec<EnumValue<E>>, i| s.get(i).map(EnumValue::to_i32),
    |s: &Vec<EnumValue<E>>| s.len()
);
enum_shape!(
    /// A repeated closed enum: `Vec<E>`.
    RepeatedClosed, REPEATED, Vec<E>,
    |s: &mut Vec<E>, raw| match E::from_i32(raw) { Some(v) => { s.push(v); true } None => false },
    |s: &Vec<E>, i| s.get(i).map(Enumeration::to_i32),
    |s: &Vec<E>| s.len()
);

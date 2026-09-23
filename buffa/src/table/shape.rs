//! Accessors for the field shapes a table entry cannot reach by offset alone:
//! message-typed fields (whose storage is a pointer or an `Option`) and
//! enum-typed fields (whose storage is an `i32` newtype or a closed enum).

use core::marker::PhantomData;

use super::bridge::Child;
use super::{Table, IMPLICIT, OPTIONAL, REPEATED};
use crate::alloc::vec::Vec;
use crate::{EnumValue, Enumeration, Message, MessageField, ProtoBox};

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

/// Descriptor of a singular message field: how to encode, size and decode the
/// child and how to reach it through the field's storage.
pub struct MsgVt {
    pub(super) child: Child,
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
    /// Whether the descriptor is for a message reached through a pointer to
    /// the message itself ([`direct`](Self::direct)), which is what a oneof
    /// member needs, and not through a field's storage.
    pub(super) direct: bool,
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

/// The accessors of a message that the table reaches through a pointer to
/// the message itself: the message is there whenever the pointer is.
///
/// # Safety
///
/// `slot` points to a live message, which is the one that is returned.
unsafe fn direct_place(slot: *mut u8) -> *mut u8 {
    slot
}

/// # Safety
///
/// `slot` points to a live message, which is the one that is returned.
unsafe fn direct_get(slot: *const u8) -> *const u8 {
    slot
}

impl MsgVt {
    /// Describe a field of type `F`, whose message is a table message and
    /// `table` is its table.
    #[must_use]
    pub const fn new<F: MsgSlot>(table: &'static Table<F::Msg>) -> Self {
        Self {
            child: Child::Table(&table.raw),
            place: place_impl::<F>,
            get: get_impl::<F>,
            direct: false,
        }
    }

    /// Describe a field of type `F` whose message is reached through its
    /// [`Message`] impl, for a message whose table is not visible here.
    /// Prefer [`MsgVt::new`] when it is, because the interpreters then decode
    /// the child without a function call. The child is decoded from a slice,
    /// so its `bytes::Bytes` fields are copied, where unrolled code decoding
    /// from a `Bytes` shares them.
    #[must_use]
    pub const fn new_via_message<F: MsgSlot>() -> Self
    where
        F::Msg: Message,
    {
        Self {
            child: Child::of::<F::Msg>(),
            place: place_impl::<F>,
            get: get_impl::<F>,
            direct: false,
        }
    }

    /// Describe a message that is reached through a pointer to the message
    /// itself, which is how a oneof member's payload of message type is
    /// reached, whether the oneof stores it boxed or inline. `table`
    /// describes the message.
    #[must_use]
    pub const fn direct<M>(table: &'static Table<M>) -> Self {
        Self {
            child: Child::Table(&table.raw),
            place: direct_place,
            get: direct_get,
            direct: true,
        }
    }

    /// As [`direct`](Self::direct), for a message whose table is not visible
    /// here, which is reached through its [`Message`] impl; see
    /// [`new_via_message`](Self::new_via_message).
    #[must_use]
    pub const fn direct_via_message<M: Message>() -> Self {
        Self {
            child: Child::of::<M>(),
            place: direct_place,
            get: direct_get,
            direct: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Repeated message fields
// ---------------------------------------------------------------------------

/// Descriptor of a repeated message field, a `Vec<T>`.
pub struct RepVt {
    pub(super) child: Child,
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
    /// Describe a `Vec<T>` field whose messages are table messages, `table`
    /// being their table.
    #[must_use]
    pub const fn new<T: Default>(table: &'static Table<T>) -> Self {
        Self {
            child: Child::Table(&table.raw),
            size: core::mem::size_of::<T>(),
            push: push_impl::<T>,
            pop: pop_impl::<T>,
            parts: parts_impl::<T>,
        }
    }

    /// Describe a `Vec<T>` field whose messages are reached through their
    /// [`Message`] impl. Like [`MsgVt::new_via_message`], it copies the
    /// `bytes::Bytes` fields of the elements when it decodes.
    #[must_use]
    pub const fn new_via_message<T: Message>() -> Self {
        Self {
            child: Child::of::<T>(),
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
    /// Whether `set` would store `raw`: `false` for a closed enum that has no
    /// variant with that number.
    pub(super) accepts: fn(i32) -> bool,
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

    /// Whether [`set`](Self::set) would store `raw`.
    fn accepts(raw: i32) -> bool;
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
            accepts: S::accepts,
        }
    }
}

macro_rules! enum_shape {
    ($(#[$m:meta])* $name:ident, $card:ident, $slot:ty, $set:expr, $get:expr, $len:expr, $accepts:expr) => {
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

            #[inline]
            fn accepts(raw: i32) -> bool {
                ($accepts)(raw)
            }
        }
    };
}

enum_shape!(
    /// An open enum with implicit presence: `EnumValue<E>`.
    ImplicitOpen, IMPLICIT, EnumValue<E>,
    |s: &mut EnumValue<E>, raw| { *s = EnumValue::from(raw); true },
    |s: &EnumValue<E>, _| Some(s.to_i32()),
    |_: &EnumValue<E>| 0,
    |_: i32| true
);
enum_shape!(
    /// A closed enum with implicit presence: `E`.
    ImplicitClosed, IMPLICIT, E,
    |s: &mut E, raw| match E::from_i32(raw) { Some(v) => { *s = v; true } None => false },
    |s: &E, _| Some(s.to_i32()),
    |_: &E| 0,
    |raw| E::from_i32(raw).is_some()
);
enum_shape!(
    /// An open enum with explicit presence: `Option<EnumValue<E>>`.
    OptionalOpen, OPTIONAL, Option<EnumValue<E>>,
    |s: &mut Option<EnumValue<E>>, raw| { *s = Some(EnumValue::from(raw)); true },
    |s: &Option<EnumValue<E>>, _| s.as_ref().map(EnumValue::to_i32),
    |_: &Option<EnumValue<E>>| 0,
    |_: i32| true
);
enum_shape!(
    /// A closed enum with explicit presence: `Option<E>`.
    OptionalClosed, OPTIONAL, Option<E>,
    |s: &mut Option<E>, raw| match E::from_i32(raw) { Some(v) => { *s = Some(v); true } None => false },
    |s: &Option<E>, _| s.as_ref().map(Enumeration::to_i32),
    |_: &Option<E>| 0,
    |raw| E::from_i32(raw).is_some()
);
enum_shape!(
    /// A repeated open enum: `Vec<EnumValue<E>>`.
    RepeatedOpen, REPEATED, Vec<EnumValue<E>>,
    |s: &mut Vec<EnumValue<E>>, raw| { s.push(EnumValue::from(raw)); true },
    |s: &Vec<EnumValue<E>>, i| s.get(i).map(EnumValue::to_i32),
    |s: &Vec<EnumValue<E>>| s.len(),
    |_: i32| true
);
enum_shape!(
    /// A repeated closed enum: `Vec<E>`.
    RepeatedClosed, REPEATED, Vec<E>,
    |s: &mut Vec<E>, raw| match E::from_i32(raw) { Some(v) => { s.push(v); true } None => false },
    |s: &Vec<E>, i| s.get(i).map(Enumeration::to_i32),
    |s: &Vec<E>| s.len(),
    |raw| E::from_i32(raw).is_some()
);

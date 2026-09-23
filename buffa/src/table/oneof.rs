//! Oneof fields: how the interpreters reach the members of a oneof enum.
//!
//! A oneof is stored as an `Option<E>`, where `E` is an enum with one variant
//! per member. The layout of that enum is unspecified, so a table cannot
//! address a member by offset as it does an ordinary field. Instead generated
//! code implements [`OneofEnum`] for `E`, which gives the interpreters the
//! number of the current member and a pointer to its value, and the table has
//! one entry per member, of kind [`Kind::OneofMember`].
//!
//! A member is written when it is set, whatever its value, so its value is
//! described by a `Required` kind, its *payload kind*, and the interpreters
//! run the same arms on it as for an ordinary field of that kind. Every member
//! entry has the offset of the `Option<E>`. The entry of the member with the
//! lowest field number, the *leader*, writes and sizes whichever member is set,
//! at that position among the message's fields, as unrolled code does. The
//! other entries only decode, so a message with a oneof writes the same bytes
//! under either strategy.

use super::{Entry, Kind};

/// How the interpreters reach the members of the oneof enum `Self`, which
/// generated code implements for the enum of every oneof of a table message.
///
/// The pointers a member's payload is reached through must be of the type its
/// payload kind names: `i32` for `Int32Required`, `String` for `StrRequired`,
/// the storage of the enum for `EnumRequired`, and the message itself,
/// reached through its pointer if it is boxed, for `MsgSingular`. A table's
/// constructor takes on that requirement.
pub trait OneofEnum: Sized {
    /// The field number of the member `self` holds.
    fn number(&self) -> u32;

    /// A pointer to the value of the member `self` holds.
    fn payload(&self) -> *const u8;

    /// As [`payload`](Self::payload), for writing.
    fn payload_mut(&mut self) -> *mut u8;

    /// A value holding the default value of the member `number`, or `None` if
    /// `Self` has no such member.
    fn with_default(number: u32) -> Option<Self>;
}

/// Descriptor of a oneof: how to read and replace the member of an
/// `Option<E>` through its address.
pub struct OneofVt {
    /// The offset of the `Option<E>` in the message struct.
    pub(super) offset: u32,
    /// The lowest field number among the members.
    pub(super) first: u32,
    /// The number of the member that is set, or `0`, and a pointer to its
    /// value.
    ///
    /// # Safety
    ///
    /// The argument points to a live `Option<E>`.
    pub(super) get: unsafe fn(*const u8) -> (u32, *const u8),
    /// Make the member `number` the one that is set, keeping its value if it
    /// already is, and return a pointer to its value.
    ///
    /// # Safety
    ///
    /// The argument points to a live `Option<E>`, and `number` is a member of
    /// `E`.
    pub(super) place: unsafe fn(*mut u8, u32) -> *mut u8,
}

/// # Safety
///
/// `slot` points to a live `Option<E>`.
unsafe fn get_impl<E: OneofEnum>(slot: *const u8) -> (u32, *const u8) {
    // SAFETY: the caller passes a pointer to a live `Option<E>`.
    match unsafe { &*slot.cast::<Option<E>>() } {
        Some(e) => (e.number(), e.payload()),
        None => (0, core::ptr::null()),
    }
}

/// # Safety
///
/// `slot` points to a live `Option<E>`.
unsafe fn place_impl<E: OneofEnum>(slot: *mut u8, number: u32) -> *mut u8 {
    // SAFETY: the caller passes a pointer to a live `Option<E>`.
    let e = unsafe { &mut *slot.cast::<Option<E>>() };
    if e.as_ref().map(E::number) != Some(number) {
        *e = E::with_default(number);
    }
    match e {
        Some(e) => e.payload_mut(),
        None => no_such_member(number),
    }
}

/// The message has an entry, or an accessor, for a member that its oneof enum
/// does not have, which only a bug in generated code can cause.
#[cold]
#[inline(never)]
pub(super) fn no_such_member(number: u32) -> ! {
    panic!("buffa table: oneof member {number} does not match the oneof enum")
}

impl OneofVt {
    /// Describe the oneof stored as an `Option<E>` at `offset` in the message
    /// struct, whose lowest member number is `first`.
    ///
    /// # Panics
    ///
    /// Panics, at compile time when used to initialise a `static`, if
    /// `offset` does not fit in a `u32` or `first` is not a field number.
    #[must_use]
    pub const fn new<E: OneofEnum>(offset: usize, first: u32) -> Self {
        assert!(offset <= u32::MAX as usize, "field offset out of range");
        assert!(first >= 1 && first < (1 << 29), "field number out of range");
        Self {
            offset: offset as u32,
            first,
            get: get_impl::<E>,
            place: place_impl::<E>,
        }
    }
}

/// One member of a oneof, referred to by the aux index of its entry.
#[derive(Clone, Copy, Debug)]
pub struct Member {
    /// The aux index of the oneof's [`Aux::Group`](super::Aux::Group).
    pub(super) group: u16,
    /// The aux index of the payload's descriptor, for a payload kind that has
    /// one.
    pub(super) aux: u16,
    /// The kind of the member's value.
    pub(super) kind: Kind,
    /// Whether this is the member with the lowest number, which writes and
    /// sizes the oneof.
    pub(super) leader: bool,
}

impl Member {
    /// A member of the oneof whose group is at aux index `group`, holding a
    /// value of kind `kind`, whose descriptor, if it needs one, is at aux
    /// index `aux`. `leader` is whether it has the lowest number in its oneof.
    #[must_use]
    pub const fn new(group: u16, kind: Kind, aux: u16, leader: bool) -> Self {
        Self {
            group,
            aux,
            kind,
            leader,
        }
    }
}

/// Check the member `m` of the entry `e` against the rest of the aux array,
/// and return whether it is the leader of its oneof.
///
/// # Panics
///
/// Panics, at compile time when used to initialise a `static`, if the member is
/// inconsistent with the oneof it names or with its payload's descriptor.
pub(super) const fn check_member(e: &Entry, m: Member, aux: &[super::Aux]) -> bool {
    assert!(
        (m.group as usize) < aux.len(),
        "buffa table: a oneof member's group index is out of range"
    );
    let super::Aux::Group(g) = &aux[m.group as usize] else {
        panic!("buffa table: a oneof member's group index is not a oneof descriptor")
    };
    assert!(
        m.kind.is_oneof_payload(),
        "buffa table: a oneof member's payload kind is not one a payload can have"
    );
    assert!(
        e.tag & 7 == m.kind.wire_type(),
        "buffa table: a oneof member's tag does not have its payload kind's wire type"
    );
    if let Some(want) = m.kind.aux_kind() {
        assert!(
            (m.aux as usize) < aux.len(),
            "buffa table: a oneof member's payload aux index is out of range"
        );
        let a = &aux[m.aux as usize];
        assert!(
            a.kind() as u8 == want as u8,
            "buffa table: a oneof member's payload descriptor is the wrong variant for its kind"
        );
        if let super::Aux::Enum(vt) = a {
            assert!(
                vt.card == super::IMPLICIT,
                "buffa table: a oneof member's enum descriptor must be for a singular field"
            );
        }
    }
    assert!(
        e.offset == g.offset,
        "buffa table: a oneof member's offset differs from its oneof's"
    );
    assert!(
        e.number() >= g.first,
        "buffa table: a oneof member is numbered below its oneof's lowest"
    );
    assert!(
        m.leader == (e.number() == g.first),
        "buffa table: the leader of a oneof must be exactly the member with its lowest number"
    );
    m.leader
}

//! Table-driven message codec: the runtime behind the table codec strategy of
//! code generation.
//!
//! Generated code normally emits `compute_size`, `write_to` and `merge_field`
//! bodies specialised to each message. With the table strategy it emits one
//! static [`Table`] per message instead: a sorted array of twelve-byte
//! [`Entry`] values, one per field, giving the field number, its byte offset
//! in the message struct, and a [`Kind`] (field type crossed with
//! cardinality). The `Message` methods forward to the interpreters here,
//! which every message shares.
//!
//! This module is support code for generated code, is not meant to be called
//! directly, and may change in any release, so generated code must be
//! regenerated with the `buffa-codegen` that matches the `buffa` it builds
//! against. [`ABI`] enforces that when the table is built. Building a
//! [`Table`] is `unsafe`, because the table's offsets and kinds must describe
//! the message struct's actual layout; the `__table!` and `__table_entry!` macros
//! that generated code uses check what the compiler can, and every other entry
//! point is safe and sound given a correct table.
//!
//! Generated code should name everything by absolute path (`::buffa::table::…`):
//! `Entry`, `Kind` and `Table` are also plausible message names.
//!
//! # Compared with unrolled code
//!
//! The wire format, the two-pass [`SizeCache`] protocol, the recursion,
//! unknown-field and element-memory limits of [`DecodeContext`], and whether
//! a given input decodes are the same, so a message can change strategy
//! without changing behaviour. The differences are:
//!
//! - Decoding runs over one contiguous slice, so a non-contiguous [`Buf`] is
//!   gathered into a buffer first, and a field that declares a length past
//!   the end of its enclosing message fails at once with
//!   [`DecodeError::UnexpectedEof`], where unrolled code reads on into the
//!   enclosing message and can report a different error. A child reached
//!   through its [`Message`](crate::Message) impl is read from the slice of
//!   the nearest enclosing table message, so a read that overruns it fails
//!   at that message's end, and not where a tree of unrolled messages would
//!   notice.
//! - [`Table::merge_field`] decodes one field and cannot gather, so it
//!   returns [`DecodeError::UnexpectedEof`] for a buffer that is not one
//!   chunk. Only a caller that drives `merge_field` itself is affected, such
//!   as the default `Message::merge_group`, so a message that is the type of a
//!   group field must not use the table strategy.
//! - [`clear`](crate::Message::clear) resets to `Default`, which releases
//!   allocations that unrolled code keeps.
//! - A child reached through its [`Message`](crate::Message) impl (see
//!   [`MsgVt::new_via_message`]) has the same wire format and accepts the same
//!   input, but it is staged in a scratch buffer and copied when it is written
//!   to any sink other than the cursor that `Message::encode` and its siblings
//!   write a [`BufMut`](crate::bytes::BufMut) through, and its `bytes::Bytes`
//!   fields are copied out of the slice it is decoded from, where unrolled
//!   code decoding from a `Bytes` shares them with the input.
//!
//! # Oneofs
//!
//! A oneof has one entry per member, and its members are reached through
//! [`OneofEnum`]; the `oneof` module's documentation has the mechanism.
//!
//! # Where the code is compiled
//!
//! `compute_size`, decoding from a contiguous buffer and encoding into a
//! [`BufMut`](crate::bytes::BufMut) through `Message::encode` and its
//! siblings are non-generic functions compiled in this crate, at this crate's
//! optimisation level, once for all messages. A build can therefore optimise
//! this crate for speed and its own generated code for size. Encoding into
//! any other sink, such as a [`Rope`](crate::Rope) or a `BufMut` passed
//! straight to `Message::write_to`, and the generic wrappers around
//! decoding are instantiated in the crate that calls them.

use core::marker::PhantomData;

use crate::alloc::{string::String, vec::Vec};
use crate::bytes::Buf;
use crate::encoding::{Tag, WireType};
use crate::{DecodeContext, DecodeError, EncodeSink, SizeCache, UnknownFields};

pub use oneof::{Member, OneofEnum, OneofVt};
pub use shape::{
    EnumShape, EnumVt, ImplicitClosed, ImplicitOpen, MsgSlot, MsgVt, OptionalClosed, OptionalOpen,
    RepVt, RepeatedClosed, RepeatedOpen,
};

use scalar::{
    Bool, Double, Fixed32, Fixed64, Float, Int32, Int64, Sc, Sfixed32, Sfixed64, Sint32, Sint64,
    Uint32, Uint64,
};

/// The version of the contract between generated tables and this module.
///
/// Generated code passes the version it was generated for to [`Table::new`],
/// which refuses to build a table for any other, because a table built for
/// different rules could make the interpreters read fields at the wrong
/// offsets.
pub const ABI: u32 = 1;

/// The `unknown` offset of a message that does not preserve unknown fields.
const NO_UNKNOWN: u32 = u32::MAX;

/// Re-exported for generated code, which needs it to build a [`Table`] and
/// must work on the crate's minimum supported Rust version.
///
/// Requires Rust 1.77; on older compilers using it is a compile error.
#[doc(hidden)]
#[rustversion::since(1.77)]
pub use core::mem::offset_of;

/// Stand-in for `core::mem::offset_of!` on compilers that lack it.
#[doc(hidden)]
#[rustversion::before(1.77)]
#[macro_export]
macro_rules! __buffa_offset_of_unavailable {
    ($($tt:tt)*) => {
        compile_error!(
            "the table codec strategy requires Rust 1.77 or newer (`core::mem::offset_of!`)"
        )
    };
}

#[doc(hidden)]
#[rustversion::before(1.77)]
pub use __buffa_offset_of_unavailable as offset_of;

// Cardinalities. The `Msg` kinds use `IMPLICIT` for a singular field. The two
// `Oneof` kinds are written with the cardinality names `LEADER` and `ONEOF`,
// which are only tokens that the kind macros match.
const IMPLICIT: u8 = 0;
const REQUIRED: u8 = 1;
const OPTIONAL: u8 = 2;
const REPEATED: u8 = 3;
const PACKED: u8 = 4;

/// Calls `$callback!` with every [`Kind`] as `Name: Family Type Cardinality;`,
/// after `$fname;` if one is given, which names the function a dispatch macro
/// defines.
///
/// The families are `Scalar`, `Str`, `Bytes`, `Enum`, `Msg` and `Oneof`. This
/// list generates the [`Kind`] enum and the three dispatch functions over
/// entries (size, write and merge), so a kind cannot be added to one and not
/// the others. [`payload_kind_table!`] is a second list, of the kinds a oneof
/// member's value can have, which generates the three dispatch functions over
/// those values.
macro_rules! kind_table {
    ($callback:ident $(, $fname:ident)?) => {
        $callback! {
            $($fname;)?
            Int32Implicit: Scalar Int32 IMPLICIT;
            Int32Required: Scalar Int32 REQUIRED;
            Int32Optional: Scalar Int32 OPTIONAL;
            Int32Repeated: Scalar Int32 REPEATED;
            Int32Packed: Scalar Int32 PACKED;
            Int64Implicit: Scalar Int64 IMPLICIT;
            Int64Required: Scalar Int64 REQUIRED;
            Int64Optional: Scalar Int64 OPTIONAL;
            Int64Repeated: Scalar Int64 REPEATED;
            Int64Packed: Scalar Int64 PACKED;
            Uint32Implicit: Scalar Uint32 IMPLICIT;
            Uint32Required: Scalar Uint32 REQUIRED;
            Uint32Optional: Scalar Uint32 OPTIONAL;
            Uint32Repeated: Scalar Uint32 REPEATED;
            Uint32Packed: Scalar Uint32 PACKED;
            Uint64Implicit: Scalar Uint64 IMPLICIT;
            Uint64Required: Scalar Uint64 REQUIRED;
            Uint64Optional: Scalar Uint64 OPTIONAL;
            Uint64Repeated: Scalar Uint64 REPEATED;
            Uint64Packed: Scalar Uint64 PACKED;
            Sint32Implicit: Scalar Sint32 IMPLICIT;
            Sint32Required: Scalar Sint32 REQUIRED;
            Sint32Optional: Scalar Sint32 OPTIONAL;
            Sint32Repeated: Scalar Sint32 REPEATED;
            Sint32Packed: Scalar Sint32 PACKED;
            Sint64Implicit: Scalar Sint64 IMPLICIT;
            Sint64Required: Scalar Sint64 REQUIRED;
            Sint64Optional: Scalar Sint64 OPTIONAL;
            Sint64Repeated: Scalar Sint64 REPEATED;
            Sint64Packed: Scalar Sint64 PACKED;
            BoolImplicit: Scalar Bool IMPLICIT;
            BoolRequired: Scalar Bool REQUIRED;
            BoolOptional: Scalar Bool OPTIONAL;
            BoolRepeated: Scalar Bool REPEATED;
            BoolPacked: Scalar Bool PACKED;
            Fixed32Implicit: Scalar Fixed32 IMPLICIT;
            Fixed32Required: Scalar Fixed32 REQUIRED;
            Fixed32Optional: Scalar Fixed32 OPTIONAL;
            Fixed32Repeated: Scalar Fixed32 REPEATED;
            Fixed32Packed: Scalar Fixed32 PACKED;
            Fixed64Implicit: Scalar Fixed64 IMPLICIT;
            Fixed64Required: Scalar Fixed64 REQUIRED;
            Fixed64Optional: Scalar Fixed64 OPTIONAL;
            Fixed64Repeated: Scalar Fixed64 REPEATED;
            Fixed64Packed: Scalar Fixed64 PACKED;
            Sfixed32Implicit: Scalar Sfixed32 IMPLICIT;
            Sfixed32Required: Scalar Sfixed32 REQUIRED;
            Sfixed32Optional: Scalar Sfixed32 OPTIONAL;
            Sfixed32Repeated: Scalar Sfixed32 REPEATED;
            Sfixed32Packed: Scalar Sfixed32 PACKED;
            Sfixed64Implicit: Scalar Sfixed64 IMPLICIT;
            Sfixed64Required: Scalar Sfixed64 REQUIRED;
            Sfixed64Optional: Scalar Sfixed64 OPTIONAL;
            Sfixed64Repeated: Scalar Sfixed64 REPEATED;
            Sfixed64Packed: Scalar Sfixed64 PACKED;
            FloatImplicit: Scalar Float IMPLICIT;
            FloatRequired: Scalar Float REQUIRED;
            FloatOptional: Scalar Float OPTIONAL;
            FloatRepeated: Scalar Float REPEATED;
            FloatPacked: Scalar Float PACKED;
            DoubleImplicit: Scalar Double IMPLICIT;
            DoubleRequired: Scalar Double REQUIRED;
            DoubleOptional: Scalar Double OPTIONAL;
            DoubleRepeated: Scalar Double REPEATED;
            DoublePacked: Scalar Double PACKED;
            StrImplicit: Str Str IMPLICIT;
            StrRequired: Str Str REQUIRED;
            StrOptional: Str Str OPTIONAL;
            StrRepeated: Str Str REPEATED;
            BytesImplicit: Bytes Bytes IMPLICIT;
            BytesRequired: Bytes Bytes REQUIRED;
            BytesOptional: Bytes Bytes OPTIONAL;
            BytesRepeated: Bytes Bytes REPEATED;
            EnumImplicit: Enum Enum IMPLICIT;
            EnumRequired: Enum Enum REQUIRED;
            EnumOptional: Enum Enum OPTIONAL;
            EnumRepeated: Enum Enum REPEATED;
            EnumPacked: Enum Enum PACKED;
            MsgSingular: Msg Msg IMPLICIT;
            MsgRepeated: Msg Msg REPEATED;
            OneofLeader: Oneof Oneof LEADER;
            OneofFollower: Oneof Oneof ONEOF;
        }
    };
}

/// [`kind_table!`] for the kinds that a oneof member's payload can have: a
/// value that is written whenever the member is set, so always the `Required`
/// cardinality.
macro_rules! payload_kind_table {
    ($callback:ident $(, $fname:ident)?) => {
        $callback! {
            $($fname;)?
            Int32Required: Scalar Int32 REQUIRED;
            Int64Required: Scalar Int64 REQUIRED;
            Uint32Required: Scalar Uint32 REQUIRED;
            Uint64Required: Scalar Uint64 REQUIRED;
            Sint32Required: Scalar Sint32 REQUIRED;
            Sint64Required: Scalar Sint64 REQUIRED;
            BoolRequired: Scalar Bool REQUIRED;
            Fixed32Required: Scalar Fixed32 REQUIRED;
            Fixed64Required: Scalar Fixed64 REQUIRED;
            Sfixed32Required: Scalar Sfixed32 REQUIRED;
            Sfixed64Required: Scalar Sfixed64 REQUIRED;
            FloatRequired: Scalar Float REQUIRED;
            DoubleRequired: Scalar Double REQUIRED;
            StrRequired: Str Str REQUIRED;
            BytesRequired: Bytes Bytes REQUIRED;
            EnumRequired: Enum Enum REQUIRED;
            MsgSingular: Msg Msg IMPLICIT;
        }
    };
}

macro_rules! define_kind {
    ($($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        /// The type and cardinality of a field: one interpreter arm each.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u8)]
        pub enum Kind {
            $($name,)*
        }

        impl Kind {
            /// The wire type of the field's tag, as a number.
            const fn wire_type(self) -> u32 {
                match self {
                    $(Kind::$name => define_kind!(@wire $fam $ty $card),)*
                }
            }

            /// Whether the entry carries an index into the table's aux array.
            const fn aux_kind(self) -> Option<AuxKind> {
                match self {
                    $(Kind::$name => define_kind!(@aux $fam $card),)*
                }
            }

            /// The cardinality a field of this kind has in an enum shape:
            /// `IMPLICIT` (also for a required field), `OPTIONAL` or `REPEATED`
            /// (also for a packed field).
            const fn shape_card(self) -> u8 {
                match self {
                    $(Kind::$name => define_kind!(@shape $card),)*
                }
            }
        }

        /// One zero-sized type per [`Kind`], named the same, for the type
        /// checks that `__table_entry!` makes.
        pub mod kinds {
            #[allow(clippy::wildcard_imports)]
            use super::*;

            $(
                #[doc = concat!("The type-level name of [`Kind::", stringify!($name), "`](super::Kind::", stringify!($name), ").")]
                pub struct $name;
                define_kind!(@slot $name $fam $ty $card);
            )*
        }
    };
    (@slot $name:ident Scalar $ty:ident IMPLICIT) => { impl KindSlot for $name { type Slot = <$ty as Sc>::V; } };
    (@slot $name:ident Scalar $ty:ident REQUIRED) => { impl KindSlot for $name { type Slot = <$ty as Sc>::V; } };
    (@slot $name:ident Scalar $ty:ident OPTIONAL) => { impl KindSlot for $name { type Slot = Option<<$ty as Sc>::V>; } };
    (@slot $name:ident Scalar $ty:ident REPEATED) => { impl KindSlot for $name { type Slot = Vec<<$ty as Sc>::V>; } };
    (@slot $name:ident Scalar $ty:ident PACKED) => { impl KindSlot for $name { type Slot = Vec<<$ty as Sc>::V>; } };
    (@slot $name:ident Str $ty:ident IMPLICIT) => { impl KindSlot for $name { type Slot = String; } };
    (@slot $name:ident Str $ty:ident REQUIRED) => { impl KindSlot for $name { type Slot = String; } };
    (@slot $name:ident Str $ty:ident OPTIONAL) => { impl KindSlot for $name { type Slot = Option<String>; } };
    (@slot $name:ident Str $ty:ident REPEATED) => { impl KindSlot for $name { type Slot = Vec<String>; } };
    (@slot $name:ident Bytes $ty:ident IMPLICIT) => { impl KindSlot for $name { type Slot = Vec<u8>; } };
    (@slot $name:ident Bytes $ty:ident REQUIRED) => { impl KindSlot for $name { type Slot = Vec<u8>; } };
    (@slot $name:ident Bytes $ty:ident OPTIONAL) => { impl KindSlot for $name { type Slot = Option<Vec<u8>>; } };
    (@slot $name:ident Bytes $ty:ident REPEATED) => { impl KindSlot for $name { type Slot = Vec<Vec<u8>>; } };
    // Enum and message fields are checked against the type in their aux
    // descriptor, which the generated entry names.
    (@slot $name:ident Enum $ty:ident $card:ident) => {};
    (@slot $name:ident Msg $ty:ident $card:ident) => {};
    // A oneof member's field is the `Option` of the oneof's enum, which its
    // aux descriptors check.
    (@slot $name:ident Oneof $ty:ident $card:ident) => {};
    (@wire Scalar $ty:ident PACKED) => { WireType::LengthDelimited as u32 };
    (@wire Scalar $ty:ident $card:ident) => { <$ty as Sc>::WIRE as u32 };
    (@wire Str $ty:ident $card:ident) => { WireType::LengthDelimited as u32 };
    (@wire Bytes $ty:ident $card:ident) => { WireType::LengthDelimited as u32 };
    (@wire Msg $ty:ident $card:ident) => { WireType::LengthDelimited as u32 };
    (@wire Enum $ty:ident PACKED) => { WireType::LengthDelimited as u32 };
    (@wire Enum $ty:ident $card:ident) => { WireType::Varint as u32 };
    (@wire Oneof $ty:ident $card:ident) => {
        panic!("a oneof member's wire type is its payload kind's, so build its entry with `Entry::oneof_member`")
    };
    (@shape IMPLICIT) => { IMPLICIT };
    (@shape REQUIRED) => { IMPLICIT };
    (@shape OPTIONAL) => { OPTIONAL };
    (@shape REPEATED) => { REPEATED };
    (@shape PACKED) => { REPEATED };
    (@shape ONEOF) => { IMPLICIT };
    (@shape LEADER) => { IMPLICIT };
    (@aux Scalar $card:ident) => { None };
    (@aux Str $card:ident) => { None };
    (@aux Bytes $card:ident) => { None };
    (@aux Enum $card:ident) => { Some(AuxKind::Enum) };
    (@aux Oneof $card:ident) => { Some(AuxKind::Member) };
    (@aux Msg REPEATED) => { Some(AuxKind::Rep) };
    (@aux Msg $card:ident) => { Some(AuxKind::Msg) };
}

/// The type of the field that an entry of a [`kinds`] type describes, for the
/// scalar, string and bytes kinds. Enum and message kinds have none, because
/// their field type depends on the enum or message.
pub trait KindSlot {
    /// The type of the field.
    type Slot;
}

kind_table!(define_kind);

macro_rules! define_payload_check {
    ($($name:ident: $fam:ident $ty:ident $card:ident;)*) => {
        impl Kind {
            /// Whether a oneof member's payload can be of this kind.
            const fn is_oneof_payload(self) -> bool {
                matches!(self, $(Kind::$name)|*)
            }
        }
    };
}

payload_kind_table!(define_payload_check);

// After the macros above, whose textual scope covers only what follows them.
mod bridge;
mod decode;
mod encode;
mod oneof;
mod scalar;
mod shape;
mod size;

/// Which [`Aux`] variant a kind needs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AuxKind {
    Msg,
    Rep,
    Enum,
    Group,
    Member,
}

/// Per-field data that a kind needs beyond the field's offset.
pub enum Aux {
    /// The descriptor of a singular message field ([`Kind::MsgSingular`]).
    Msg(&'static MsgVt),
    /// The descriptor of a repeated message field ([`Kind::MsgRepeated`]).
    Rep(&'static RepVt),
    /// The descriptor of an enum field (the `Enum*` kinds).
    Enum(&'static EnumVt),
    /// The descriptor of a oneof, which its members' [`Member`] aux items
    /// refer to by index. No entry refers to it directly.
    Group(&'static OneofVt),
    /// One member of a oneof ([`Kind::OneofLeader`] or [`Kind::OneofFollower`]).
    Member(Member),
}

impl Aux {
    const fn kind(&self) -> AuxKind {
        match self {
            Aux::Msg(_) => AuxKind::Msg,
            Aux::Rep(_) => AuxKind::Rep,
            Aux::Enum(_) => AuxKind::Enum,
            Aux::Group(_) => AuxKind::Group,
            Aux::Member(_) => AuxKind::Member,
        }
    }
}

const _: () = assert!(core::mem::size_of::<Entry>() == 12);

/// One field of a message.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Entry {
    /// The full wire tag, `(number << 3) | wire_type`, as written on encode
    /// (the length-delimited tag for a packed field).
    tag: u32,
    /// The byte offset of the field within the message struct.
    offset: u32,
    kind: Kind,
    /// The encoded length of `tag`.
    tag_len: u8,
    /// The index into [`Table`]'s aux array, for kinds that use one.
    aux: u16,
}

impl Entry {
    /// An entry for the field `number` of kind `kind`, stored `offset` bytes
    /// into the message struct, with the aux data at index `aux` if the kind
    /// uses any.
    ///
    /// # Panics
    ///
    /// Panics, at compile time when used to initialise a `static`, if
    /// `number` is not a valid field number or `offset` does not fit in a
    /// `u32`.
    #[must_use]
    pub const fn new(kind: Kind, number: u32, offset: usize, aux: u16) -> Self {
        assert!(
            number >= 1 && number < (1 << 29),
            "field number out of range"
        );
        assert!(offset <= u32::MAX as usize, "field offset out of range");
        let tag = (number << 3) | kind.wire_type();
        let mut tag_len = 1u8;
        let mut rest = tag >> 7;
        while rest != 0 {
            tag_len += 1;
            rest >>= 7;
        }
        Self {
            tag,
            offset: offset as u32,
            kind,
            tag_len,
            aux,
        }
    }

    /// An entry for member `number` of a oneof stored `offset` bytes into the
    /// message struct, whose value is of kind `payload`. `aux` is the index of
    /// its [`Aux::Member`] item, and `leader` is whether it is the member with
    /// the lowest number, which sizes and writes the oneof.
    ///
    /// # Panics
    ///
    /// Panics, at compile time when used to initialise a `static`, if
    /// `payload` is not a kind that a oneof member can have, or as for
    /// [`Entry::new`].
    #[must_use]
    pub const fn oneof_member(
        payload: Kind,
        leader: bool,
        number: u32,
        offset: usize,
        aux: u16,
    ) -> Self {
        assert!(
            payload.is_oneof_payload(),
            "a oneof member's payload must be a `Required` scalar, string, bytes or enum kind, or `MsgSingular`"
        );
        let mut e = Self::new(payload, number, offset, aux);
        e.kind = if leader {
            Kind::OneofLeader
        } else {
            Kind::OneofFollower
        };
        e
    }

    const fn number(&self) -> u32 {
        self.tag >> 3
    }
}

/// The untyped part of a [`Table`], which the interpreters recurse through.
pub struct MessageTable {
    /// The fields, sorted by number.
    entries: &'static [Entry],
    /// `dense[n]` is one plus the index into `entries` of field number `n`,
    /// or `0` if there is none, for the numbers below `dense.len()`.
    dense: &'static [u8],
    aux: &'static [Aux],
    /// The offset of the message's `UnknownFields`, or [`NO_UNKNOWN`].
    unknown: u32,
}

impl MessageTable {
    #[inline]
    fn find(&self, number: u32) -> Option<&Entry> {
        if let Some(&i) = self.dense.get(number as usize) {
            return match i {
                0 => None,
                i => self.entries.get(usize::from(i) - 1),
            };
        }
        self.entries
            .binary_search_by_key(&number, Entry::number)
            .ok()
            .map(|i| &self.entries[i])
    }

    #[inline]
    fn msg_vt(&self, e: &Entry) -> &'static MsgVt {
        match &self.aux[usize::from(e.aux)] {
            Aux::Msg(vt) => vt,
            _ => unreachable!("`Table::new` checked that message entries index `MsgVt`s"),
        }
    }

    #[inline]
    fn rep_vt(&self, e: &Entry) -> &'static RepVt {
        match &self.aux[usize::from(e.aux)] {
            Aux::Rep(vt) => vt,
            _ => unreachable!("`Table::new` checked that repeated message entries index `RepVt`s"),
        }
    }

    #[inline]
    fn enum_vt(&self, e: &Entry) -> &'static EnumVt {
        match &self.aux[usize::from(e.aux)] {
            Aux::Enum(vt) => vt,
            _ => unreachable!("`Table::new` checked that enum entries index enum descriptors"),
        }
    }

    #[inline]
    fn member(&self, e: &Entry) -> Member {
        match &self.aux[usize::from(e.aux)] {
            Aux::Member(m) => *m,
            _ => unreachable!("`Table::new` checked that oneof entries index `Member`s"),
        }
    }

    #[inline]
    fn group(&self, m: Member) -> &'static OneofVt {
        match &self.aux[usize::from(m.group)] {
            Aux::Group(g) => g,
            _ => unreachable!("`Table::new` checked that members index oneof descriptors"),
        }
    }

    /// The entry of the member `number` of the oneof whose descriptor is at
    /// aux index `group`, with its kind and aux index replaced by those of its
    /// payload, which the payload arms of the interpreters take.
    ///
    /// # Panics
    ///
    /// Panics if the message has no such oneof member, which only an incorrect
    /// [`OneofEnum`] can cause.
    #[inline]
    fn payload_entry(&self, group: u16, number: u32) -> Entry {
        let Some(e) = self.find(number) else {
            oneof::no_such_member(number)
        };
        if !matches!(e.kind, Kind::OneofLeader | Kind::OneofFollower) {
            oneof::no_such_member(number)
        }
        let m = self.member(e);
        if m.group != group {
            oneof::no_such_member(number)
        }
        Entry {
            kind: m.kind,
            aux: m.aux,
            ..*e
        }
    }
}

/// The static description of message type `M`, from which the interpreters
/// in this module encode, size and decode an `M`.
pub struct Table<M> {
    raw: MessageTable,
    _marker: PhantomData<fn(&M)>,
}

impl<M> Table<M> {
    /// Describe `M`. Generated code builds tables with `__table!`.
    ///
    /// `abi` is the [`ABI`] the caller was generated for. `entries` are the
    /// fields sorted by number; `dense` is the lookup array described on
    /// [`MessageTable`], which must be empty if there are 255 entries or more;
    /// `aux` holds the descriptors that entries index by their `aux` value;
    /// `unknown` is the offset of the message's `UnknownFields`, if it keeps
    /// any.
    ///
    /// # Panics
    ///
    /// Panics, at compile time when used to initialise a `static`, if `abi`
    /// is not [`ABI`], if an entry lies outside `M`, if the entries are not in
    /// strictly increasing field-number order, if an entry's aux index is out
    /// of range or names a descriptor of the wrong variant or, for an enum,
    /// the wrong cardinality, if `dense` disagrees with `entries`, if
    /// `unknown` does not leave room for an `UnknownFields`, or if a oneof is
    /// inconsistent: a member whose group, payload kind, tag, offset or
    /// leader kind disagrees with its oneof's descriptor, or a oneof whose
    /// lowest-numbered member is not its only leader.
    ///
    /// # Safety
    ///
    /// For every entry, `offset` must be the offset within `M` of a field
    /// whose Rust type is exactly the one the entry's [`Kind`] names, in the
    /// default representation:
    ///
    /// - `Int32*` and the other scalar kinds: the scalar type itself
    ///   (`i32`, `u64`, `bool`, `f32`, ...) for `Implicit` and `Required`,
    ///   `Option<T>` for `Optional`, and `Vec<T>` for `Repeated` and `Packed`;
    /// - `Str*`: `String`, `Option<String>` or `Vec<String>` likewise, and
    ///   `Bytes*`: `Vec<u8>`, `Option<Vec<u8>>` or `Vec<Vec<u8>>`;
    /// - `Enum*`: the storage the entry's [`EnumVt`] was built for;
    /// - `MsgSingular`: the storage the [`MsgVt`] was built for, and
    ///   `MsgRepeated`: the `Vec` the [`RepVt`] was built for;
    /// - `OneofLeader` and `OneofFollower`: an `Option<E>`, where the [`OneofVt`] of the member's
    ///   group was built for `E`, and `E`'s [`OneofEnum`] implementation
    ///   gives, for the member's number, a pointer to a value of the member's
    ///   payload kind, under the same rules as the kinds above (for
    ///   `MsgSingular`, a [`MsgVt::direct`] or [`MsgVt::direct_via_message`]
    ///   descriptor of the message).
    ///
    /// `unknown`, if present, must be the offset of a field of type
    /// `UnknownFields`. The `__table_entry!` macro checks the field types
    /// that the compiler can; the pairing of an entry with its aux index and
    /// the `dense` lookup are the generator's to get right.
    #[must_use]
    pub const unsafe fn new(
        abi: u32,
        entries: &'static [Entry],
        dense: &'static [u8],
        aux: &'static [Aux],
        unknown: Option<usize>,
    ) -> Self {
        assert!(
            abi == ABI,
            "buffa table: the generated code is for a different table ABI; \
             regenerate it with the buffa-codegen that matches this buffa"
        );
        let size = core::mem::size_of::<M>();
        let mut leaders = 0;
        let mut i = 0;
        while i < entries.len() {
            let e = &entries[i];
            assert!(
                i == 0 || entries[i - 1].number() < e.number(),
                "buffa table: entries are not in strictly increasing field-number order"
            );
            assert!(
                (e.offset as usize) < size,
                "buffa table: an entry's offset is outside the message struct"
            );
            if let Some(want) = e.kind.aux_kind() {
                assert!(
                    (e.aux as usize) < aux.len(),
                    "buffa table: an entry's aux index is out of range"
                );
                let a = &aux[e.aux as usize];
                assert!(
                    a.kind() as u8 == want as u8,
                    "buffa table: an entry's aux descriptor is the wrong variant for its kind"
                );
                if let Aux::Enum(vt) = a {
                    assert!(
                        vt.card == e.kind.shape_card(),
                        "buffa table: an enum entry's descriptor has the wrong cardinality for its kind"
                    );
                }
                if let Aux::Member(m) = a {
                    if oneof::check_member(e, *m, aux) {
                        leaders += 1;
                    }
                }
            }
            i += 1;
        }
        // Every oneof has a leader, and no two share one, so the messages
        // written for its members are written once.
        let mut groups = 0;
        i = 0;
        while i < aux.len() {
            if let Aux::Group(_) = &aux[i] {
                groups += 1;
            }
            i += 1;
        }
        assert!(
            leaders == groups,
            "buffa table: every oneof descriptor needs exactly one leading member"
        );
        assert!(
            dense.is_empty() || entries.len() < 255,
            "buffa table: the dense lookup needs fewer than 255 entries"
        );
        // Every dense slot names the entry with its number, and every entry
        // below the dense range has a slot, so the two agree.
        let mut named = 0;
        let mut n = 0;
        while n < dense.len() {
            let d = dense[n] as usize;
            if d != 0 {
                assert!(
                    d <= entries.len() && entries[d - 1].number() as usize == n,
                    "buffa table: the dense lookup names an entry with the wrong field number"
                );
                named += 1;
            }
            n += 1;
        }
        let mut covered = 0;
        i = 0;
        while i < entries.len() {
            if (entries[i].number() as usize) < dense.len() {
                covered += 1;
            }
            i += 1;
        }
        assert!(
            named == covered,
            "buffa table: the dense lookup omits an entry"
        );
        let unknown = match unknown {
            None => NO_UNKNOWN,
            Some(offset) => {
                assert!(
                    offset < NO_UNKNOWN as usize
                        && offset <= size
                        && size - offset >= core::mem::size_of::<UnknownFields>()
                        && offset % core::mem::align_of::<UnknownFields>() == 0,
                    "buffa table: the unknown-fields offset does not fit an `UnknownFields` in the message struct"
                );
                offset as u32
            }
        };
        Self {
            raw: MessageTable {
                entries,
                dense,
                aux,
                unknown,
            },
            _marker: PhantomData,
        }
    }

    /// The encoded size of `msg`, recording nested message sizes in `cache`.
    ///
    /// See [`Message::compute_size`](crate::Message::compute_size).
    #[inline]
    pub fn compute_size(&self, msg: &M, cache: &mut SizeCache) -> u32 {
        // SAFETY: `Table::new`'s contract says the table describes `M`.
        unsafe { size::compute_size(&self.raw, (msg as *const M).cast(), cache) }
    }

    /// Write `msg` to `buf`, taking nested message sizes from `cache`, which
    /// `compute_size` filled.
    ///
    /// A [`PreSized`](crate::encode_sink::PreSized) cursor, which is what
    /// `Message::encode` and its siblings write a `BufMut` through, is
    /// written by one function compiled in this crate, and any other sink by an
    /// instance compiled in the caller's crate.
    ///
    /// See [`Message::write_to`](crate::Message::write_to).
    #[inline]
    pub fn write_to<K: EncodeSink>(&self, msg: &M, cache: &mut SizeCache, buf: &mut K) {
        let base = (msg as *const M).cast::<u8>();
        // SAFETY: `Table::new`'s contract says the table describes `M`.
        unsafe { encode::write_to(&self.raw, base, cache, buf) }
    }

    /// Decode into `msg` until `buf` has `limit` bytes remaining.
    ///
    /// A `buf` whose current chunk is shorter than the message is gathered
    /// into one contiguous buffer first.
    ///
    /// See [`Message::merge_to_limit`](crate::Message::merge_to_limit).
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] for malformed input or an exhausted limit.
    #[inline]
    pub fn merge_to_limit<B: Buf>(
        &self,
        msg: &mut M,
        buf: &mut B,
        ctx: DecodeContext<'_>,
        limit: usize,
    ) -> Result<(), DecodeError> {
        let base = (msg as *mut M).cast::<u8>();
        // SAFETY: `Table::new`'s contract says the table describes `M`.
        unsafe { decode::merge_to_limit(&self.raw, base, buf, ctx, limit) }
    }

    /// Decode a length-prefixed message into `msg`.
    ///
    /// See [`Message::merge_length_delimited`](crate::Message::merge_length_delimited).
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] for malformed input or an exhausted limit.
    #[inline]
    pub fn merge_length_delimited<B: Buf>(
        &self,
        msg: &mut M,
        buf: &mut B,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let base = (msg as *mut M).cast::<u8>();
        // SAFETY: `Table::new`'s contract says the table describes `M`.
        unsafe { decode::merge_length_delimited(&self.raw, base, buf, ctx) }
    }

    /// Decode one field, whose `tag` has been read, into `msg`.
    ///
    /// Unlike the other decode entry points this does not gather a
    /// non-contiguous `buf`: it requires everything remaining in `buf` to be
    /// one chunk, and returns [`DecodeError::UnexpectedEof`] if it is not.
    ///
    /// See [`Message::merge_field`](crate::Message::merge_field).
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] for malformed input or an exhausted limit.
    #[inline]
    pub fn merge_field<B: Buf>(
        &self,
        msg: &mut M,
        tag: Tag,
        buf: &mut B,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let base = (msg as *mut M).cast::<u8>();
        // SAFETY: `Table::new`'s contract says the table describes `M`.
        unsafe { decode::merge_field(&self.raw, base, tag, buf, ctx) }
    }
}

/// Build the `static` [`Table`] of message `$msg`, for generated code.
///
/// `unknown` is `none` for a message that drops unknown fields, or the name of
/// its `UnknownFields` field. The `unsafe` is written here, in this crate, so
/// the generated code compiles under `#![forbid(unsafe_code)]`.
///
/// # Example
///
/// A message with one `int32` field, as generated code builds it:
///
/// ```
/// #![forbid(unsafe_code)]
/// use buffa::encoding::Tag;
/// use buffa::{DecodeContext, DecodeError, EncodeSink, Message, SizeCache, UnknownFields};
///
/// #[derive(Clone, Debug, Default, PartialEq)]
/// struct Point {
///     x: i32,
///     unknown: UnknownFields,
/// }
/// buffa::impl_default_instance!(Point);
///
/// static POINT: buffa::table::Table<Point> = buffa::__table!(
///     Point,
///     abi = buffa::table::ABI,
///     entries = [buffa::__table_entry!(Point, x, Int32Implicit, 1)],
///     dense = &[0, 1],
///     aux = [],
///     unknown = unknown,
/// );
///
/// impl Message for Point {
///     fn compute_size(&self, cache: &mut SizeCache) -> u32 {
///         POINT.compute_size(self, cache)
///     }
///     fn write_to(&self, cache: &mut SizeCache, buf: &mut impl EncodeSink) {
///         POINT.write_to(self, cache, buf);
///     }
///     fn merge_field(
///         &mut self,
///         tag: Tag,
///         buf: &mut impl buffa::bytes::Buf,
///         ctx: DecodeContext<'_>,
///     ) -> Result<(), DecodeError> {
///         POINT.merge_field(self, tag, buf, ctx)
///     }
///     fn clear(&mut self) {
///         *self = Self::default();
///     }
/// }
///
/// let point = Point { x: 150, ..Point::default() };
/// assert_eq!(point.encode_to_vec(), [0x08, 0x96, 0x01]);
/// assert_eq!(Point::decode_from_slice(&[0x08, 0x96, 0x01]).unwrap(), point);
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __table {
    (
        $msg:ty,
        abi = $abi:expr,
        entries = [$($entry:expr),* $(,)?],
        dense = $dense:expr,
        aux = [$($aux:expr),* $(,)?],
        unknown = none $(,)?
    ) => {
        // SAFETY: each entry's field type is checked by `__table_entry!`; the
        // generator pairs entries with their aux descriptors and builds the
        // dense lookup, and `Table::new` checks the rest at compile time.
        unsafe {
            $crate::table::Table::<$msg>::new(
                $abi,
                &[$($entry),*],
                $dense,
                &[$($aux),*],
                ::core::option::Option::None,
            )
        }
    };
    (
        $msg:ty,
        abi = $abi:expr,
        entries = [$($entry:expr),* $(,)?],
        dense = $dense:expr,
        aux = [$($aux:expr),* $(,)?],
        unknown = $unknown:ident $(,)?
    ) => {
        {
            const _: fn(&$msg) -> *const $crate::UnknownFields =
                |m| ::core::ptr::addr_of!(m.$unknown);
            // SAFETY: as above, and the field just checked is an
            // `UnknownFields`.
            unsafe {
                $crate::table::Table::<$msg>::new(
                    $abi,
                    &[$($entry),*],
                    $dense,
                    &[$($aux),*],
                    ::core::option::Option::Some($crate::table::offset_of!($msg, $unknown)),
                )
            }
        }
    };
}

/// Build one [`Entry`] of message `$msg`'s table, for generated code.
///
/// The field `$field` of `$msg` must have the type `$kind` names, or the
/// generated code does not compile:
///
/// ```compile_fail,E0308
/// struct Point {
///     x: i32,
/// }
/// // `x` is not a `String`.
/// let _ = buffa::__table_entry!(Point, x, StrImplicit, 1);
/// ```
///
/// The check is exact, so a field whose type only derefs to the expected one
/// does not pass:
///
/// ```compile_fail,E0308
/// struct Point {
///     x: Box<String>,
/// }
/// let _ = buffa::__table_entry!(Point, x, StrImplicit, 1);
/// ```
///
/// The scalar, string and bytes kinds have a fixed field type. The enum and
/// message kinds take the type explicitly, as `aux = <index>, slot = <type>`,
/// where the type is the one their aux descriptor was built for. A oneof
/// member is written `oneof(<payload kind>)`, with the `Option` of the oneof's
/// enum as its slot type and the index of its [`Member`] as `aux`.
#[doc(hidden)]
#[macro_export]
macro_rules! __table_entry {
    (
        $msg:ty, $field:ident, oneof($payload:ident, $leader:expr), $number:expr,
        aux = $aux:expr, slot = $slot:ty $(,)?
    ) => {{
        const _: fn(&$msg) -> *const $slot = |m| ::core::ptr::addr_of!(m.$field);
        $crate::table::Entry::oneof_member(
            $crate::table::Kind::$payload,
            $leader,
            $number,
            $crate::table::offset_of!($msg, $field),
            $aux,
        )
    }};
    ($msg:ty, $field:ident, $kind:ident, $number:expr $(,)?) => {{
        // A raw pointer, unlike a reference, cannot be deref-coerced, so this
        // needs the field's type to be exactly the slot type.
        const _: fn(
            &$msg,
        )
            -> *const <$crate::table::kinds::$kind as $crate::table::KindSlot>::Slot =
            |m| ::core::ptr::addr_of!(m.$field);
        $crate::table::Entry::new(
            $crate::table::Kind::$kind,
            $number,
            $crate::table::offset_of!($msg, $field),
            0,
        )
    }};
    ($msg:ty, $field:ident, $kind:ident, $number:expr, aux = $aux:expr, slot = $slot:ty $(,)?) => {{
        const _: fn(&$msg) -> *const $slot = |m| ::core::ptr::addr_of!(m.$field);
        $crate::table::Entry::new(
            $crate::table::Kind::$kind,
            $number,
            $crate::table::offset_of!($msg, $field),
            $aux,
        )
    }};
}

// The tests build tables with `offset_of!`, which needs Rust 1.77. The module
// is inline because `rustversion` cannot gate an out-of-line one.
#[cfg(test)]
#[rustversion::since(1.77)]
mod tests {
    include!("tests.rs");
}

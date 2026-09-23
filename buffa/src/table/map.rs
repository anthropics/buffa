//! Map fields: `map<K, V>` stored as a [`MapStorage`] collection.
//!
//! A map is written as one length-delimited record per entry, each holding
//! the key as field 1 and the value as field 2. The interpreters treat the
//! key and the value as two fields of a message with two entries and reach
//! them through pointers to the collection's storage, so every key and value
//! kind is handled by the arms that handle it as an ordinary field. Only
//! what depends on the collection's type is generic: iterating it, and
//! inserting an entry that has been decoded into locals.
//!
//! # Why the loops test for a map
//!
//! The dispatch functions `size_kind`, `write_kind` and `merge_kind` have an
//! arm for every kind, and the arm for [`Kind::Map`] is `unreachable!`.
//! `compute_size`, `write_message` and `merge_one` (which `merge_slice` calls
//! for each field) test for a map first and call [`size_map`], [`write_map`]
//! or `merge_map` themselves. `merge_kind` is also called by `merge_entry`,
//! for a key and a value, which are never maps.
//!
//! Calling the map interpreters from the arms made the dispatch functions
//! large enough that, at `opt-level = 3`, the compiler stopped inlining them
//! into the loops and called them once per field. With fat LTO, one codegen
//! unit and Rust 1.95, the `whatsapp.proto` of `waproto` was 4% larger at
//! `opt-level = 3`, and a schema of table messages without maps 2.6% larger;
//! at `opt-level = "z"` the two layouts were within 0.1% of each other. This
//! is an inlining threshold, so it depends on the compiler and the size of the
//! dispatch functions, and any change to them or a new toolchain can bring it
//! back. To check, build a schema of table messages without maps at
//! `opt-level = 3` (fat LTO, one codegen unit) and compare the binary's size
//! with a build before a change to the dispatch functions: a jump of a few
//! percent is this.

use super::decode::{merge_kind, take_len_delimited};
use super::encode::{put_tag, write_kind};
use super::shape::{DirectMsgVt, EnumVtOf};
use super::size::size_kind;
use super::{Aux, Entry, EnumShape, Kind, KindSlot, MessageTable, IMPLICIT, NO_UNKNOWN, REQUIRED};
use crate::encoding::{
    check_wire_type, encode_varint, skip_field_depth, varint_len, Tag, WireType,
};
use crate::map_codec::MapStorage;
use crate::{
    types, DecodeContext, DecodeError, EncodeSink, SizeCache, UnknownField, UnknownFieldData,
    UnknownFields,
};

/// Descriptor of a map field: the kinds of its key and value, and functions
/// that iterate the collection and insert a decoded entry, instantiated for
/// its type.
pub struct MapVt {
    /// The key (field 1) and the value (field 2), as the entries of a message
    /// whose fields are reached by pointer, so their offsets are unused.
    entries: [Entry; 2],
    /// The descriptor of the value, if its kind has one, which its entry
    /// indexes at 0.
    value_aux: Option<Aux>,
    /// The size in bytes of a key and a value together, which an entry read
    /// from the wire counts towards the element-memory limit.
    footprint: usize,
    /// Call the function with a pointer to each key and its value.
    ///
    /// # Safety
    ///
    /// The first argument points to a live collection of the type the
    /// descriptor was built for, which nothing mutates during the call.
    for_each: unsafe fn(*const u8, &mut dyn FnMut(*const u8, *const u8)),
    /// Create a key and a value with their defaults, pass pointers to them to
    /// the function, and insert them into the collection if it returns
    /// `true`. Returns what the function returned.
    ///
    /// # Safety
    ///
    /// The first argument points to a live collection of the type the
    /// descriptor was built for, to which the caller has exclusive access for
    /// the whole call, as a `&mut` would give it: nothing else reads or
    /// writes the collection while it runs, including through the function.
    decode_entry: unsafe fn(*mut u8, &mut EntryReader<'_>) -> Result<bool, DecodeError>,
}

/// Reads the key and value of one entry through pointers to them and returns
/// whether to insert the entry.
type EntryReader<'a> = dyn FnMut(*mut u8, *mut u8) -> Result<bool, DecodeError> + 'a;

/// # Safety
///
/// `map` points to a live `Mp`, which nothing mutates during the call.
unsafe fn for_each_impl<Mp: MapStorage>(map: *const u8, f: &mut dyn FnMut(*const u8, *const u8)) {
    // SAFETY: the caller passes a pointer to a live `Mp`.
    let map = unsafe { &*map.cast::<Mp>() };
    for (key, value) in map.storage_iter() {
        f(
            (key as *const Mp::Key).cast(),
            (value as *const Mp::Value).cast(),
        );
    }
}

/// # Safety
///
/// `map` points to a live `Mp` to which the caller has exclusive access for
/// the call.
unsafe fn decode_entry_impl<Mp: MapStorage>(
    map: *mut u8,
    f: &mut EntryReader<'_>,
) -> Result<bool, DecodeError>
where
    Mp::Key: Default,
    Mp::Value: Default,
{
    let mut key = Mp::Key::default();
    let mut value = Mp::Value::default();
    let keep = f(
        (&mut key as *mut Mp::Key).cast(),
        (&mut value as *mut Mp::Value).cast(),
    )?;
    if keep {
        // SAFETY: the caller passes a pointer to a live `Mp` that it has
        // exclusive access to.
        unsafe { (*map.cast::<Mp>()).storage_insert(key, value) };
    }
    Ok(keep)
}

impl MapVt {
    /// Describe a map of type `Mp` whose keys are of the kind `KK` names and
    /// whose values are of the string, bytes or scalar kind `VK` names.
    ///
    /// # Panics
    ///
    /// Panics, at compile time when used to initialise a `static`, if `KK` or
    /// `VK` does not have required cardinality (`Int32Required`, not
    /// `Int32Optional` or `Int32Repeated`). The key is not restricted to the
    /// types protobuf allows for one (integers, `bool` and `string`): any
    /// scalar, string or bytes kind with required cardinality is accepted, and
    /// the collection type decides whether it can be keyed by that type.
    #[must_use]
    pub const fn new<Mp, KK, VK>() -> Self
    where
        KK: KindSlot,
        VK: KindSlot,
        Mp: MapStorage<Key = KK::Slot, Value = VK::Slot>,
        KK::Slot: Default,
        VK::Slot: Default,
    {
        Self::build::<Mp>(KK::KIND, VK::KIND, None)
    }

    /// Describe a map of type `Mp` whose keys are of the kind `KK` names and
    /// whose values are enums stored as `S`, which must have implicit
    /// presence: [`ImplicitOpen`](super::ImplicitOpen) or
    /// [`ImplicitClosed`](super::ImplicitClosed).
    ///
    /// A value that a closed enum does not know makes the whole entry
    /// unknown, as in unrolled code.
    ///
    /// # Panics
    ///
    /// As for [`new`](Self::new), and if `S` is not an implicit shape.
    #[must_use]
    pub const fn with_enum<Mp, KK, S>() -> Self
    where
        KK: KindSlot,
        S: EnumShape,
        Mp: MapStorage<Key = KK::Slot, Value = S::Slot>,
        KK::Slot: Default,
        S::Slot: Default,
    {
        assert!(
            S::CARD == IMPLICIT,
            "buffa table: a map's enum values must have an implicit shape"
        );
        Self::build::<Mp>(
            KK::KIND,
            Kind::EnumRequired,
            Some(Aux::Enum(&EnumVtOf::<S>::VT)),
        )
    }

    /// Describe a map of type `Mp` whose keys are of the kind `KK` names and
    /// whose values are messages of type `M`, which `vt` describes: a
    /// [`DirectMsgVt::new`] for a message that has a table, or
    /// [`DirectMsgVt::via_message`] for one that has not, which is reached
    /// through its [`Message`](crate::Message) impl. Its type ties `vt` to the
    /// map's value type, so a descriptor of another message is a type error:
    ///
    /// ```
    /// use buffa::table::{kinds, DirectMsgVt, MapVt, Table};
    /// use std::collections::BTreeMap;
    ///
    /// #[derive(Default)]
    /// struct Point {
    ///     x: i32,
    /// }
    /// static POINT: Table<Point> = buffa::__table!(
    ///     Point,
    ///     abi = buffa::table::ABI,
    ///     entries = [buffa::__table_entry!(Point, x, Int32Implicit, 1)],
    ///     dense = &[0, 1],
    ///     aux = [],
    ///     unknown = none,
    /// );
    /// static VT: DirectMsgVt<Point> = DirectMsgVt::new(&POINT);
    /// let _ = MapVt::with_msg::<BTreeMap<String, Point>, kinds::StrRequired, Point>(&VT);
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use buffa::table::{kinds, DirectMsgVt, MapVt, Table};
    /// use std::collections::BTreeMap;
    ///
    /// #[derive(Default)]
    /// struct Point {
    ///     x: i32,
    /// }
    /// #[derive(Default)]
    /// struct Other;
    /// static POINT: Table<Point> = buffa::__table!(
    ///     Point,
    ///     abi = buffa::table::ABI,
    ///     entries = [buffa::__table_entry!(Point, x, Int32Implicit, 1)],
    ///     dense = &[0, 1],
    ///     aux = [],
    ///     unknown = none,
    /// );
    /// static VT: DirectMsgVt<Point> = DirectMsgVt::new(&POINT);
    /// // The values are `Other`s, but the descriptor is a `Point`'s.
    /// let _ = MapVt::with_msg::<BTreeMap<String, Other>, kinds::StrRequired, Other>(&VT);
    /// ```
    ///
    /// # Panics
    ///
    /// As for [`new`](Self::new).
    #[must_use]
    pub const fn with_msg<Mp, KK, M>(vt: &'static DirectMsgVt<M>) -> Self
    where
        KK: KindSlot,
        Mp: MapStorage<Key = KK::Slot, Value = M>,
        KK::Slot: Default,
        M: Default,
    {
        Self::build::<Mp>(KK::KIND, Kind::MsgSingular, Some(Aux::Msg(&vt.vt)))
    }

    const fn build<Mp: MapStorage>(key: Kind, value: Kind, value_aux: Option<Aux>) -> Self
    where
        Mp::Key: Default,
        Mp::Value: Default,
    {
        assert!(
            key.card() == REQUIRED,
            "buffa table: a map key must be of a required scalar or string kind"
        );
        assert!(
            value as u8 == Kind::MsgSingular as u8 || value.card() == REQUIRED,
            "buffa table: a map value must be of a required kind or a singular message"
        );
        Self {
            entries: [Entry::new(key, 1, 0, 0), Entry::new(value, 2, 0, 0)],
            value_aux,
            footprint: core::mem::size_of::<Mp::Key>() + core::mem::size_of::<Mp::Value>(),
            for_each: for_each_impl::<Mp>,
            decode_entry: decode_entry_impl::<Mp>,
        }
    }

    /// The key and value as a two-field message, for the interpreters.
    #[inline]
    fn entry_table(&'static self) -> MessageTable {
        MessageTable {
            entries: &self.entries,
            dense: &[],
            aux: match &self.value_aux {
                Some(aux) => core::slice::from_ref(aux),
                None => &[],
            },
            unknown: NO_UNKNOWN,
        }
    }
}

/// The encoded size of the map at `slot`, whose field is `e`, recording the
/// sizes of message values in `cache`.
///
/// # Safety
///
/// `slot` points to the map field `e` describes.
#[inline(never)]
pub(super) unsafe fn size_map(
    table: &MessageTable,
    e: &Entry,
    slot: *const u8,
    cache: &mut SizeCache,
) -> u64 {
    let tl = u64::from(e.tag_len);
    let vt = table.map_vt(e);
    let entry_table = vt.entry_table();
    let mut size = 0u64;
    // SAFETY: the descriptor was built for the slot's type, and the entry
    // table describes what it passes to the closure.
    unsafe {
        (vt.for_each)(slot, &mut |key, value| {
            let entry = size_kind(&entry_table, &vt.entries[0], key, cache)
                + size_kind(&entry_table, &vt.entries[1], value, cache);
            size += tl + varint_len(entry) as u64 + entry;
        });
    }
    size
}

/// Write the map at `slot`, whose field is `e`, taking the sizes of message
/// values from `cache` in the order `size_map` recorded them.
///
/// # Safety
///
/// As for [`size_map`].
#[inline(never)]
pub(super) unsafe fn write_map<K: EncodeSink>(
    table: &MessageTable,
    e: &Entry,
    slot: *const u8,
    cache: &mut SizeCache,
    buf: &mut K,
) {
    let vt = table.map_vt(e);
    let entry_table = vt.entry_table();
    let [key_entry, value_entry] = &vt.entries;
    // SAFETY: as for `size_map`.
    unsafe {
        (vt.for_each)(slot, &mut |key, value| {
            let key_len = size_kind(&entry_table, key_entry, key, cache);
            if value_entry.kind == Kind::MsgSingular {
                // The size pass recorded the value's size, and the value's own
                // write takes the sizes of what it holds after it.
                let len = cache.consume_next();
                let inner = u64::from(len);
                let entry =
                    key_len + u64::from(value_entry.tag_len) + varint_len(inner) as u64 + inner;
                put_tag(e, buf);
                encode_varint(entry, buf);
                write_kind(&entry_table, key_entry, key, cache, buf);
                put_tag(value_entry, buf);
                encode_varint(inner, buf);
                let msg = entry_table.msg_vt(value_entry);
                msg.child.write_to((msg.get)(value), len, cache, buf);
            } else {
                let entry = key_len + size_kind(&entry_table, value_entry, value, cache);
                put_tag(e, buf);
                encode_varint(entry, buf);
                write_kind(&entry_table, key_entry, key, cache, buf);
                write_kind(&entry_table, value_entry, value, cache, buf);
            }
        });
    }
}

/// Decode one entry of the map at `slot`, whose field is `e`, from the front
/// of `buf`, after its `tag`.
///
/// An entry whose value is a closed enum's unknown number is dropped, or kept
/// whole as an unknown field of the message at `base` if it keeps any.
///
/// # Safety
///
/// `slot` points to the map field `e` describes, inside the live message at
/// `base` of the type `table` describes.
#[inline(never)]
pub(super) unsafe fn merge_map(
    table: &MessageTable,
    e: &Entry,
    base: *mut u8,
    slot: *mut u8,
    tag: Tag,
    buf: &mut &[u8],
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError> {
    check_wire_type(tag, WireType::LengthDelimited)?;
    let vt = table.map_vt(e);
    let payload = take_len_delimited(buf)?;
    // An entry amplifies as a repeated element does: an omitted message value
    // still materialises a whole value in the map, and a few bytes of key buy
    // a slot. Count both before decoding either.
    ctx.register_element_memory(vt.footprint)?;
    let entry_table = vt.entry_table();
    // SAFETY: the descriptor was built for the slot's type, and the entry
    // table describes what it passes to the closure.
    let inserted = unsafe {
        (vt.decode_entry)(slot, &mut |key, value| {
            merge_entry(&entry_table, key, value, payload, ctx)
        })?
    };
    if !inserted && table.unknown != NO_UNKNOWN {
        ctx.register_unknown_field()?;
        // SAFETY: `unknown` is the offset of the message's `UnknownFields`.
        unsafe {
            (*base.add(table.unknown as usize).cast::<UnknownFields>()).push(UnknownField {
                number: e.tag >> 3,
                data: UnknownFieldData::LengthDelimited(payload.to_vec()),
            });
        }
    }
    Ok(())
}

/// Decode the fields of one entry, `payload`, into the key at `key` and the
/// value at `value`. Returns `false` if the last occurrence of the value is
/// an enum number that a closed enum does not know.
///
/// # Safety
///
/// `key` and `value` point to live values of the types the two entries of
/// `table`, the entry table of a map's descriptor, describe.
unsafe fn merge_entry(
    table: &MessageTable,
    key: *mut u8,
    value: *mut u8,
    mut payload: &[u8],
    ctx: DecodeContext<'_>,
) -> Result<bool, DecodeError> {
    let [key_entry, value_entry] = table.entries else {
        unreachable!("an entry table has a key and a value")
    };
    let mut known = true;
    while !payload.is_empty() {
        let tag = Tag::decode(&mut payload)?;
        match tag.field_number() {
            1 => {
                // SAFETY: `key` points to the value the key entry describes,
                // and the entry table has no unknown fields, so `merge_kind`
                // does not read the base pointer.
                unsafe {
                    merge_kind(
                        table,
                        key_entry,
                        core::ptr::null_mut(),
                        key,
                        tag,
                        &mut payload,
                        ctx,
                    )
                }?;
            }
            2 if value_entry.kind == Kind::EnumRequired => {
                check_wire_type(tag, WireType::Varint)?;
                let raw = types::decode_int32(&mut payload)?;
                // SAFETY: `value` points to the enum the value entry's
                // descriptor was built for.
                known = unsafe { (table.enum_vt(value_entry).set)(value, raw) };
            }
            2 => {
                // SAFETY: as for the key.
                unsafe {
                    merge_kind(
                        table,
                        value_entry,
                        core::ptr::null_mut(),
                        value,
                        tag,
                        &mut payload,
                        ctx,
                    )
                }?;
            }
            _ => skip_field_depth(tag, &mut payload, ctx.depth())?,
        }
    }
    Ok(known)
}

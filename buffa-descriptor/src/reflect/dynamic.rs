//! Map-backed dynamic protobuf message.
//!
//! [`DynamicMessage`] holds field values keyed by field number in a
//! `BTreeMap<u32, Value>` plus a descriptor reference. Encode and decode are
//! driven by the descriptor — there is no per-type generated code. This is
//! the v1 reflection target for runtime-loaded schemas (schema registries,
//! gRPC reflection, transcoding gateways) and the bridge target for
//! generated messages reflected via encode/decode round-trip.
//!
//! Wire round-tripping preserves unknown fields. Field-number ordering is
//! deterministic (`BTreeMap` iteration) so re-encoding produces canonical
//! output for known fields; unknown fields are appended in arrival order.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::{
    DescriptorPool, EnumIndex, FieldDescriptor, FieldKind, MessageDescriptor, MessageIndex,
    ScalarType, SingularKind,
};
use buffa::bytes::{Buf, BufMut};
use buffa::encoding::{
    decode_unknown_field, decode_varint, encode_varint, skip_field_depth, Tag, WireType,
};
use buffa::types::{
    decode_bool, decode_double, decode_fixed32, decode_fixed64, decode_float, decode_int32,
    decode_int64, decode_sfixed32, decode_sfixed64, decode_sint32, decode_sint64, decode_uint32,
    decode_uint64, encode_bool, encode_double, encode_fixed32, encode_fixed64, encode_float,
    encode_int32, encode_int64, encode_sfixed32, encode_sfixed64, encode_sint32, encode_sint64,
    encode_uint32, encode_uint64, int32_encoded_len, int64_encoded_len, sint32_encoded_len,
    sint64_encoded_len, uint32_encoded_len, uint64_encoded_len,
};
use buffa::unknown_fields::UnknownFields;
use buffa::{DecodeContext, DecodeError, Message, DEFAULT_UNKNOWN_FIELD_LIMIT, RECURSION_LIMIT};

use super::message::{ReflectCow, ReflectError, ReflectMessage, ReflectMessageMut};
use super::value::{MapKey, MapValue, Value, ValueRef};

/// A dynamically-typed protobuf message.
///
/// Holds a descriptor reference (an [`Arc<DescriptorPool>`] plus a
/// [`MessageIndex`]) and a `BTreeMap` of field values keyed by field number.
/// Encode, decode, get, set, has, and clear are all descriptor-driven.
#[derive(Clone)]
pub struct DynamicMessage {
    pool: Arc<DescriptorPool>,
    msg_idx: MessageIndex,
    fields: BTreeMap<u32, Value>,
    unknown: UnknownFields,
}

impl core::fmt::Debug for DynamicMessage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicMessage")
            .field("type", &self.message_descriptor().full_name)
            .field("fields", &RedactedFields(self))
            .finish_non_exhaustive()
    }
}

/// `Debug` adapter for [`DynamicMessage`]'s field map: values whose field
/// carries `[debug_redact = true]` print the `[REDACTED]` marker instead of
/// their contents, mirroring the redaction in generated `Debug` impls.
/// Unredacted entries print exactly as the underlying `BTreeMap` would.
///
/// The marker literal must stay in sync with `buffa-codegen`'s
/// `DEBUG_REDACT_PLACEHOLDER` so generated and reflective output agree.
struct RedactedFields<'a>(&'a DynamicMessage);

impl core::fmt::Debug for RedactedFields<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut map = f.debug_map();
        for (number, value) in &self.0.fields {
            // Numbers with no resolvable descriptor print unredacted; `fields`
            // never holds such entries today (unknown numbers are routed to
            // `unknown` during decode, and `set` is descriptor-keyed), so this
            // default only applies to genuinely-unannotated fields.
            let redact = self
                .0
                .field_or_extension(*number)
                .and_then(FieldDescriptor::options)
                .and_then(|o| o.debug_redact)
                .unwrap_or(false);
            if redact {
                map.entry(number, &core::format_args!("[REDACTED]"));
            } else {
                map.entry(number, value);
            }
        }
        map.finish()
    }
}

impl PartialEq for DynamicMessage {
    /// Structural equality **between instances from the same pool only.**
    ///
    /// The pool comparison is by `Arc` pointer identity (`Arc::ptr_eq`),
    /// not content — two messages of the same type decoded against
    /// semantically-identical pools built from the same `FileDescriptorSet`
    /// compare unequal. This is the right call for the bridge-mode
    /// round-trip (the typed message and the dynamic snapshot share one
    /// pool) but will surprise consumers comparing `DynamicMessage`s from
    /// independent decode pipelines. For those, compare `field_by_number`
    /// values directly or compare the re-encoded wire bytes.
    ///
    /// Unknown-field comparison is by count, not contents — a structural
    /// limitation of the prototype, since `UnknownFields` does not implement
    /// `PartialEq`.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pool, &other.pool)
            && self.msg_idx == other.msg_idx
            && self.fields == other.fields
            && self.unknown.len() == other.unknown.len()
    }
}

impl DynamicMessage {
    /// Create an empty dynamic message for the given message type.
    #[must_use]
    pub fn new(pool: Arc<DescriptorPool>, msg_idx: MessageIndex) -> Self {
        Self {
            pool,
            msg_idx,
            fields: BTreeMap::new(),
            unknown: UnknownFields::new(),
        }
    }

    /// Create an empty dynamic message by fully-qualified type name.
    ///
    /// Returns `None` if `full_name` is not in the pool.
    #[must_use]
    pub fn new_by_name(pool: Arc<DescriptorPool>, full_name: &str) -> Option<Self> {
        let idx = pool.message_index(full_name)?;
        Some(Self::new(pool, idx))
    }

    /// The descriptor pool this message's descriptor lives in.
    #[must_use]
    pub fn pool(&self) -> &Arc<DescriptorPool> {
        &self.pool
    }

    /// The pool index of this message's descriptor.
    #[must_use]
    pub fn message_index(&self) -> MessageIndex {
        self.msg_idx
    }

    /// Resolve `number` to a declared field of this message, or — when it
    /// isn't one — to a registered extension of this message.
    ///
    /// This is the lookup every codec path uses to interpret a stored or
    /// incoming field number. Extensions resolve to the [`FieldDescriptor`]
    /// inside their [`ExtensionDescriptor`](crate::ExtensionDescriptor), so
    /// the caller treats them identically to declared fields.
    ///
    /// Cost: the declared-field binary search covers the overwhelmingly
    /// common case. The pool's extension index is only consulted when the
    /// extendee declares at least one extension range *and* the number falls
    /// inside it — `in_extension_range` short-circuits on the empty slice
    /// that proto3 messages and most proto2 messages have.
    fn field_or_extension(&self, number: u32) -> Option<&FieldDescriptor> {
        let md = self.message_descriptor();
        if let Some(fd) = md.field(number) {
            return Some(fd);
        }
        if md.in_extension_range(number) {
            return self
                .pool
                .extension_for(self.msg_idx, number)
                .map(crate::ExtensionDescriptor::field);
        }
        None
    }

    fn field_descriptor_is_member(&self, field: &FieldDescriptor) -> bool {
        self.field_or_extension(field.number())
            .is_some_and(|f| core::ptr::eq(f, field))
    }

    fn validate_field_descriptor(&self, field: &FieldDescriptor) -> Result<(), ReflectError> {
        if self.field_descriptor_is_member(field) {
            Ok(())
        } else {
            Err(ReflectError::field_not_member(
                self.message_descriptor(),
                field,
            ))
        }
    }

    fn validate_field_value(
        &self,
        field: &FieldDescriptor,
        value: &Value,
    ) -> Result<(), ReflectError> {
        validate_value_shape(field.kind, value, &self.pool).map_err(|err| {
            ReflectError::wrong_value_kind(
                self.message_descriptor(),
                field,
                err.expected,
                err.actual,
            )
        })
    }

    /// Decode wire bytes against the descriptor.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if the wire data is malformed.
    pub fn decode(
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
        bytes: &[u8],
    ) -> Result<Self, DecodeError> {
        let mut msg = Self::new(pool, msg_idx);
        msg.merge(bytes)?;
        Ok(msg)
    }

    /// Merge additional wire bytes into this message.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if the wire data is malformed.
    pub fn merge(&mut self, bytes: &[u8]) -> Result<(), DecodeError> {
        let limit = core::cell::Cell::new(DEFAULT_UNKNOWN_FIELD_LIMIT);
        let mut buf = bytes;
        self.merge_buf(&mut buf, DecodeContext::new(RECURSION_LIMIT, &limit))
    }

    fn merge_buf(&mut self, buf: &mut impl Buf, ctx: DecodeContext<'_>) -> Result<(), DecodeError> {
        while buf.has_remaining() {
            let tag = Tag::decode(buf)?;
            self.merge_one_field(tag, buf, ctx)?;
        }
        Ok(())
    }

    /// Merge wire bytes that are bracketed by a `StartGroup`/`EndGroup` pair.
    fn merge_group(
        &mut self,
        buf: &mut impl Buf,
        group_field_number: u32,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        loop {
            let tag = Tag::decode(buf)?;
            if tag.wire_type() == WireType::EndGroup {
                if tag.field_number() != group_field_number {
                    return Err(DecodeError::InvalidEndGroup(tag.field_number()));
                }
                return Ok(());
            }
            self.merge_one_field(tag, buf, ctx)?;
        }
    }

    fn merge_one_field(
        &mut self,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let number = tag.field_number();
        // Take the FieldDescriptor by index to avoid borrowing the
        // MessageDescriptor (which lives behind self.pool) across the
        // mutation of self.fields. Extract the small `Copy` parts we need.
        // A number that isn't a declared field may be a registered extension
        // — resolve it through the pool so it decodes typed (and can be
        // re-emitted as JSON). Unregistered extension-range numbers fall
        // through to unknown fields, preserving the binary round-trip.
        let (kind, oneof_index, delimited) = match self.field_or_extension(number) {
            Some(fd) => (fd.kind, fd.oneof_index, fd.delimited),
            None => {
                self.unknown.push(decode_unknown_field(tag, buf, ctx)?);
                return Ok(());
            }
        };
        // Per the protobuf spec, a field whose wire type does not match the
        // schema is treated as an unknown field, not a decode error. The
        // generated decoder dispatches `match (number, wire_type)` and
        // reaches its `_` arm for a mismatch; the reflective decoder must
        // do the same explicitly. Without this check, `decode_scalar` would
        // read the wrong number of bytes from the buffer (e.g. 4 bytes of a
        // varint payload for a Fixed32 field) and silently corrupt every
        // subsequent field in the stream.
        if !wire_type_compatible(kind, tag.wire_type(), delimited) {
            self.unknown.push(decode_unknown_field(tag, buf, ctx)?);
            return Ok(());
        }
        match kind {
            FieldKind::Singular(sk) => {
                // Oneof semantics: setting any member clears all other members
                // of the same oneof. Synthetic oneofs (proto3 `optional`) have
                // a single member so this is a no-op for them.
                if let Some(oi) = oneof_index {
                    self.clear_other_oneof_members(oi, number);
                }
                // Singular message merge semantics: when the same field appears
                // multiple times on the wire, the parser merges the messages
                // (each sub-field merged) rather than replacing wholesale.
                if let SingularKind::Message(midx) = sk {
                    if let Some(Value::Message(_)) = self.fields.get(&number) {
                        // Decode the new bytes into the existing message.
                        return self.merge_into_existing_message(number, midx, tag, buf, ctx);
                    }
                }
                let v = self.decode_element(sk, tag, buf, ctx)?;
                self.fields.insert(number, v);
            }
            FieldKind::List(sk) => {
                self.merge_list_field(number, sk, tag, buf, ctx)?;
            }
            FieldKind::Map { key, value } => {
                self.merge_map_field(number, key, value, tag, buf, ctx)?;
            }
        }
        Ok(())
    }

    /// Clear every set member of oneof `oi` except `keep_number`.
    fn clear_other_oneof_members(&mut self, oi: u16, keep_number: u32) {
        let md = self.message_descriptor();
        let Some(o) = md.oneofs.get(oi as usize) else {
            return;
        };
        // Collect the numbers to clear without holding the descriptor borrow.
        let to_clear: alloc::vec::Vec<u32> = o
            .field_indices
            .iter()
            .filter_map(|&fi| md.fields.get(fi as usize))
            .map(|f| f.number)
            .filter(|&n| n != keep_number)
            .collect();
        for n in to_clear {
            self.fields.remove(&n);
        }
    }

    /// Merge new wire bytes for a singular message field into the existing
    /// `Value::Message` at `number`, instead of replacing it.
    fn merge_into_existing_message(
        &mut self,
        number: u32,
        midx: MessageIndex,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let _ = midx;
        let ctx = ctx.descend()?;
        // Take the existing message out of the map so we can borrow `self`
        // immutably for the descriptor lookup while merging into it.
        let Some(Value::Message(mut existing)) = self.fields.remove(&number) else {
            // Caller checked this — defensive.
            return Ok(());
        };
        let result = match tag.wire_type() {
            WireType::LengthDelimited => {
                let len = decode_varint(buf)?;
                let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
                if buf.remaining() < len {
                    return Err(DecodeError::UnexpectedEof);
                }
                let mut sub = buf.copy_to_bytes(len);
                existing.merge_buf(&mut sub, ctx)
            }
            WireType::StartGroup => existing.merge_group(buf, tag.field_number(), ctx),
            wt => Err(DecodeError::WireTypeMismatch {
                field_number: number,
                expected: WireType::LengthDelimited as u8,
                actual: wt as u8,
            }),
        };
        // Put the (possibly partially-merged) message back regardless of
        // outcome, so a decode error doesn't silently drop earlier data.
        self.fields.insert(number, Value::Message(existing));
        result
    }

    fn merge_list_field(
        &mut self,
        number: u32,
        elem: SingularKind,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let list = match self
            .fields
            .entry(number)
            .or_insert_with(|| Value::List(Vec::new()))
        {
            Value::List(l) => l,
            other => {
                // A previous decode wrote a non-list value for this number
                // (corrupt stream); replace it.
                *other = Value::List(Vec::new());
                let Value::List(l) = other else {
                    unreachable!()
                };
                l
            }
        };
        let packable = is_packable(elem);
        if tag.wire_type() == WireType::LengthDelimited && packable {
            // Packed encoding: a length-delimited blob of consecutive elements.
            let len = decode_varint(buf)?;
            let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
            if buf.remaining() < len {
                return Err(DecodeError::UnexpectedEof);
            }
            // Take a sub-buffer of `len` bytes and decode elements from it.
            let mut packed = buf.copy_to_bytes(len);
            while packed.has_remaining() {
                list.push(decode_packed_element(elem, number, &mut packed)?);
            }
            return Ok(());
        }
        // Unpacked: decode one element. We need to be careful not to alias
        // self.pool and self.fields, so for nested messages we collect what
        // we need first.
        // SAFETY: re-borrow `self.fields` after the descriptor look-up. Since
        // we already extracted `elem` as a Copy, no aliasing occurs.
        let v = self.decode_element_no_alias(elem, tag, buf, ctx)?;
        // Re-fetch the list — `decode_element_no_alias` may have allocated
        // entries in `self.fields` if `elem` is a message... it didn't, but
        // the borrow checker doesn't know that. Re-fetch is cheap.
        if let Some(Value::List(l)) = self.fields.get_mut(&number) {
            l.push(v);
        } else {
            self.fields.insert(number, Value::List(alloc::vec![v]));
        }
        Ok(())
    }

    fn merge_map_field(
        &mut self,
        number: u32,
        key_ty: ScalarType,
        value_kind: SingularKind,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        // A map entry is a length-delimited message with fields 1 (key) and
        // 2 (value).
        if tag.wire_type() != WireType::LengthDelimited {
            // Unexpected wire type — skip and preserve as unknown.
            self.unknown.push(decode_unknown_field(tag, buf, ctx)?);
            return Ok(());
        }
        let len = decode_varint(buf)?;
        let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
        if buf.remaining() < len {
            return Err(DecodeError::UnexpectedEof);
        }
        let mut entry = buf.copy_to_bytes(len);
        let mut key: Option<MapKey> = None;
        let mut value: Option<Value> = None;
        while entry.has_remaining() {
            let entry_tag = Tag::decode(&mut entry)?;
            match entry_tag.field_number() {
                1 => key = Some(decode_map_key(key_ty, entry_tag, &mut entry)?),
                2 => {
                    // The map entry is a sub-message on the wire, so it
                    // consumes one depth level. (The previous code used
                    // `depth.saturating_sub(1)` here, which let scalar map
                    // values through at exactly depth 0; erroring one level
                    // earlier is intentional.)
                    value = Some(self.decode_element_no_alias(
                        value_kind,
                        entry_tag,
                        &mut entry,
                        ctx.descend()?,
                    )?);
                }
                _ => skip_field_depth(entry_tag, &mut entry, ctx.depth())?,
            }
        }
        let k = key.unwrap_or_else(|| default_map_key(key_ty));
        let v = value.unwrap_or_else(|| default_value(value_kind, &self.pool));
        match self
            .fields
            .entry(number)
            .or_insert_with(|| Value::Map(MapValue::new()))
        {
            Value::Map(m) => {
                m.insert(k, v);
            }
            other => {
                let mut m = MapValue::new();
                m.insert(k, v);
                *other = Value::Map(m);
            }
        }
        Ok(())
    }

    /// Decode one singular element. Borrows `self` immutably so the caller
    /// can hold a mutable borrow on `self.fields`.
    fn decode_element(
        &self,
        kind: SingularKind,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<Value, DecodeError> {
        self.decode_element_no_alias(kind, tag, buf, ctx)
    }

    /// Decode one singular element. Named to distinguish call sites where
    /// the borrow checker can't see that no aliasing occurs.
    fn decode_element_no_alias(
        &self,
        kind: SingularKind,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<Value, DecodeError> {
        match kind {
            SingularKind::Scalar(s) => decode_scalar(s, tag.wire_type(), buf),
            SingularKind::Enum(_) => {
                // Enums are int32 varints on the wire.
                Ok(Value::EnumNumber(decode_int32(buf)?))
            }
            SingularKind::Message(midx) => {
                let mut nested = DynamicMessage::new(Arc::clone(&self.pool), midx);
                let ctx = ctx.descend()?;
                match tag.wire_type() {
                    WireType::LengthDelimited => {
                        let len = decode_varint(buf)?;
                        let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
                        if buf.remaining() < len {
                            return Err(DecodeError::UnexpectedEof);
                        }
                        let mut sub = buf.copy_to_bytes(len);
                        nested.merge_buf(&mut sub, ctx)?;
                    }
                    WireType::StartGroup => {
                        nested.merge_group(buf, tag.field_number(), ctx)?;
                    }
                    _ => {
                        return Err(DecodeError::WireTypeMismatch {
                            field_number: tag.field_number(),
                            expected: WireType::LengthDelimited as u8,
                            actual: tag.wire_type() as u8,
                        })
                    }
                }
                Ok(Value::Message(nested))
            }
        }
    }

    // ── Encode ──────────────────────────────────────────────────────────────

    /// Encode this message into `buf`.
    ///
    /// A stored `Value` whose shape does not match its field descriptor —
    /// only reachable by bypassing [`try_set`]'s validation via
    /// [`field_by_number_mut`](Self::field_by_number_mut) /
    /// [`field_mut`](Self::field_mut) — is silently omitted, as the whole
    /// field: one invalid element suppresses its entire repeated/map field.
    /// A nested `Value::Message` counts as invalid unless it was built from
    /// the *same* [`DescriptorPool`] instance (pointer identity, not
    /// full-name equality). [`encoded_len`](Self::encoded_len) applies the
    /// same rule, so length and bytes always agree. The JSON serializer
    /// represents the same invalid values as `null` rather than omitting
    /// the field.
    ///
    /// [`try_set`]: crate::reflect::ReflectMessageMut::try_set
    pub fn encode(&self, buf: &mut impl BufMut) {
        for (&number, value) in &self.fields {
            // Skip if neither the descriptor nor the extension index
            // recognizes this number anymore (defensive — the fields map
            // should be in sync with the descriptor).
            if let Some(fd) = self.field_or_extension(number) {
                if should_skip_on_encode(fd, value) {
                    continue;
                }
                encode_field(fd, value, &self.pool, buf);
            }
        }
        self.unknown.write_to(buf);
    }

    /// Encode this message to a fresh `Vec<u8>`.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_len());
        self.encode(&mut buf);
        buf
    }

    /// Compute the encoded length.
    ///
    /// Applies the same invalid-value omission rule as
    /// [`encode`](Self::encode), so the result always matches the bytes
    /// `encode` writes.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let mut len = self.unknown.encoded_len();
        for (&number, value) in &self.fields {
            if let Some(fd) = self.field_or_extension(number) {
                if should_skip_on_encode(fd, value) {
                    continue;
                }
                len += encoded_field_len(fd, value, &self.pool);
            }
        }
        len
    }

    // ── Bridge ──────────────────────────────────────────────────────────────

    /// Build a dynamic snapshot of a generated message via wire round-trip.
    ///
    /// # Panics
    ///
    /// Panics if `msg.encode_to_vec()` produces bytes that fail to decode
    /// against `msg_idx`'s descriptor. This indicates a mismatch between the
    /// descriptor in the pool and the generated `Message` impl.
    #[must_use]
    pub fn from_message<M: Message>(
        msg: &M,
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
    ) -> Self {
        let bytes = msg.encode_to_vec();
        Self::decode(pool, msg_idx, &bytes)
            .expect("generated message must round-trip through its own descriptor")
    }

    /// Reconstitute a generated message from this dynamic snapshot.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if the encoded bytes cannot be decoded into
    /// `M` — typically a descriptor mismatch between the pool and the
    /// generated type.
    pub fn to_message<M: Message + Default>(&self) -> Result<M, DecodeError> {
        let bytes = self.encode_to_vec();
        let mut m = M::default();
        m.merge_from_slice(&bytes)?;
        Ok(m)
    }

    // ── Direct field access ─────────────────────────────────────────────────

    /// Look up a field value by field number, if present.
    #[must_use]
    pub fn field_by_number(&self, number: u32) -> Option<&Value> {
        self.fields.get(&number)
    }

    /// Mutably borrow a field's value by field number, if present.
    ///
    /// In-place mutation, the counterpart to [`field_by_number`](Self::field_by_number).
    /// Returns `None` if the field is unset — use
    /// [`ReflectMessageMut::set`](crate::reflect::ReflectMessageMut::set) to
    /// assign an absent field.
    ///
    /// This is the primitive for deep mutation (e.g. an interceptor redacting
    /// PII strings at any nesting depth) without the read-clone-set-back
    /// dance: match the returned `&mut Value` and mutate the scalar, recurse
    /// into a `Value::Message`, or walk a `Value::List`/`Value::Map`. The
    /// descriptor-keyed [`field_mut`](Self::field_mut) is the ergonomic
    /// convenience over this.
    ///
    /// Clone the [`Arc<DescriptorPool>`](Self::pool) before the loop so the
    /// descriptor borrow is independent of the `&mut self` borrow:
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use buffa_descriptor::reflect::{DynamicMessage, Value};
    /// fn redact(msg: &mut DynamicMessage) {
    ///     let pool = Arc::clone(msg.pool());
    ///     let md = pool.message(msg.message_index());
    ///     for fd in md.fields() {
    ///         match msg.field_by_number_mut(fd.number()) {
    ///             Some(Value::String(s)) => *s = "[REDACTED]".into(),
    ///             Some(Value::Message(inner)) => redact(inner), // recurse
    ///             _ => {}
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// Replacing the value with one whose shape doesn't match the field's
    /// kind bypasses [`ReflectMessageMut::try_set`]'s validation. The encoder
    /// defensively skips invalid stored values, but callers that replace a
    /// whole value should prefer `try_set`.
    #[must_use]
    pub fn field_by_number_mut(&mut self, number: u32) -> Option<&mut Value> {
        self.fields.get_mut(&number)
    }

    /// Mutably borrow a field's value, if present.
    ///
    /// Descriptor-keyed convenience over [`field_by_number_mut`](Self::field_by_number_mut),
    /// paralleling [`ReflectMessage::get`](crate::reflect::ReflectMessage::get).
    /// `field` may be a declared field or a registered extension of this
    /// message; both resolve by number.
    ///
    /// Replacing the value with one whose shape doesn't match the field's
    /// kind bypasses [`ReflectMessageMut::try_set`]'s validation; the
    /// encoder defensively skips invalid stored values (see
    /// [`encode`](Self::encode)).
    ///
    /// # Panics
    ///
    /// Debug builds assert that `field` is a member of this message's
    /// descriptor (a declared field or a registered extension). A foreign
    /// descriptor with a colliding number would otherwise silently hand back
    /// the wrong field for mutation. Use [`field_by_number_mut`](Self::field_by_number_mut)
    /// for the unchecked, number-keyed path.
    #[must_use]
    pub fn field_mut(&mut self, field: &FieldDescriptor) -> Option<&mut Value> {
        debug_assert!(
            self.field_descriptor_is_member(field),
            "FieldDescriptor passed to field_mut() is not a member of {}",
            self.message_descriptor().full_name,
        );
        self.fields.get_mut(&field.number())
    }

    /// Insert a field value by number, bypassing the descriptor-keyed
    /// `ReflectMessageMut::set` API. Used internally by the WKT JSON codecs,
    /// which know the field layout statically and do not need oneof clearing.
    #[cfg_attr(not(feature = "json"), allow(dead_code))]
    pub(crate) fn insert_value(&mut self, number: u32, v: Value) {
        self.fields.insert(number, v);
    }

    /// The unknown fields preserved from decode.
    #[must_use]
    pub fn unknown_fields(&self) -> &UnknownFields {
        &self.unknown
    }

    // ── google.protobuf.Any ─────────────────────────────────────────────────

    /// Decode the message wrapped in this `google.protobuf.Any`.
    ///
    /// Reads `type_url` (field 1), resolves the type name (the segment after
    /// the last `/`) against this message's pool, and decodes `value`
    /// (field 2) into a [`DynamicMessage`] of that type. This is the binary
    /// counterpart of the `Any` JSON `@type` expansion; CEL `dyn` evaluation
    /// unpacks `Any` values this way.
    ///
    /// # Errors
    ///
    /// - [`AnyError::NotAny`] if this message is not a `google.protobuf.Any`.
    /// - [`AnyError::MissingTypeUrl`] if the `Any` has no `type_url`.
    /// - [`AnyError::UnknownType`] if the `type_url` names a type the pool
    ///   doesn't carry.
    /// - [`AnyError::Decode`] if the wrapped bytes are malformed. Note that
    ///   an `Any` whose `value` is absent or empty is **not** an error: it
    ///   decodes to a default-valued message of the resolved type, per the
    ///   protobuf default-value semantics.
    pub fn unpack_any(&self) -> Result<DynamicMessage, AnyError> {
        if self.message_descriptor().full_name != ANY_FULL_NAME {
            return Err(AnyError::NotAny {
                full_name: self.message_descriptor().full_name.clone(),
            });
        }
        let type_url = match self.field_by_number(1) {
            Some(Value::String(u)) if !u.is_empty() => u.as_str(),
            // An absent, empty, or wrong-shape type_url is a distinct
            // failure from "the URL named a type we don't have".
            _ => return Err(AnyError::MissingTypeUrl),
        };
        let value: &[u8] = match self.field_by_number(2) {
            Some(Value::Bytes(b)) => b,
            _ => &[],
        };
        let type_name = type_url.rsplit('/').next().unwrap_or(type_url);
        let idx = self
            .pool
            .message_index(type_name)
            .ok_or_else(|| AnyError::UnknownType {
                type_url: type_url.to_owned(),
            })?;
        DynamicMessage::decode(Arc::clone(&self.pool), idx, value).map_err(|source| {
            AnyError::Decode {
                type_url: type_url.to_owned(),
                source,
            }
        })
    }

    /// Wrap this message in a `google.protobuf.Any`.
    ///
    /// Produces an `Any` with `type_url` = `type.googleapis.com/<full_name>`
    /// (the Google convention; the prefix is informational and only the last
    /// path segment is load-bearing on unpack) and `value` = this message's
    /// encoded bytes, using the `google.protobuf.Any` descriptor from this
    /// message's pool. Unknown fields on `self` are preserved in the encoded
    /// `value`.
    ///
    /// `&self` is borrowed, not consumed — `self` remains usable.
    ///
    /// # Errors
    ///
    /// [`AnyError::AnyNotRegistered`] if the pool does not contain
    /// `google.protobuf.Any` (e.g. a `FileDescriptorSet` built without the
    /// well-known types).
    pub fn pack_any(&self) -> Result<DynamicMessage, AnyError> {
        let any_idx = self
            .pool
            .message_index(ANY_FULL_NAME)
            .ok_or(AnyError::AnyNotRegistered)?;
        let type_url = alloc::format!(
            "type.googleapis.com/{}",
            self.message_descriptor().full_name
        );
        let mut any = DynamicMessage::new(Arc::clone(&self.pool), any_idx);
        any.insert_value(1, Value::String(type_url));
        any.insert_value(2, Value::Bytes(self.encode_to_vec()));
        Ok(any)
    }

    /// Decode a generated `*Options` message into a `DynamicMessage` of the
    /// corresponding `google.protobuf.*Options` type, so its **custom
    /// options** (extensions of the options type) can be read reflectively
    /// by name.
    ///
    /// This is the convenience behind the custom-option flow documented on
    /// [`FieldDescriptor::options`](crate::FieldDescriptor::options): it
    /// resolves the options type by `O::FULL_NAME`, re-encodes `options`
    /// (custom options survive on its unknown fields), and decodes against
    /// the pool's descriptor for that type.
    ///
    /// ```no_run
    /// # #[cfg(feature = "json")] {
    /// # use std::sync::Arc;
    /// # use buffa_descriptor::{DescriptorPool, reflect::{DynamicMessage, ReflectMessage}};
    /// # fn demo(pool: Arc<DescriptorPool>, field: &buffa_descriptor::FieldDescriptor) -> Option<()> {
    /// let dyn_opts = DynamicMessage::from_options(Arc::clone(&pool), field.options()?)?;
    /// let ext = pool.extension_by_name("my.pkg.opt")?;
    /// let value = dyn_opts.get(ext.field());
    /// # let _ = value; Some(())
    /// # }
    /// # }
    /// ```
    ///
    /// Returns `None` if the options type (e.g. `google.protobuf.FieldOptions`)
    /// is not registered in `pool` — register `descriptor.proto` alongside the
    /// option-defining proto — or if the re-encoded options fail to decode.
    #[must_use]
    pub fn from_options<O>(pool: Arc<DescriptorPool>, options: &O) -> Option<Self>
    where
        O: Message + buffa::MessageName,
    {
        let idx = pool.message_index(O::FULL_NAME)?;
        Self::decode(pool, idx, &options.encode_to_vec()).ok()
    }
}

/// The fully-qualified name of `google.protobuf.Any`.
const ANY_FULL_NAME: &str = "google.protobuf.Any";

/// An error from [`DynamicMessage::unpack_any`] or
/// [`DynamicMessage::pack_any`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AnyError {
    /// [`unpack_any`](DynamicMessage::unpack_any) was called on a message
    /// that is not a `google.protobuf.Any`.
    NotAny {
        /// The actual message type.
        full_name: String,
    },
    /// The pool does not contain `google.protobuf.Any`, so an `Any` cannot
    /// be constructed.
    AnyNotRegistered,
    /// The `Any` has no `type_url` (field 1 absent, empty, or not a string).
    MissingTypeUrl,
    /// The `Any`'s `type_url` names a type the pool does not carry.
    UnknownType {
        /// The unresolvable type URL.
        type_url: String,
    },
    /// The wrapped `value` bytes failed to decode against the resolved type.
    Decode {
        /// The `type_url` that resolved before the decode failed.
        type_url: String,
        /// The underlying wire-format error.
        source: DecodeError,
    },
}

impl core::fmt::Display for AnyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAny { full_name } => {
                write!(f, "not a google.protobuf.Any: {full_name}")
            }
            Self::AnyNotRegistered => {
                write!(f, "google.protobuf.Any is not registered in the pool")
            }
            Self::MissingTypeUrl => write!(f, "Any has no type_url"),
            Self::UnknownType { type_url } => {
                write!(f, "Any type_url {type_url:?} not registered in the pool")
            }
            Self::Decode { type_url, source } => {
                write!(f, "Any value for {type_url:?} failed to decode: {source}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AnyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl ReflectMessage for DynamicMessage {
    fn message_descriptor(&self) -> &MessageDescriptor {
        self.pool.message(self.msg_idx)
    }

    fn pool(&self) -> &Arc<DescriptorPool> {
        &self.pool
    }

    fn get(&self, field: &FieldDescriptor) -> ValueRef<'_> {
        // The descriptor must belong to this message (a declared field or a
        // registered extension of it) — looking up by `field.number` against
        // a foreign descriptor with a colliding number would silently return
        // the wrong value, which is worse than a panic. The check is
        // debug-only because `get` is the hot path.
        debug_assert!(
            self.field_descriptor_is_member(field),
            "FieldDescriptor passed to get() is not a member of {}",
            self.message_descriptor().full_name,
        );
        match self.fields.get(&field.number()) {
            Some(v) => v.as_ref(),
            None => default_value_ref(field.kind, &self.pool),
        }
    }

    fn has(&self, field: &FieldDescriptor) -> bool {
        debug_assert!(
            self.field_descriptor_is_member(field),
            "FieldDescriptor passed to has() is not a member of {}",
            self.message_descriptor().full_name,
        );
        match self.fields.get(&field.number()) {
            None => false,
            Some(Value::List(l)) => !l.is_empty(),
            Some(Value::Map(m)) => !m.is_empty(),
            // Implicit-presence singular fields are "present" only when
            // non-default. Explicit-presence and `LegacyRequired` fields
            // are present whenever they appear in the field map. This is
            // what makes `set(fd, default)` on an implicit-presence field
            // round-trip to "absent" through both binary and JSON encode.
            Some(v) if field.presence == buffa::editions::FieldPresence::Implicit => {
                !is_default_scalar(v)
            }
            Some(_) => true,
        }
    }

    fn for_each_set(&self, f: &mut dyn FnMut(&FieldDescriptor, ValueRef<'_>)) {
        // Extensions present on this message are visited alongside declared
        // fields, matching protobuf-go's `Message.Range`. Callers that need
        // to distinguish can check `message_descriptor().field(fd.number())`
        // — `None` means the descriptor came from the extension index.
        for (&number, value) in &self.fields {
            if let Some(fd) = self.field_or_extension(number) {
                if !self.has(fd) {
                    continue;
                }
                f(fd, value.as_ref());
            }
        }
    }

    fn unknown_fields(&self) -> &buffa::UnknownFields {
        &self.unknown
    }

    fn to_dynamic(&self) -> DynamicMessage {
        self.clone()
    }
}

impl ReflectMessageMut for DynamicMessage {
    fn try_set(&mut self, field: &FieldDescriptor, value: Value) -> Result<(), ReflectError> {
        self.validate_field_descriptor(field)?;
        self.validate_field_value(field, &value)?;
        // Setting a oneof member must clear its siblings, otherwise a
        // subsequent encode writes multiple oneof members onto the wire,
        // which violates the proto spec. The wire decoder and the JSON
        // deserializer enforce this too — `set` is the third write path,
        // and a `ReflectMessageMut`-driven mutation (a CEL evaluator, a
        // fuzzer, generic field-mask application) must not be the one that
        // breaks the invariant.
        if let Some(oi) = field.oneof_index() {
            self.clear_other_oneof_members(oi, field.number());
        }
        self.fields.insert(field.number(), value);
        Ok(())
    }

    fn set(&mut self, field: &FieldDescriptor, value: Value) {
        self.try_set(field, value)
            .unwrap_or_else(|err| panic!("{err}"));
    }

    fn try_clear(&mut self, field: &FieldDescriptor) -> Result<(), ReflectError> {
        self.validate_field_descriptor(field)?;
        self.fields.remove(&field.number());
        Ok(())
    }

    fn clear(&mut self, field: &FieldDescriptor) {
        self.try_clear(field).unwrap_or_else(|err| panic!("{err}"));
    }
}

// ── Free helper functions ───────────────────────────────────────────────────

/// Whether a stored field value should be skipped when encoding.
///
/// Implicit-presence singular scalars and enums are not serialized when
/// they hold the type's default value — that's the proto3 implicit-presence
/// semantics. Explicit-presence and `LegacyRequired` fields always serialize
/// when present; repeated/map fields serialize when non-empty (which is
/// already implied by the `Value::List`/`Value::Map` shape: an empty
/// container produces zero bytes from `encode_field`).
///
/// IEEE -0.0 is considered non-default for float/double — the proto3 spec
/// treats only +0.0 as the default, and the conformance suite checks that
/// -0.0 round-trips through implicit-presence fields.
fn should_skip_on_encode(fd: &FieldDescriptor, value: &Value) -> bool {
    if fd.presence != buffa::editions::FieldPresence::Implicit {
        return false;
    }
    match (&fd.kind, value) {
        (FieldKind::Singular(SingularKind::Scalar(_) | SingularKind::Enum(_)), v) => {
            is_default_scalar(v)
        }
        // Singular message fields always serialize when present — they
        // never have implicit presence in practice, but be defensive.
        // List/Map skipping for empty containers happens in `encode_field`.
        _ => false,
    }
}

#[derive(Debug)]
struct ValueShapeMismatch {
    expected: String,
    actual: String,
}

fn validate_value_shape(
    kind: FieldKind,
    value: &Value,
    pool: &Arc<DescriptorPool>,
) -> Result<(), ValueShapeMismatch> {
    match kind {
        FieldKind::Singular(sk) => validate_singular_shape(sk, value, pool),
        FieldKind::List(sk) => {
            let Value::List(items) = value else {
                return Err(field_shape_mismatch(kind, pool, value_shape(value)));
            };
            for item in items {
                validate_singular_shape(sk, item, pool).map_err(|err| {
                    field_shape_mismatch(kind, pool, format!("list element {}", err.actual))
                })?;
            }
            Ok(())
        }
        FieldKind::Map { key, value: vk } => {
            let Value::Map(entries) = value else {
                return Err(field_shape_mismatch(kind, pool, value_shape(value)));
            };
            for (map_key, map_value) in entries {
                if !map_key_matches_scalar(key, map_key) {
                    return Err(field_shape_mismatch(
                        kind,
                        pool,
                        format!("map key {}", map_key_shape(map_key)),
                    ));
                }
                validate_singular_shape(vk, map_value, pool).map_err(|err| {
                    field_shape_mismatch(kind, pool, format!("map value {}", err.actual))
                })?;
            }
            Ok(())
        }
    }
}

fn validate_singular_shape(
    kind: SingularKind,
    value: &Value,
    pool: &Arc<DescriptorPool>,
) -> Result<(), ValueShapeMismatch> {
    let matches = match kind {
        SingularKind::Scalar(s) => value_matches_scalar(s, value),
        // Enums are validated by variant only — no enum-descriptor identity
        // or known-value check, deliberately asymmetric with the message arm
        // below: enums are plain i32 on the wire (proto3 keeps unknown
        // numbers), and JSON serialization looks the number up against the
        // *field's* enum descriptor, so a foreign or unknown number cannot
        // corrupt output.
        SingularKind::Enum(_) => matches!(value, Value::EnumNumber(_)),
        // Pointer identity, not full-name equality: a structurally-identical
        // message built from a different pool instance is foreign, matching
        // `FieldNotMember`'s identity philosophy.
        SingularKind::Message(midx) => match value {
            Value::Message(msg) => Arc::ptr_eq(&msg.pool, pool) && msg.msg_idx == midx,
            _ => false,
        },
    };
    if matches {
        Ok(())
    } else {
        Err(ValueShapeMismatch {
            expected: singular_kind_name(kind, pool),
            actual: value_shape(value),
        })
    }
}

fn field_shape_mismatch(
    kind: FieldKind,
    pool: &DescriptorPool,
    actual: String,
) -> ValueShapeMismatch {
    ValueShapeMismatch {
        expected: field_kind_name(kind, pool),
        actual,
    }
}

fn field_kind_name(kind: FieldKind, pool: &DescriptorPool) -> String {
    match kind {
        FieldKind::Singular(sk) => singular_kind_name(sk, pool),
        FieldKind::List(sk) => format!("list<{}>", singular_kind_name(sk, pool)),
        FieldKind::Map { key, value } => {
            format!(
                "map<{}, {}>",
                scalar_type_name(key),
                singular_kind_name(value, pool)
            )
        }
    }
}

fn value_matches_field_shape(kind: FieldKind, value: &Value, pool: &Arc<DescriptorPool>) -> bool {
    validate_value_shape(kind, value, pool).is_ok()
}

fn value_matches_scalar(s: ScalarType, value: &Value) -> bool {
    matches!(
        (s, value),
        (ScalarType::Double, Value::F64(_))
            | (ScalarType::Float, Value::F32(_))
            | (
                ScalarType::Int64 | ScalarType::Sfixed64 | ScalarType::Sint64,
                Value::I64(_)
            )
            | (ScalarType::Uint64 | ScalarType::Fixed64, Value::U64(_))
            | (
                ScalarType::Int32 | ScalarType::Sfixed32 | ScalarType::Sint32,
                Value::I32(_)
            )
            | (ScalarType::Uint32 | ScalarType::Fixed32, Value::U32(_))
            | (ScalarType::Bool, Value::Bool(_))
            | (ScalarType::String, Value::String(_))
            | (ScalarType::Bytes, Value::Bytes(_))
    )
}

fn map_key_matches_scalar(s: ScalarType, key: &MapKey) -> bool {
    matches!(
        (s, key),
        (ScalarType::Bool, MapKey::Bool(_))
            | (
                ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32,
                MapKey::I32(_)
            )
            | (
                ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64,
                MapKey::I64(_)
            )
            | (ScalarType::Uint32 | ScalarType::Fixed32, MapKey::U32(_))
            | (ScalarType::Uint64 | ScalarType::Fixed64, MapKey::U64(_))
            | (ScalarType::String, MapKey::String(_))
    )
}

fn singular_kind_name(kind: SingularKind, pool: &DescriptorPool) -> String {
    match kind {
        SingularKind::Scalar(s) => scalar_type_name(s).into(),
        SingularKind::Enum(eidx) => format!("enum {}", pool.enumeration(eidx).full_name()),
        SingularKind::Message(midx) => format!("message {}", pool.message(midx).full_name()),
    }
}

fn scalar_type_name(s: ScalarType) -> &'static str {
    match s {
        ScalarType::Double => "double",
        ScalarType::Float => "float",
        ScalarType::Int64 => "int64",
        ScalarType::Uint64 => "uint64",
        ScalarType::Int32 => "int32",
        ScalarType::Fixed64 => "fixed64",
        ScalarType::Fixed32 => "fixed32",
        ScalarType::Bool => "bool",
        ScalarType::String => "string",
        ScalarType::Bytes => "bytes",
        ScalarType::Uint32 => "uint32",
        ScalarType::Sfixed32 => "sfixed32",
        ScalarType::Sfixed64 => "sfixed64",
        ScalarType::Sint32 => "sint32",
        ScalarType::Sint64 => "sint64",
    }
}

fn value_shape(value: &Value) -> String {
    match value {
        Value::Bool(_) => "bool".into(),
        Value::I32(_) => "i32".into(),
        Value::I64(_) => "i64".into(),
        Value::U32(_) => "u32".into(),
        Value::U64(_) => "u64".into(),
        Value::F32(_) => "f32".into(),
        Value::F64(_) => "f64".into(),
        Value::String(_) => "string".into(),
        Value::Bytes(_) => "bytes".into(),
        Value::EnumNumber(_) => "enum number".into(),
        Value::Message(msg) => format!("message {}", msg.message_descriptor().full_name()),
        Value::List(_) => "list".into(),
        Value::Map(_) => "map".into(),
    }
}

fn map_key_shape(key: &MapKey) -> &'static str {
    match key {
        MapKey::Bool(_) => "bool",
        MapKey::I32(_) => "i32",
        MapKey::I64(_) => "i64",
        MapKey::U32(_) => "u32",
        MapKey::U64(_) => "u64",
        MapKey::String(_) => "string",
    }
}

/// Whether a `Value` is its proto type's default. Used for implicit-presence
/// encode skipping. Floats use `to_bits()` so -0.0 is treated as non-default.
fn is_default_scalar(v: &Value) -> bool {
    match v {
        Value::Bool(b) => !b,
        Value::I32(n) => *n == 0,
        Value::I64(n) => *n == 0,
        Value::U32(n) => *n == 0,
        Value::U64(n) => *n == 0,
        Value::F32(f) => f.to_bits() == 0,
        Value::F64(f) => f.to_bits() == 0,
        Value::String(s) => s.is_empty(),
        Value::Bytes(b) => b.is_empty(),
        Value::EnumNumber(n) => *n == 0,
        Value::Message(_) | Value::List(_) | Value::Map(_) => false,
    }
}

fn is_packable(kind: SingularKind) -> bool {
    match kind {
        SingularKind::Scalar(s) => !matches!(s, ScalarType::String | ScalarType::Bytes),
        SingularKind::Enum(_) => true,
        SingularKind::Message(_) => false,
    }
}

fn decode_scalar(s: ScalarType, wt: WireType, buf: &mut impl Buf) -> Result<Value, DecodeError> {
    let _ = wt; // wire type is implied by scalar type for length-prefixed scalars
    Ok(match s {
        ScalarType::Double => Value::F64(decode_double(buf)?),
        ScalarType::Float => Value::F32(decode_float(buf)?),
        ScalarType::Int64 => Value::I64(decode_int64(buf)?),
        ScalarType::Uint64 => Value::U64(decode_uint64(buf)?),
        ScalarType::Int32 => Value::I32(decode_int32(buf)?),
        ScalarType::Fixed64 => Value::U64(decode_fixed64(buf)?),
        ScalarType::Fixed32 => Value::U32(decode_fixed32(buf)?),
        ScalarType::Bool => Value::Bool(decode_bool(buf)?),
        ScalarType::String => Value::String(buffa::types::decode_string(buf)?),
        ScalarType::Bytes => Value::Bytes(buffa::types::decode_bytes(buf)?),
        ScalarType::Uint32 => Value::U32(decode_uint32(buf)?),
        ScalarType::Sfixed32 => Value::I32(decode_sfixed32(buf)?),
        ScalarType::Sfixed64 => Value::I64(decode_sfixed64(buf)?),
        ScalarType::Sint32 => Value::I32(decode_sint32(buf)?),
        ScalarType::Sint64 => Value::I64(decode_sint64(buf)?),
    })
}

fn decode_packed_element(
    kind: SingularKind,
    field_number: u32,
    buf: &mut impl Buf,
) -> Result<Value, DecodeError> {
    match kind {
        SingularKind::Scalar(s) => decode_scalar(s, WireType::Varint, buf),
        SingularKind::Enum(_) => Ok(Value::EnumNumber(decode_int32(buf)?)),
        // Message-typed elements are never packed — a descriptor that says
        // otherwise is structurally inconsistent. The expected wire type for
        // the packed payload is Varint (the element-by-element loop), the
        // actual is LengthDelimited (the message framing).
        SingularKind::Message(_) => Err(DecodeError::WireTypeMismatch {
            field_number,
            expected: WireType::Varint as u8,
            actual: WireType::LengthDelimited as u8,
        }),
    }
}

fn decode_map_key(s: ScalarType, tag: Tag, buf: &mut impl Buf) -> Result<MapKey, DecodeError> {
    Ok(match s {
        ScalarType::Bool => MapKey::Bool(decode_bool(buf)?),
        ScalarType::Int32 => MapKey::I32(decode_int32(buf)?),
        ScalarType::Sint32 => MapKey::I32(decode_sint32(buf)?),
        ScalarType::Sfixed32 => MapKey::I32(decode_sfixed32(buf)?),
        ScalarType::Int64 => MapKey::I64(decode_int64(buf)?),
        ScalarType::Sint64 => MapKey::I64(decode_sint64(buf)?),
        ScalarType::Sfixed64 => MapKey::I64(decode_sfixed64(buf)?),
        ScalarType::Uint32 => MapKey::U32(decode_uint32(buf)?),
        ScalarType::Fixed32 => MapKey::U32(decode_fixed32(buf)?),
        ScalarType::Uint64 => MapKey::U64(decode_uint64(buf)?),
        ScalarType::Fixed64 => MapKey::U64(decode_fixed64(buf)?),
        ScalarType::String => MapKey::String(buffa::types::decode_string(buf)?),
        // Floats and bytes are not valid map key types; the pool linker
        // rejects these, so this is unreachable for a well-formed pool. The
        // error names the map-entry tag so corrupt input still produces a
        // useful diagnostic.
        ScalarType::Double | ScalarType::Float | ScalarType::Bytes => {
            return Err(DecodeError::WireTypeMismatch {
                field_number: tag.field_number(),
                expected: WireType::Varint as u8,
                actual: tag.wire_type() as u8,
            });
        }
    })
}

fn default_map_key(s: ScalarType) -> MapKey {
    match s {
        ScalarType::Bool => MapKey::Bool(false),
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => MapKey::I32(0),
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => MapKey::I64(0),
        ScalarType::Uint32 | ScalarType::Fixed32 => MapKey::U32(0),
        ScalarType::Uint64 | ScalarType::Fixed64 => MapKey::U64(0),
        ScalarType::String => MapKey::String(String::new()),
        ScalarType::Double | ScalarType::Float | ScalarType::Bytes => MapKey::Bool(false),
    }
}

fn default_value(kind: SingularKind, pool: &Arc<DescriptorPool>) -> Value {
    match kind {
        SingularKind::Scalar(s) => default_scalar_value(s),
        SingularKind::Enum(_) => Value::EnumNumber(0),
        SingularKind::Message(midx) => Value::Message(DynamicMessage::new(Arc::clone(pool), midx)),
    }
}

pub(super) fn default_scalar_value(s: ScalarType) -> Value {
    match s {
        ScalarType::Double => Value::F64(0.0),
        ScalarType::Float => Value::F32(0.0),
        ScalarType::Int64 | ScalarType::Sfixed64 | ScalarType::Sint64 => Value::I64(0),
        ScalarType::Uint64 | ScalarType::Fixed64 => Value::U64(0),
        ScalarType::Int32 | ScalarType::Sfixed32 | ScalarType::Sint32 => Value::I32(0),
        ScalarType::Uint32 | ScalarType::Fixed32 => Value::U32(0),
        ScalarType::Bool => Value::Bool(false),
        ScalarType::String => Value::String(String::new()),
        ScalarType::Bytes => Value::Bytes(Vec::new()),
    }
}

/// Build a default `ValueRef` for an absent field. Containers borrow
/// statically-allocated empties; scalars are inline. Message-typed singulars
/// allocate a fresh empty `DynamicMessage` boxed into a `ReflectCow::Owned`,
/// since there's no `'static` empty to borrow — this is the one path where an
/// absent-field read allocates.
fn default_value_ref(kind: FieldKind, pool: &Arc<DescriptorPool>) -> ValueRef<'static> {
    // `Vec::new()` and `MapValue::new()` are both `const fn`, so both
    // empties are real `static`s — no leak pattern, no `OnceLock`, no
    // unsafe. The `&dyn ReflectList`/`&dyn ReflectMap` coercions are
    // unsizing casts on shared statics.
    static EMPTY_LIST: Vec<Value> = Vec::new();
    static EMPTY_MAP: MapValue = MapValue::new();
    match kind {
        FieldKind::Singular(SingularKind::Scalar(s)) => default_scalar_ref(s),
        FieldKind::Singular(SingularKind::Enum(_)) => ValueRef::EnumNumber(0),
        FieldKind::Singular(SingularKind::Message(midx)) => ValueRef::Message(ReflectCow::Owned(
            alloc::boxed::Box::new(DynamicMessage::new(Arc::clone(pool), midx)),
        )),
        FieldKind::List(_) => ValueRef::List(&EMPTY_LIST),
        FieldKind::Map { .. } => ValueRef::Map(&EMPTY_MAP),
    }
}

fn default_scalar_ref(s: ScalarType) -> ValueRef<'static> {
    match s {
        ScalarType::Double => ValueRef::F64(0.0),
        ScalarType::Float => ValueRef::F32(0.0),
        ScalarType::Int64 | ScalarType::Sfixed64 | ScalarType::Sint64 => ValueRef::I64(0),
        ScalarType::Uint64 | ScalarType::Fixed64 => ValueRef::U64(0),
        ScalarType::Int32 | ScalarType::Sfixed32 | ScalarType::Sint32 => ValueRef::I32(0),
        ScalarType::Uint32 | ScalarType::Fixed32 => ValueRef::U32(0),
        ScalarType::Bool => ValueRef::Bool(false),
        ScalarType::String => ValueRef::String(""),
        ScalarType::Bytes => ValueRef::Bytes(&[]),
    }
}

// ── Encode helpers ──────────────────────────────────────────────────────────

fn encode_field(
    fd: &FieldDescriptor,
    value: &Value,
    pool: &Arc<DescriptorPool>,
    buf: &mut impl BufMut,
) {
    if !value_matches_field_shape(fd.kind, value, pool) {
        return;
    }
    match (&fd.kind, value) {
        (FieldKind::Singular(sk), v) => {
            encode_singular_with_tag(fd.number, *sk, fd.delimited, v, buf);
        }
        (FieldKind::List(sk), Value::List(items)) => {
            if items.is_empty() {
                return;
            }
            if fd.packed && is_packable(*sk) {
                // Packed: one tag + length + concatenated payload.
                Tag::new(fd.number, WireType::LengthDelimited).encode(buf);
                let payload_len: usize = items.iter().map(|i| packed_element_len(*sk, i)).sum();
                encode_varint(payload_len as u64, buf);
                for item in items {
                    encode_packed_element(*sk, item, buf);
                }
            } else {
                for item in items {
                    encode_singular_with_tag(fd.number, *sk, fd.delimited, item, buf);
                }
            }
        }
        (FieldKind::Map { key, value: vk }, Value::Map(m)) => {
            for (k, v) in m {
                Tag::new(fd.number, WireType::LengthDelimited).encode(buf);
                let entry_len = map_key_len(*key, k)
                    + 1 // key tag is always 1 byte (field 1)
                    + map_value_len(*vk, v)
                    + 1; // value tag is always 1 byte (field 2)
                encode_varint(entry_len as u64, buf);
                Tag::new(1, map_key_wire_type(*key)).encode(buf);
                encode_map_key(*key, k, buf);
                Tag::new(2, singular_wire_type(*vk, false)).encode(buf);
                encode_packed_element(*vk, v, buf); // map values are never groups
            }
        }
        _ => {
            // `value_matches_field_shape` checked this above.
        }
    }
}

fn encoded_field_len(fd: &FieldDescriptor, value: &Value, pool: &Arc<DescriptorPool>) -> usize {
    if !value_matches_field_shape(fd.kind, value, pool) {
        return 0;
    }
    match (&fd.kind, value) {
        (FieldKind::Singular(sk), v) => singular_len_with_tag(fd.number, *sk, fd.delimited, v),
        (FieldKind::List(sk), Value::List(items)) => {
            if items.is_empty() {
                return 0;
            }
            if fd.packed && is_packable(*sk) {
                let payload: usize = items.iter().map(|i| packed_element_len(*sk, i)).sum();
                tag_len(fd.number, WireType::LengthDelimited)
                    + uint64_encoded_len(payload as u64)
                    + payload
            } else {
                items
                    .iter()
                    .map(|i| singular_len_with_tag(fd.number, *sk, fd.delimited, i))
                    .sum()
            }
        }
        (FieldKind::Map { key, value: vk }, Value::Map(m)) => m
            .iter()
            .map(|(k, v)| {
                let entry_len = 1 + map_key_len(*key, k) + 1 + map_value_len(*vk, v);
                tag_len(fd.number, WireType::LengthDelimited)
                    + uint64_encoded_len(entry_len as u64)
                    + entry_len
            })
            .sum(),
        _ => 0,
    }
}

fn singular_wire_type(kind: SingularKind, delimited: bool) -> WireType {
    match kind {
        SingularKind::Scalar(s) => match s {
            ScalarType::Double | ScalarType::Fixed64 | ScalarType::Sfixed64 => WireType::Fixed64,
            ScalarType::Float | ScalarType::Fixed32 | ScalarType::Sfixed32 => WireType::Fixed32,
            ScalarType::String | ScalarType::Bytes => WireType::LengthDelimited,
            _ => WireType::Varint,
        },
        SingularKind::Enum(_) => WireType::Varint,
        SingularKind::Message(_) => {
            if delimited {
                WireType::StartGroup
            } else {
                WireType::LengthDelimited
            }
        }
    }
}

/// Whether `actual` is an acceptable wire type for a field of the given
/// `kind`. Mismatched fields are treated as unknown per the protobuf spec.
///
/// Lists and maps accept `LengthDelimited` (the packed/map-entry framing) in
/// addition to the per-element wire type; messages accept both
/// `LengthDelimited` and `StartGroup` regardless of the resolved `delimited`
/// flag, because a peer using the other encoding is interoperable per spec.
fn wire_type_compatible(kind: FieldKind, actual: WireType, delimited: bool) -> bool {
    match kind {
        FieldKind::Singular(SingularKind::Message(_)) => {
            // Both encodings are valid wire forms for a message; the
            // decoder branches on the actual wire type. `delimited` only
            // affects the *encode* side.
            let _ = delimited;
            matches!(actual, WireType::LengthDelimited | WireType::StartGroup)
        }
        FieldKind::Singular(sk) => actual == singular_wire_type(sk, false),
        // Lists: packed payload is `LengthDelimited`; expanded/non-packable
        // is the per-element wire type. Accept either, since the spec
        // permits a packable repeated field to appear in either form.
        FieldKind::List(SingularKind::Message(_)) => {
            matches!(actual, WireType::LengthDelimited | WireType::StartGroup)
        }
        FieldKind::List(sk) => {
            actual == WireType::LengthDelimited || actual == singular_wire_type(sk, false)
        }
        // Maps: each entry is a `LengthDelimited` synthetic message.
        FieldKind::Map { .. } => actual == WireType::LengthDelimited,
    }
}

fn encode_singular_with_tag(
    number: u32,
    kind: SingularKind,
    delimited: bool,
    value: &Value,
    buf: &mut impl BufMut,
) {
    match (kind, value) {
        (SingularKind::Scalar(s), v) if value_matches_scalar(s, v) => {
            Tag::new(number, singular_wire_type(kind, delimited)).encode(buf);
            encode_scalar(s, v, buf);
        }
        (SingularKind::Enum(_), Value::EnumNumber(n)) => {
            Tag::new(number, singular_wire_type(kind, delimited)).encode(buf);
            encode_int32(*n, buf);
        }
        (SingularKind::Message(_), Value::Message(m)) => {
            Tag::new(number, singular_wire_type(kind, delimited)).encode(buf);
            if delimited {
                m.encode(buf);
                Tag::new(number, WireType::EndGroup).encode(buf);
            } else {
                let len = m.encoded_len();
                encode_varint(len as u64, buf);
                m.encode(buf);
            }
        }
        _ => {}
    }
}

fn singular_len_with_tag(number: u32, kind: SingularKind, delimited: bool, value: &Value) -> usize {
    let tag_bytes = tag_len(number, singular_wire_type(kind, delimited));
    let payload = match (kind, value) {
        (SingularKind::Scalar(s), v) if value_matches_scalar(s, v) => scalar_len(s, v),
        (SingularKind::Enum(_), Value::EnumNumber(n)) => int32_encoded_len(*n),
        (SingularKind::Message(_), Value::Message(m)) => {
            if delimited {
                // payload + end-group tag
                m.encoded_len() + tag_len(number, WireType::EndGroup)
            } else {
                let inner = m.encoded_len();
                uint64_encoded_len(inner as u64) + inner
            }
        }
        _ => return 0,
    };
    tag_bytes + payload
}

fn encode_scalar(s: ScalarType, v: &Value, buf: &mut impl BufMut) {
    match (s, v) {
        (ScalarType::Double, Value::F64(x)) => encode_double(*x, buf),
        (ScalarType::Float, Value::F32(x)) => encode_float(*x, buf),
        (ScalarType::Int64, Value::I64(x)) => encode_int64(*x, buf),
        (ScalarType::Uint64, Value::U64(x)) => encode_uint64(*x, buf),
        (ScalarType::Int32, Value::I32(x)) => encode_int32(*x, buf),
        (ScalarType::Fixed64, Value::U64(x)) => encode_fixed64(*x, buf),
        (ScalarType::Fixed32, Value::U32(x)) => encode_fixed32(*x, buf),
        (ScalarType::Bool, Value::Bool(x)) => encode_bool(*x, buf),
        (ScalarType::String, Value::String(x)) => buffa::types::encode_string(x, buf),
        (ScalarType::Bytes, Value::Bytes(x)) => buffa::types::encode_bytes(x, buf),
        (ScalarType::Uint32, Value::U32(x)) => encode_uint32(*x, buf),
        (ScalarType::Sfixed32, Value::I32(x)) => encode_sfixed32(*x, buf),
        (ScalarType::Sfixed64, Value::I64(x)) => encode_sfixed64(*x, buf),
        (ScalarType::Sint32, Value::I32(x)) => encode_sint32(*x, buf),
        (ScalarType::Sint64, Value::I64(x)) => encode_sint64(*x, buf),
        _ => {}
    }
}

fn scalar_len(s: ScalarType, v: &Value) -> usize {
    match (s, v) {
        (ScalarType::Double, Value::F64(_)) => 8,
        (ScalarType::Fixed64, Value::U64(_)) | (ScalarType::Sfixed64, Value::I64(_)) => 8,
        (ScalarType::Float, Value::F32(_)) => 4,
        (ScalarType::Fixed32, Value::U32(_)) | (ScalarType::Sfixed32, Value::I32(_)) => 4,
        (ScalarType::Bool, Value::Bool(_)) => buffa::types::BOOL_ENCODED_LEN,
        (ScalarType::Int64, Value::I64(x)) => int64_encoded_len(*x),
        (ScalarType::Uint64, Value::U64(x)) => uint64_encoded_len(*x),
        (ScalarType::Int32, Value::I32(x)) => int32_encoded_len(*x),
        (ScalarType::Uint32, Value::U32(x)) => uint32_encoded_len(*x),
        (ScalarType::Sint32, Value::I32(x)) => sint32_encoded_len(*x),
        (ScalarType::Sint64, Value::I64(x)) => sint64_encoded_len(*x),
        (ScalarType::String, Value::String(x)) => uint64_encoded_len(x.len() as u64) + x.len(),
        (ScalarType::Bytes, Value::Bytes(x)) => uint64_encoded_len(x.len() as u64) + x.len(),
        _ => 0,
    }
}

fn encode_packed_element(kind: SingularKind, v: &Value, buf: &mut impl BufMut) {
    match (kind, v) {
        (SingularKind::Scalar(s), v) => encode_scalar(s, v, buf),
        (SingularKind::Enum(_), Value::EnumNumber(n)) => encode_int32(*n, buf),
        (SingularKind::Message(_), Value::Message(m)) => {
            // Map values are length-prefixed messages.
            let len = m.encoded_len();
            encode_varint(len as u64, buf);
            m.encode(buf);
        }
        _ => {}
    }
}

fn packed_element_len(kind: SingularKind, v: &Value) -> usize {
    match (kind, v) {
        (SingularKind::Scalar(s), v) => scalar_len(s, v),
        (SingularKind::Enum(_), Value::EnumNumber(n)) => int32_encoded_len(*n),
        (SingularKind::Message(_), Value::Message(m)) => {
            let inner = m.encoded_len();
            uint64_encoded_len(inner as u64) + inner
        }
        _ => 0,
    }
}

fn map_value_len(kind: SingularKind, v: &Value) -> usize {
    packed_element_len(kind, v)
}

fn map_key_wire_type(s: ScalarType) -> WireType {
    match s {
        ScalarType::Fixed32 | ScalarType::Sfixed32 => WireType::Fixed32,
        ScalarType::Fixed64 | ScalarType::Sfixed64 => WireType::Fixed64,
        ScalarType::String => WireType::LengthDelimited,
        _ => WireType::Varint,
    }
}

fn encode_map_key(s: ScalarType, k: &MapKey, buf: &mut impl BufMut) {
    match (s, k) {
        (ScalarType::Bool, MapKey::Bool(x)) => encode_bool(*x, buf),
        (ScalarType::Int32, MapKey::I32(x)) => encode_int32(*x, buf),
        (ScalarType::Sint32, MapKey::I32(x)) => encode_sint32(*x, buf),
        (ScalarType::Sfixed32, MapKey::I32(x)) => encode_sfixed32(*x, buf),
        (ScalarType::Int64, MapKey::I64(x)) => encode_int64(*x, buf),
        (ScalarType::Sint64, MapKey::I64(x)) => encode_sint64(*x, buf),
        (ScalarType::Sfixed64, MapKey::I64(x)) => encode_sfixed64(*x, buf),
        (ScalarType::Uint32, MapKey::U32(x)) => encode_uint32(*x, buf),
        (ScalarType::Fixed32, MapKey::U32(x)) => encode_fixed32(*x, buf),
        (ScalarType::Uint64, MapKey::U64(x)) => encode_uint64(*x, buf),
        (ScalarType::Fixed64, MapKey::U64(x)) => encode_fixed64(*x, buf),
        (ScalarType::String, MapKey::String(x)) => buffa::types::encode_string(x, buf),
        _ => {}
    }
}

fn map_key_len(s: ScalarType, k: &MapKey) -> usize {
    match (s, k) {
        (ScalarType::Fixed32, MapKey::U32(_)) | (ScalarType::Sfixed32, MapKey::I32(_)) => 4,
        (ScalarType::Fixed64, MapKey::U64(_)) | (ScalarType::Sfixed64, MapKey::I64(_)) => 8,
        (ScalarType::Bool, MapKey::Bool(_)) => buffa::types::BOOL_ENCODED_LEN,
        (ScalarType::Int32, MapKey::I32(x)) => int32_encoded_len(*x),
        (ScalarType::Sint32, MapKey::I32(x)) => sint32_encoded_len(*x),
        (ScalarType::Int64, MapKey::I64(x)) => int64_encoded_len(*x),
        (ScalarType::Sint64, MapKey::I64(x)) => sint64_encoded_len(*x),
        (ScalarType::Uint32, MapKey::U32(x)) => uint32_encoded_len(*x),
        (ScalarType::Uint64, MapKey::U64(x)) => uint64_encoded_len(*x),
        (ScalarType::String, MapKey::String(x)) => uint64_encoded_len(x.len() as u64) + x.len(),
        _ => 0,
    }
}

// EnumIndex unused in this module's public surface — silence an unused-import
// warning while keeping the import for symmetry.
#[allow(dead_code)]
const _: fn(EnumIndex) = |_| {};

/// Compute the encoded length of a tag without writing it.
fn tag_len(field_number: u32, wt: WireType) -> usize {
    uint64_encoded_len(((field_number as u64) << 3) | (wt as u64))
}

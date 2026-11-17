//! Tests for `ProtoBytes::decode_length_delimited` and
//! `ProtoString::decode_length_delimited`.
//!
//! The override types reject everything in `from_wire`, so a successful decode
//! proves that dispatch reached the override rather than the default.

use super::*;

/// Wire bytes of a three-byte field followed by one byte of the next field.
const FIELD_THEN_NEXT: [u8; 5] = [3, b'a', b'b', b'c', 0xff];

#[derive(Clone, Debug, Default, PartialEq)]
struct Override<T>(T);
impl<T> From<T> for Override<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}
impl<T: core::ops::Deref> core::ops::Deref for Override<T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsRef<[u8]> for Override<Vec<u8>> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<str> for Override<String> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Copy every remaining byte of `payload`, chunk by chunk.
fn drain(payload: &mut FieldPayload<impl Buf>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.remaining());
    while payload.has_remaining() {
        let chunk = payload.chunk();
        out.extend_from_slice(chunk);
        let read = chunk.len();
        payload.advance(read);
    }
    out
}

impl ProtoBytes for Override<Vec<u8>> {
    fn from_wire(_: WirePayload<'_>) -> Result<Self, DecodeError> {
        Err(DecodeError::Custom("from_wire must not be called"))
    }
    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        Ok(Self(drain(payload)))
    }
}
impl ProtoString for Override<String> {
    fn from_wire(_: WirePayload<'_>) -> Result<Self, DecodeError> {
        Err(DecodeError::Custom("from_wire must not be called"))
    }
    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        String::from_utf8(drain(payload))
            .map(Self)
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}

#[test]
fn overrides_receive_exactly_one_field_from_contiguous_input() {
    let mut input = FIELD_THEN_NEXT.as_slice();
    let value = decode_bytes_to::<Override<Vec<u8>>>(&mut input).unwrap();
    assert_eq!(&*value, b"abc");
    assert_eq!(input, [0xff]);

    let mut input = FIELD_THEN_NEXT.as_slice();
    let value = decode_string_to::<Override<String>>(&mut input).unwrap();
    assert_eq!(&*value, "abc");
    assert_eq!(input, [0xff]);
}

#[test]
fn overrides_receive_exactly_one_field_at_every_chunk_boundary() {
    for split in 1..FIELD_THEN_NEXT.len() {
        let (head, tail) = FIELD_THEN_NEXT.split_at(split);

        let mut input = head.chain(tail);
        let value = decode_bytes_to::<Override<Vec<u8>>>(&mut input).unwrap();
        assert_eq!(&*value, b"abc", "split {split}");
        assert_eq!(input.remaining(), 1, "split {split}");
        assert_eq!(input.get_u8(), 0xff, "split {split}");

        let mut input = head.chain(tail);
        let value = decode_string_to::<Override<String>>(&mut input).unwrap();
        assert_eq!(&*value, "abc", "split {split}");
        assert_eq!(input.remaining(), 1, "split {split}");
        assert_eq!(input.get_u8(), 0xff, "split {split}");
    }
}

/// An override that reads only the first byte of the field.
#[derive(Clone, Debug, Default, PartialEq)]
struct FirstByte(Vec<u8>);
impl From<Vec<u8>> for FirstByte {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for FirstByte {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<[u8]> for FirstByte {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl ProtoBytes for FirstByte {
    fn from_wire(_: WirePayload<'_>) -> Result<Self, DecodeError> {
        Err(DecodeError::Custom("from_wire must not be called"))
    }
    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        if !payload.has_remaining() {
            return Ok(Self::default());
        }
        Ok(Self(vec![payload.get_u8()]))
    }
}

#[test]
fn bytes_an_override_leaves_unread_are_skipped() {
    let mut input = FIELD_THEN_NEXT.as_slice();
    let value = decode_bytes_to::<FirstByte>(&mut input).unwrap();
    assert_eq!(&*value, b"a");
    assert_eq!(input, [0xff]);

    let (head, tail) = FIELD_THEN_NEXT.split_at(2);
    let mut input = head.chain(tail);
    let value = decode_bytes_to::<FirstByte>(&mut input).unwrap();
    assert_eq!(&*value, b"a");
    assert_eq!(input.remaining(), 1);
    assert_eq!(input.get_u8(), 0xff);
}

#[test]
fn empty_fields_reach_overrides_with_an_empty_payload() {
    let mut input = &[0, 0xff][..];
    let value = decode_bytes_to::<Override<Vec<u8>>>(&mut input).unwrap();
    assert!(value.is_empty());
    assert_eq!(input, [0xff]);

    let mut input = &[0, 0xff][..];
    let value = decode_string_to::<Override<String>>(&mut input).unwrap();
    assert!(value.is_empty());
    assert_eq!(input, [0xff]);
}

#[test]
fn length_prefix_errors_are_reported_before_overrides_run() {
    // Declared length 5, only one byte present.
    assert_eq!(
        decode_bytes_to::<Override<Vec<u8>>>(&mut &[5, b'x'][..]),
        Err(DecodeError::UnexpectedEof)
    );
    assert_eq!(
        decode_string_to::<Override<String>>(&mut &[5, b'x'][..]),
        Err(DecodeError::UnexpectedEof)
    );
    // Unterminated varint length prefix.
    assert_eq!(
        decode_bytes_to::<Override<Vec<u8>>>(&mut &[0xff, 0xff][..]),
        Err(DecodeError::UnexpectedEof)
    );
    // Eleven continuation bytes exceed the varint limit.
    let overlong = [0xff; 11];
    assert_eq!(
        decode_bytes_to::<Override<Vec<u8>>>(&mut &overlong[..]),
        Err(DecodeError::VarintTooLong)
    );
}

#[test]
fn override_errors_propagate() {
    assert_eq!(
        decode_string_to::<Override<String>>(&mut &[1, 0xff][..]),
        Err(DecodeError::InvalidUtf8)
    );
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ViaDefault<const OWNED: bool>(Vec<u8>);
impl<const OWNED: bool> From<Vec<u8>> for ViaDefault<OWNED> {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
impl<const OWNED: bool> core::ops::Deref for ViaDefault<OWNED> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl<const OWNED: bool> AsRef<[u8]> for ViaDefault<OWNED> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl<const OWNED: bool> ProtoBytes for ViaDefault<OWNED> {
    const PREFERS_OWNED_PAYLOAD: bool = OWNED;
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        if payload.as_slice() == b"bad" {
            return Err(DecodeError::Custom("rejected"));
        }
        Ok(Self(payload.as_slice().to_vec()))
    }
}

#[test]
fn default_hook_reads_only_the_bounded_payload() {
    let mut input = FIELD_THEN_NEXT.as_slice();
    let mut payload = FieldPayload::new(&mut input, 2);
    let value = ViaDefault::<false>::decode_length_delimited(&mut payload).unwrap();
    assert_eq!(&*value, &FIELD_THEN_NEXT[..2]);
    assert_eq!(payload.remaining(), 0);
    assert_eq!(input, &FIELD_THEN_NEXT[2..]);

    let mut input = FIELD_THEN_NEXT.as_slice();
    let mut payload = FieldPayload::new(&mut input, 2);
    let value = ViaDefault::<true>::decode_length_delimited(&mut payload).unwrap();
    assert_eq!(&*value, &FIELD_THEN_NEXT[..2]);
    assert_eq!(input, &FIELD_THEN_NEXT[2..]);
}

#[test]
fn nested_take_cannot_widen_the_opaque_field_window() {
    for split in 0..=FIELD_THEN_NEXT.len() {
        let (head, tail) = FIELD_THEN_NEXT.split_at(split);
        let mut input = head.chain(tail);
        assert_eq!(read_payload_len(&mut input).unwrap(), 3);
        let mut payload = FieldPayload::new(&mut input, 3);
        // Even the escape hatches of a newly created Take can only reach
        // the opaque window, never the next field's input buffer.
        let mut outer = (&mut payload).take(usize::MAX);
        outer.set_limit(usize::MAX);
        let bounded = outer.get_mut();
        assert_eq!(bounded.copy_to_bytes(bounded.remaining()).as_ref(), b"abc");
        assert!(!bounded.has_remaining());
        assert_eq!(input.remaining(), 1);
        assert_eq!(input.get_u8(), 0xff);
    }
}

#[test]
fn default_hook_propagates_from_wire_errors() {
    let wire = [3, b'b', b'a', b'd'];
    assert_eq!(
        decode_bytes_to::<ViaDefault<false>>(&mut &wire[..]),
        Err(DecodeError::Custom("rejected"))
    );
    assert_eq!(
        decode_bytes_to::<ViaDefault<true>>(&mut &wire[..]),
        Err(DecodeError::Custom("rejected"))
    );
    let (head, tail) = wire.split_at(2);
    assert_eq!(
        decode_bytes_to::<ViaDefault<false>>(&mut head.chain(tail)),
        Err(DecodeError::Custom("rejected"))
    );
}

/// A second decoding strategy may reject without rejecting this representation.
/// Recovering here must retain the same value as `from_wire`, including when
/// the first strategy consumed the payload before it rejected it.
#[derive(Clone, Debug, Default, PartialEq)]
struct RecoverDefault<const OWNED: bool, const PREFIX: bool>(Vec<u8>);

impl<const O: bool, const P: bool> From<Vec<u8>> for RecoverDefault<O, P> {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
impl<const O: bool, const P: bool> core::ops::Deref for RecoverDefault<O, P> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl<const O: bool, const P: bool> AsRef<[u8]> for RecoverDefault<O, P> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl<const O: bool, const P: bool> ProtoBytes for RecoverDefault<O, P> {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(Self(payload.as_slice().to_vec()))
    }

    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        let mut value = Vec::new();
        if P && payload.has_remaining() {
            value.push(payload.get_u8());
        }
        match ViaDefault::<O>::decode_length_delimited(payload) {
            Ok(decoded) => value.extend_from_slice(&decoded),
            Err(error) => {
                assert_eq!(error, DecodeError::Custom("rejected"));
                assert_rejected_window(payload);
                // Only the exact payload `bad` is rejected by ViaDefault.
                // A borrowed rejection leaves it unread; an owned rejection
                // has consumed it. A retry must preserve either state.
                if payload.has_remaining() {
                    assert_eq!(
                        ViaDefault::<O>::decode_length_delimited(payload),
                        Err(DecodeError::Custom("rejected"))
                    );
                } else {
                    assert!(ViaDefault::<O>::decode_length_delimited(payload)?.is_empty());
                }
                assert_rejected_window(payload);
                value.extend_from_slice(b"bad");
            }
        }
        // Deliberately leave a borrowed rejection unread: the caller must
        // skip exactly those bytes after this successful fallback.
        Ok(Self(value))
    }
}

fn assert_rejected_window(payload: &FieldPayload<impl Buf>) {
    match payload.remaining() {
        0 => assert!(payload.chunk().is_empty()),
        3 => assert_eq!(payload.chunk(), b"bad"),
        remaining => panic!("rejected field exposes {remaining} bytes"),
    }
    #[cfg(feature = "std")]
    {
        let mut slices = [std::io::IoSlice::new(&[]); 4];
        let count = payload.chunks_vectored(&mut slices);
        let visible: Vec<u8> = slices[..count]
            .iter()
            .flat_map(|slice| slice.iter().copied())
            .collect();
        assert_eq!(visible.as_slice(), payload.chunk());
    }
}

#[test]
fn recovering_hooks_keep_following_fields_and_frames_intact() {
    fn check<const O: bool, const P: bool>() {
        for contents in [b"bad".as_slice(), b"xbad", b"good", b""] {
            let mut wire = vec![u8::try_from(contents.len()).unwrap()];
            wire.extend_from_slice(contents);
            wire.extend_from_slice(&[0x10, 7]);
            for split in 0..=wire.len() {
                let (head, tail) = wire.split_at(split);
                let mut input = head.chain(tail);
                let value = decode_bytes_to_ctx::<RecoverDefault<O, P>>(
                    &mut input,
                    crate::test_ctx(crate::RECURSION_LIMIT).with_message_end(2),
                )
                .unwrap();
                assert_eq!(&*value, contents, "owned {O}, prefix {P}, split {split}");
                assert_eq!(input.copy_to_bytes(input.remaining()).as_ref(), [0x10, 7]);
            }
        }
    }
    check::<false, false>();
    check::<true, false>();
    check::<false, true>();
    check::<true, true>();
}

#[test]
fn rejected_default_conversions_remain_bounded_when_the_error_is_propagated() {
    let wire = [b'b', b'a', b'd', 0x10, 7];
    for split in 0..=wire.len() {
        for owned in [false, true] {
            let (head, tail) = wire.split_at(split);
            let mut input = head.chain(tail);
            let mut payload = FieldPayload::new(&mut input, 3);
            let result = if owned {
                ViaDefault::<true>::decode_length_delimited(&mut payload).map(|_| ())
            } else {
                ViaDefault::<false>::decode_length_delimited(&mut payload).map(|_| ())
            };
            assert_eq!(result, Err(DecodeError::Custom("rejected")));
            assert_rejected_window(&payload);
            let unread = drain(&mut payload);
            assert!(unread.is_empty() || unread == b"bad");
            assert_eq!(input.copy_to_bytes(input.remaining()).as_ref(), [0x10, 7]);
        }
    }
    // Invalid UTF-8 follows the same borrowed/fragmented default path for
    // strings. Its error may move the cursor within the field, never beyond it.
    let wire = [0xff, 0xff, 0xff, 0x10, 7];
    for split in 0..=wire.len() {
        let (head, tail) = wire.split_at(split);
        let mut input = head.chain(tail);
        let mut payload = FieldPayload::new(&mut input, 3);
        assert_eq!(
            String::decode_length_delimited(&mut payload),
            Err(DecodeError::InvalidUtf8)
        );
        let unread = drain(&mut payload);
        assert!(unread.is_empty() || unread == [0xff; 3]);
        assert_eq!(input.copy_to_bytes(input.remaining()).as_ref(), [0x10, 7]);
    }
}

/// A representation whose override must never run.
#[derive(Clone, Debug, Default, PartialEq)]
struct Unreachable<T>(T);
impl<T> From<T> for Unreachable<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}
impl<T: core::ops::Deref> core::ops::Deref for Unreachable<T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsRef<[u8]> for Unreachable<Vec<u8>> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<str> for Unreachable<String> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl ProtoBytes for Unreachable<Vec<u8>> {
    fn from_wire(_: WirePayload<'_>) -> Result<Self, DecodeError> {
        unreachable!("from_wire must not be called")
    }
    fn decode_length_delimited(_: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        unreachable!("the override must not run")
    }
}
impl ProtoString for Unreachable<String> {
    fn from_wire(_: WirePayload<'_>) -> Result<Self, DecodeError> {
        unreachable!("from_wire must not be called")
    }
    fn decode_length_delimited(_: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        unreachable!("the override must not run")
    }
}

#[test]
fn bounded_entry_points_stop_at_the_end_of_the_enclosing_message() {
    let ctx = crate::test_ctx(crate::RECURSION_LIMIT);
    for split in 0..=FIELD_THEN_NEXT.len() {
        let (head, tail) = FIELD_THEN_NEXT.split_at(split);

        // The message holding the field ends one byte before the input does:
        // the field fits.
        let mut input = head.chain(tail);
        let value =
            decode_bytes_to_ctx::<Override<Vec<u8>>>(&mut input, ctx.with_message_end(1)).unwrap();
        assert_eq!(&*value, b"abc", "split {split}");
        assert_eq!(input.remaining(), 1, "split {split}");
        let mut input = head.chain(tail);
        let value =
            decode_string_to_ctx::<Override<String>>(&mut input, ctx.with_message_end(1)).unwrap();
        assert_eq!(&*value, "abc", "split {split}");
        assert_eq!(input.remaining(), 1, "split {split}");

        // The message ends two bytes before the input does, so the field
        // would reach past it: rejected before the override runs. A frame end
        // beyond the whole input is rejected the same way.
        for message_end in [2, 5] {
            assert_eq!(
                decode_bytes_to_ctx::<Unreachable<Vec<u8>>>(
                    &mut head.chain(tail),
                    ctx.with_message_end(message_end)
                ),
                Err(DecodeError::UnexpectedEof),
                "split {split}, frame end {message_end}"
            );
            assert_eq!(
                decode_string_to_ctx::<Unreachable<String>>(
                    &mut head.chain(tail),
                    ctx.with_message_end(message_end)
                ),
                Err(DecodeError::UnexpectedEof),
                "split {split}, frame end {message_end}"
            );
        }
    }
}

#[test]
fn a_length_prefix_cannot_cross_the_enclosing_message_even_when_it_decodes_to_zero() {
    let ctx = crate::test_ctx(crate::RECURSION_LIMIT).with_message_end(1);
    for wire in [b"\x00".as_slice(), b"\x80\x00"] {
        for split in 0..=wire.len() {
            let (head, tail) = wire.split_at(split);
            assert_eq!(
                decode_bytes_to_ctx::<Unreachable<Vec<u8>>>(&mut head.chain(tail), ctx),
                Err(DecodeError::UnexpectedEof)
            );
            assert_eq!(
                decode_string_to_ctx::<Unreachable<String>>(&mut head.chain(tail), ctx),
                Err(DecodeError::UnexpectedEof)
            );
        }
    }
}

#[test]
fn empty_payloads_at_the_enclosing_end_keep_canonical_and_noncanonical_prefixes() {
    let ctx = crate::test_ctx(crate::RECURSION_LIMIT).with_message_end(1);
    for wire in [b"\x00\xff".as_slice(), b"\x80\x00\xff"] {
        for split in 0..=wire.len() {
            let (head, tail) = wire.split_at(split);
            let mut input = head.chain(tail);
            assert!(decode_bytes_to_ctx::<Override<Vec<u8>>>(&mut input, ctx)
                .unwrap()
                .is_empty());
            assert_eq!(input.get_u8(), 0xff);
            let mut input = head.chain(tail);
            assert!(decode_string_to_ctx::<Override<String>>(&mut input, ctx)
                .unwrap()
                .is_empty());
            assert_eq!(input.get_u8(), 0xff);
        }
    }
}

/// A message with one custom string field, 1, decoded through the bounded
/// entry point as generated code does.
#[derive(Clone, Debug, Default, PartialEq)]
struct Leaf {
    s: Override<String>,
}

impl crate::DefaultInstance for Leaf {
    fn default_instance() -> &'static Self {
        static INST: crate::__private::OnceBox<Leaf> = crate::__private::OnceBox::new();
        INST.get_or_init(|| crate::alloc::boxed::Box::new(Leaf::default()))
    }
}

impl crate::Message for Leaf {
    fn compute_size(&self, _: &mut crate::SizeCache) -> u32 {
        unreachable!("only decoded")
    }
    fn write_to(&self, _: &mut crate::SizeCache, _: &mut impl EncodeSink) {
        unreachable!("only decoded")
    }
    fn merge_field(
        &mut self,
        tag: Tag,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        match tag.field_number() {
            1 => self.s = decode_string_to_ctx(buf, ctx)?,
            _ => crate::encoding::skip_field_depth(tag, buf, ctx.depth())?,
        }
        Ok(())
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[test]
fn merge_to_limit_bounds_the_fields_of_the_message() {
    use crate::Message;
    let ctx = crate::test_ctx(crate::RECURSION_LIMIT);
    // Field 1 declares three bytes; the message ends one byte before them.
    let wire = [0x0a, 3, b'a', b'b', b'c'];
    let mut leaf = Leaf::default();
    assert_eq!(
        leaf.merge_to_limit(&mut &wire[..], ctx, 1),
        Err(DecodeError::UnexpectedEof)
    );
    assert_eq!(leaf, Leaf::default());
    leaf.merge_to_limit(&mut &wire[..], ctx, 0).unwrap();
    assert_eq!(&*leaf.s, "abc");

    // `merge` decodes a buffer of its own, so a context recording the end of
    // a message in another buffer does not limit it.
    let mut leaf = Leaf::default();
    leaf.merge(&mut &wire[..], ctx.with_message_end(4)).unwrap();
    assert_eq!(&*leaf.s, "abc");
}

#[test]
fn merge_length_delimited_rejects_a_sub_message_past_the_enclosing_one() {
    use crate::Message;
    let ctx = crate::test_ctx(crate::RECURSION_LIMIT);
    // A three-byte `Leaf` holding `s = "x"`, then one byte of something else.
    let wire = [3, 0x0a, 1, b'x', 0xff];
    for split in 0..=wire.len() {
        let (head, tail) = wire.split_at(split);

        let mut leaf = Leaf::default();
        let mut input = head.chain(tail);
        leaf.merge_length_delimited(&mut input, ctx.with_message_end(1))
            .unwrap();
        assert_eq!(&*leaf.s, "x", "split {split}");
        assert_eq!(input.remaining(), 1, "split {split}");

        // The enclosing message ends two bytes before the input does, so the
        // `Leaf` reaches past it and none of it is decoded.
        let mut leaf = Leaf::default();
        assert_eq!(
            leaf.merge_length_delimited(&mut head.chain(tail), ctx.with_message_end(2)),
            Err(DecodeError::UnexpectedEof),
            "split {split}"
        );
        assert_eq!(leaf, Leaf::default(), "split {split}");
    }
}

#[test]
fn an_empty_sub_message_still_requires_its_length_prefix_within_the_enclosing_one() {
    use crate::Message;
    let ctx = crate::test_ctx(crate::RECURSION_LIMIT).with_message_end(1);
    for split in 0..=2 {
        let (head, tail) = b"\x80\x00".split_at(split);
        assert_eq!(
            Leaf::default().merge_length_delimited(&mut head.chain(tail), ctx),
            Err(DecodeError::UnexpectedEof)
        );
    }
    for split in 0..=3 {
        let (head, tail) = b"\x80\x00\xff".split_at(split);
        let mut input = head.chain(tail);
        Leaf::default()
            .merge_length_delimited(&mut input, ctx)
            .unwrap();
        assert_eq!(input.get_u8(), 0xff);
    }
}

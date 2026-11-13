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

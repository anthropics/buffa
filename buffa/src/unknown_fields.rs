//! Unknown field preservation for round-trip fidelity.

use alloc::vec::Vec;
use bytes::BufMut;

/// A collection of unknown fields encountered during decoding.
///
/// When a message is decoded with a schema that doesn't include all fields
/// present on the wire, the unknown fields are stored here so they can be
/// re-encoded without data loss.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct UnknownFields {
    fields: Vec<UnknownField>,
}

impl UnknownFields {
    /// Creates an empty set of unknown fields.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if no unknown fields have been recorded.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Returns the number of unknown fields recorded.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Appends an unknown field.
    pub fn push(&mut self, field: UnknownField) {
        self.fields.push(field);
    }

    /// Returns an iterator over the recorded unknown fields.
    pub fn iter(&self) -> core::slice::Iter<'_, UnknownField> {
        self.fields.iter()
    }

    /// Removes all recorded unknown fields.
    pub fn clear(&mut self) {
        self.fields.clear();
    }

    /// Retain only the fields for which the predicate returns `true`.
    ///
    /// Used by [`ExtensionSet::set_extension`](crate::ExtensionSet::set_extension)
    /// and [`clear_extension`](crate::ExtensionSet::clear_extension) to remove
    /// prior occurrences at a given field number.
    pub fn retain(&mut self, f: impl FnMut(&UnknownField) -> bool) {
        self.fields.retain(f);
    }

    /// Compute the encoded size of all unknown fields.
    pub fn encoded_len(&self) -> usize {
        self.fields.iter().map(|f| f.encoded_len()).sum()
    }

    /// Re-encode all unknown fields to `buf` in their original wire format.
    pub fn write_to(&self, buf: &mut impl BufMut) {
        for field in &self.fields {
            field.write_to(buf);
        }
    }

    /// Decode a concatenation of wire-format fields into [`UnknownFields`].
    ///
    /// Reads tag/data pairs until `data` is exhausted. Each field is decoded
    /// via [`decode_unknown_field`](crate::encoding::decode_unknown_field) with
    /// the full [`RECURSION_LIMIT`](crate::message::RECURSION_LIMIT) budget.
    ///
    /// Used by [`GroupCodec`](crate::extension::codecs::GroupCodec) to turn a
    /// message's encoded bytes back into the inner-field representation that
    /// [`UnknownFieldData::Group`] wraps.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`](crate::DecodeError) if `data` contains a
    /// malformed tag, truncated field, or exceeds the recursion limit.
    pub fn decode_from_slice(mut data: &[u8]) -> Result<Self, crate::DecodeError> {
        use crate::encoding::{decode_unknown_field, Tag};
        use crate::message::RECURSION_LIMIT;
        let mut out = Self::new();
        while !data.is_empty() {
            let tag = Tag::decode(&mut data)?;
            out.push(decode_unknown_field(tag, &mut data, RECURSION_LIMIT)?);
        }
        Ok(out)
    }
}

impl<'a> IntoIterator for &'a UnknownFields {
    type Item = &'a UnknownField;
    type IntoIter = core::slice::Iter<'a, UnknownField>;

    fn into_iter(self) -> Self::IntoIter {
        self.fields.iter()
    }
}

impl IntoIterator for UnknownFields {
    type Item = UnknownField;
    type IntoIter = alloc::vec::IntoIter<UnknownField>;

    fn into_iter(self) -> Self::IntoIter {
        self.fields.into_iter()
    }
}

/// A single unknown field (field number + wire data).
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct UnknownField {
    pub number: u32,
    pub data: UnknownFieldData,
}

impl UnknownField {
    /// Compute the encoded size of this field (tag + data).
    pub fn encoded_len(&self) -> usize {
        let tag_len =
            crate::encoding::varint_len(((self.number as u64) << 3) | self.data.wire_type_value());
        tag_len + self.data.encoded_len(self.number)
    }

    /// Re-encode this field (tag + data) to `buf` in its original wire format.
    pub fn write_to(&self, buf: &mut impl BufMut) {
        use crate::encoding::encode_varint;
        let tag_value = ((self.number as u64) << 3) | self.data.wire_type_value();
        encode_varint(tag_value, buf);
        match &self.data {
            UnknownFieldData::Varint(v) => encode_varint(*v, buf),
            UnknownFieldData::Fixed64(v) => buf.put_u64_le(*v),
            UnknownFieldData::Fixed32(v) => buf.put_u32_le(*v),
            UnknownFieldData::LengthDelimited(data) => {
                encode_varint(data.len() as u64, buf);
                buf.put_slice(data);
            }
            UnknownFieldData::Group(fields) => {
                fields.write_to(buf);
                // End-group tag (wire type 4, same field number).
                encode_varint(((self.number as u64) << 3) | 4, buf);
            }
        }
    }
}

/// The wire data for an unknown field.
#[derive(Clone, Debug, PartialEq, Hash)]
pub enum UnknownFieldData {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    LengthDelimited(Vec<u8>),
    Group(UnknownFields),
}

impl UnknownFieldData {
    fn wire_type_value(&self) -> u64 {
        match self {
            UnknownFieldData::Varint(_) => 0,
            UnknownFieldData::Fixed64(_) => 1,
            UnknownFieldData::LengthDelimited(_) => 2,
            UnknownFieldData::Group(_) => 3,
            UnknownFieldData::Fixed32(_) => 5,
        }
    }

    fn encoded_len(&self, field_number: u32) -> usize {
        match self {
            UnknownFieldData::Varint(v) => crate::encoding::varint_len(*v),
            UnknownFieldData::Fixed64(_) => 8,
            UnknownFieldData::Fixed32(_) => 4,
            UnknownFieldData::LengthDelimited(data) => {
                crate::encoding::varint_len(data.len() as u64) + data.len()
            }
            UnknownFieldData::Group(fields) => {
                // Group content + end-group tag (wire type 4, same field number).
                let end_tag_len = crate::encoding::varint_len((field_number as u64) << 3 | 4);
                fields.encoded_len() + end_tag_len
            }
        }
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for UnknownFields {
    fn arbitrary(_u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(UnknownFields::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::MAX_FIELD_NUMBER;

    fn varint_field(number: u32, value: u64) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Varint(value),
        }
    }

    fn fixed32_field(number: u32, value: u32) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Fixed32(value),
        }
    }

    fn fixed64_field(number: u32, value: u64) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Fixed64(value),
        }
    }

    fn ld_field(number: u32, data: Vec<u8>) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::LengthDelimited(data),
        }
    }

    fn group_field(number: u32, fields: UnknownFields) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Group(fields),
        }
    }

    #[test]
    fn test_empty_encoded_len() {
        assert_eq!(UnknownFields::new().encoded_len(), 0);
    }

    #[test]
    fn test_varint_encoded_len() {
        // field 1, value 0: tag=(1<<3)|0=8 (1 byte) + varint(0) (1 byte) = 2
        assert_eq!(varint_field(1, 0).encoded_len(), 2);
        // field 1, value 128: tag (1 byte) + varint(128) (2 bytes) = 3
        assert_eq!(varint_field(1, 128).encoded_len(), 3);
    }

    #[test]
    fn test_fixed32_encoded_len() {
        // field 1: tag=(1<<3)|5=13 (1 byte) + 4 bytes = 5
        assert_eq!(fixed32_field(1, 0xDEAD_BEEF).encoded_len(), 5);
    }

    #[test]
    fn test_fixed64_encoded_len() {
        // field 1: tag=(1<<3)|1=9 (1 byte) + 8 bytes = 9
        assert_eq!(fixed64_field(1, 0xDEAD_BEEF_CAFE_BABE).encoded_len(), 9);
    }

    #[test]
    fn test_length_delimited_encoded_len() {
        // field 1, 3-byte data: tag (1) + len_varint(3)=1 + 3 = 5
        assert_eq!(ld_field(1, vec![0xAA, 0xBB, 0xCC]).encoded_len(), 5);
        // field 1, empty: tag (1) + len_varint(0)=1 + 0 = 2
        assert_eq!(ld_field(1, vec![]).encoded_len(), 2);
        // field 1, 128-byte data: tag (1) + len_varint(128)=2 + 128 = 131
        assert_eq!(ld_field(1, vec![0u8; 128]).encoded_len(), 131);
    }

    #[test]
    fn test_group_encoded_len() {
        // field 1, empty group: start_tag (1) + end_tag (1) = 2
        assert_eq!(group_field(1, UnknownFields::new()).encoded_len(), 2);
        // field 1, group containing one varint(0) field (2 bytes):
        //   start_tag (1) + inner (2) + end_tag (1) = 4
        let mut inner = UnknownFields::new();
        inner.push(varint_field(1, 0));
        assert_eq!(group_field(1, inner).encoded_len(), 4);
    }

    #[test]
    fn test_large_field_number_two_byte_tag() {
        // field 16: tag=(16<<3)|0=128, varint_len(128)=2; varint(0)=1 → total 3
        assert_eq!(varint_field(16, 0).encoded_len(), 3);
    }

    #[test]
    fn test_multiple_fields_sum() {
        let mut fields = UnknownFields::new();
        fields.push(varint_field(1, 0)); // tag(1) + varint(0)(1) = 2
        fields.push(fixed32_field(2, 0)); // tag=(2<<3)|5=21(1) + 4 = 5
        assert_eq!(fields.encoded_len(), 7);
    }

    #[test]
    fn test_into_iter_by_ref() {
        let mut fields = UnknownFields::new();
        fields.push(varint_field(1, 0));
        fields.push(varint_field(2, 0));
        let numbers: Vec<u32> = (&fields).into_iter().map(|f| f.number).collect();
        assert_eq!(numbers, vec![1, 2]);
        // `for f in &fields` syntax works:
        let mut count = 0;
        for _ in &fields {
            count += 1;
        }
        assert_eq!(count, 2);
    }

    // ── encoded_len vs write_to consistency ──────────────────────────────
    //
    // Size/write divergence corrupts downstream encoding (compute_size
    // feeds the length prefix of length-delimited messages). These tests
    // build fields spanning the full variant matrix + boundary field
    // numbers, and assert write_to produces exactly encoded_len bytes.

    fn assert_len_matches_write(f: UnknownField) {
        let mut fields = UnknownFields::new();
        let claimed = f.encoded_len();
        fields.push(f);
        let mut buf = alloc::vec::Vec::new();
        fields.write_to(&mut buf);
        assert_eq!(
            buf.len(),
            claimed,
            "encoded_len={claimed} but wrote {} bytes for {fields:?}",
            buf.len()
        );
    }

    #[test]
    fn test_encoded_len_matches_write_varint() {
        for &num in &[1, 15, 16, 2047, 2048, MAX_FIELD_NUMBER] {
            for &val in &[0, 1, 127, 128, u32::MAX as u64, u64::MAX] {
                assert_len_matches_write(varint_field(num, val));
            }
        }
    }

    #[test]
    fn test_encoded_len_matches_write_fixed() {
        for &num in &[1, 16, MAX_FIELD_NUMBER] {
            assert_len_matches_write(fixed32_field(num, 0));
            assert_len_matches_write(fixed32_field(num, u32::MAX));
            assert_len_matches_write(fixed64_field(num, 0));
            assert_len_matches_write(fixed64_field(num, u64::MAX));
        }
    }

    #[test]
    fn test_encoded_len_matches_write_length_delimited() {
        for &num in &[1, 16, MAX_FIELD_NUMBER] {
            assert_len_matches_write(ld_field(num, alloc::vec![]));
            assert_len_matches_write(ld_field(num, alloc::vec![0xAB]));
            assert_len_matches_write(ld_field(num, alloc::vec![0; 127]));
            assert_len_matches_write(ld_field(num, alloc::vec![0; 128]));
        }
    }

    #[test]
    fn test_encoded_len_matches_write_group() {
        for &num in &[1, 16, MAX_FIELD_NUMBER] {
            // Empty group.
            assert_len_matches_write(group_field(num, UnknownFields::new()));
            // Group with mixed children.
            let mut inner = UnknownFields::new();
            inner.push(varint_field(2, 42));
            inner.push(ld_field(3, alloc::vec![1, 2, 3]));
            assert_len_matches_write(group_field(num, inner.clone()));
            // Nested group.
            let mut inner2 = UnknownFields::new();
            inner2.push(group_field(4, inner));
            assert_len_matches_write(group_field(num, inner2));
        }
    }

    #[test]
    fn test_encoded_len_matches_write_multi_field() {
        // All variants in one UnknownFields.
        let mut fields = UnknownFields::new();
        fields.push(varint_field(1, 42));
        fields.push(fixed32_field(2, 0xDEADBEEF));
        fields.push(fixed64_field(3, u64::MAX));
        fields.push(ld_field(4, alloc::vec![1, 2, 3, 4, 5]));
        let mut nested = UnknownFields::new();
        nested.push(varint_field(10, 7));
        fields.push(group_field(5, nested));
        let claimed = fields.encoded_len();
        let mut buf = alloc::vec::Vec::new();
        fields.write_to(&mut buf);
        assert_eq!(buf.len(), claimed);
    }

    // ── decode_from_slice ────────────────────────────────────────────────

    #[test]
    fn test_decode_from_slice_empty() {
        let out = UnknownFields::decode_from_slice(&[]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn test_decode_from_slice_roundtrip() {
        // Build a multi-variant field set, write, decode back, compare.
        let mut orig = UnknownFields::new();
        orig.push(varint_field(1, 42));
        orig.push(fixed32_field(2, 0xDEAD_BEEF));
        orig.push(fixed64_field(3, u64::MAX));
        orig.push(ld_field(4, vec![1, 2, 3]));
        let mut nested = UnknownFields::new();
        nested.push(varint_field(10, 7));
        orig.push(group_field(5, nested));

        let mut buf = Vec::new();
        orig.write_to(&mut buf);
        let decoded = UnknownFields::decode_from_slice(&buf).unwrap();
        assert_eq!(decoded, orig);
    }

    #[test]
    fn test_decode_from_slice_truncated() {
        // Tag 0x08 (field 1, varint) with no value — truncated.
        assert!(UnknownFields::decode_from_slice(&[0x08]).is_err());
    }
}

//! Borrowed views of packed varint fields: see [`PackedView`].
//!
//! A [`PackedView`] checks a packed payload once, counts its elements, and keeps
//! borrowing it; elements are decoded as they are iterated, without a `Vec`.
//! A decoder written by hand with [`encoding`](crate::encoding) and
//! [`types`](crate::types) can read a packed field with it:
//!
//! ```
//! use buffa::encoding::{Tag, WireType};
//! use buffa::packed_view::{PackedView, Uint32};
//! use buffa::types::borrow_bytes;
//!
//! // Field 4 of a message, `repeated uint32 geometry` (packed): 9, 2, 4.
//! let msg = [0x22, 0x03, 0x09, 0x02, 0x04];
//! let mut cur = &msg[..];
//! let tag = Tag::decode(&mut cur)?;
//! assert_eq!((tag.field_number(), tag.wire_type()), (4, WireType::LengthDelimited));
//!
//! let geometry = PackedView::<Uint32>::from_packed(borrow_bytes(&mut cur)?)?;
//! assert_eq!(geometry.len(), 3);
//! assert_eq!(geometry.iter().collect::<Vec<_>>(), [9, 2, 4]);
//! # Ok::<(), buffa::DecodeError>(())
//! ```

use alloc::vec::Vec;
use core::fmt;

use crate::encoding::decode_varint_packed;
use crate::error::DecodeError;

mod sealed {
    pub trait Sealed {}
}

/// How the elements of a packed field are encoded as varints: one of the markers in
/// this module, such as [`Uint32`]. A Rust type alone does not say this: `i32` is
/// both `int32` and `sint32`.
pub trait PackedCodec: sealed::Sealed {
    /// The element type.
    type Elem: Copy + fmt::Debug;

    /// The element a varint decodes to.
    #[doc(hidden)]
    fn from_varint(v: u64) -> Self::Elem;

    /// The varint an element encodes to.
    #[doc(hidden)]
    fn to_varint(e: Self::Elem) -> u64;
}

macro_rules! codecs {
    ($($(#[$doc:meta])* $name:ident: $elem:ty, |$v:ident| $from:expr, |$e:ident| $to:expr;)*) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy)]
        pub enum $name {}

        impl sealed::Sealed for $name {}

        impl PackedCodec for $name {
            type Elem = $elem;

            #[inline]
            fn from_varint($v: u64) -> $elem {
                $from
            }

            #[inline]
            fn to_varint($e: $elem) -> u64 {
                $to
            }
        }
    )*};
}

codecs! {
    /// `int32`.
    Int32: i32, |v| v as i32, |e| e as i64 as u64;
    /// `int64`.
    Int64: i64, |v| v as i64, |e| e as u64;
    /// `uint32`.
    Uint32: u32, |v| v as u32, |e| u64::from(e);
    /// `uint64`.
    Uint64: u64, |v| v, |e| e;
    /// `sint32`.
    Sint32: i32, |v| crate::types::zigzag_decode_i32(v as u32), |e| u64::from(crate::types::zigzag_encode_i32(e));
    /// `sint64`.
    Sint64: i64, |v| crate::types::zigzag_decode_i64(v), |e| crate::types::zigzag_encode_i64(e);
    /// `bool`.
    Bool: bool, |v| v != 0, |e| u64::from(e);
}

/// Checks a packed varint payload and counts its elements, with the errors that
/// decoding the elements one by one gives: a varint of more than ten bytes, or whose
/// tenth byte sets bits above bit 63, and a payload that ends inside a varint.
fn check_varints(payload: &[u8]) -> Result<usize, DecodeError> {
    let (mut count, mut run) = (0, 0);
    for &b in payload {
        if b >= 0x80 {
            run += 1;
            if run == 10 {
                return Err(DecodeError::VarintTooLong);
            }
        } else if run == 9 && b > 1 {
            return Err(DecodeError::VarintTooLong);
        } else {
            count += 1;
            run = 0;
        }
    }
    if run == 0 {
        Ok(count)
    } else {
        Err(DecodeError::UnexpectedEof)
    }
}

/// A packed varint field: its packed payload, checked and borrowed, whose elements
/// [`iter`](Self::iter) decodes. A field that arrives in more than one record (split,
/// or unpacked, which decoders must accept) holds decoded elements instead.
#[derive(Clone)]
pub struct PackedView<'a, C: PackedCodec> {
    repr: Repr<'a, C>,
}

#[derive(Clone)]
enum Repr<'a, C: PackedCodec> {
    /// A checked payload and its number of elements.
    Borrowed(&'a [u8], usize),
    Decoded(Vec<C::Elem>),
}

impl<'a, C: PackedCodec> PackedView<'a, C> {
    /// A view of one packed record.
    ///
    /// # Errors
    ///
    /// The error decoding the payload's elements one by one gives:
    /// [`DecodeError::VarintTooLong`] or [`DecodeError::UnexpectedEof`].
    pub fn from_packed(payload: &'a [u8]) -> Result<Self, DecodeError> {
        let mut view = Self::default();
        view.extend_packed(payload)?;
        Ok(view)
    }

    /// The number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.repr {
            Repr::Borrowed(_, len) => *len,
            Repr::Decoded(v) => v.len(),
        }
    }

    /// Whether the field has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The elements, decoded as they are iterated.
    #[must_use]
    pub fn iter(&self) -> PackedIter<'_, C> {
        match &self.repr {
            Repr::Borrowed(bytes, len) => PackedIter(Iter::Borrowed(bytes, *len)),
            Repr::Decoded(v) => PackedIter(Iter::Decoded(v.iter())),
        }
    }

    /// The elements, decoded into a `Vec`.
    #[must_use]
    pub fn to_vec(&self) -> Vec<C::Elem> {
        self.iter().collect()
    }

    /// Appends a packed record of the field. The first one is borrowed; a later one
    /// moves the field to decoded elements.
    ///
    /// # Errors
    ///
    /// As [`from_packed`](Self::from_packed); the field is then unchanged.
    pub fn extend_packed(&mut self, payload: &'a [u8]) -> Result<(), DecodeError> {
        let len = check_varints(payload)?;
        if len == 0 {
            return Ok(());
        }
        if self.is_empty() {
            self.repr = Repr::Borrowed(payload, len);
        } else {
            self.decoded()
                .extend(PackedIter::<C>(Iter::Borrowed(payload, len)));
        }
        Ok(())
    }

    /// Appends an element of the field that arrived unpacked.
    pub fn push(&mut self, elem: C::Elem) {
        self.decoded().push(elem);
    }

    fn decoded(&mut self) -> &mut Vec<C::Elem> {
        if let Repr::Borrowed(..) = self.repr {
            self.repr = Repr::Decoded(self.to_vec());
        }
        match &mut self.repr {
            Repr::Decoded(v) => v,
            Repr::Borrowed(..) => unreachable!("replaced above"),
        }
    }
}

impl<C: PackedCodec> Default for PackedView<'_, C> {
    fn default() -> Self {
        Self {
            repr: Repr::Borrowed(&[], 0),
        }
    }
}

impl<'s, C: PackedCodec> IntoIterator for &'s PackedView<'_, C> {
    type Item = C::Elem;
    type IntoIter = PackedIter<'s, C>;

    fn into_iter(self) -> PackedIter<'s, C> {
        self.iter()
    }
}

impl<C: PackedCodec> fmt::Debug for PackedView<'_, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// The elements of a [`PackedView`], decoded as they are iterated.
#[derive(Clone, Debug)]
pub struct PackedIter<'s, C: PackedCodec>(Iter<'s, C>);

#[derive(Clone, Debug)]
enum Iter<'s, C: PackedCodec> {
    /// The rest of a checked payload, and its number of elements.
    Borrowed(&'s [u8], usize),
    Decoded(core::slice::Iter<'s, C::Elem>),
}

impl<C: PackedCodec> Iterator for PackedIter<'_, C> {
    type Item = C::Elem;

    #[inline]
    fn next(&mut self) -> Option<C::Elem> {
        match &mut self.0 {
            Iter::Borrowed(_, 0) => None,
            Iter::Borrowed(bytes, remaining) => {
                *remaining -= 1;
                let v = decode_varint_packed(bytes);
                debug_assert!(
                    v.is_ok(),
                    "a checked payload holds only well-formed varints"
                );
                Some(C::from_varint(v.unwrap_or(0)))
            }
            Iter::Decoded(elems) => elems.next().copied(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match &self.0 {
            Iter::Borrowed(_, remaining) => *remaining,
            Iter::Decoded(elems) => elems.len(),
        };
        (len, Some(len))
    }
}

impl<C: PackedCodec> ExactSizeIterator for PackedIter<'_, C> {}

#[cfg(test)]
mod tests {
    use alloc::{format, vec};

    use super::*;
    use crate::encoding::encode_varint;

    fn varints(values: &[u64]) -> Vec<u8> {
        let mut out = Vec::new();
        values.iter().for_each(|&v| encode_varint(v, &mut out));
        out
    }

    #[test]
    fn check_matches_decoding_each_element() {
        // Every run of continuation bytes up to 11 long, ended by each kind of byte or not ended.
        for run in 0..12 {
            for last in [
                None,
                Some(0x00),
                Some(0x01),
                Some(0x02),
                Some(0x7f),
                Some(0x80),
            ] {
                let mut payload = vec![0x05];
                payload.extend(core::iter::repeat_n(0xff, run));
                payload.extend(last);
                let mut cur = &payload[..];
                let mut expected = Ok(0);
                while !cur.is_empty() {
                    match crate::encoding::decode_varint(&mut cur) {
                        Ok(_) => expected = expected.map(|n| n + 1),
                        Err(e) => {
                            expected = Err(e);
                            break;
                        }
                    }
                }
                assert_eq!(check_varints(&payload), expected, "{payload:02x?}");
                let view = PackedView::<Uint64>::from_packed(&payload);
                assert_eq!(view.map(|v| v.len()), expected);
            }
        }
    }

    #[test]
    fn borrows_one_record_and_decodes_elements() {
        let bytes = varints(&[1, 300, 1 << 40]);
        let mut view = PackedView::<Uint32>::from_packed(&bytes).unwrap();
        view.extend_packed(&[]).unwrap();
        // uint32 keeps the low 32 bits, as decode_uint32 does.
        assert_eq!(view.to_vec(), [1, 300, 0]);
        let mut iter = view.iter();
        iter.next();
        assert_eq!(iter.len(), 2);
        assert_eq!((&view).into_iter().sum::<u32>(), 301);
    }

    #[test]
    fn later_records_and_unpacked_elements_join() {
        let encoded = |v: &[i64]| {
            v.iter()
                .flat_map(|&e| varints(&[Sint64::to_varint(e)]))
                .collect::<Vec<_>>()
        };
        let (first, second) = (encoded(&[-1, 2]), encoded(&[i64::MIN]));
        let mut view = PackedView::<Sint64>::from_packed(&first).unwrap();
        view.extend_packed(&second).unwrap();
        view.push(70_000);
        assert_eq!(view.to_vec(), [-1, 2, i64::MIN, 70_000]);
        assert_eq!(format!("{view:?}"), "[-1, 2, -9223372036854775808, 70000]");
        // A malformed record leaves the field unchanged.
        assert_eq!(view.extend_packed(&[0x80]), Err(DecodeError::UnexpectedEof));
        assert_eq!(view.len(), 4);
    }
}

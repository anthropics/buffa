use crate::bytes::{buf::Take, Buf, Bytes};

/// A read-only window over exactly one validated length-delimited field.
///
/// Buffa constructs this buffer after checking the field's length against the
/// input. Overrides of [`super::ProtoBytes::decode_length_delimited`] and
/// [`super::ProtoString::decode_length_delimited`] can use every [`Buf`] reader,
/// but cannot widen the window or access the following field. Unread bytes are
/// skipped by the caller when an override returns successfully.
///
/// The underlying buffer and limit are deliberately private. A `Take` would
/// expose both controls, making the field boundary depend on each override.
///
/// ```compile_fail
/// use buffa::{bytes::Buf, FieldPayload};
/// fn widen(payload: &mut FieldPayload<impl Buf>) {
///     payload.set_limit(usize::MAX);
/// }
/// ```
///
/// ```compile_fail
/// use buffa::{bytes::Buf, FieldPayload};
/// fn escape(payload: &mut FieldPayload<impl Buf>) {
///     payload.get_mut().advance(1);
/// }
/// ```
pub struct FieldPayload<B> {
    pub(super) inner: Take<B>,
}

impl<B: Buf> FieldPayload<B> {
    // The caller has already validated this length against buf.remaining().
    pub(super) fn new(buf: B, length: usize) -> Self {
        Self {
            inner: buf.take(length),
        }
    }
}

impl<B: Buf> Buf for FieldPayload<B> {
    fn remaining(&self) -> usize {
        self.inner.remaining()
    }

    fn chunk(&self) -> &[u8] {
        self.inner.chunk()
    }

    #[cfg(feature = "std")]
    fn chunks_vectored<'a>(&'a self, destination: &mut [std::io::IoSlice<'a>]) -> usize {
        self.inner.chunks_vectored(destination)
    }

    fn advance(&mut self, count: usize) {
        self.inner.advance(count);
    }

    fn copy_to_bytes(&mut self, length: usize) -> Bytes {
        // Preserve Bytes' shared-allocation path, with Take enforcing the
        // boundary before forwarding to the underlying buffer.
        self.inner.copy_to_bytes(length)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    #[test]
    fn vectored_chunks_remain_inside_the_validated_window() {
        let input = b"abc".as_slice().chain(b"def".as_slice());
        let payload = FieldPayload::new(input, 4);
        let mut chunks = [std::io::IoSlice::new(&[]), std::io::IoSlice::new(&[])];
        assert_eq!(payload.chunks_vectored(&mut chunks), 2);
        assert_eq!(&*chunks[0], b"abc");
        assert_eq!(&*chunks[1], b"d");
        assert_eq!(payload.remaining(), 4);
    }
}

impl<B> core::fmt::Debug for FieldPayload<B> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("FieldPayload([REDACTED])")
    }
}

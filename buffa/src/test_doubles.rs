//! Shared test doubles for the crate's `#[cfg(test)]` modules.

use bytes::{Buf, BufMut};

use crate::error::DecodeError;
use crate::message_field::DefaultInstance;

/// Test double whose `compute_size` reports a caller-chosen value and whose
/// `write_to` writes nothing — lets over-limit encode paths be exercised
/// without materializing gigabytes. (buffa-types carries its own copy in
/// `any_ext.rs`; `#[cfg(test)]` items don't cross the crate boundary.)
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SizedMsg {
    pub(crate) reported_size: u32,
}

impl DefaultInstance for SizedMsg {
    fn default_instance() -> &'static Self {
        static INST: crate::__private::OnceBox<SizedMsg> = crate::__private::OnceBox::new();
        INST.get_or_init(|| alloc::boxed::Box::new(SizedMsg::default()))
    }
}

impl crate::Message for SizedMsg {
    fn compute_size(&self, _cache: &mut crate::SizeCache) -> u32 {
        self.reported_size
    }
    fn write_to(&self, _cache: &mut crate::SizeCache, _buf: &mut impl crate::EncodeSink) {}
    fn merge_field(
        &mut self,
        tag: crate::encoding::Tag,
        buf: &mut impl Buf,
        _ctx: crate::DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        crate::encoding::skip_field(tag, buf)?;
        Ok(())
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A sink that discards everything, for tests that push a fake-sized message
/// through the write path without allocating its declared size. It is not a
/// `BufMut`, so the encode entry points write to it through `write_to`
/// instead of the pre-sized cursor.
pub(crate) struct NullSink;

impl crate::EncodeSink for NullSink {
    fn put_u8(&mut self, _: u8) {}
    fn put_slice(&mut self, _: &[u8]) {}
    fn put_u32_le(&mut self, _: u32) {}
    fn put_u64_le(&mut self, _: u64) {}
}

/// Test double that declares `declared` bytes in `compute_size` and writes
/// `written` bytes in `write_to`, for exercising a `write_to` that disagrees
/// with its size pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Mismatched {
    pub(crate) declared: u32,
    pub(crate) written: usize,
}

impl DefaultInstance for Mismatched {
    fn default_instance() -> &'static Self {
        static INST: crate::__private::OnceBox<Mismatched> = crate::__private::OnceBox::new();
        INST.get_or_init(|| alloc::boxed::Box::new(Mismatched::default()))
    }
}

impl crate::Message for Mismatched {
    fn compute_size(&self, _cache: &mut crate::SizeCache) -> u32 {
        self.declared
    }
    fn write_to(&self, _cache: &mut crate::SizeCache, buf: &mut impl crate::EncodeSink) {
        for _ in 0..self.written {
            buf.put_u8(0x55);
        }
    }
    fn merge_field(
        &mut self,
        tag: crate::encoding::Tag,
        buf: &mut impl Buf,
        _ctx: crate::DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        crate::encoding::skip_field(tag, buf)?;
        Ok(())
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A `Vec<u8>` that counts the calls `write_contiguous` makes on it, to
/// tell the in-place path from the scratch path.
#[derive(Default)]
pub(crate) struct Probe {
    pub(crate) inner: Vec<u8>,
    pub(crate) chunk_mut_calls: usize,
    pub(crate) advance_mut_calls: usize,
    pub(crate) put_slice_calls: usize,
    /// Report at most this many spare bytes from `chunk_mut`.
    pub(crate) chunk_limit: Option<usize>,
}

// SAFETY: every method forwards to the `Vec<u8>`, except that `chunk_mut`
// may report a shorter chunk, which `BufMut` allows.
unsafe impl BufMut for Probe {
    fn remaining_mut(&self) -> usize {
        self.inner.remaining_mut()
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.advance_mut_calls += 1;
        // SAFETY: forwarded from the caller.
        unsafe { self.inner.advance_mut(cnt) }
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        self.chunk_mut_calls += 1;
        let limit = self.chunk_limit;
        let chunk = self.inner.chunk_mut();
        match limit {
            Some(limit) if limit < chunk.len() => &mut chunk[..limit],
            _ => chunk,
        }
    }

    fn put_slice(&mut self, src: &[u8]) {
        self.put_slice_calls += 1;
        self.inner.extend_from_slice(src);
    }
}

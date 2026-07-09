//! Output abstraction for message encoding.
//!
//! [`EncodeSink`] is the byte sink every encode path writes into. Its four
//! required methods are the [`BufMut`] primitives the encoders bottom out
//! in (the fixed-width signed/float writers are provided on top), plus
//! [`put_shared`](EncodeSink::put_shared) for splicing an owned [`Bytes`]
//! segment into the output without copying it.
//!
//! Every [`BufMut`] implementor is an `EncodeSink` through a blanket impl,
//! with `put_shared` copying — the contiguous behavior every existing caller
//! already has. [`Rope`] implements the trait directly (it is deliberately
//! *not* a `BufMut`) and captures large segments by reference count instead,
//! so encoding a message whose dominant content is one large `bytes` field
//! costs O(everything-but-the-payload) rather than O(payload).

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::alloc::vec::Vec;

/// Byte sink for message encoding.
///
/// The write surface is deliberately the narrow subset of [`BufMut`] that
/// the encoding primitives use ([`put_u8`](Self::put_u8),
/// [`put_slice`](Self::put_slice), the little-endian fixed-width writers),
/// plus [`put_shared`](Self::put_shared) for owned segments. Other `BufMut`
/// methods (`put_u16`, big-endian writers, `advance_mut`, …) are
/// intentionally absent — a manual `write_to` that needs them should
/// assemble into a concrete buffer first.
///
/// # Implementors
///
/// You almost never implement this trait: the blanket impl covers every
/// [`BufMut`] (`Vec<u8>`, [`BytesMut`], `&mut [u8]`, …), and [`Rope`] is the
/// built-in segmented sink. A custom implementation must append bytes in
/// call order — the encoders rely on the sink being strictly sequential.
///
/// # Contiguous callers are unaffected
///
/// `message.encode(&mut vec)` and `message.encode(&mut bytes_mut)` compile
/// and behave exactly as before this trait existed; the blanket impl's
/// `put_shared` copies, which is the pre-existing semantics.
///
/// # Method-name overlap with `BufMut`
///
/// The method names intentionally match `BufMut`'s. If both traits are in
/// scope, calling `put_u8`/`put_slice`/`put_u32_le`/`put_u64_le` on a
/// concrete `BufMut` type is ambiguous (E0034) — disambiguate with
/// `BufMut::put_slice(&mut buf, ..)` or drop the unneeded import.
pub trait EncodeSink {
    /// Whether this sink can take ownership of [`Bytes`] segments without
    /// copying ([`put_shared`](Self::put_shared) is more than a copy).
    ///
    /// Encode helpers use this to skip producing a shared handle (an atomic
    /// refcount clone) when the sink would only copy it anyway; the constant
    /// folds at monomorphization, so contiguous sinks keep the exact
    /// pre-`EncodeSink` code. `false` is always correct — it only disables
    /// the zero-copy fast path.
    const IS_SEGMENTED: bool = false;

    /// Append a single byte.
    fn put_u8(&mut self, value: u8);

    /// Append a borrowed slice.
    fn put_slice(&mut self, src: &[u8]);

    /// Append a `u32` in little-endian byte order (protobuf `fixed32`).
    fn put_u32_le(&mut self, value: u32);

    /// Append a `u64` in little-endian byte order (protobuf `fixed64`).
    fn put_u64_le(&mut self, value: u64);

    /// Append an `i32` in little-endian byte order (protobuf `sfixed32`).
    #[inline]
    fn put_i32_le(&mut self, value: i32) {
        // Bit-preserving cast: sfixed32 is the two's-complement bytes.
        #[allow(clippy::cast_sign_loss)]
        self.put_u32_le(value as u32);
    }

    /// Append an `i64` in little-endian byte order (protobuf `sfixed64`).
    #[inline]
    fn put_i64_le(&mut self, value: i64) {
        // Bit-preserving cast: sfixed64 is the two's-complement bytes.
        #[allow(clippy::cast_sign_loss)]
        self.put_u64_le(value as u64);
    }

    /// Append an `f32` in little-endian byte order (protobuf `float`).
    #[inline]
    fn put_f32_le(&mut self, value: f32) {
        self.put_u32_le(value.to_bits());
    }

    /// Append an `f64` in little-endian byte order (protobuf `double`).
    #[inline]
    fn put_f64_le(&mut self, value: f64) {
        self.put_u64_le(value.to_bits());
    }

    /// Append an owned segment.
    ///
    /// Contiguous sinks copy (the default). Segmented sinks such as
    /// [`Rope`] may take ownership of `bytes` by reference count when it is
    /// large enough to be worth a segment, making the append O(1) in the
    /// segment's length.
    #[inline]
    fn put_shared(&mut self, bytes: Bytes) {
        self.put_slice(&bytes);
    }
}

impl<T: BufMut + ?Sized> EncodeSink for T {
    #[inline]
    fn put_u8(&mut self, value: u8) {
        BufMut::put_u8(self, value);
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        BufMut::put_slice(self, src);
    }

    #[inline]
    fn put_u32_le(&mut self, value: u32) {
        BufMut::put_u32_le(self, value);
    }

    #[inline]
    fn put_u64_le(&mut self, value: u64) {
        BufMut::put_u64_le(self, value);
    }
}

/// Default minimum segment size for [`Rope`]: payloads below this are
/// copied into the small-write tail buffer rather than kept as their own
/// segment.
///
/// The trade-off is a payload-sized memcpy (tens of GiB/s) against the
/// per-segment overhead downstream: refcount bookkeeping, one more vectored
/// I/O slot, one more frame for HTTP bodies. The crossover is in the
/// single-digit KiB; 4 KiB is conservative.
pub const DEFAULT_MIN_SEGMENT: usize = 4 * 1024;

/// A segmented encode sink: an ordered sequence of [`Bytes`] segments.
///
/// Small writes (tags, varints, scalar fields, short strings) accumulate in
/// a tail buffer; large owned segments arriving via
/// [`put_shared`](EncodeSink::put_shared) — and large borrowed slices that
/// provably lie inside the optional [backing buffer](Self::with_backing) —
/// are captured by reference count instead of copied. Concatenating the
/// [`segments`](Self::into_segments) reproduces exactly the bytes a
/// contiguous sink would have received.
///
/// ```rust,ignore
/// let mut rope = Rope::new();
/// message.encode(&mut rope);
/// for segment in rope.into_segments() {
///     body.send_data(segment); // refcount handles, no payload copy
/// }
/// ```
// Deliberately NOT a `BufMut`: the blanket `impl<T: BufMut> EncodeSink for T`
// would overlap with `Rope`'s own impl (E0119) if it ever became one.
#[derive(Debug)]
pub struct Rope {
    /// Finalized segments, in output order.
    segments: Vec<Bytes>,
    /// Accumulator for writes below the segment threshold.
    tail: BytesMut,
    /// Buffer that borrowed slices may point into (view encoding); a slice
    /// inside it can be captured zero-copy via `Bytes::slice_ref`.
    backing: Option<Bytes>,
    /// Minimum length for a write to become its own segment.
    min_segment: usize,
}

impl Default for Rope {
    /// Equivalent to [`Rope::new`]. (Not derived: the derive would default
    /// `min_segment` to `0`, bypassing the clamp every constructor applies.)
    fn default() -> Self {
        Self::new()
    }
}

impl Rope {
    /// Create a rope with [`DEFAULT_MIN_SEGMENT`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_min_segment(DEFAULT_MIN_SEGMENT)
    }

    /// Create a rope with a custom minimum segment size.
    ///
    /// `min_segment = usize::MAX` never segments (every write is copied
    /// into one contiguous tail — useful for differential testing);
    /// `min_segment = 0` is clamped to 1 so empty payloads never produce
    /// empty segments.
    #[must_use]
    pub fn with_min_segment(min_segment: usize) -> Self {
        Self {
            segments: Vec::new(),
            tail: BytesMut::new(),
            backing: None,
            min_segment: min_segment.max(1),
        }
    }

    /// Attach the buffer that borrowed slices were decoded from.
    ///
    /// With a backing buffer attached, [`put_slice`](EncodeSink::put_slice)
    /// captures any large slice that lies inside it via
    /// [`Bytes::slice_ref`] — zero-copy — instead of copying. This is the
    /// view-encoding hook: a view's `&[u8]` fields borrow from the buffer
    /// the view was decoded from, so re-encoding a view through a rope
    /// backed by that buffer never copies the large fields.
    ///
    /// Pass exactly the buffer the view was decoded from. Slices outside
    /// the backing buffer (modified fields, other sources, a mismatched
    /// buffer) are copied as usual — the output stays correct, but the
    /// zero-copy capture silently does not engage. The containment check is
    /// two pointer compares.
    #[must_use]
    pub fn with_backing(mut self, backing: Bytes) -> Self {
        self.backing = Some(backing);
        self
    }

    /// Total byte length across all segments and the tail.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.iter().map(Bytes::len).sum::<usize>() + self.tail.len()
    }

    /// Whether the rope contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        // Segments are never empty: every path that pushes one requires
        // `len >= min_segment >= 1` (or a non-empty tail flush).
        self.tail.is_empty() && self.segments.is_empty()
    }

    /// Number of segments [`into_segments`](Self::into_segments) will yield
    /// (including the pending tail, if non-empty).
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len() + usize::from(!self.tail.is_empty())
    }

    /// Finish the rope, yielding its segments in output order.
    #[must_use]
    pub fn into_segments(mut self) -> Vec<Bytes> {
        self.flush_tail();
        self.segments
    }

    /// Copy the rope out into one contiguous [`Bytes`].
    ///
    /// For consumers that need contiguous output after all — this performs
    /// the copies the rope avoided, so prefer
    /// [`into_segments`](Self::into_segments) wherever the consumer can
    /// take a segment sequence. (Named distinctly from
    /// [`Buf::copy_to_bytes`], which consumes a length-bounded prefix; this
    /// is a non-consuming full copy.)
    #[must_use]
    pub fn to_contiguous_bytes(&self) -> Bytes {
        let mut out = BytesMut::with_capacity(self.len());
        for segment in &self.segments {
            BufMut::put_slice(&mut out, segment);
        }
        BufMut::put_slice(&mut out, &self.tail);
        out.freeze()
    }

    /// Move the accumulated tail into the segment list.
    fn flush_tail(&mut self) {
        if !self.tail.is_empty() {
            self.segments.push(self.tail.split().freeze());
        }
    }
}

impl EncodeSink for Rope {
    const IS_SEGMENTED: bool = true;

    #[inline]
    fn put_u8(&mut self, value: u8) {
        BufMut::put_u8(&mut self.tail, value);
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        // `min_segment >= 1` also excludes empty slices, whose dangling
        // pointers must not reach the containment check.
        if src.len() >= self.min_segment {
            if let Some(segment) = self
                .backing
                .as_ref()
                .and_then(|b| crate::view::try_slice_ref(b, src))
            {
                self.flush_tail();
                self.segments.push(segment);
                return;
            }
        }
        BufMut::put_slice(&mut self.tail, src);
    }

    #[inline]
    fn put_u32_le(&mut self, value: u32) {
        BufMut::put_u32_le(&mut self.tail, value);
    }

    #[inline]
    fn put_u64_le(&mut self, value: u64) {
        BufMut::put_u64_le(&mut self.tail, value);
    }

    #[inline]
    fn put_shared(&mut self, bytes: Bytes) {
        if bytes.len() >= self.min_segment {
            self.flush_tail();
            self.segments.push(bytes);
        } else {
            BufMut::put_slice(&mut self.tail, &bytes);
        }
    }
}

// The reborrow pattern (`&mut rope` passed where `impl EncodeSink` is
// expected) needs an explicit forwarding impl: the blanket impl only covers
// `BufMut` types, whose own `impl BufMut for &mut T` supplies this for
// contiguous sinks. A generic `impl EncodeSink for &mut S` would overlap
// with the blanket, so `Rope` forwards concretely.
impl EncodeSink for &mut Rope {
    const IS_SEGMENTED: bool = true;

    #[inline]
    fn put_u8(&mut self, value: u8) {
        Rope::put_u8(self, value);
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        Rope::put_slice(self, src);
    }

    #[inline]
    fn put_u32_le(&mut self, value: u32) {
        Rope::put_u32_le(self, value);
    }

    #[inline]
    fn put_u64_le(&mut self, value: u64) {
        Rope::put_u64_le(self, value);
    }

    #[inline]
    fn put_shared(&mut self, bytes: Bytes) {
        Rope::put_shared(self, bytes);
    }
}

/// A [`Buf`] over a finished rope's segments, for consumers (hyper, h2,
/// vectored writers) that take any `Buf` and iterate its chunks.
#[derive(Debug)]
pub struct RopeBuf {
    /// Segments not yet fully consumed, in order; `pos` indexes the front.
    segments: Vec<Bytes>,
    pos: usize,
    remaining: usize,
}

impl From<Rope> for RopeBuf {
    fn from(rope: Rope) -> Self {
        let segments = rope.into_segments();
        let remaining = segments.iter().map(Bytes::len).sum();
        Self {
            segments,
            pos: 0,
            remaining,
        }
    }
}

impl Buf for RopeBuf {
    fn remaining(&self) -> usize {
        self.remaining
    }

    fn chunk(&self) -> &[u8] {
        self.segments.get(self.pos).map_or(&[], |b| &b[..])
    }

    fn advance(&mut self, mut cnt: usize) {
        assert!(cnt <= self.remaining, "advance past end of RopeBuf");
        self.remaining -= cnt;
        while cnt > 0 {
            let front = &mut self.segments[self.pos];
            if cnt < front.len() {
                front.advance(cnt);
                return;
            }
            cnt -= front.len();
            // Release the consumed segment's refcount now — for
            // backing-captured segments it may pin an entire wire buffer —
            // rather than holding every consumed handle until drop.
            *front = Bytes::new();
            self.pos += 1;
        }
    }

    /// Expose every remaining segment as its own I/O slice so vectored
    /// writers (h2, tokio `write_vectored`) can emit the whole rope in one
    /// syscall without copying. The trait's default would surface only the
    /// front segment per call.
    #[cfg(feature = "std")]
    fn chunks_vectored<'a>(&'a self, dst: &mut [std::io::IoSlice<'a>]) -> usize {
        let mut n = 0;
        for segment in &self.segments[self.pos..] {
            if n == dst.len() {
                break;
            }
            // `Rope` never yields empty segments, and consumed segments sit
            // behind `pos`.
            debug_assert!(!segment.is_empty());
            dst[n] = std::io::IoSlice::new(segment);
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Rope::default()` must behave exactly like `Rope::new()` — the
    /// hand-written impl exists because a derived `Default` would zero
    /// `min_segment`, bypassing the clamp and permitting empty segments.
    #[test]
    fn default_matches_new() {
        let mut rope = Rope::default();
        // A small shared write must coalesce into the tail (min_segment is
        // the 4 KiB default, not 0).
        rope.put_shared(Bytes::from_static(b"tiny"));
        rope.put_shared(Bytes::new());
        assert_eq!(rope.segment_count(), 1);
        let segments = rope.into_segments();
        assert_eq!(segments.len(), 1);
        assert!(!segments[0].is_empty());
    }

    /// `&mut Rope` is itself an `EncodeSink` (the reborrow pattern users
    /// get for free from `BufMut`'s `&mut T` impl on contiguous sinks).
    #[test]
    fn mut_ref_forwarding_impl() {
        fn write_through(mut sink: impl EncodeSink) {
            sink.put_slice(b"via reborrow");
            sink.put_shared(Bytes::from(crate::alloc::vec![9u8; 64]));
        }
        let mut rope = Rope::with_min_segment(64);
        write_through(&mut rope);
        assert_eq!(rope.segment_count(), 2);
        assert_eq!(rope.len(), 12 + 64);
    }

    /// Consuming a segment through `RopeBuf::advance` releases its
    /// refcount immediately, not at `RopeBuf` drop.
    #[test]
    fn advance_releases_consumed_segments() {
        let payload = Bytes::from(crate::alloc::vec![3u8; 128]);
        let mut rope = Rope::with_min_segment(64);
        rope.put_shared(payload.clone());
        rope.put_slice(b"after");
        let mut buf = RopeBuf::from(rope);

        // Two handles exist: `payload` and the segment inside `buf`.
        assert!(payload.clone().try_into_mut().is_err(), "not unique yet");
        buf.advance(128);
        // The consumed segment's handle is dropped; `payload` is unique.
        assert!(
            payload.try_into_mut().is_ok(),
            "consumed segment must be released before RopeBuf drop"
        );
        assert_eq!(buf.copy_to_bytes(buf.remaining()), &b"after"[..]);
    }

    /// Writes below the threshold accumulate in one tail segment.
    #[test]
    fn small_writes_coalesce() {
        let mut rope = Rope::with_min_segment(16);
        rope.put_u8(1);
        rope.put_slice(b"abc");
        rope.put_u32_le(7);
        rope.put_u64_le(9);
        assert_eq!(rope.segment_count(), 1);
        let segments = rope.into_segments();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].len(), 1 + 3 + 4 + 8);
    }

    /// A large shared segment is captured by refcount, and ordering with
    /// surrounding small writes is preserved.
    #[test]
    fn large_shared_segment_is_not_copied() {
        let payload = Bytes::from(crate::alloc::vec![0xAB; 64]);
        let ptr = payload.as_ptr();

        let mut rope = Rope::with_min_segment(64);
        rope.put_slice(b"head");
        rope.put_shared(payload);
        rope.put_slice(b"tail");

        let segments = rope.into_segments();
        assert_eq!(segments.len(), 3);
        assert_eq!(&segments[0][..], b"head");
        assert!(core::ptr::eq(segments[1].as_ptr(), ptr), "must not copy");
        assert_eq!(&segments[2][..], b"tail");
    }

    /// A small shared write folds into the tail instead of fragmenting.
    #[test]
    fn small_shared_write_coalesces() {
        let mut rope = Rope::with_min_segment(64);
        rope.put_slice(b"a");
        rope.put_shared(Bytes::from_static(b"bc"));
        rope.put_slice(b"d");
        let segments = rope.into_segments();
        assert_eq!(segments.len(), 1);
        assert_eq!(&segments[0][..], b"abcd");
    }

    /// Borrowed slices inside the backing buffer are captured zero-copy.
    #[test]
    fn backed_slice_is_captured_zero_copy() {
        let backing = Bytes::from(crate::alloc::vec![0x5A; 256]);
        let inside: &[u8] = &backing[32..224];
        let outside = crate::alloc::vec![0x5A; 192];

        let mut rope = Rope::with_min_segment(64).with_backing(backing.clone());
        rope.put_slice(inside);
        rope.put_slice(&outside);

        let segments = rope.into_segments();
        assert_eq!(segments.len(), 2);
        assert!(
            core::ptr::eq(segments[0].as_ptr(), inside.as_ptr()),
            "backed slice must be zero-copy"
        );
        assert!(
            !core::ptr::eq(segments[1].as_ptr(), outside.as_ptr()),
            "unbacked slice must be copied"
        );
        assert_eq!(&segments[1][..], &outside[..]);
    }

    /// Concatenated segments reproduce a contiguous sink's bytes exactly.
    #[test]
    fn segments_concatenate_to_contiguous_output() {
        let payload = Bytes::from(crate::alloc::vec![0x11; 128]);

        let mut contiguous: Vec<u8> = Vec::new();
        EncodeSink::put_u8(&mut contiguous, 0x0A);
        EncodeSink::put_slice(&mut contiguous, &payload);
        EncodeSink::put_u32_le(&mut contiguous, 42);

        let mut rope = Rope::with_min_segment(64);
        rope.put_u8(0x0A);
        rope.put_shared(payload);
        rope.put_u32_le(42);

        assert_eq!(&rope.to_contiguous_bytes()[..], &contiguous[..]);
    }

    /// `RopeBuf` walks the segments in order and honors partial advances.
    #[test]
    fn rope_buf_traverses_segments() {
        let mut rope = Rope::with_min_segment(4);
        rope.put_slice(b"ab");
        rope.put_shared(Bytes::from_static(b"cdefgh"));
        rope.put_slice(b"ij");
        let mut buf = RopeBuf::from(rope);

        assert_eq!(buf.remaining(), 10);
        assert_eq!(buf.chunk(), b"ab");
        buf.advance(3); // cross a segment boundary
        assert_eq!(buf.chunk(), b"defgh");
        let rest = buf.copy_to_bytes(buf.remaining());
        assert_eq!(&rest[..], b"defghij");
    }

    /// `min_segment = usize::MAX` degrades to fully contiguous output.
    #[test]
    fn max_threshold_never_segments() {
        let mut rope = Rope::with_min_segment(usize::MAX);
        rope.put_shared(Bytes::from(crate::alloc::vec![1u8; 1024]));
        rope.put_slice(&crate::alloc::vec![2u8; 1024]);
        assert_eq!(rope.segment_count(), 1);
    }
}

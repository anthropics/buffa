//! Output abstraction for message encoding.
//!
//! [`EncodeSink`] is the byte sink every encode path writes into. Its four
//! required methods are the [`BufMut`] primitives the encoders bottom out
//! in (the fixed-width signed/float writers are provided on top), plus
//! [`put_shared`](EncodeSink::put_shared) for splicing an owned [`Bytes`]
//! segment into the output without copying it.
//!
//! Every [`BufMut`] implementor is an `EncodeSink` through a blanket impl,
//! with `put_shared` copying. [`Rope`] implements the trait directly (it is deliberately
//! *not* a `BufMut`) and captures large segments by reference count instead,
//! so encoding a message whose dominant content is one large `bytes` field
//! costs O(everything-but-the-payload) rather than O(payload).
//!
//! # One `write_to` instance per message
//!
//! `Message::write_to` is generic over the sink, so a program that encodes
//! into a `Vec<u8>` in one place and a `BytesMut` in another would compile
//! two copies of every message's `write_to`. The provided encode methods of
//! [`Message`](crate::Message) and [`ViewEncode`](crate::ViewEncode)
//! (`encode`, `encode_to_vec`, and their `try_`, `_with_cache`, bounded and
//! length-delimited variants) therefore write every [`BufMut`] through one
//! shared, pre-sized cursor. They compute the exact size first and ask the
//! sink for that many contiguous bytes: `Vec<u8>` and `BytesMut` offer
//! their spare capacity, and `write_to` fills it with stores that are
//! bounds-checked but never grow the sink. A sink whose current chunk is
//! shorter than the message (an empty `Vec::new()` or `BytesMut::new()`
//! offers 64 bytes, and a chain of buffers offers its first) is filled
//! through a scratch `Vec` of the exact size and appended with one
//! [`put_slice`](EncodeSink::put_slice), so reserving
//! [`encoded_len`](crate::Message::encoded_len) bytes first avoids the
//! extra allocation and copy. The length prefix of
//! `encode_length_delimited` is written to the sink before the message, so
//! reserve up to five bytes more for it.
//!
//! A type that implements `EncodeSink` and not `BufMut`, and a segmented
//! sink ([`IS_SEGMENTED`](EncodeSink::IS_SEGMENTED), such as [`Rope`]),
//! receives every write individually through its own instantiation of
//! `write_to`.
//! Generated lazy views and `DynamicMessage` have their own encode methods,
//! which do not use the shared cursor.

use core::mem::MaybeUninit;

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
/// call order — the encoders rely on the sink being strictly sequential —
/// and receives every write individually.
///
/// # Contiguous callers
///
/// The blanket impl's `put_shared` copies. The encode entry points write a
/// `BufMut` through its `chunk_mut` and `advance_mut` rather than through
/// `put_slice` and `put_u8` (see the [module documentation](self)), with
/// two consequences:
///
/// - A `write_to` that produces more bytes than its `compute_size`
///   declared panics.
/// - A `BufMut` wrapper that observes bytes in its own `put_*` overrides,
///   such as a checksum or a tee, does not see them individually: it sees
///   `advance_mut` when the message fits its chunk and one `put_slice`
///   otherwise. Observe them in `advance_mut`, or implement `EncodeSink`
///   and not `BufMut`.
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
    /// refcount clone) when the sink would only copy it anyway, and the
    /// encode entry points write a segmented sink through its own `write_to`
    /// instantiation instead of the shared pre-sized cursor. The constant
    /// folds at monomorphization. `false` is correct for any sink that does not
    /// capture segments.
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

    /// Whether the encode entry points may write this sink through a
    /// [`PreSized`] cursor by calling
    /// [`__write_pre_sized`](Self::__write_pre_sized). The blanket `BufMut`
    /// impl sets it.
    #[doc(hidden)]
    const __PRE_SIZED: bool = false;

    /// Run `f` on this sink if it is a [`PreSized`] cursor, and return
    /// whether it did.
    ///
    /// Lets the table interpreters switch, once per message, from an instance
    /// generic over the sink to one compiled in this crate. `f` gets the
    /// cursor itself, and a `PreSized` cannot be built or altered outside this
    /// crate, so `f` cannot claim more bytes than were written:
    ///
    /// ```compile_fail,E0616
    /// use buffa::EncodeSink;
    ///
    /// let mut vec: Vec<u8> = Vec::new();
    /// vec.__with_pre_sized(&mut |cursor| cursor.pos = 4);
    /// ```
    ///
    /// Implementations outside this crate keep the default, which does not
    /// call `f`.
    #[doc(hidden)]
    #[inline]
    fn __with_pre_sized(&mut self, f: &mut dyn FnMut(&mut PreSized<'_>)) -> bool {
        let _ = f;
        false
    }

    /// Run `fill` over `len` bytes of contiguous space at the end of the
    /// sink and append the bytes it wrote, or give `fill` back unrun if the
    /// sink's current chunk is shorter than `len`.
    ///
    /// `fill` may write fewer than `len` bytes, and only those are appended.
    /// If it panics the sink is unchanged. Implementations that override
    /// this must also set [`__PRE_SIZED`](Self::__PRE_SIZED). Outside this
    /// crate, forward to another sink's `__write_pre_sized` or keep the
    /// default: a [`PreSized`] cannot be built, or read, from outside.
    #[doc(hidden)]
    #[inline]
    fn __write_pre_sized<F: FnOnce(&mut PreSized<'_>)>(
        &mut self,
        len: usize,
        fill: F,
    ) -> Result<(), F> {
        let _ = len;
        Err(fill)
    }
}

impl<T: BufMut + ?Sized> EncodeSink for T {
    const __PRE_SIZED: bool = true;

    #[inline]
    fn __write_pre_sized<F: FnOnce(&mut PreSized<'_>)>(
        &mut self,
        len: usize,
        fill: F,
    ) -> Result<(), F> {
        // An empty message asks nothing of the sink: `chunk_mut` may reserve.
        let spare: &mut [MaybeUninit<u8>] = if len == 0 {
            &mut []
        } else {
            let chunk = BufMut::chunk_mut(self);
            if chunk.len() < len {
                return Err(fill);
            }
            // SAFETY: `chunk` is writable memory of at least `len` bytes, and
            // `MaybeUninit<u8>` has the layout of `u8`. `fill` cannot reach
            // `self`, so the slice is the only access to those bytes until
            // `advance_mut`, and it is only ever written through.
            unsafe {
                core::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<MaybeUninit<u8>>(), len)
            }
        };
        let mut cursor = PreSized::new(spare);
        fill(&mut cursor);
        let written = cursor.written();
        // SAFETY: the cursor initialised its first `written` bytes, all inside
        // `chunk`, and counts only bytes it wrote itself.
        unsafe { BufMut::advance_mut(self, written) };
        Ok(())
    }

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

/// A cursor over a fixed-size, possibly uninitialised buffer, sized exactly
/// by a preceding `compute_size` pass.
///
/// Each write is a bounds check against the end of the buffer followed by a
/// store, with no call to grow. A write past the end means `write_to`
/// produced more bytes than `compute_size` declared, which is a bug in a
/// manual implementation: the cursor panics rather than write out of
/// bounds. It counts only the bytes it has written, so the first `written`
/// bytes of the buffer are initialised.
#[doc(hidden)]
pub struct PreSized<'a> {
    dst: &'a mut [MaybeUninit<u8>],
    pos: usize,
}

#[cold]
#[inline(never)]
fn pre_sized_overflow() -> ! {
    panic!("write_to produced more bytes than compute_size declared (two-pass traversal mismatch)")
}

impl<'a> PreSized<'a> {
    /// A cursor at the start of `dst`.
    ///
    /// Not public: [`EncodeSink::__write_pre_sized`] trusts the count of a
    /// cursor it built itself, which is only sound if code outside this crate
    /// cannot build a different cursor and swap it in.
    #[inline]
    pub(crate) fn new(dst: &'a mut [MaybeUninit<u8>]) -> Self {
        Self { dst, pos: 0 }
    }

    /// The number of bytes written so far.
    #[inline]
    pub(crate) const fn written(&self) -> usize {
        self.pos
    }
}

impl EncodeSink for PreSized<'_> {
    #[inline]
    fn __with_pre_sized(&mut self, f: &mut dyn FnMut(&mut PreSized<'_>)) -> bool {
        f(self);
        true
    }

    #[inline]
    fn put_u8(&mut self, value: u8) {
        let Some(slot) = self.dst.get_mut(self.pos) else {
            pre_sized_overflow()
        };
        slot.write(value);
        self.pos += 1;
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        let Some(dst) = self
            .dst
            .get_mut(self.pos..)
            .and_then(|rest| rest.get_mut(..src.len()))
        else {
            pre_sized_overflow()
        };
        // SAFETY: `dst` is `src.len()` writable bytes, and `src` cannot
        // overlap it because `dst` is behind an exclusive borrow.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr().cast::<u8>(), src.len());
        }
        self.pos += src.len();
    }

    #[inline]
    fn put_u32_le(&mut self, value: u32) {
        self.put_slice(&value.to_le_bytes());
    }

    #[inline]
    fn put_u64_le(&mut self, value: u64) {
        self.put_slice(&value.to_le_bytes());
    }
}

/// Write a message whose encoded size is `size` into `buf`.
///
/// `pre_sized` writes through a [`PreSized`] cursor and `per_write` writes to
/// the sink itself; exactly one runs. A `BufMut` takes `pre_sized`, so
/// however many `BufMut` types a program encodes into, one copy of each
/// message's `write_to` serves them all. Any other sink takes `per_write` and
/// receives every write individually.
///
/// `pre_sized` may write fewer than `size` bytes, in which case only the
/// bytes written are appended, and a debug build panics; writing more panics
/// (see [`PreSized`]). The cache is a parameter so that the two closures,
/// which cannot both capture it mutably, need only borrow the message.
#[inline]
pub(crate) fn write_contiguous<S: EncodeSink>(
    size: usize,
    cache: &mut crate::SizeCache,
    buf: &mut S,
    pre_sized: impl FnOnce(&mut crate::SizeCache, &mut PreSized<'_>),
    per_write: impl FnOnce(&mut crate::SizeCache, &mut S),
) {
    if S::IS_SEGMENTED || !S::__PRE_SIZED {
        per_write(cache, buf);
        return;
    }
    let fill = move |cursor: &mut PreSized<'_>| {
        pre_sized(cache, cursor);
        crate::message::debug_assert_two_pass(cursor.written(), size);
    };
    if let Err(fill) = buf.__write_pre_sized(size, fill) {
        let scratch = write_to_new_vec(size, fill);
        buf.put_slice(&scratch);
    }
}

/// Write a message whose encoded size is `size` into a new `Vec` of exactly
/// that capacity.
#[inline]
pub(crate) fn write_to_new_vec(size: usize, fill: impl FnOnce(&mut PreSized<'_>)) -> Vec<u8> {
    debug_assert!(size <= crate::MAX_MESSAGE_BYTES as usize);
    let mut vec = Vec::with_capacity(size);
    let written = {
        let mut cursor = PreSized::new(&mut vec.spare_capacity_mut()[..size]);
        fill(&mut cursor);
        cursor.written()
    };
    // SAFETY: the cursor initialised the first `written` bytes of the
    // vector's spare capacity, and `written <= size <= capacity`.
    unsafe { vec.set_len(written) };
    vec
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
        let mut out = Vec::with_capacity(self.len());
        for segment in &self.segments {
            out.extend_from_slice(segment);
        }
        out.extend_from_slice(&self.tail);
        Bytes::from(out)
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

    // The tail is written through `BytesMut::extend_from_slice` (inherent,
    // `#[inline]` since bytes 1.5) rather than its `BufMut::put_*` impls,
    // which are an out-of-line call per tag/varint byte even under fat LTO.
    #[inline]
    fn put_u8(&mut self, value: u8) {
        self.tail.extend_from_slice(&[value]);
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
        self.tail.extend_from_slice(src);
    }

    #[inline]
    fn put_u32_le(&mut self, value: u32) {
        self.tail.extend_from_slice(&value.to_le_bytes());
    }

    #[inline]
    fn put_u64_le(&mut self, value: u64) {
        self.tail.extend_from_slice(&value.to_le_bytes());
    }

    #[inline]
    fn put_shared(&mut self, bytes: Bytes) {
        if bytes.len() >= self.min_segment {
            self.flush_tail();
            self.segments.push(bytes);
        } else {
            self.tail.extend_from_slice(&bytes);
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
    use crate::test_doubles::Probe;

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
        EncodeSink::put_u64_le(&mut contiguous, 0x0102_0304_0506_0708);

        let mut rope = Rope::with_min_segment(64);
        rope.put_u8(0x0A);
        rope.put_shared(payload);
        rope.put_u32_le(42);
        rope.put_u64_le(0x0102_0304_0506_0708);

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

    // ── Pre-sized cursor and contiguous writes ─────────────────────────

    /// 1 + 3 + 4 + 8 = 16 bytes through every write primitive.
    const PAYLOAD_LEN: usize = 16;

    fn write_payload<S: EncodeSink>(sink: &mut S) {
        sink.put_u8(0xAA);
        sink.put_slice(b"abc");
        sink.put_u32_le(0x0403_0201);
        sink.put_u64_le(0x0807_0605_0403_0201);
    }

    fn payload() -> Vec<u8> {
        let mut expected = Vec::new();
        write_payload(&mut expected);
        assert_eq!(expected.len(), PAYLOAD_LEN);
        expected
    }

    /// Which closure `write_contiguous` ran.
    #[derive(Debug, PartialEq)]
    enum Ran {
        Cursor,
        Sink,
        Neither,
    }

    fn run<S: EncodeSink>(size: usize, buf: &mut S) -> Ran {
        let ran = core::cell::Cell::new(Ran::Neither);
        write_contiguous(
            size,
            &mut crate::SizeCache::new(),
            buf,
            |_, cursor| {
                ran.set(Ran::Cursor);
                write_payload(cursor);
            },
            |_, sink| {
                ran.set(Ran::Sink);
                write_payload(sink);
            },
        );
        ran.into_inner()
    }

    #[test]
    fn pre_sized_matches_vec_for_every_write() {
        let mut storage = [MaybeUninit::<u8>::uninit(); PAYLOAD_LEN];
        let mut cursor = PreSized::new(&mut storage);
        write_payload(&mut cursor);
        assert_eq!(cursor.written(), PAYLOAD_LEN);
        // SAFETY: the cursor initialised all `PAYLOAD_LEN` bytes.
        let got: Vec<u8> = storage.iter().map(|b| unsafe { b.assume_init() }).collect();
        assert_eq!(got, payload());
    }

    #[test]
    #[should_panic(expected = "more bytes than compute_size declared")]
    fn pre_sized_put_u8_past_end_panics() {
        let mut storage = [MaybeUninit::<u8>::uninit(); 1];
        let mut cursor = PreSized::new(&mut storage);
        cursor.put_u8(1);
        cursor.put_u8(2);
    }

    #[test]
    #[should_panic(expected = "more bytes than compute_size declared")]
    fn pre_sized_put_slice_past_end_panics() {
        let mut storage = [MaybeUninit::<u8>::uninit(); 2];
        PreSized::new(&mut storage).put_slice(b"abc");
    }

    #[test]
    #[should_panic(expected = "more bytes than compute_size declared")]
    fn pre_sized_fixed_width_past_end_panics() {
        let mut storage = [MaybeUninit::<u8>::uninit(); 7];
        PreSized::new(&mut storage).put_u64_le(1);
    }

    #[test]
    fn spare_capacity_is_filled_in_place() {
        let mut sink = Probe {
            inner: Vec::with_capacity(64),
            ..Probe::default()
        };
        sink.inner.extend_from_slice(b"prefix");
        assert_eq!(run(PAYLOAD_LEN, &mut sink), Ran::Cursor);
        assert_eq!(sink.put_slice_calls, 0, "no scratch buffer");
        assert_eq!(sink.advance_mut_calls, 1);
        assert_eq!(&sink.inner[..6], b"prefix");
        assert_eq!(&sink.inner[6..], &payload()[..]);
        assert_eq!(sink.inner.capacity(), 64, "the sink did not grow");
    }

    /// A chunk shorter than the message (an empty `Vec` offers 64 bytes, a
    /// chain offers its first buffer) is filled through one `put_slice`.
    #[test]
    fn short_chunk_is_filled_through_a_scratch_buffer() {
        let mut sink = Probe {
            chunk_limit: Some(PAYLOAD_LEN - 1),
            ..Probe::default()
        };
        assert_eq!(run(PAYLOAD_LEN, &mut sink), Ran::Cursor);
        assert_eq!(sink.advance_mut_calls, 0);
        assert_eq!(sink.put_slice_calls, 1);
        assert_eq!(sink.inner, payload());
    }

    #[test]
    fn chained_buffers_receive_the_bytes_across_the_boundary() {
        let mut head = [0u8; 4];
        let mut tail = Vec::new();
        let mut sink = (&mut head[..]).chain_mut(&mut tail);
        assert_eq!(run(PAYLOAD_LEN, &mut sink), Ran::Cursor);
        let expected = payload();
        assert_eq!(head, expected[..4]);
        assert_eq!(tail, expected[4..]);
    }

    #[test]
    fn bytes_mut_and_slice_sinks_receive_the_same_bytes() {
        let expected = payload();

        let mut bytes_mut = BytesMut::with_capacity(PAYLOAD_LEN);
        assert_eq!(run(PAYLOAD_LEN, &mut bytes_mut), Ran::Cursor);
        assert_eq!(&bytes_mut[..], &expected[..]);

        let mut storage = [0u8; PAYLOAD_LEN];
        let mut slice: &mut [u8] = &mut storage;
        assert_eq!(run(PAYLOAD_LEN, &mut slice), Ran::Cursor);
        assert_eq!(
            slice.len(),
            0,
            "the slice sink was advanced past the message"
        );
        assert_eq!(&storage[..], &expected[..]);
    }

    /// The panic text belongs to `bytes`, so only the panic is asserted.
    #[test]
    #[should_panic]
    fn slice_sink_too_small_still_panics() {
        let mut storage = [0u8; PAYLOAD_LEN - 1];
        let mut slice: &mut [u8] = &mut storage;
        run(PAYLOAD_LEN, &mut slice);
    }

    #[test]
    fn empty_message_appends_nothing() {
        let mut sink = Probe::default();
        write_contiguous(
            0,
            &mut crate::SizeCache::new(),
            &mut sink,
            |_, _| {},
            |_, _| unreachable!("a `BufMut` is written through the cursor"),
        );
        assert!(sink.inner.is_empty());
        assert_eq!(sink.chunk_mut_calls, 0);
        assert_eq!(sink.put_slice_calls, 0);
    }

    /// A type that implements `EncodeSink` and not `BufMut` keeps receiving
    /// each write individually.
    #[test]
    fn direct_encode_sink_receives_every_write() {
        #[derive(Default)]
        struct Recording {
            puts: Vec<&'static str>,
            bytes: Vec<u8>,
        }
        impl EncodeSink for Recording {
            fn put_u8(&mut self, v: u8) {
                self.puts.push("u8");
                self.bytes.push(v);
            }
            fn put_slice(&mut self, src: &[u8]) {
                self.puts.push("slice");
                self.bytes.extend_from_slice(src);
            }
            fn put_u32_le(&mut self, v: u32) {
                self.puts.push("u32");
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
            fn put_u64_le(&mut self, v: u64) {
                self.puts.push("u64");
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
        }

        let mut sink = Recording::default();
        assert_eq!(run(PAYLOAD_LEN, &mut sink), Ran::Sink);
        assert_eq!(sink.puts, ["u8", "slice", "u32", "u64"]);
        assert_eq!(sink.bytes, payload());
    }

    /// Segmented sinks take the per-write path, so `Rope` still sees each write.
    #[test]
    fn rope_takes_the_sink_path() {
        let mut rope = Rope::new();
        assert_eq!(run(PAYLOAD_LEN, &mut rope), Ran::Sink);
        assert_eq!(&rope.to_contiguous_bytes()[..], &payload()[..]);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "two-pass traversal mismatch")]
    fn under_writing_is_caught_in_debug_builds() {
        write_contiguous(
            10,
            &mut crate::SizeCache::new(),
            &mut Vec::with_capacity(32),
            |_, cursor| cursor.put_slice(b"abc"),
            |_, _| unreachable!("a Vec is written through the cursor"),
        );
    }

    #[test]
    fn write_to_new_vec_is_exactly_sized() {
        let vec = write_to_new_vec(PAYLOAD_LEN, |cursor| write_payload(cursor));
        assert_eq!(vec, payload());
        assert_eq!(vec.capacity(), PAYLOAD_LEN);
    }

    #[test]
    fn under_writing_publishes_only_the_written_bytes() {
        let mut vec = Vec::with_capacity(32);
        assert!(vec
            .__write_pre_sized(10, |cursor| cursor.put_slice(b"abc"))
            .is_ok());
        assert_eq!(vec, b"abc");

        let scratch = write_to_new_vec(10, |cursor| cursor.put_slice(b"abc"));
        assert_eq!(scratch, b"abc");
        assert_eq!(scratch.capacity(), 10);
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_panic_mid_write_leaves_the_sink_unchanged() {
        let mut vec = Vec::with_capacity(32);
        vec.extend_from_slice(b"kept");
        let result = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            let _ = vec.__write_pre_sized(8, |cursor| {
                cursor.put_slice(b"abc");
                panic!("write_to failed");
            });
        }));
        assert!(result.is_err());
        assert_eq!(vec, b"kept");
    }

    #[test]
    fn limit_sink_is_filled_within_its_limit() {
        let mut sink = Vec::new().limit(PAYLOAD_LEN);
        assert_eq!(run(PAYLOAD_LEN, &mut sink), Ran::Cursor);
        assert_eq!(sink.remaining_mut(), 0);
        assert_eq!(sink.into_inner(), payload());
    }

    #[test]
    #[should_panic(expected = "more bytes than compute_size declared")]
    fn empty_message_that_writes_anyway_panics() {
        run(0, &mut Vec::<u8>::new());
    }
}

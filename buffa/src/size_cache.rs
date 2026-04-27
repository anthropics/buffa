//! External size cache for linear-time serialization.
//!
//! Protobuf's wire format requires knowing the encoded size of a sub-message
//! before writing it (for the length-delimited prefix). Without caching, each
//! nesting level recomputes all sizes below it — O(depth²) for chains,
//! exponential for branchy trees. prost has this problem.
//!
//! `SizeCache` records sub-message sizes in a `Vec<u32>` indexed by
//! pre-order DFS traversal, populated by `compute_size` and consumed in the
//! same order by `write_to`. Both passes are O(n).
//!
//! The cache is external to message structs — generated types hold no
//! serialization state, so `let Msg { a, b, .. } = m;` is not forced by
//! hidden plumbing fields. A fresh `SizeCache` is constructed inside the
//! provided `Message::encode*` / `ViewEncode::encode*` methods; manual
//! implementers thread it through their `compute_size` / `write_to`.
//!
//! # Traversal-order invariant
//!
//! `reserve`/`set` calls during `compute_size` must occur in the same
//! order as `consume_next` calls during `write_to`. Generated code guarantees
//! this by iterating fields identically in both functions and by guarding
//! both with identical presence checks (both take `&self`, so the message
//! is immutable between passes). Manual `Message` implementations must
//! uphold the same ordering.

use alloc::vec::Vec;

/// Number of nested-message sizes stored inline (no heap allocation).
///
/// `Message::encode*` constructs a fresh `SizeCache` per call, so messages
/// with ≤ `INLINE_CAP` length-delimited sub-messages encode with zero
/// allocation for the cache. 16 covers the vast majority of message shapes
/// (the official protobuf benchmark messages all fit) at 64 bytes of stack.
const INLINE_CAP: usize = 16;

/// Transient pre-order cache of nested-message sizes for the two-pass
/// serialization model (`compute_size` populates, `write_to` consumes).
///
/// `Message::encode` and friends construct and discard a `SizeCache`
/// internally — most callers never name this type. It appears in the
/// `compute_size` / `write_to` signatures so that manual `Message`
/// implementations can thread it through nested-message recursion.
///
/// Storage is a small inline `[u32; 16]` array with a `Vec<u32>` spill for
/// the (uncommon) case of more than 16 nested length-delimited sub-messages,
/// so a fresh cache is allocation-free for typical messages.
///
/// Reusable across encodes: call [`clear`](Self::clear) between uses to
/// retain the spill allocation. `SizeCache` is intentionally not `Clone`
/// — it is transient encode state, not data. Reuse via
/// [`clear()`](Self::clear).
#[derive(Debug)]
pub struct SizeCache {
    inline: [u32; INLINE_CAP],
    spill: Vec<u32>,
    len: u32,
    cursor: u32,
}

impl Default for SizeCache {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl SizeCache {
    /// Create an empty cache. No heap allocation.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inline: [0u32; INLINE_CAP],
            spill: Vec::new(),
            len: 0,
            cursor: 0,
        }
    }

    /// Clear the cache for reuse. Retains the spill allocation's capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.spill.clear();
        self.len = 0;
        self.cursor = 0;
    }

    /// Reserve a slot for a nested message's size. Call immediately before
    /// recursing into `child.compute_size(cache)`, then fill the slot with
    /// [`set`](Self::set) after the recursion returns. This reserves the slot
    /// in pre-order even though the size is known in post-order.
    ///
    /// Used by generated `compute_size` implementations.
    #[inline]
    pub fn reserve(&mut self) -> usize {
        debug_assert!(self.len < u32::MAX, "SizeCache slot count overflow");
        let idx = self.len as usize;
        if idx < INLINE_CAP {
            // Placeholder so a buggy caller that reserves-without-set reads
            // a deterministic 0, including after `clear()` reuse.
            self.inline[idx] = 0;
        } else {
            self.spill.push(0);
        }
        self.len += 1;
        idx
    }

    /// Fill a previously-reserved slot.
    ///
    /// Used by generated `compute_size` implementations.
    ///
    /// # Panics
    ///
    /// Panics if `idx` was not returned by a prior [`reserve`](Self::reserve)
    /// on this cache (i.e. `idx >= len`).
    #[inline]
    #[track_caller]
    pub fn set(&mut self, idx: usize, size: u32) {
        assert!(
            idx < self.len as usize,
            "SizeCache::set: slot {idx} not reserved (len {})",
            self.len
        );
        if idx < INLINE_CAP {
            self.inline[idx] = size;
        } else {
            self.spill[idx - INLINE_CAP] = size;
        }
    }

    /// Consume the next cached size in pre-order.
    ///
    /// Used by generated `write_to` implementations for length-delimited
    /// nested message headers.
    ///
    /// # Panics
    ///
    /// Panics if the cursor runs past the end of the cache — i.e. if
    /// `write_to` traversal diverges from `compute_size` traversal. For
    /// generated code this indicates a codegen bug; for manual `Message`
    /// implementations it indicates a traversal-order mismatch.
    #[inline]
    #[track_caller]
    pub fn consume_next(&mut self) -> u32 {
        let idx = self.cursor as usize;
        if idx >= self.len as usize {
            Self::overrun(idx, self.len);
        }
        self.cursor += 1;
        if idx < INLINE_CAP {
            self.inline[idx]
        } else {
            self.spill[idx - INLINE_CAP]
        }
    }

    #[cold]
    #[inline(never)]
    #[track_caller]
    fn overrun(idx: usize, len: u32) -> ! {
        panic!(
            "SizeCache cursor overrun: write_to consumed {} slots but \
             compute_size produced {len} (traversal-order mismatch)",
            idx + 1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_is_default() {
        let c = SizeCache::new();
        assert_eq!(c.len, 0);
        assert_eq!(c.cursor, 0);
        assert!(c.spill.is_empty());
    }

    #[test]
    fn spill_past_inline_cap_preserves_order() {
        const N: usize = INLINE_CAP * 2 + 5;
        let mut c = SizeCache::new();
        let slots: alloc::vec::Vec<usize> = (0..N).map(|_| c.reserve()).collect();
        // Fill in reverse to prove set() addresses by slot index, not push order.
        for (i, &s) in slots.iter().enumerate().rev() {
            c.set(s, i as u32 * 7);
        }
        assert_eq!(c.spill.len(), N - INLINE_CAP);
        for i in 0..N {
            assert_eq!(c.consume_next(), i as u32 * 7);
        }
    }

    #[test]
    fn boundary_at_inline_cap() {
        let mut c = SizeCache::new();
        for i in 0..INLINE_CAP {
            let s = c.reserve();
            c.set(s, i as u32);
        }
        assert!(c.spill.is_empty(), "no spill at exactly INLINE_CAP");
        let s = c.reserve();
        c.set(s, 999);
        assert_eq!(c.spill.len(), 1);
        for i in 0..INLINE_CAP {
            assert_eq!(c.consume_next(), i as u32);
        }
        assert_eq!(c.consume_next(), 999);
    }

    #[test]
    fn reserve_set_next_roundtrip() {
        let mut c = SizeCache::new();
        let s0 = c.reserve();
        let s1 = c.reserve();
        c.set(s0, 10);
        c.set(s1, 20);
        assert_eq!(c.consume_next(), 10);
        assert_eq!(c.consume_next(), 20);
    }

    #[test]
    fn preorder_reservation_with_nested_recursion() {
        // Simulates: root has children [A, B]; A has child X.
        // compute_size pre-order entry: A, X, B
        // write_to consumes in the same order.
        let mut c = SizeCache::new();

        // compute root:
        //   reserve slot for A
        let slot_a = c.reserve();
        //     compute A:
        //       reserve slot for X
        let slot_x = c.reserve();
        //         compute X: leaf, no nested messages, returns 5
        c.set(slot_x, 5);
        //       A returns 7 (includes X's 5 plus framing)
        c.set(slot_a, 7);
        //   reserve slot for B
        let slot_b = c.reserve();
        //     compute B: leaf, returns 3
        c.set(slot_b, 3);

        // write_to root consumes A, X, B in pre-order:
        assert_eq!(c.consume_next(), 7); // A's length prefix
        assert_eq!(c.consume_next(), 5); // X's length prefix (inside A.write_to)
        assert_eq!(c.consume_next(), 3); // B's length prefix
    }

    #[test]
    fn clear_resets_and_retains_capacity() {
        let mut c = SizeCache::new();
        for _ in 0..(INLINE_CAP + 4) {
            c.reserve();
        }
        let cap = c.spill.capacity();
        assert!(cap >= 4);
        c.clear();
        assert_eq!(c.len, 0);
        assert_eq!(c.cursor, 0);
        assert!(c.spill.capacity() >= cap);
        // Reusable after clear:
        let s = c.reserve();
        c.set(s, 99);
        assert_eq!(c.consume_next(), 99);
    }

    #[test]
    fn reserve_without_set_yields_zero() {
        let mut c = SizeCache::new();
        let _ = c.reserve();
        assert_eq!(c.consume_next(), 0);
    }

    #[test]
    fn clear_then_reserve_without_set_yields_zero() {
        let mut c = SizeCache::new();
        for i in 0..(INLINE_CAP + 3) {
            let s = c.reserve();
            c.set(s, (i + 100) as u32);
        }
        c.clear();
        // After clear, a fresh reserve() must overwrite stale inline data.
        let _ = c.reserve();
        assert_eq!(c.consume_next(), 0);
    }

    #[test]
    #[should_panic(expected = "SizeCache cursor overrun")]
    fn next_past_end_panics() {
        let mut c = SizeCache::new();
        c.consume_next();
    }
}

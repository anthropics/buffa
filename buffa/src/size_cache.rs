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
//! provided `Message::encode*` methods; manual implementers of `Message`
//! thread it through their `compute_size` / `write_to`.
//!
//! # Traversal-order invariant
//!
//! `reserve`/`set` calls during `compute_size` must occur in the same
//! order as `next` calls during `write_to`. Generated code guarantees this
//! by iterating fields identically in both functions and by guarding both
//! with identical presence checks (both take `&self`, so the message is
//! immutable between passes). Manual `Message` implementations must uphold
//! the same ordering.

use alloc::vec::Vec;

/// Transient pre-order cache of nested-message sizes for the two-pass
/// serialization model (`compute_size` populates, `write_to` consumes).
///
/// `Message::encode` and friends construct and discard a `SizeCache`
/// internally — most callers never name this type. It appears in the
/// `compute_size` / `write_to` signatures so that manual `Message`
/// implementations can thread it through nested-message recursion.
///
/// Reusable across encodes: call [`clear`](Self::clear) between uses to
/// retain the allocation.
#[derive(Debug, Default)]
pub struct SizeCache {
    sizes: Vec<u32>,
    cursor: usize,
}

impl SizeCache {
    /// Create an empty cache.
    #[inline]
    pub const fn new() -> Self {
        Self {
            sizes: Vec::new(),
            cursor: 0,
        }
    }

    /// Clear the cache for reuse. Retains the allocated capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.sizes.clear();
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
        let idx = self.sizes.len();
        self.sizes.push(0);
        idx
    }

    /// Fill a previously-reserved slot.
    ///
    /// Used by generated `compute_size` implementations.
    #[inline]
    pub fn set(&mut self, idx: usize, size: u32) {
        self.sizes[idx] = size;
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
    pub fn next_size(&mut self) -> u32 {
        let size = self.sizes[self.cursor];
        self.cursor += 1;
        size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_is_default() {
        let c = SizeCache::new();
        assert_eq!(c.sizes.len(), 0);
        assert_eq!(c.cursor, 0);
    }

    #[test]
    fn reserve_set_next_roundtrip() {
        let mut c = SizeCache::new();
        let s0 = c.reserve();
        let s1 = c.reserve();
        c.set(s0, 10);
        c.set(s1, 20);
        assert_eq!(c.next_size(), 10);
        assert_eq!(c.next_size(), 20);
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
        assert_eq!(c.next_size(), 7); // A's length prefix
        assert_eq!(c.next_size(), 5); // X's length prefix (inside A.write_to)
        assert_eq!(c.next_size(), 3); // B's length prefix
    }

    #[test]
    fn clear_resets_and_retains_capacity() {
        let mut c = SizeCache::new();
        c.reserve();
        c.set(0, 42);
        let cap = c.sizes.capacity();
        c.clear();
        assert_eq!(c.sizes.len(), 0);
        assert_eq!(c.cursor, 0);
        assert!(c.sizes.capacity() >= cap);
        // Reusable after clear:
        let s = c.reserve();
        c.set(s, 99);
        assert_eq!(c.next_size(), 99);
    }

    #[test]
    #[should_panic]
    fn next_past_end_panics() {
        let mut c = SizeCache::new();
        c.next_size();
    }
}

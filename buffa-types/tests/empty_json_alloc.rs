//! Ignoring an unescaped object key must not copy its potentially large text.
#![cfg(all(feature = "json", feature = "reflect"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

const KEY_BYTES: usize = 256 * 1024;
static TRACK: AtomicBool = AtomicBool::new(false);
static LARGE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
struct CountingAllocator;
fn record(size: usize, pointer: *mut u8) {
    if !pointer.is_null() && size >= KEY_BYTES && TRACK.load(Ordering::Relaxed) {
        LARGE_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}
// SAFETY: Every operation forwards the original pointer/layout to System;
// tracking reads only sizes and never accesses allocation contents.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller provides the allocator's required valid layout.
        let pointer = unsafe { System.alloc(layout) };
        record(layout.size(), pointer);
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Layout and zero-initialization obligations are unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record(layout.size(), pointer);
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: System owns this pointer and receives its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller provides the original allocation and valid size.
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        record(size, pointer);
        pointer
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn counted<T>(run: impl FnOnce() -> T) -> (T, usize) {
    LARGE_ALLOCATIONS.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    let value = run();
    TRACK.store(false, Ordering::Relaxed);
    (value, LARGE_ALLOCATIONS.load(Ordering::Relaxed))
}

#[test]
fn empty_ignores_large_keys_without_retaining_their_text() {
    use buffa_descriptor::{DescriptorPool, DynamicMessageSeed};
    let input = format!("{{\"{}\":null}}", "k".repeat(KEY_BYTES));
    let pool = Arc::new(
        DescriptorPool::decode(include_bytes!(
            "../../buffa-descriptor/tests/protos/json_wrong_kind_test.fds"
        ))
        .unwrap(),
    );
    let index = pool.message_index("google.protobuf.Empty").unwrap();
    let (typed, typed_allocations) =
        counted(|| serde_json::from_str::<buffa_types::google::protobuf::Empty>(&input));
    typed.unwrap();
    let (reflected, reflected_allocations) = counted(|| {
        DynamicMessageSeed::new(Arc::clone(&pool), index)
            .ignore_unknown_fields(true)
            .with_element_memory_limit(0)
            .parse_json(&input)
    });
    reflected.unwrap();
    assert_eq!([typed_allocations, reflected_allocations], [0, 0]);
}

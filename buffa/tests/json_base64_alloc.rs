//! Heap allocations made by `json_helpers::bytes`.
//!
//! A counting global allocator records the size of every allocation the
//! current thread makes while recording is on. It never reads allocation
//! contents. Other test threads are unaffected, because the switch and the
//! record are thread-local.

#![cfg(all(feature = "json", feature = "std"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};

use buffa::json_helpers::bytes;

struct Counting;

const MAX_RECORDED: usize = 64;

thread_local! {
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    static SIZES: RefCell<([usize; MAX_RECORDED], usize)> =
        const { RefCell::new(([0; MAX_RECORDED], 0)) };
}

// SAFETY: every method forwards to `System` with the caller's arguments
// unchanged, so `System`'s guarantees carry over. The bookkeeping writes only
// to const-initialized thread-locals without drop glue, which on targets with
// native thread-local support (every tier-1 target) are plain statics that
// never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: forwarded unchanged; the caller upholds `alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: forwarded unchanged; the caller upholds `alloc_zeroed`'s contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        // SAFETY: forwarded unchanged; the caller upholds `realloc`'s contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged; the caller upholds `dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn record(size: usize) {
    // `try_with` and `try_borrow_mut` keep the allocator usable while a
    // thread-local is being torn down or the record is already borrowed.
    if RECORDING.try_with(Cell::get).unwrap_or(false) {
        let _ = SIZES.try_with(|sizes| {
            if let Ok(mut sizes) = sizes.try_borrow_mut() {
                let (slots, len) = &mut *sizes;
                if *len < MAX_RECORDED {
                    slots[*len] = size;
                }
                *len += 1;
            }
        });
    }
}

/// Run `f` and return the sizes of the allocations it made on this thread.
fn allocations<R>(f: impl FnOnce() -> R) -> (R, Vec<usize>) {
    SIZES.with(|sizes| sizes.borrow_mut().1 = 0);
    RECORDING.with(|on| on.set(true));
    let result = f();
    RECORDING.with(|on| on.set(false));
    let sizes = SIZES.with(|sizes| {
        let (slots, len) = *sizes.borrow();
        slots[..len.min(MAX_RECORDED)].to_vec()
    });
    (result, sizes)
}

const SECRET_LEN: usize = 3000;

fn secret() -> Vec<u8> {
    (0..SECRET_LEN).map(|i| (i * 31 + 7) as u8).collect()
}

#[test]
fn serialize_streams_base64_without_an_encoded_copy() {
    let secret = secret();
    let encoded_len = SECRET_LEN.div_ceil(3) * 4;
    let mut out = Vec::with_capacity(encoded_len + 16);
    let (result, sizes) = allocations(|| {
        let mut serializer = serde_json::Serializer::new(&mut out);
        bytes::serialize(&secret, &mut serializer)
    });
    result.unwrap();
    assert!(sizes.is_empty(), "allocations during serialize: {sizes:?}");
    use base64::Engine as _;
    let expected = format!(
        "\"{}\"",
        base64::engine::general_purpose::STANDARD.encode(&secret)
    );
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn url_safe_input_is_decoded_once() {
    use base64::Engine as _;
    let secret = secret();
    let standard = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&secret);
    let url_safe = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&secret);
    assert!(
        url_safe.contains(['-', '_']),
        "fixture must use the URL-safe symbols"
    );
    let estimate = bytes::max_decoded_len(&url_safe);

    for text in [&standard, &url_safe] {
        let json = format!("\"{text}\"");
        let (decoded, sizes) = allocations(|| {
            let mut de = serde_json::Deserializer::from_str(&json);
            bytes::deserialize::<Vec<u8>, _>(&mut de)
        });
        assert_eq!(decoded.unwrap(), secret);
        assert_eq!(sizes, [estimate], "allocations decoding {}", &text[..8]);
    }
}

#[test]
fn decode_into_does_not_allocate() {
    use base64::Engine as _;
    let secret = secret();
    let url_safe = base64::engine::general_purpose::URL_SAFE.encode(&secret);
    let mut out = vec![0u8; bytes::max_decoded_len(&url_safe)];
    let (written, sizes) =
        allocations(|| bytes::decode_into::<serde_json::Error>(&url_safe, &mut out));
    assert_eq!(&out[..written.unwrap()], secret.as_slice());
    assert!(
        sizes.is_empty(),
        "allocations during decode_into: {sizes:?}"
    );
}

//! Inspect selected allocations immediately before the system allocator frees
//! them. Reading freed pointers would be undefined behavior; observing in
//! dealloc instead verifies the actual release path, including spare capacity.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use buffa::{bytes::Buf, Message};
use buffa_test::guarded::{credentials::Credential, Credentials, DirectCredentials};
use buffa_test::guarded_repr::{SecretBytes, SecretString};

struct ObservedAllocator;

struct WatchedAllocation {
    pointer: AtomicPtr<u8>,
    length: AtomicUsize,
}
impl WatchedAllocation {
    const fn new() -> Self {
        Self {
            pointer: AtomicPtr::new(std::ptr::null_mut()),
            length: AtomicUsize::new(0),
        }
    }
}

// The allocator hook cannot allocate its own ledger. Exhaustion is a test
// failure, so increasing the number of live buffers cannot silently hide one.
const OBSERVATION_SLOTS: usize = 8;
static WATCHED: [WatchedAllocation; OBSERVATION_SLOTS] = [
    WatchedAllocation::new(),
    WatchedAllocation::new(),
    WatchedAllocation::new(),
    WatchedAllocation::new(),
    WatchedAllocation::new(),
    WatchedAllocation::new(),
    WatchedAllocation::new(),
    WatchedAllocation::new(),
];
static ALLOCATION_SIZE: AtomicUsize = AtomicUsize::new(0);
static OBSERVED: AtomicUsize = AtomicUsize::new(0);
static RELEASED: AtomicUsize = AtomicUsize::new(0);
static ALL_WIPED: AtomicBool = AtomicBool::new(true);
static OVERFLOWED: AtomicBool = AtomicBool::new(false);
static REALLOCATED: AtomicBool = AtomicBool::new(false);

// SAFETY: layouts and releases are forwarded unchanged to System. This
// observer is specific to the u8/str buffers in these fixtures; no typed values
// with padding are moved into watched blocks. Matched allocations are fully
// initialized, including spare capacity, and explicit watches cover buffers
// initialized beforehand. Reallocations involving a watched block or target
// size fail the observation without inspecting potentially uninitialized
// copied capacity. No allocation occurs in these hooks.
unsafe impl GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: this forwards the caller's valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if layout.align() == 1
            && layout.size() == ALLOCATION_SIZE.load(Ordering::SeqCst)
            && !pointer.is_null()
        {
            // SAFETY: pointer owns layout.size() bytes. Initializing them does
            // not change GlobalAlloc's contract and avoids reading undefined
            // spare capacity when this allocation is later inspected.
            unsafe { pointer.write_bytes(0, layout.size()) };
            for slot in &WATCHED {
                if slot
                    .pointer
                    .compare_exchange(
                        std::ptr::null_mut(),
                        pointer,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    // The new pointer cannot be released before alloc returns.
                    slot.length.store(layout.size(), Ordering::SeqCst);
                    OBSERVED.fetch_add(1, Ordering::SeqCst);
                    return pointer;
                }
            }
            OVERFLOWED.store(true, Ordering::SeqCst);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // System's realloc can copy uninitialized spare capacity from an
        // unobserved old block. Reject that topology instead of inspecting it.
        if layout.align() == 1 && new_size == ALLOCATION_SIZE.load(Ordering::SeqCst) {
            REALLOCATED.store(true, Ordering::SeqCst);
        }
        for slot in &WATCHED {
            if slot
                .pointer
                .compare_exchange(
                    pointer,
                    std::ptr::null_mut(),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                REALLOCATED.store(true, Ordering::SeqCst);
                break;
            }
        }
        // SAFETY: all arguments are forwarded from the allocator caller.
        unsafe { System.realloc(pointer, layout, new_size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        for slot in &WATCHED {
            if slot.pointer.load(Ordering::SeqCst) != pointer {
                continue;
            }
            // Read length before freeing the slot for reuse by an allocation
            // on another thread. The current pointer is still live here.
            let length = slot.length.load(Ordering::SeqCst).min(layout.size());
            if slot
                .pointer
                .compare_exchange(
                    pointer,
                    std::ptr::null_mut(),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                // SAFETY: the allocation is live and length <= layout.size().
                // Its observed bytes were initialized as described above.
                let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
                ALL_WIPED.fetch_and(bytes.iter().all(|byte| *byte == 0), Ordering::SeqCst);
                RELEASED.fetch_add(1, Ordering::SeqCst);
                break;
            }
        }
        // SAFETY: pointer and layout are forwarded from the allocator caller.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;
const SECRET_LENGTH: usize = 4093;

fn reset_observation() {
    ALLOCATION_SIZE.store(0, Ordering::SeqCst);
    assert!(WATCHED
        .iter()
        .all(|slot| slot.pointer.load(Ordering::SeqCst).is_null()));
    OBSERVED.store(0, Ordering::SeqCst);
    RELEASED.store(0, Ordering::SeqCst);
    ALL_WIPED.store(true, Ordering::SeqCst);
    OVERFLOWED.store(false, Ordering::SeqCst);
    REALLOCATED.store(false, Ordering::SeqCst);
}

// Every call site initialized the full capacity before truncating or wrapping
// the buffer, so the observer can inspect even its no-longer-live elements.
fn watch(pointer: *const u8, capacity: usize) {
    reset_observation();
    WATCHED[0].length.store(capacity, Ordering::SeqCst);
    WATCHED[0]
        .pointer
        .store(pointer.cast_mut(), Ordering::SeqCst);
    OBSERVED.store(1, Ordering::SeqCst);
}

fn watch_next_allocation(size: usize) {
    reset_observation();
    ALLOCATION_SIZE.store(size, Ordering::SeqCst);
}

fn finish_observation() -> bool {
    ALLOCATION_SIZE.store(0, Ordering::SeqCst);
    assert!(
        !REALLOCATED.load(Ordering::SeqCst),
        "watched buffers must not reallocate during observation"
    );
    assert!(
        !OVERFLOWED.load(Ordering::SeqCst),
        "allocation observation ledger exhausted"
    );
    assert!(
        OBSERVED.load(Ordering::SeqCst) > 0,
        "no matching allocation was observed"
    );
    assert_eq!(
        RELEASED.load(Ordering::SeqCst),
        OBSERVED.load(Ordering::SeqCst),
        "watched allocations remain live"
    );
    assert!(WATCHED
        .iter()
        .all(|slot| slot.pointer.load(Ordering::SeqCst).is_null()));
    ALL_WIPED.load(Ordering::SeqCst)
}

fn assert_wiped() {
    assert!(
        finish_observation(),
        "storage was released before zeroization"
    );
}

fn length_delimited(field: u32, payload: &[u8], wire: &mut Vec<u8>) {
    buffa::encoding::Tag::new(field, buffa::encoding::WireType::LengthDelimited).encode(wire);
    buffa::types::encode_bytes(payload, wire);
}

// Keep these observations in one test: the allocator's watch slot must never
// be shared by concurrent test cases in this integration-test executable.
#[test]
fn guarded_allocations_are_wiped_before_release_on_every_lifecycle_path() {
    // An unwiped release must stay visible even if another equal-sized
    // allocation is subsequently wiped correctly.
    watch_next_allocation(SECRET_LENGTH);
    let unprotected = vec![b'U'; SECRET_LENGTH];
    let protected = SecretBytes::from(vec![b'K'; SECRET_LENGTH]);
    std::hint::black_box((&unprotected, &protected));
    drop(unprotected);
    drop(protected);
    assert!(
        !finish_observation(),
        "an earlier unwiped release was masked"
    );
    assert_eq!(OBSERVED.load(Ordering::SeqCst), 2);

    // Reject reallocations both into and out of the watched size. A failed
    // observation must not try to read newly copied, uninitialized capacity.
    for length in [SECRET_LENGTH.checked_sub(1).unwrap(), SECRET_LENGTH] {
        watch_next_allocation(SECRET_LENGTH);
        let mut bytes = vec![b'R'; length];
        bytes.reserve_exact(1);
        std::hint::black_box(&bytes);
        drop(bytes);
        assert!(REALLOCATED.load(Ordering::SeqCst));
        reset_observation();
    }

    let mut raw = vec![b'K'; SECRET_LENGTH];
    let pointer = raw.as_ptr();
    let capacity = raw.capacity();
    raw.truncate(17);
    let bytes = SecretBytes::from(raw);
    watch(pointer, capacity);
    drop(bytes);
    assert_wiped();

    let mut raw = "S".repeat(SECRET_LENGTH);
    let pointer = raw.as_ptr();
    let capacity = raw.capacity();
    raw.truncate(17);
    let text = SecretString::from(raw);
    watch(pointer, capacity);
    drop(text);
    assert_wiped();

    let mut message = Credentials {
        key: vec![b'K'; SECRET_LENGTH].into(),
        ..Default::default()
    };
    watch(message.key.as_ptr(), SECRET_LENGTH);
    message.clear();
    assert_wiped();

    // Replacement through ordinary field assignment is protected too.
    message.key = vec![b'K'; SECRET_LENGTH].into();
    watch(message.key.as_ptr(), SECRET_LENGTH);
    message.key = b"new".to_vec().into();
    assert_wiped();

    for field in [1, 2, 3, 11] {
        let mut wire = Vec::new();
        length_delimited(field, &vec![b'K'; SECRET_LENGTH], &mut wire);
        length_delimited(field, b"replacement", &mut wire);
        watch_next_allocation(SECRET_LENGTH);
        let result = Credentials::decode_from_slice(&wire).unwrap();
        assert_wiped();
        drop(result);
    }

    let mut wire = Vec::new();
    length_delimited(11, &vec![b'K'; SECRET_LENGTH], &mut wire);
    length_delimited(12, b"different variant", &mut wire);
    watch_next_allocation(SECRET_LENGTH);
    let result = Credentials::decode_from_slice(&wire).unwrap();
    assert!(matches!(result.credential, Some(Credential::Text(_))));
    assert_wiped();

    // Invalid UTF-8 spans Buf chunks. The intermediate byte allocation must
    // be wiped even though a SecretString was never successfully constructed.
    let mut wire = Vec::new();
    length_delimited(2, &vec![0xff; SECRET_LENGTH], &mut wire);
    let (first, rest) = wire.split_at(7);
    watch_next_allocation(SECRET_LENGTH);
    assert!(Credentials::decode(&mut first.chain(rest)).is_err());
    assert_wiped();

    let mut wire = Vec::new();
    length_delimited(1, &vec![b'K'; SECRET_LENGTH], &mut wire);
    wire.push(0);
    watch_next_allocation(SECRET_LENGTH);
    assert!(Credentials::decode_from_slice(&wire).is_err());
    assert_wiped();

    let mut entry = Vec::new();
    length_delimited(1, b"same", &mut entry);
    length_delimited(2, &vec![b'K'; SECRET_LENGTH], &mut entry);
    let mut wire = Vec::new();
    length_delimited(8, &entry, &mut wire);
    entry.clear();
    length_delimited(1, b"same", &mut entry);
    length_delimited(2, b"new", &mut entry);
    length_delimited(8, &entry, &mut wire);
    watch_next_allocation(SECRET_LENGTH);
    let result = Credentials::decode_from_slice(&wire).unwrap();
    assert_wiped();
    drop(result);

    // The first JSON field succeeds; a later error drops its guarded value.
    let encoded = "S0tL".repeat(1364);
    let capacity = encoded.len();
    for json in [
        format!("{{\"key\":\"{encoded}\",\"identity\":[]}}"),
        format!("{{\"token\":\"{encoded}\",\"identity\":[]}}"),
        format!("{{\"chain\":[\"{encoded}\",\"%%%%\"]}}"),
        format!("{{\"keys\":{{\"first\":\"{encoded}\",\"second\":\"%%%%\"}}}}"),
        format!("{{\"raw\":\"{encoded}\",\"identity\":[]}}"),
    ] {
        watch_next_allocation(capacity);
        assert!(serde_json::from_str::<Credentials>(&json).is_err());
        assert_wiped();
    }

    // A message without oneofs uses serde's derived visitor instead of the
    // generated oneof visitor. Both paths must reach the guarded field type.
    for json in [
        format!("{{\"key\":\"{encoded}\",\"enabled\":[]}}"),
        format!("{{\"token\":\"{encoded}\",\"enabled\":[]}}"),
    ] {
        watch_next_allocation(capacity);
        assert!(serde_json::from_str::<DirectCredentials>(&json).is_err());
        assert_wiped();
    }

    // A malformed final base64 quantum fails after earlier output was written.
    let json = format!("{{\"key\":\"{encoded}%%%%\"}}");
    watch_next_allocation(capacity.checked_add(4).unwrap());
    assert!(serde_json::from_str::<Credentials>(&json).is_err());
    assert_wiped();
    // Native Serialize must run in every shape too: its base64 scratch string
    // is a separate allocation from the caller-owned serializer output.
    for field in [1, 3, 4, 8, 11] {
        let bytes = || SecretBytes::from(vec![b'K'; SECRET_LENGTH]);
        let mut value = Credentials::default();
        match field {
            1 => value.key = bytes(),
            3 => value.token = Some(bytes()),
            4 => value.chain.push(bytes()),
            8 => {
                value.keys.insert("name".to_owned().into(), bytes());
            }
            11 => value.credential = Some(Credential::Raw(bytes())),
            _ => unreachable!("fixed fixture fields"),
        }
        let encoded_length = SECRET_LENGTH
            .checked_add(2)
            .unwrap()
            .checked_div(3)
            .unwrap()
            .checked_mul(4)
            .unwrap();
        watch_next_allocation(encoded_length);
        let output = serde_json::to_string(&value).unwrap();
        assert_wiped();
        drop(output);
    }
    for optional in [false, true] {
        let bytes = SecretBytes::from(vec![b'K'; SECRET_LENGTH]);
        let mut value = DirectCredentials::default();
        if optional {
            value.token = Some(bytes);
        } else {
            value.key = bytes;
        }
        let encoded_length = SECRET_LENGTH
            .checked_add(2)
            .unwrap()
            .checked_div(3)
            .unwrap()
            .checked_mul(4)
            .unwrap();
        watch_next_allocation(encoded_length);
        let output = serde_json::to_string(&value).unwrap();
        assert_wiped();
        drop(output);
    }
}

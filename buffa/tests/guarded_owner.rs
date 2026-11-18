//! Wire erasure is independent of generated recursive message cleanup.
#![cfg(feature = "zeroize")]
use buffa::sensitive::{GuardedOwnedView, GuardedViewError};
use buffa::{DecodeOptions, Message};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
#[path = "support/guarded_owner.rs"]
mod sample;

struct Observer;
static WATCHED: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static LENGTH: AtomicUsize = AtomicUsize::new(0);
std::thread_local! {
    static NEXT_SIZE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
static RELEASED: AtomicBool = AtomicBool::new(false);
static WIPED: AtomicBool = AtomicBool::new(false);
// SAFETY: allocator arguments are forwarded unchanged. Registered ranges
// contain initialized wire bytes or an initialized control Vec, never object
// headers or padding. Reads happen before System releases the allocation.
unsafe impl GlobalAlloc for Observer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies the valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        let selected = layout.align() == 1
            && NEXT_SIZE
                .try_with(|next| {
                    if next.get() == layout.size() {
                        next.set(0);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
        if selected && !pointer.is_null() {
            // SAFETY: the exact wire byte block is newly allocated and untyped.
            // Initializing every byte also makes a failed-wipe regression safe
            // to observe if construction returns before filling its capacity.
            unsafe { std::ptr::write_bytes(pointer, 0x37, layout.size()) };
            LENGTH.store(layout.size(), Ordering::SeqCst);
            WATCHED.store(pointer, Ordering::SeqCst);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if WATCHED
            .compare_exchange(
                pointer,
                std::ptr::null_mut(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            let length = LENGTH.load(Ordering::SeqCst);
            let wiped = length <= layout.size() && {
                // SAFETY: registration identifies an initialized live byte prefix.
                unsafe { std::slice::from_raw_parts(pointer, length) }
                    .iter()
                    .all(|byte| *byte == 0)
            };
            WIPED.store(wiped, Ordering::SeqCst);
            RELEASED.store(true, Ordering::SeqCst);
        }
        // SAFETY: the original allocation and layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Observer = Observer;
fn watch(pointer: *const u8, length: usize) {
    LENGTH.store(length, Ordering::SeqCst);
    RELEASED.store(false, Ordering::SeqCst);
    WIPED.store(false, Ordering::SeqCst);
    WATCHED.store(pointer.cast_mut(), Ordering::SeqCst);
}
fn released(wiped: bool) {
    assert!(
        RELEASED.load(Ordering::SeqCst),
        "wire allocation was not released"
    );
    assert_eq!(
        WIPED.load(Ordering::SeqCst),
        wiped,
        "released wire storage was not wiped"
    );
}
#[test]
fn private_wire_storage_is_bounded_and_wiped_on_success_and_failure() {
    let control = vec![0x37u8; 37];
    watch(control.as_ptr(), control.len());
    drop(control);
    released(false);
    let message = sample::Packet {
        data: vec![0x37; 37],
        reported_len: None,
    };
    let wire = message.encode_to_vec();
    type Owner = GuardedOwnedView<sample::PacketView<'static>>;
    let owner = Owner::from_owned(&message, &DecodeOptions::new()).unwrap();
    assert_eq!(owner.bytes(), wire);
    assert_eq!(owner.to_owned_message().unwrap(), message);
    assert!(format!("{owner:?}").contains("REDACTED"));
    watch(owner.bytes().as_ptr(), owner.bytes().len());
    drop(owner);
    released(true);
    let owner = Owner::decode(&wire, &DecodeOptions::new()).unwrap();
    assert_ne!(owner.bytes().as_ptr(), wire.as_ptr());
    assert_eq!(owner.bytes(), wire);
    watch(owner.bytes().as_ptr(), owner.bytes().len());
    drop(owner);
    released(true);
    assert_eq!(wire, message.encode_to_vec());
    assert!(matches!(
        Owner::decode(&wire, &DecodeOptions::new().with_max_message_size(1)),
        Err(GuardedViewError::SizeLimit)
    ));
    assert!(matches!(
        Owner::from_owned(&message, &DecodeOptions::new().with_max_message_size(1)),
        Err(GuardedViewError::SizeLimit)
    ));
    assert!(Owner::decode(&[], &DecodeOptions::new())
        .unwrap()
        .bytes()
        .is_empty());
    for reported_len in [1, 99] {
        let dishonest = sample::Packet {
            data: vec![0x37; 37],
            reported_len: Some(reported_len),
        };
        assert!(matches!(
            Owner::from_owned(&dishonest, &DecodeOptions::new()),
            Err(GuardedViewError::EncodingLength)
        ));
    }
    // Invalid wire is copied into exactly one wiping allocation before the
    // eager decoder rejects its first tag. No view payload allocation occurs.
    RELEASED.store(false, Ordering::SeqCst);
    WIPED.store(false, Ordering::SeqCst);
    NEXT_SIZE.with(|next| next.set(37));
    assert!(matches!(
        Owner::decode(&[0x37; 37], &DecodeOptions::new()),
        Err(GuardedViewError::Decode(_))
    ));
    released(true);
    assert_eq!(NEXT_SIZE.with(std::cell::Cell::get), 0);
}

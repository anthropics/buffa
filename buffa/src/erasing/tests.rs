use super::*;
use alloc::{boxed::Box, rc::Rc};
use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};
static ERASED: AtomicUsize = AtomicUsize::new(0);
fn erase(value: &mut u64) {
    assert_eq!(*value, 42);
    *value = 0;
    ERASED.fetch_add(1, Ordering::SeqCst);
}
#[test]
fn cleanup_runs_on_discard_but_not_after_transfer() {
    ERASED.store(0, Ordering::SeqCst);
    drop(Erasing::new(42, erase));
    assert_eq!(ERASED.load(Ordering::SeqCst), 1);
    let value = Erasing::new(42, erase).into_inner();
    assert_eq!(value, 42);
    assert_eq!(ERASED.load(Ordering::SeqCst), 1);
}

#[test]
fn heap_ownership_is_transferred_or_destroyed_exactly_once() {
    struct OwnedSlot {
        number: Box<u64>,
        erased: Rc<Cell<usize>>,
        dropped: Rc<Cell<usize>>,
        observed: Rc<Cell<u64>>,
    }
    impl OwnedSlot {
        fn erase(&mut self) {
            *self.number = 0;
            self.erased.set(self.erased.get() + 1);
        }
    }
    impl Drop for OwnedSlot {
        fn drop(&mut self) {
            self.observed.set(*self.number);
            self.dropped.set(self.dropped.get() + 1);
        }
    }

    let erased = Rc::new(Cell::new(0));
    let dropped = Rc::new(Cell::new(0));
    let observed = Rc::new(Cell::new(0));
    let make_slot = || OwnedSlot {
        number: Box::new(37),
        erased: erased.clone(),
        dropped: dropped.clone(),
        observed: observed.clone(),
    };

    // A failed parse destroys its partial owner after cleanup, even when
    // the value owns allocations and therefore has invalid all-zero bits.
    let fail = || -> Result<(), ()> {
        let mut slot = Erasing::new(make_slot(), OwnedSlot::erase);
        *slot.number = 99;
        Err(())
    };
    assert_eq!(fail(), Err(()));
    assert_eq!(erased.get(), 1);
    assert_eq!(dropped.get(), 1);
    assert_eq!(observed.get(), 0);

    // Wiping the vacated slot must not invalidate or destroy the ownership
    // that into_inner moved out. Miri checks the Box and Rc provenance.
    let transferred = Erasing::new(make_slot(), OwnedSlot::erase).into_inner();
    assert_eq!(*transferred.number, 37);
    assert_eq!(erased.get(), 1);
    assert_eq!(dropped.get(), 1);
    drop(transferred);
    assert_eq!(erased.get(), 1);
    assert_eq!(dropped.get(), 2);
    assert_eq!(observed.get(), 37);
}

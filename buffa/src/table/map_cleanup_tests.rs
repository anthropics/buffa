use super::*;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicUsize, Ordering};

static KEYS: AtomicUsize = AtomicUsize::new(0);
static VALUES: AtomicUsize = AtomicUsize::new(0);
#[derive(Default)]
struct Map(BTreeMap<u32, u64>);
impl MapStorage for Map {
    type Key = u32;
    type Value = u64;
    fn storage_len(&self) -> usize {
        self.0.len()
    }
    fn storage_clear(&mut self) {
        self.0.clear();
    }
    fn storage_insert(&mut self, key: u32, value: u64) {
        self.0.insert(key, value);
    }
    fn storage_erase_key(key: &mut u32) {
        assert_eq!(*key, 7);
        *key = 0;
        KEYS.fetch_add(1, Ordering::SeqCst);
    }
    fn storage_erase_value(value: &mut u64) {
        assert_eq!(*value, 37);
        *value = 0;
        VALUES.fetch_add(1, Ordering::SeqCst);
    }
    fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a u32, &'a u64)>
    where
        Self::Key: 'a,
        Self::Value: 'a,
    {
        self.0.iter()
    }
}
#[test]
fn manual_table_map_slots_follow_the_destination_cleanup_policy() {
    KEYS.store(0, Ordering::SeqCst);
    VALUES.store(0, Ordering::SeqCst);
    let mut map = Map::default();
    for outcome in 0..3 {
        let mut read = |key: *mut u8, value: *mut u8| {
            // SAFETY: decode_entry_impl supplies exclusively borrowed,
            // initialized u32/u64 slots for this exact MapStorage type.
            unsafe {
                *key.cast::<u32>() = 7;
                *value.cast::<u64>() = 37;
            }
            if outcome == 0 {
                Err(DecodeError::UnexpectedEof)
            } else {
                Ok(outcome == 2)
            }
        };
        // SAFETY: the pointer names a live Map with exclusive access
        // throughout the callback; its entry types match the callback.
        let result = unsafe { decode_entry_impl::<Map>((&mut map as *mut Map).cast(), &mut read) };
        match outcome {
            0 => assert_eq!(result, Err(DecodeError::UnexpectedEof)),
            1 => assert_eq!(result, Ok(false)),
            _ => assert_eq!(result, Ok(true)),
        }
        let erased = if outcome == 0 { 1 } else { 2 };
        assert_eq!(KEYS.load(Ordering::SeqCst), erased);
        assert_eq!(VALUES.load(Ordering::SeqCst), erased);
    }
    assert_eq!(map.0.get(&7), Some(&37));
}

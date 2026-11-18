//! Temporary map slots; cleanup is opt-in with the `zeroize` feature.
#[cfg(feature = "zeroize")]
pub use guarded::Erasing;
#[cfg(not(feature = "zeroize"))]
pub use plain::Erasing;

// The ordinary codec retains a move-only slot with no destructor or unsafe
// storage access. The optimizer can remove this wrapper entirely.
#[cfg(not(feature = "zeroize"))]
mod plain {
    use core::ops::{Deref, DerefMut};
    pub struct Erasing<T>(T);
    impl<T> Erasing<T> {
        pub fn new(value: T, _erase: fn(&mut T)) -> Self {
            Self(value)
        }
        pub fn into_inner(self) -> T {
            self.0
        }
    }
    impl<T> Deref for Erasing<T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }
    impl<T> DerefMut for Erasing<T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.0
        }
    }
}

#[cfg(feature = "zeroize")]
mod guarded {
    use core::{
        mem::MaybeUninit,
        ops::{Deref, DerefMut},
    };

    /// A partially decoded slot with a fixed cleanup function.
    /// The cleanup function must not panic or allocate.
    #[doc(hidden)]
    pub struct Erasing<T> {
        value: MaybeUninit<T>,
        initialized: bool,
        erase: fn(&mut T),
    }
    impl<T> Erasing<T> {
        pub fn new(value: T, erase: fn(&mut T)) -> Self {
            Self {
                value: MaybeUninit::new(value),
                initialized: true,
                erase,
            }
        }
        pub fn into_inner(mut self) -> T {
            // SAFETY: new initializes the slot. This method consumes the guard,
            // so no other access can observe the now-vacant storage.
            let value = unsafe { self.value.assume_init_read() };
            self.initialized = false;
            value
        }
    }
    impl<T> Deref for Erasing<T> {
        type Target = T;
        fn deref(&self) -> &T {
            // SAFETY: only into_inner marks this false, and it consumes self.
            unsafe { self.value.assume_init_ref() }
        }
    }
    impl<T> DerefMut for Erasing<T> {
        fn deref_mut(&mut self) -> &mut T {
            // SAFETY: same initialized-slot invariant, with an exclusive borrow.
            unsafe { self.value.assume_init_mut() }
        }
    }
    impl<T> Drop for Erasing<T> {
        fn drop(&mut self) {
            if self.initialized {
                // SAFETY: initialized means the value was not transferred. Cleanup
                // runs before its destructor, including on a parse error.
                unsafe {
                    (self.erase)(self.value.assume_init_mut());
                    self.value.assume_init_drop();
                }
            }
            // Erase vacated inline scalar bytes as well as any inactive payload.
            // MaybeUninit permits zero bytes even for types with invalid zero values.
            zeroize::Zeroize::zeroize(&mut self.value);
        }
    }
}

#[cfg(all(test, feature = "zeroize"))]
mod tests;

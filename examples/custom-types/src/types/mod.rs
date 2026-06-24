//! Crate-local newtypes that satisfy buffa's pluggable owned-type traits.
//!
//! Each newtype wraps a *foreign* storage type (`flexstr`, `smallvec`,
//! `indexmap`, `smallbox`). The orphan rule forbids implementing buffa's
//! traits on those types directly, so a thin `#[repr(transparent)]` wrapper in
//! this crate is the bridge — the same pattern as `buffa-smolstr`, reproduced
//! here so the example is self-contained.
//!
//! One file per newtype, named for the `buffa_build` knob it backs.

/// `#[repr(transparent)]` guarantees a newtype has the same layout and ABI as
/// its inner type, so the wrapper is zero-cost at every boundary. Freeze that
/// against accidental regression (e.g. a second field sneaking in).
macro_rules! assert_transparent {
    ($outer:ty, $inner:ty) => {
        const _: () = {
            assert!(core::mem::size_of::<$outer>() == core::mem::size_of::<$inner>());
            assert!(core::mem::align_of::<$outer>() == core::mem::align_of::<$inner>());
        };
    };
}
pub(crate) use assert_transparent;

mod flex_str;
mod index_map;
mod small_box;
mod small_bytes;
mod small_vec;

pub use flex_str::FlexStr;
pub use index_map::IndexMap;
pub use small_box::SmallBox;
pub use small_bytes::SmallBytes;
pub use small_vec::SmallVec;

//! Wire ownership that erases its private allocation before release.
mod owned_view;
pub use owned_view::{GuardedOwnedView, GuardedViewError};
pub use zeroize::{ZeroizeOnDrop, Zeroizing};

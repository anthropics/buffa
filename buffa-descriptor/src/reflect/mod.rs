//! Runtime reflection over protobuf messages.
//!
//! This module provides:
//!
//! - [`DynamicMessage`] — a map-backed message holding [`Value`]s keyed by
//!   field number, with descriptor-driven encode/decode.
//! - [`Value`] / [`ValueRef`] / [`MapKey`] — the runtime value representation.
//! - [`ReflectMessage`] / [`ReflectMessageMut`] — the dyn-safe,
//!   storage-agnostic accessor traits.
//! - [`Reflectable`] / [`ReflectCow`] — the codegen entry point and the
//!   clone-on-write handle that absorbs the bridge/vtable mode difference.
//!
//! ## Architecture note
//!
//! The reflection design (`docs/investigations/reflection.md`) sketches this
//! module as `buffa/src/reflect/`. Implementation found that creates a
//! dependency cycle: `buffa-descriptor` already depends on `buffa` (the
//! generated descriptor types use `buffa::Message`), so `buffa` cannot
//! depend on `buffa-descriptor`. The reflection runtime needs both
//! `buffa::encoding` (wire primitives) and `buffa-descriptor` (descriptor
//! types and the pool), so it has to live in `buffa-descriptor` — which is
//! also the natural home, since reflection consumers already declare
//! `buffa-descriptor` for the descriptor types.

mod containers;
mod dynamic;
#[cfg(feature = "json")]
mod json;
mod message;
mod value;

pub use containers::{ReflectElement, ReflectMapKey};
pub use dynamic::{AnyError, DynamicMessage};
#[cfg(feature = "json")]
pub use json::DynamicMessageSeed;
pub use message::{ReflectCow, ReflectError, ReflectMessage, ReflectMessageMut, Reflectable};
pub use value::{MapKey, MapKeyRef, MapValue, ReflectList, ReflectMap, Value, ValueRef};

/// Per-message reflection mode, selected at codegen time.
///
/// See [`Reflectable`] for the call-site contract: code written against
/// `foo.reflect()` works identically in `Bridge` and `VTable` mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ReflectMode {
    /// No `Reflectable` impl is emitted.
    Off,
    /// `Reflectable::reflect` boxes a [`DynamicMessage`] from an
    /// encode/decode round-trip.
    #[default]
    Bridge,
    /// `Reflectable::reflect` borrows the struct directly via a codegen-emitted
    /// vtable. **Deferred** — codegen support does not exist yet.
    VTable,
}

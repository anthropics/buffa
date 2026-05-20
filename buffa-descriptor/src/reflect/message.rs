//! The [`ReflectMessage`] trait, [`ReflectCow`], and the [`Reflectable`]
//! entry-point trait.
//!
//! `ReflectMessage` is **dyn-safe and storage-agnostic** by design. The
//! v1 implementation is map-backed [`DynamicMessage`](super::DynamicMessage);
//! a future vtable-backed implementation on generated types must slot in as
//! a *second* impl of the same trait, with no call-site changes. That
//! constraint dictates the signature shape:
//!
//! - Accessors take `&FieldDescriptor`, not a generic key — the vtable will
//!   index directly off the descriptor, the map will look up by number.
//! - Accessors return [`ValueRef<'_>`], not an associated type — both impls
//!   produce the same enum.
//! - `for_each_set` takes `&mut dyn FnMut`, not `impl FnMut` — `dyn` traits
//!   can't have generic methods.
//!
//! [`Reflectable`] is the codegen-emitted entry point: every generated message
//! gets an impl whenever any reflection is enabled, and the body varies by
//! [`ReflectMode`](super::ReflectMode). The call site is always
//! `foo.reflect().get(fd)`; bridge mode pays an encode/decode round-trip,
//! vtable mode is zero-cost. Flipping a message between modes requires no
//! diff at the call site.

use alloc::boxed::Box;

use super::value::ValueRef;
use super::DynamicMessage;
use crate::{DescriptorPool, FieldDescriptor, MessageDescriptor, OneofDescriptor};

/// Reflection over a protobuf message.
///
/// Implemented by [`DynamicMessage`] (map-backed) and, in vtable mode, by
/// generated message structs. See the module documentation for the dyn-safety
/// contract.
pub trait ReflectMessage {
    /// The descriptor for this message type.
    fn message_descriptor(&self) -> &MessageDescriptor;

    /// The pool the descriptor lives in. Use this to dereference
    /// [`MessageIndex`](crate::MessageIndex) /
    /// [`EnumIndex`](crate::EnumIndex) from [`FieldKind`](crate::FieldKind),
    /// or `Arc::clone` it to construct sibling [`DynamicMessage`]s while
    /// navigating nested fields.
    fn pool(&self) -> &alloc::sync::Arc<DescriptorPool>;

    /// Get a field's value.
    ///
    /// For absent singular fields, returns the type's default value. For
    /// absent repeated/map fields, returns an empty container.
    ///
    /// # Panics
    ///
    /// May panic if `field` is not a member of this message's descriptor.
    /// Implementations are encouraged to `debug_assert!` rather than check
    /// in release.
    fn get(&self, field: &FieldDescriptor) -> ValueRef<'_>;

    /// Whether a field is present.
    ///
    /// For explicit-presence fields (proto2 `optional`/`required`, proto3
    /// `optional`, message-typed fields), this is "was a value written".
    /// For implicit-presence fields, this is "is non-default". For
    /// repeated/map fields, this is "non-empty".
    fn has(&self, field: &FieldDescriptor) -> bool;

    /// Visit every set field.
    ///
    /// "Set" follows the same semantics as [`Self::has`]. **Unknown fields
    /// are excluded** — they have no `FieldDescriptor`. Access them through
    /// the implementation's own surface (e.g. `DynamicMessage::unknown_fields`).
    fn for_each_set(&self, f: &mut dyn FnMut(&FieldDescriptor, ValueRef<'_>));

    /// Which member of `oneof` is set, if any.
    ///
    /// The default implementation checks each member field's
    /// [`has()`](Self::has). Implementations that track oneof discriminants
    /// directly may override for `O(1)` dispatch.
    ///
    /// Synthetic oneofs (proto3 `optional`) have exactly one member; this
    /// returns it iff the field is present.
    ///
    /// `oneof` must come from `self`'s [`message_descriptor()`](Self::message_descriptor) —
    /// passing a `OneofDescriptor` from a different message returns `None`
    /// or an unrelated member, the same cross-descriptor hazard
    /// [`get()`](Self::get) documents.
    fn which_oneof(&self, oneof: &OneofDescriptor) -> Option<&FieldDescriptor> {
        let md = self.message_descriptor();
        for &i in oneof.field_indices() {
            if let Some(fd) = md.fields().get(i as usize) {
                if self.has(fd) {
                    return Some(fd);
                }
            }
        }
        None
    }

    /// Snapshot this message as a [`DynamicMessage`].
    ///
    /// For an already-dynamic message this is a clone; for a generated
    /// message in bridge mode this is the encode/decode round-trip. The
    /// default implementation is provided so that `dyn ReflectMessage` can
    /// be converted, which `ReflectCow::to_dynamic` needs.
    fn to_dynamic(&self) -> DynamicMessage;
}

/// Mutable reflection over a protobuf message.
///
/// Separated from [`ReflectMessage`] because read-only reflection is the
/// common case (interceptors inspecting a request) and shouldn't require
/// `&mut`.
pub trait ReflectMessageMut: ReflectMessage {
    /// Set a field's value.
    ///
    /// Setting a singular field replaces it. Setting a `List` or `Map`
    /// value replaces the whole container.
    fn set(&mut self, field: &FieldDescriptor, value: super::Value);

    /// Clear a field, returning it to its default/absent state.
    fn clear(&mut self, field: &FieldDescriptor);
}

/// A clone-on-write reflective handle.
///
/// `Borrowed` is the vtable path — a fat pointer to a generated struct that
/// directly implements [`ReflectMessage`]. `Owned` is the bridge path — a
/// boxed [`DynamicMessage`] produced by encode/decode round-trip.
///
/// Boxing the `Owned` variant is load-bearing for [`ValueRef`](super::ValueRef)'s
/// size budget. The dominant variant is `Borrowed(&dyn ReflectMessage)`, a
/// 16-byte fat pointer; with the 1-byte discriminant aligned to 8 bytes,
/// `ReflectCow` is 24 bytes. `Owned(Box<DynamicMessage>)` is a thin 8-byte
/// pointer, so it doesn't increase the footprint. If `DynamicMessage`
/// (~56 bytes: an `Arc`, a `MessageIndex`, a `BTreeMap`, and an
/// `UnknownFields`) were inlined instead of boxed, `ReflectCow` would jump
/// to ~64 bytes — and since `ValueRef::Message(ReflectCow)` sets the floor
/// for `ValueRef`'s size, that would triple `ValueRef` from 32 to ~72 bytes,
/// pushing every `get()` (including hot-path scalar reads) across two cache
/// lines. The one extra heap allocation per `Owned` fires only at entry
/// points and mixed-mode boundaries, where a full encode/decode is already
/// happening — noise against that backdrop.
///
/// The `const _:` assertion in `value.rs` locks the budget in.
pub enum ReflectCow<'a> {
    /// Borrowed reflective view over the source — the vtable path.
    Borrowed(&'a dyn ReflectMessage),
    /// Owned dynamic snapshot — the bridge path.
    Owned(Box<DynamicMessage>),
}

impl core::fmt::Debug for ReflectCow<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Borrowed(_) => write!(f, "ReflectCow::Borrowed(..)"),
            Self::Owned(d) => f.debug_tuple("ReflectCow::Owned").field(d).finish(),
        }
    }
}

impl<'a> ReflectCow<'a> {
    /// Snapshot the underlying message as a [`DynamicMessage`].
    #[must_use]
    pub fn to_dynamic(&self) -> DynamicMessage {
        match self {
            Self::Borrowed(m) => m.to_dynamic(),
            Self::Owned(d) => (**d).clone(),
        }
    }
}

impl<'a> core::ops::Deref for ReflectCow<'a> {
    type Target = dyn ReflectMessage + 'a;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(m) => *m,
            Self::Owned(d) => &**d,
        }
    }
}

/// Codegen entry point for reflection.
///
/// Codegen emits an impl for every generated message type whenever any
/// reflection mode is enabled. The body varies by mode: bridge mode boxes a
/// [`DynamicMessage`], vtable mode borrows the struct directly. The call site
/// is always `foo.reflect()` — flipping modes requires no diff.
pub trait Reflectable {
    /// A read-only reflective handle over `self`.
    ///
    /// # Performance
    ///
    /// In the codegen-emitted **bridge mode** impl, `reflect()` is one full
    /// encode + decode round-trip plus a heap allocation per call. Hold
    /// onto the returned handle for repeated field reads rather than
    /// calling `reflect()` per field. The first call also pays a one-time
    /// pool build cost (linking the embedded `FileDescriptorSet`).
    ///
    /// The vtable mode (deferred) is a borrow with no round-trip; the call
    /// site doesn't change between modes.
    ///
    /// # Panics
    ///
    /// The codegen-emitted bridge impl panics if the embedded
    /// `FileDescriptorSet` is malformed or `Self::FULL_NAME` is not
    /// registered in the package pool — both indicate a codegen bug, not
    /// consumer misuse.
    ///
    /// # Setup
    ///
    /// The `Reflectable` impl is generated by enabling
    /// `buffa_build::Config::generate_reflection(true)` in `build.rs`. The
    /// consuming crate must also depend on `buffa-descriptor` with its
    /// `reflect` feature and on `std`. See the `generate_reflection` doc
    /// for the `Cargo.toml` pattern.
    #[must_use = "reflect() round-trips through encode/decode in bridge mode; bind the handle"]
    fn reflect(&self) -> ReflectCow<'_>;

    // `reflect_mut(&mut self) -> ReflectCowMut<'_>` is part of the design but
    // deferred to the MergeSink work in this prototype — see merge.rs.
}

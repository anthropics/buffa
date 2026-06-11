//! Zero-copy borrowed message views.
//!
//! Buffa generates two representations for each protobuf message:
//!
//! - **Owned** (`MyMessage`): uses `String`, `Vec<u8>`, `Vec<T>` for fields.
//!   Suitable for building messages, long-lived storage, and mutation.
//!
//! - **Borrowed** (`MyMessageView<'a>`): uses `&'a str`, `&'a [u8]`, and
//!   slice-backed repeated fields. Borrows directly from the input buffer
//!   for zero-copy deserialization on the read path.
//!
//! # Motivation
//!
//! In a typical RPC handler, the request is parsed from a buffer, fields are
//! read, and the buffer is discarded. With owned types, every string and bytes
//! field requires an allocation + copy. With view types, strings and bytes
//! borrow directly from the input buffer — no allocation at all.
//!
//! This is analogous to how Cap'n Proto's Rust implementation works, and how
//! Go achieves zero-copy string deserialization via its garbage collector.
//!
//! # Usage pattern
//!
//! ```rust,ignore
//! // Decode a view (zero-copy, borrows from `wire_bytes`)
//! let request = MyRequestView::decode_view(&wire_bytes)?;
//! println!("name: {}", request.name);  // &str, no allocation
//!
//! // Build an owned response
//! let response = MyResponse {
//!     id: request.id,
//!     status: "ok".into(),
//!     ..Default::default()
//! };
//!
//! // Convert view to owned if needed for storage
//! let owned: MyRequest = request.to_owned_message();
//! ```
//!
//! # Reborrowing from `OwnedView`
//!
//! [`OwnedView<V>`](OwnedView) wraps a decoded view with the lifetime erased
//! to `'static`. The inner view is reached through
//! [`OwnedView::reborrow`], which ties the borrow to the `OwnedView` itself —
//! field reads, assigning the view to a binding, passing it to a function
//! with a non-`'static` lifetime parameter, and returning a borrowed field
//! all go through the same call:
//!
//! ```no_run
//! # use buffa::view::OwnedView;
//! # use buffa::__doctest_fixtures::PersonView;
//! // reborrow ties the returned borrow to the OwnedView's lifetime.
//! fn handler<'a>(req: &'a OwnedView<PersonView<'static>>) -> &'a str {
//!     req.reborrow().name
//! }
//! ```
//!
//! The view is deliberately not exposed as `&V` (e.g. via `Deref`): `V` is
//! `FooView<'static>`, so its borrowed fields would *appear* `'static` to the
//! compiler and could outlive the buffer they point into. `reborrow` narrows
//! that synthetic `'static` down to the `OwnedView`'s real lifetime. Generated
//! code also provides a per-message `FooOwnedView` wrapper with field accessor
//! methods, so handler code rarely needs to call `reborrow` directly. See
//! [`OwnedView`] for details.
//!
//! # Generated code shape
//!
//! For a message like:
//! ```protobuf
//! message Person {
//!   string name = 1;
//!   int32 id = 2;
//!   bytes avatar = 3;
//!   repeated string tags = 4;
//!   Address address = 5;
//! }
//! ```
//!
//! Buffa generates:
//! ```rust,ignore
//! // Owned type (heap-allocated strings and vecs)
//! pub struct Person {
//!     pub name: String,
//!     pub id: i32,
//!     pub avatar: Vec<u8>,
//!     pub tags: Vec<String>,
//!     pub address: MessageField<Address>,
//!     #[doc(hidden)] pub __buffa_unknown_fields: UnknownFields,
//! }
//!
//! // Borrowed view type (zero-copy from input buffer)
//! pub struct PersonView<'a> {
//!     pub name: &'a str,
//!     pub id: i32,
//!     pub avatar: &'a [u8],
//!     pub tags: RepeatedView<'a, &'a str>,
//!     pub address: MessageFieldView<AddressView<'a>>,
//!     pub __buffa_unknown_fields: UnknownFieldsView<'a>,
//! }
//! ```

use crate::error::DecodeError;
use crate::message::Message as _;
use bytes::{BufMut, Bytes};

/// Trait for zero-copy borrowed message views.
///
/// View types borrow from the input buffer and provide read-only access
/// to message fields without allocation. Each generated `MyMessageView<'a>`
/// implements this trait.
///
/// The lifetime `'a` ties the view to the input buffer — the view cannot
/// outlive the buffer it was decoded from.
pub trait MessageView<'a>: Sized {
    /// The corresponding owned message type.
    type Owned: crate::Message;

    /// Decode a view from a buffer, borrowing string/bytes fields directly.
    ///
    /// The returned view borrows from `buf`'s underlying bytes. The caller
    /// must ensure the buffer is contiguous (e.g., `&[u8]` or `bytes::Bytes`).
    ///
    /// Decoding validates the whole message tree eagerly. For
    /// decode-on-access of large sub-message trees, see the opt-in
    /// [`LazyMessageView`] family (`lazy_views` codegen option).
    fn decode_view(buf: &'a [u8]) -> Result<Self, DecodeError>;

    /// Decode a view under custom decode limits.
    ///
    /// Used by [`DecodeOptions::decode_view`](crate::DecodeOptions::decode_view)
    /// to pass a non-default recursion depth and unknown-field allowance.
    /// The default implementation delegates to
    /// [`decode_view`](Self::decode_view) and **ignores the context** —
    /// a hand-written `MessageView` that recurses or preserves unknown
    /// fields must override this method to honor the limits configured on
    /// `DecodeOptions`. Generated code always overrides it, calling
    /// `_decode_ctx(buf, ctx)`.
    fn decode_view_with_ctx(
        buf: &'a [u8],
        _ctx: crate::DecodeContext<'_>,
    ) -> Result<Self, DecodeError> {
        Self::decode_view(buf)
    }

    /// Convert this view to the owned message type.
    ///
    /// This allocates and copies all borrowed fields. Equivalent to
    /// [`to_owned_from_source(None)`](Self::to_owned_from_source).
    ///
    /// # Errors
    ///
    /// Returns an error if re-materializing preserved unknown fields fails —
    /// notably [`DecodeError::UnknownFieldLimitExceeded`] when the view
    /// holds more unknown fields than the allowance it was decoded under
    /// (each owned `UnknownField` counts, unlike the coalesced spans the
    /// view itself stores).
    fn to_owned_message(&self) -> Result<Self::Owned, DecodeError>;

    /// Convert this view to the owned message type, optionally slicing
    /// `bytes::Bytes`-typed fields from `source` instead of copying.
    ///
    /// When `source` is the [`Bytes`] buffer this view was decoded from,
    /// owned fields configured for `bytes::Bytes` (via the `bytes_fields`
    /// codegen option) are produced via [`Bytes::slice_ref`] — a refcount
    /// bump, no allocation or copy. Borrowed fields that fall outside
    /// `source` (e.g. on a manually-constructed view) and the `None` case
    /// fall back to [`Bytes::copy_from_slice`].
    ///
    /// Generated view types override this; the default delegates to
    /// [`to_owned_message`](Self::to_owned_message) so hand-written impls
    /// need only provide that method.
    ///
    /// # Errors
    ///
    /// Same contract as [`to_owned_message`](Self::to_owned_message).
    fn to_owned_from_source(&self, source: Option<&Bytes>) -> Result<Self::Owned, DecodeError> {
        let _ = source;
        self.to_owned_message()
    }
}

/// Exposes the real lifetime of an [`OwnedView`]'s borrows.
///
/// `OwnedView<V>` stores `V` with a `'static` lifetime — the actual borrows
/// point into its internal [`Bytes`] buffer. `ViewReborrow` lets
/// [`OwnedView::reborrow`] return a reference typed as `&'b V::Reborrowed<'b>`,
/// tying the borrow to `&'b self` so the compiler can reason about it correctly.
///
/// Codegen emits `impl ViewReborrow` automatically for every generated view
/// type. Hand-written view types must provide it manually if
/// [`OwnedView::reborrow`] is needed.
///
/// # Soundness
///
/// `ViewReborrow` is a **safe** trait. Soundness is established mechanically
/// by the compiler at each impl site: the [`reborrow`](Self::reborrow) method
/// body coerces a `&'b Self` (where `Self = FooView<'static>`) to
/// `&'b Self::Reborrowed<'b>` (= `&'b FooView<'b>`). Rust accepts this only
/// when `FooView` is **covariant** in its lifetime parameter — a covariant
/// `FooView<'static>` is a subtype of `FooView<'b>` and the coercion is a
/// standard subtyping move. Invariant fields (`Cell<&'a T>`, `&'a mut T`,
/// `fn(&'a T)`) make the type invariant in `'a`; the trait body then fails
/// to compile and the impl is rejected — which is exactly what should
/// happen, because narrowing the lifetime of an invariant view *would* be
/// unsound.
///
/// Hand-written impls cannot accidentally introduce undefined behaviour
/// without writing `unsafe` themselves: the canonical body is just `this`,
/// which the type checker accepts iff the variance permits the coercion.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement `ViewReborrow` — required by `OwnedView::reborrow`",
    note = "for a generated view type, this impl is emitted automatically by codegen",
    note = "for a hand-written view type `MyView<'a>`, add:\n    impl ViewReborrow for MyView<'static> {{\n        type Reborrowed<'b> = MyView<'b>;\n        fn reborrow<'b>(this: &'b Self) -> &'b Self::Reborrowed<'b> {{ this }}\n    }}",
    note = "your `MessageView` impl must be parametric over the lifetime — `impl<'a> MessageView<'a> for MyView<'a>` — so that both `Self: MessageView<'static>` and `Reborrowed<'b>: MessageView<'b>` hold",
    note = "`MyView` must be covariant in its lifetime — fields like `&'a T` and `MessageFieldView<...>` are covariant; `Cell<&'a T>` and `&'a mut T` are not, and the trait body `{{ this }}` will fail to compile for invariant types"
)]
pub trait ViewReborrow: MessageView<'static> {
    /// The same view type with its lifetime shortened to `'b`.
    type Reborrowed<'b>: MessageView<'b, Owned = <Self as MessageView<'static>>::Owned>
    where
        Self: 'b;

    /// Coerce `&'b Self` (= `&'b FooView<'static>`) to
    /// `&'b Self::Reborrowed<'b>` (= `&'b FooView<'b>`). The canonical body
    /// is just `this`; the compiler accepts it via standard lifetime
    /// variance for covariant view types.
    ///
    /// Called by [`OwnedView::reborrow`]; users shouldn't need to call this
    /// method directly.
    fn reborrow<'b>(this: &'b Self) -> &'b Self::Reborrowed<'b>;
}

/// Links an owned message type to its generated zero-copy view types.
///
/// For a message `Foo`, generated code implements this trait as
/// `View<'a> = FooView<'a>` (the borrowed view) and
/// `ViewHandle = FooOwnedView` (the self-contained `'static` handle). The
/// trait lets code that is generic over an owned message name those types —
/// for example an RPC framework that decodes `M::View<'_>` from a request
/// body it owns, or holds `M::ViewHandle` items in a stream — without
/// per-message glue on the consumer's side.
///
/// The associated types intentionally carry only structural bounds:
///
/// - [`View<'a>`](Self::View) is the message's view, with
///   [`Owned`](MessageView::Owned)` = Self`.
/// - [`ViewHandle`](Self::ViewHandle) is convertible from, and exposes via
///   [`AsRef`], the corresponding `OwnedView<Self::View<'static>>`, so
///   generic code can reach [`reborrow`](OwnedView::reborrow),
///   [`bytes`](OwnedView::bytes), and
///   [`to_owned_message`](OwnedView::to_owned_message) without naming the
///   concrete wrapper. The wrapper's per-field accessor methods remain
///   inherent on the concrete type.
///
/// Generic code that wants to reborrow through the handle
/// (`handle.as_ref().reborrow()`) adds `M::View<'static>: ViewReborrow` as a
/// bound at the use site; every generated view satisfies it. (The bound
/// cannot live on the trait itself: a `where Self::View<'static>:
/// ViewReborrow` clause currently trips a GAT normalization error, E0308
/// "expected `MessageView<'a>`, found `MessageView<'static>`".)
///
/// # Implementing
///
/// Implementations are generated alongside the view and owned-view wrapper
/// (and are therefore gated with them). Hand-written implementations are only
/// needed for hand-written view types and must follow the same shape.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement `HasMessageView` — its message-view family was not generated or is not enabled",
    note = "the `HasMessageView` impl is emitted next to each message's view types: \
            regenerate the crate that defines `{Self}` with buffa 0.7.0 or newer and \
            views enabled — `generate_views(true)` (on by default) in a buffa-build / \
            buffa-codegen config, or `views=true` for protoc-gen-buffa",
    note = "if the defining crate feature-gates its generated impls, enabling its views \
            feature is enough — no regeneration needed"
)]
pub trait HasMessageView: crate::Message + Sized {
    /// The zero-copy view of `Self`, borrowing from a buffer with lifetime
    /// `'a`.
    type View<'a>: MessageView<'a, Owned = Self> + Send + Sync;

    /// The generated `'static` owned-view handle for `Self`
    /// (`FooOwnedView`).
    type ViewHandle: From<OwnedView<Self::View<'static>>>
        + AsRef<OwnedView<Self::View<'static>>>
        + Send
        + Sync
        + 'static;

    /// Decode a [`ViewHandle`](Self::ViewHandle) from a [`Bytes`] buffer.
    ///
    /// Convenience for generic code; equivalent to decoding an
    /// [`OwnedView<Self::View<'static>>`](OwnedView) and converting it with
    /// `From`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the buffer contains invalid protobuf data.
    fn decode_view_handle(bytes: Bytes) -> Result<Self::ViewHandle, DecodeError> {
        Ok(Self::ViewHandle::from(
            OwnedView::<Self::View<'static>>::decode(bytes)?,
        ))
    }

    /// Decode a [`ViewHandle`](Self::ViewHandle) with custom
    /// [`DecodeOptions`](crate::DecodeOptions) (recursion limit, max message
    /// size).
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the buffer is invalid or exceeds the
    /// configured limits.
    fn decode_view_handle_with_options(
        bytes: Bytes,
        opts: &crate::DecodeOptions,
    ) -> Result<Self::ViewHandle, DecodeError> {
        Ok(Self::ViewHandle::from(
            OwnedView::<Self::View<'static>>::decode_with_options(bytes, opts)?,
        ))
    }
}

/// Produce a [`Bytes`] for a borrowed slice, preferring a zero-copy
/// [`Bytes::slice_ref`] into `source` when the slice lies within it.
///
/// Used by generated [`MessageView::to_owned_from_source`] for
/// `bytes_fields`. Empty slices return [`Bytes::new`]; slices outside
/// `source` (or `source = None`) fall back to [`Bytes::copy_from_slice`].
#[doc(hidden)]
#[inline]
pub fn bytes_from_source(source: Option<&Bytes>, slice: &[u8]) -> Bytes {
    if slice.is_empty() {
        return Bytes::new();
    }
    if let Some(b) = source {
        // Mirrors `slice_ref`'s own containment precondition so we fall back
        // to copy (rather than panic) for slices outside `source`.
        let b_start = b.as_ptr() as usize;
        let s_start = slice.as_ptr() as usize;
        if let (Some(b_end), Some(s_end)) = (
            b_start.checked_add(b.len()),
            s_start.checked_add(slice.len()),
        ) {
            if s_start >= b_start && s_end <= b_end {
                return b.slice_ref(slice);
            }
        }
    }
    Bytes::copy_from_slice(slice)
}

/// Serialize a [`MessageView`] directly from its borrowed fields.
///
/// Symmetric with [`Message`](crate::Message)'s two-pass
/// `compute_size` / `write_to` model, but the `&'a str` / `&'a [u8]` /
/// [`MapView`] / [`RepeatedView`] fields are written by borrow — no
/// owned-struct intermediary, no per-field `String`/`Vec<u8>` allocations.
///
/// Generated `*View<'a>` types implement this trait whenever views are
/// generated (`generate_views(true)`, the default). Serialization state
/// lives in the external [`SizeCache`](crate::SizeCache), not the view —
/// view structs hold no interior mutability and remain `Send + Sync`.
///
/// ## When to use
///
/// Reach for `ViewEncode` when the source data is already in memory and
/// you would otherwise allocate an owned message just to encode-then-drop
/// it — e.g. an RPC handler serializing from app state. If you already
/// hold the owned message, use [`Message::encode`](crate::Message::encode)
/// instead; the wire output is identical.
///
/// ```rust,ignore
/// let view = PersonView {
///     name: "borrowed",
///     tags: ["a", "b"].iter().copied().collect(),
///     ..Default::default()
/// };
/// let bytes = view.encode_to_vec();
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement `ViewEncode` — view types were not generated for this message",
    note = "ViewEncode is implemented on every generated `*View<'a>` type; enable `generate_views(true)` (on by default) in your buffa-build / buffa-codegen config"
)]
pub trait ViewEncode<'a>: MessageView<'a> {
    /// Compute the encoded byte size of this view, recording nested
    /// sub-message sizes in `cache` for [`write_to`](Self::write_to)
    /// to consume.
    ///
    /// Most callers should use [`encode`](Self::encode) instead, which runs
    /// both passes with a fresh cache.
    fn compute_size(&self, cache: &mut crate::SizeCache) -> u32;

    /// Write this view's encoded bytes to a buffer, consuming
    /// nested-message sizes from `cache` (populated by a prior
    /// [`compute_size`](Self::compute_size) call on the same cache).
    ///
    /// Most callers should use [`encode`](Self::encode) instead.
    fn write_to(&self, cache: &mut crate::SizeCache, buf: &mut impl BufMut);

    /// Compute size, then write. Primary view-encode entry point.
    fn encode(&self, buf: &mut impl BufMut) {
        let mut cache = crate::SizeCache::new();
        self.compute_size(&mut cache);
        self.write_to(&mut cache, buf);
    }

    /// Encode using a caller-supplied [`SizeCache`](crate::SizeCache), for
    /// reuse across many encodes in a hot loop. Clears the cache first.
    fn encode_with_cache(&self, cache: &mut crate::SizeCache, buf: &mut impl BufMut) {
        cache.clear();
        self.compute_size(cache);
        self.write_to(cache, buf);
    }

    /// Compute the encoded byte size of this view.
    ///
    /// Walks the view tree, discarding the intermediate
    /// [`SizeCache`](crate::SizeCache). If you also intend to encode,
    /// prefer [`encode`](Self::encode) or [`encode_to_vec`](Self::encode_to_vec)
    /// — they do a single size pass and reuse the cache for the write.
    #[must_use]
    fn encoded_len(&self) -> u32 {
        self.compute_size(&mut crate::SizeCache::new())
    }

    /// Encode this view as a length-delimited byte sequence.
    fn encode_length_delimited(&self, buf: &mut impl BufMut) {
        let mut cache = crate::SizeCache::new();
        let len = self.compute_size(&mut cache);
        crate::encoding::encode_varint(len as u64, buf);
        self.write_to(&mut cache, buf);
    }

    /// Encode this view to a new `Vec<u8>`.
    #[must_use]
    fn encode_to_vec(&self) -> alloc::vec::Vec<u8> {
        let mut cache = crate::SizeCache::new();
        let size = self.compute_size(&mut cache) as usize;
        let mut buf = alloc::vec::Vec::with_capacity(size);
        self.write_to(&mut cache, &mut buf);
        buf
    }

    /// Encode this view to a new [`bytes::Bytes`].
    #[must_use]
    fn encode_to_bytes(&self) -> Bytes {
        let mut cache = crate::SizeCache::new();
        let size = self.compute_size(&mut cache) as usize;
        let mut buf = bytes::BytesMut::with_capacity(size);
        self.write_to(&mut cache, &mut buf);
        buf.freeze()
    }
}

/// Provides access to a lazily-initialized default view instance.
///
/// View types implement this trait so that [`MessageFieldView`] can
/// dereference to a default when unset, just as [`MessageField`](crate::MessageField)
/// does for owned types via [`DefaultInstance`](crate::DefaultInstance).
///
/// Generated view types like `FooView<'a>` contain only covariant borrows
/// (`&'a str`, `&'a [u8]`, etc.). A default view holds only `'static` data
/// (`""`, `&[]`, `0`), so an implementation stores a single
/// `&'static FooView<'static>` and returns it at the caller's lifetime via
/// ordinary covariant subtyping — the compiler verifies covariance at the
/// `impl` site, so no `unsafe` is required.
///
/// # Recommended implementation
///
/// The pattern codegen uses (and the recommended pattern for hand-written
/// view types) stores the instance in a static
/// [`once_cell::race::OnceBox`] (re-exported as
/// `::buffa::__private::OnceBox`):
///
/// ```rust,ignore
/// impl<'v> DefaultViewInstance for MyView<'v> {
///     fn default_view_instance<'a>() -> &'a Self
///     where
///         Self: 'a,
///     {
///         static VALUE: ::buffa::__private::OnceBox<MyView<'static>>
///             = ::buffa::__private::OnceBox::new();
///         VALUE.get_or_init(|| Box::new(<MyView<'static>>::default()))
///     }
/// }
/// ```
///
/// The return expression has type `&'static MyView<'static>`; the compiler
/// coerces it to `&'a MyView<'v>` iff `MyView` is covariant in `'v` —
/// non-covariant view types fail to compile here rather than risk an
/// unsound cast.
///
/// # Non-covariant types are rejected
///
/// A type that is invariant in its lifetime parameter cannot satisfy the
/// recommended pattern, because the `&'static T<'static> → &'a T<'v>`
/// coercion is refused:
///
/// ```compile_fail
/// # use core::marker::PhantomData;
/// // `fn(&'v ()) -> &'v ()` is invariant in 'v, making `Invariant<'v>` invariant.
/// struct Invariant<'v>(PhantomData<fn(&'v ()) -> &'v ()>);
/// static INST: Invariant<'static> = Invariant(PhantomData);
///
/// impl<'v> buffa::view::DefaultViewInstance for Invariant<'v> {
///     fn default_view_instance<'a>() -> &'a Self where Self: 'a {
///         // error: lifetime may not live long enough
///         //   note: requirement occurs because of the type `Invariant<'_>`,
///         //         which makes the generic argument `'_` invariant
///         &INST
///     }
/// }
/// ```
pub trait DefaultViewInstance {
    /// Return a reference to the single default view instance.
    ///
    /// The lifetime `'a` is caller-chosen up to `Self: 'a`, so a
    /// `FooView<'v>` can serve its `'static` default at any `'a ≤ 'v`.
    fn default_view_instance<'a>() -> &'a Self
    where
        Self: 'a;
}

/// A borrowed view of an optional message field.
///
/// Analogous to [`MessageField<T>`](crate::MessageField) but for the view
/// layer. Like `MessageField`, the inner view is **boxed** — recursive
/// message types (`Foo { NestedMessage { corecursive: Foo } }`) would
/// otherwise have infinite size. The box is API-transparent: `Deref`
/// returns `&V`, and `set()` takes `V` by value.
///
/// When `V` implements [`DefaultViewInstance`], this type implements
/// [`Deref<Target = V>`](core::ops::Deref), returning a reference to a
/// static default instance when the field is unset — making view code
/// identical to owned code for field access:
///
/// ```rust,ignore
/// // Both work the same, regardless of whether `address` is set:
/// let city = owned_msg.address.city;    // MessageField<Address>
/// let city = view_msg.address.city;     // MessageFieldView<AddressView>
/// ```
///
/// The lifetime of the contained view type `V` (e.g. `AddressView<'a>`)
/// ties this to the input buffer — no separate lifetime parameter is
/// needed here.
#[derive(Clone, Debug)]
pub struct MessageFieldView<V> {
    inner: Option<alloc::boxed::Box<V>>,
}

impl<V> MessageFieldView<V> {
    /// An unset field (the default).
    #[inline]
    pub const fn unset() -> Self {
        Self { inner: None }
    }

    /// A set field with the given view value.
    #[inline]
    pub fn set(v: V) -> Self {
        Self {
            inner: Some(alloc::boxed::Box::new(v)),
        }
    }

    /// Alias for [`set`](Self::set), mirroring owned
    /// [`MessageField::some`](crate::MessageField::some).
    #[inline]
    pub fn some(v: V) -> Self {
        Self::set(v)
    }

    /// Returns `true` if the field has a value.
    #[inline]
    pub fn is_set(&self) -> bool {
        self.inner.is_some()
    }

    /// Returns `true` if the field has no value.
    #[inline]
    pub fn is_unset(&self) -> bool {
        self.inner.is_none()
    }

    /// Get a reference to the inner view, or `None` if unset.
    #[inline]
    pub fn as_option(&self) -> Option<&V> {
        self.inner.as_deref()
    }

    /// Get a mutable reference to the inner view, or `None` if unset.
    ///
    /// Used by generated decode code to merge a second occurrence of a
    /// message field into an existing value (proto merge semantics).
    #[inline]
    pub fn as_mut(&mut self) -> Option<&mut V> {
        self.inner.as_deref_mut()
    }
}

impl<'a, V: ViewEncode<'a>> MessageFieldView<V> {
    /// Forward to the inner view's [`compute_size`](ViewEncode::compute_size),
    /// or `0` if unset. Generated `compute_size` calls this for nested-message
    /// fields, mirroring [`MessageField`](crate::MessageField) on the owned side.
    #[inline]
    pub fn compute_size(&self, cache: &mut crate::SizeCache) -> u32 {
        self.inner.as_deref().map_or(0, |v| v.compute_size(cache))
    }

    /// Forward to the inner view's [`write_to`](ViewEncode::write_to);
    /// no-op if unset.
    #[inline]
    pub fn write_to(&self, cache: &mut crate::SizeCache, buf: &mut impl BufMut) {
        if let Some(v) = self.inner.as_deref() {
            v.write_to(cache, buf);
        }
    }
}

impl<V> Default for MessageFieldView<V> {
    #[inline]
    fn default() -> Self {
        Self::unset()
    }
}

impl<V> From<V> for MessageFieldView<V> {
    #[inline]
    fn from(v: V) -> Self {
        Self::set(v)
    }
}

impl<V: DefaultViewInstance> core::ops::Deref for MessageFieldView<V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &V {
        self.inner
            .as_deref()
            .unwrap_or_else(V::default_view_instance)
    }
}

/// Wire-equivalent equality: `Unset` equals `Set(v)` when `v` equals the
/// default instance.
///
/// This matches [`MessageField::eq`](crate::MessageField) on the owned side,
/// so `view_a == view_b` agrees with
/// `view_a.to_owned_message() == view_b.to_owned_message()`.
///
/// The comparison against the default routes through the
/// [`Deref`](core::ops::Deref) impl.
impl<V: PartialEq + DefaultViewInstance> PartialEq for MessageFieldView<V> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.inner, &other.inner) {
            // Short-circuit: two unset fields are equal regardless of whether
            // V::PartialEq is reflexive (e.g. a view containing an f64 NaN).
            (None, None) => true,
            // At least one side is set. Deref handles None → default.
            _ => {
                <Self as core::ops::Deref>::deref(self) == <Self as core::ops::Deref>::deref(other)
            }
        }
    }
}

impl<V: Eq + DefaultViewInstance> Eq for MessageFieldView<V> {}

// ---------------------------------------------------------------------------
// Lazy views (generated under the `lazy_views` codegen option)
// ---------------------------------------------------------------------------

/// The trait implemented by generated lazy view types (`FooLazyView<'a>`).
///
/// Lazy views are a separate, additive type family generated alongside the
/// eager `FooView` family under the `lazy_views` codegen option. Where
/// [`MessageView`]'s contract is "decode succeeded ⇒ the whole tree was
/// validated", a lazy view's [`decode_lazy`](Self::decode_lazy) performs a
/// single non-recursive scan over the message's own fields: scalar, string,
/// and bytes fields are borrowed exactly as in the eager view, while nested
/// and repeated message fields are *recorded* as undecoded byte ranges (see
/// [`LazyMessageFieldView`] / [`LazyRepeatedView`]) and decoded only on
/// access. Deferred validation is therefore visible in the type and trait
/// bound — generic code over `MessageView` never silently inherits it.
pub trait LazyMessageView<'a>: Sized {
    /// The corresponding owned message type.
    type Owned: crate::Message;

    /// Decode a lazy view from `buf`: one scan over the message's own
    /// fields, deferring nested message fields.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the message's *own* fields are malformed.
    /// Deferred sub-message bytes are **not** validated here; they surface
    /// errors on access.
    fn decode_lazy(buf: &'a [u8]) -> Result<Self, DecodeError>;

    /// Decode a lazy view under custom decode limits.
    ///
    /// Used by [`DecodeOptions::decode_lazy_view`](crate::DecodeOptions::decode_lazy_view).
    /// The budgets remaining at each deferred field's position are recorded
    /// and charged when that field is accessed, so custom limits flow through
    /// deferred decoding. The default implementation delegates to
    /// [`decode_lazy`](Self::decode_lazy) and **ignores the context**;
    /// generated code always overrides it.
    ///
    /// # Errors
    ///
    /// Same contract as [`decode_lazy`](Self::decode_lazy), plus
    /// [`DecodeError::RecursionLimitExceeded`] /
    /// [`DecodeError::UnknownFieldLimitExceeded`] when `ctx`'s budgets are
    /// exhausted by the message's own fields.
    fn decode_lazy_with_ctx(
        buf: &'a [u8],
        ctx: crate::DecodeContext<'_>,
    ) -> Result<Self, DecodeError> {
        let _ = ctx;
        Self::decode_lazy(buf)
    }

    /// Merge fields decoded from `buf` into this view (proto merge
    /// semantics: singular scalars last-wins, repeated append, deferred
    /// message fragments accumulate).
    ///
    /// Used by [`LazyMessageFieldView::get`] to reassemble a field whose
    /// value was split across multiple wire occurrences — application code
    /// rarely calls this directly.
    ///
    /// # Errors
    ///
    /// Same contract as [`decode_lazy_with_ctx`](Self::decode_lazy_with_ctx).
    fn merge_lazy(
        &mut self,
        buf: &'a [u8],
        ctx: crate::DecodeContext<'_>,
    ) -> Result<(), DecodeError>;

    /// Convert this view to the owned message type.
    ///
    /// This decodes every deferred sub-message, so it is where deferred
    /// validation errors surface. Each deferred subtree decodes under its
    /// own replayed unknown-field allowance (see
    /// [`LazyMessageFieldView::get`]), so the conversion's total
    /// unknown-field records are bounded per subtree, not globally as in an
    /// eager decode.
    ///
    /// # Errors
    ///
    /// Returns the [`DecodeError`] that accessing a malformed or
    /// over-budget deferred field would have reported, or an unknown-field
    /// re-materialization failure (as in
    /// [`MessageView::to_owned_message`]).
    fn to_owned_message(&self) -> Result<Self::Owned, DecodeError>;
}

/// Fragments of one singular message field. `Many` only arises when an
/// encoder split the field across occurrences, keeping the common
/// single-occurrence path allocation-free.
#[derive(Clone)]
enum LazyFragments<'a> {
    None,
    One(&'a [u8]),
    Many(alloc::vec::Vec<&'a [u8]>),
}

/// A deferred view of a singular message field on a lazy view.
///
/// Unlike [`MessageFieldView`] — which eagerly decodes (and boxes) the
/// sub-message during decode — this stores only the field's undecoded wire
/// bytes and decodes a fresh `V` on each [`get`](Self::get), so decoding the
/// enclosing message does not allocate or recurse into sub-messages the
/// caller never reads.
///
/// `get` returns a freshly-decoded view each call (views are thin borrows,
/// so this is cheap) and does not cache — bind the result when reading
/// several fields.
///
/// # Merge semantics
///
/// A singular message field may legally appear more than once on the wire;
/// decoders must merge the occurrences. This type stores each occurrence's
/// bytes as a separate fragment and [`get`](Self::get) replays them in order
/// (decode the first, [`LazyMessageView::merge_lazy`] the rest), so the
/// result matches the eager and owned decoders.
///
/// # Deferred validation and budgets
///
/// The fragment bytes are *not* validated when the enclosing view is
/// decoded; a malformed sub-message surfaces as a [`DecodeError`] from
/// [`get`](Self::get). The recursion budget and unknown-field allowance
/// remaining when the field was recorded are stored alongside the fragments,
/// and each access replays them as a fresh per-subtree budget (see
/// [`get`](Self::get) for the approximation this implies). Deep lazy chains
/// fail with [`DecodeError::RecursionLimitExceeded`] at the same boundary as
/// the eager decoder, and custom limits passed to the enclosing
/// [`decode_lazy_with_ctx`](LazyMessageView::decode_lazy_with_ctx) flow
/// through.
///
/// # Re-encoding
///
/// `ViewEncode` on the enclosing lazy view replays the recorded fragments
/// byte-for-byte **without validating them** — re-encoding a never-accessed
/// malformed field round-trips its bytes silently.
pub struct LazyMessageFieldView<'a, V> {
    raw: LazyFragments<'a>,
    depth: u32,
    allowance: usize,
    _marker: core::marker::PhantomData<fn() -> V>,
}

impl<'a, V> LazyMessageFieldView<'a, V> {
    /// An unset field (the default).
    #[inline]
    pub const fn unset() -> Self {
        Self {
            raw: LazyFragments::None,
            // Sentinels: the first `push_fragment` lowers these to its
            // recorded budgets, so custom limits above the defaults aren't
            // clamped.
            depth: u32::MAX,
            allowance: usize::MAX,
            _marker: core::marker::PhantomData,
        }
    }

    /// A set field carrying the sub-message's undecoded wire bytes, with the
    /// default recursion and unknown-field budgets for access.
    #[inline]
    pub const fn from_bytes(raw: &'a [u8]) -> Self {
        Self {
            raw: LazyFragments::One(raw),
            depth: crate::RECURSION_LIMIT,
            allowance: crate::DEFAULT_UNKNOWN_FIELD_LIMIT,
            _marker: core::marker::PhantomData,
        }
    }

    /// Append one wire occurrence of the field (used by generated
    /// `decode_lazy`). Fragments accumulate in wire order; [`get`](Self::get)
    /// merges them. `ctx` carries the recursion budget and unknown-field
    /// allowance remaining at the record site; the smallest pushed budgets
    /// are charged on access.
    #[doc(hidden)]
    #[inline]
    pub fn push_fragment(&mut self, raw: &'a [u8], ctx: crate::DecodeContext<'_>) {
        self.depth = self.depth.min(ctx.depth());
        self.allowance = self.allowance.min(ctx.remaining_unknown_fields());
        self.raw = match core::mem::replace(&mut self.raw, LazyFragments::None) {
            LazyFragments::None => LazyFragments::One(raw),
            LazyFragments::One(first) => LazyFragments::Many(alloc::vec![first, raw]),
            LazyFragments::Many(mut frags) => {
                frags.push(raw);
                LazyFragments::Many(frags)
            }
        };
    }

    /// Whether the field is present.
    #[inline]
    pub const fn is_set(&self) -> bool {
        !matches!(self.raw, LazyFragments::None)
    }

    /// Whether the field has no value.
    #[inline]
    pub const fn is_unset(&self) -> bool {
        matches!(self.raw, LazyFragments::None)
    }

    /// The undecoded wire fragments, in wire order (empty if unset).
    ///
    /// A singular message field that appeared exactly once on the wire — the
    /// common case — yields one fragment. Encoders that split the field
    /// across multiple occurrences yield one fragment per occurrence;
    /// [`get`](Self::get) merges them per proto semantics.
    #[inline]
    pub fn fragments(&self) -> &[&'a [u8]] {
        match &self.raw {
            LazyFragments::None => &[],
            LazyFragments::One(raw) => core::slice::from_ref(raw),
            LazyFragments::Many(frags) => frags,
        }
    }
}

impl<'a, V: LazyMessageView<'a>> LazyMessageFieldView<'a, V> {
    /// Decode and return the sub-message view, or `None` if unset.
    ///
    /// Multiple wire fragments are merged per proto semantics (see the type
    /// docs). The view is re-decoded on every call; there is no cache — bind
    /// the result when reading several fields. Note the shape difference
    /// from [`LazyRepeatedView::get`], which returns `Option<Result<V, _>>`.
    ///
    /// Each access rebuilds a fresh decode context from the budgets recorded
    /// at decode time, so every deferred subtree independently gets the full
    /// recorded unknown-field allowance rather than sharing one pool with
    /// its siblings (the original decode call's shared allowance is gone by
    /// access time). The unknown-field limit is therefore a *per-subtree*
    /// bound on the lazy path, not the global decode-time cap the eager
    /// decoder enforces: a full traversal can materialize unknown-field
    /// records proportional to input size, where eager
    /// [`decode_view`](crate::DecodeOptions::decode_view) rejects such input
    /// up front. Prefer the eager path for untrusted input if that global
    /// bound matters.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the deferred bytes are not a valid
    /// encoding of `V` — validation happens here, not when the enclosing
    /// view was decoded — [`DecodeError::RecursionLimitExceeded`] when the
    /// recursion budget recorded at decode time is exhausted, or
    /// [`DecodeError::UnknownFieldLimitExceeded`] when the unknown-field
    /// allowance recorded at decode time is exhausted.
    #[inline]
    pub fn get(&self) -> Result<Option<V>, DecodeError> {
        let allowance = core::cell::Cell::new(self.allowance);
        let ctx = crate::DecodeContext::new(self.depth, &allowance);
        match &self.raw {
            LazyFragments::None => Ok(None),
            LazyFragments::One(raw) => V::decode_lazy_with_ctx(raw, ctx).map(Some),
            LazyFragments::Many(frags) => {
                // `Many` always holds ≥ 2 fragments (see `push_fragment`);
                // the guard is belt-and-suspenders.
                let mut iter = frags.iter();
                let Some(first) = iter.next() else {
                    return Ok(None);
                };
                let mut view = V::decode_lazy_with_ctx(first, ctx)?;
                for frag in iter {
                    view.merge_lazy(frag, ctx)?;
                }
                Ok(Some(view))
            }
        }
    }

    /// Like [`get`](Self::get), but an unset field decodes to the default
    /// view — the lazy analogue of [`MessageFieldView`]'s deref-to-default,
    /// for the common read path:
    ///
    /// ```rust,ignore
    /// let city = view.address.get_or_default()?.city;
    /// ```
    ///
    /// # Errors
    ///
    /// Same as [`get`](Self::get).
    #[inline]
    pub fn get_or_default(&self) -> Result<V, DecodeError>
    where
        V: Default,
    {
        Ok(self.get()?.unwrap_or_default())
    }
}

impl<V> Clone for LazyMessageFieldView<'_, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            depth: self.depth,
            allowance: self.allowance,
            _marker: core::marker::PhantomData,
        }
    }
}
impl<V> Default for LazyMessageFieldView<'_, V> {
    #[inline]
    fn default() -> Self {
        Self::unset()
    }
}
impl<V> core::fmt::Debug for LazyMessageFieldView<'_, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LazyMessageFieldView")
            .field("is_set", &self.is_set())
            .field("fragments", &self.fragments().len())
            .finish()
    }
}

/// A deferred view of a repeated message field on a lazy view.
///
/// Holds the wire byte-slice of each element (cheap pointers) and decodes a
/// fresh element view on access ([`get`](Self::get)) or iteration
/// ([`iter`](Self::iter)), instead of eagerly decoding every element into a
/// `Vec` like [`RepeatedView`].
///
/// Element bytes are *not* validated when the enclosing view is decoded; a
/// malformed element surfaces as a [`DecodeError`] from `get`/`iter`. Unlike
/// [`RepeatedView`], this type is not slice-backed: there is no `Deref` or
/// indexing, use `get`/`iter`/`len`. Budgets and re-encoding behave as on
/// [`LazyMessageFieldView`].
pub struct LazyRepeatedView<'a, V> {
    elements: alloc::vec::Vec<&'a [u8]>,
    depth: u32,
    allowance: usize,
    _marker: core::marker::PhantomData<fn() -> V>,
}

impl<'a, V> LazyRepeatedView<'a, V> {
    /// An empty repeated field.
    #[inline]
    pub fn new() -> Self {
        Self {
            elements: alloc::vec::Vec::new(),
            // Sentinels — see `LazyMessageFieldView::unset`.
            depth: u32::MAX,
            allowance: usize::MAX,
            _marker: core::marker::PhantomData,
        }
    }

    /// Number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the field has no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The undecoded wire bytes of each element, in wire order.
    #[inline]
    pub fn raw_elements(&self) -> &[&'a [u8]] {
        &self.elements
    }

    /// Append an element's undecoded bytes (used by generated `decode_lazy`).
    /// `ctx` carries the recursion budget and unknown-field allowance
    /// remaining at the record site; the smallest pushed budgets are charged
    /// on access.
    #[doc(hidden)]
    #[inline]
    pub fn push_bytes(&mut self, raw: &'a [u8], ctx: crate::DecodeContext<'_>) {
        self.depth = self.depth.min(ctx.depth());
        self.allowance = self.allowance.min(ctx.remaining_unknown_fields());
        self.elements.push(raw);
    }
}

/// Decode one deferred element under a fresh context carrying the budgets
/// recorded at decode time. Each access decodes independently, so each gets
/// the full recorded allowance — a per-subtree bound, not the eager
/// decoder's shared global pool (see [`LazyMessageFieldView::get`]).
#[inline]
fn decode_deferred<'a, V: LazyMessageView<'a>>(
    raw: &'a [u8],
    depth: u32,
    allowance: usize,
) -> Result<V, DecodeError> {
    let cell = core::cell::Cell::new(allowance);
    V::decode_lazy_with_ctx(raw, crate::DecodeContext::new(depth, &cell))
}

impl<'a, V: LazyMessageView<'a>> LazyRepeatedView<'a, V> {
    /// Decode the element at `index`, or `None` if out of range.
    ///
    /// Re-decodes on every call (no cache) — bind the result when reading
    /// multiple fields, and avoid calling it inside a tight loop over the
    /// same index. Note the shape difference from
    /// [`LazyMessageFieldView::get`], which returns `Result<Option<V>, _>`.
    #[inline]
    pub fn get(&self, index: usize) -> Option<Result<V, DecodeError>> {
        self.elements
            .get(index)
            .map(|b| decode_deferred(b, self.depth, self.allowance))
    }

    /// Like [`get`](Self::get) with the layers flipped to match
    /// [`LazyMessageFieldView::get`]'s `Result<Option<_>, _>` shape:
    /// out-of-range yields `Ok(None)`.
    ///
    /// # Errors
    ///
    /// Same as [`get`](Self::get).
    #[inline]
    pub fn try_get(&self, index: usize) -> Result<Option<V>, DecodeError> {
        self.get(index).transpose()
    }

    /// Iterate the elements, decoding each on the fly.
    ///
    /// Yields `Result<V, DecodeError>` — element bytes are validated here,
    /// not when the enclosing view was decoded. Each pass over the iterator
    /// re-decodes the elements (no cache).
    #[inline]
    pub fn iter(&self) -> LazyRepeatedIter<'_, 'a, V> {
        LazyRepeatedIter {
            inner: self.elements.iter(),
            depth: self.depth,
            allowance: self.allowance,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'s, 'a, V: LazyMessageView<'a>> IntoIterator for &'s LazyRepeatedView<'a, V> {
    type Item = Result<V, DecodeError>;
    type IntoIter = LazyRepeatedIter<'s, 'a, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over a [`LazyRepeatedView`], decoding each element on `next`.
#[derive(Clone, Debug)]
pub struct LazyRepeatedIter<'s, 'a, V> {
    inner: core::slice::Iter<'s, &'a [u8]>,
    depth: u32,
    allowance: usize,
    _marker: core::marker::PhantomData<fn() -> V>,
}

impl<'a, V: LazyMessageView<'a>> Iterator for LazyRepeatedIter<'_, 'a, V> {
    type Item = Result<V, DecodeError>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|b| decode_deferred(b, self.depth, self.allowance))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, V: LazyMessageView<'a>> DoubleEndedIterator for LazyRepeatedIter<'_, 'a, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|b| decode_deferred(b, self.depth, self.allowance))
    }
}

impl<'a, V: LazyMessageView<'a>> ExactSizeIterator for LazyRepeatedIter<'_, 'a, V> {}

impl<V> Clone for LazyRepeatedView<'_, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            elements: self.elements.clone(),
            depth: self.depth,
            allowance: self.allowance,
            _marker: core::marker::PhantomData,
        }
    }
}
impl<V> Default for LazyRepeatedView<'_, V> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
impl<V> core::fmt::Debug for LazyRepeatedView<'_, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LazyRepeatedView")
            .field("len", &self.len())
            .finish()
    }
}

/// A borrowed view of a repeated field.
///
/// For scalar repeated fields, this is backed by a decoded `Vec` (scalars
/// can't be zero-copy because they require varint decoding). For string and
/// bytes repeated fields, elements borrow from the input buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepeatedView<'a, T> {
    elements: alloc::vec::Vec<T>,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a, T> RepeatedView<'a, T> {
    /// Create from a vec of decoded elements.
    pub fn new(elements: alloc::vec::Vec<T>) -> Self {
        Self {
            elements,
            _marker: core::marker::PhantomData,
        }
    }

    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Returns `true` if the repeated field contains no elements.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Append an element (used by generated `decode_view` code).
    #[doc(hidden)]
    pub fn push(&mut self, elem: T) {
        self.elements.push(elem);
    }

    /// Reserve capacity for at least `additional` more elements (used by
    /// generated `decode_view` code as a pre-allocation hint for packed
    /// repeated scalars). For varint elements the hint is an upper bound
    /// (every element occupies at least one byte on the wire); for fixed-
    /// size elements it is the exact remaining element count.
    #[doc(hidden)]
    pub fn reserve(&mut self, additional: usize) {
        self.elements.reserve(additional);
    }

    /// Returns an iterator over the elements.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.elements.iter()
    }
}

impl<'a, T> Default for RepeatedView<'a, T> {
    fn default() -> Self {
        Self {
            elements: alloc::vec::Vec::new(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'a, T> core::ops::Deref for RepeatedView<'a, T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.elements
    }
}

impl<'a, T> IntoIterator for RepeatedView<'a, T> {
    type Item = T;
    type IntoIter = alloc::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.elements.into_iter()
    }
}

impl<'b, 'a, T> IntoIterator for &'b RepeatedView<'a, T> {
    type Item = &'b T;
    type IntoIter = core::slice::Iter<'b, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.elements.iter()
    }
}

impl<'a, T> From<alloc::vec::Vec<T>> for RepeatedView<'a, T> {
    fn from(elements: alloc::vec::Vec<T>) -> Self {
        Self::new(elements)
    }
}

impl<'a, T> FromIterator<T> for RepeatedView<'a, T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

/// A borrowed view of a map field.
///
/// Protobuf `map<K, V>` fields are encoded as repeated sub-messages, each
/// containing a key (field 1) and value (field 2). This type stores the
/// decoded entries in a `Vec<(K, V)>`, borrowing string and bytes keys/values
/// directly from the input buffer.
///
/// Lookup is O(n) linear scan, which is appropriate for the typically small
/// maps found in protobuf messages (metadata labels, headers, etc.).
/// If duplicate keys appear on the wire, [`get`](MapView::get) returns the
/// last occurrence (last-write-wins, per the protobuf spec).
///
/// For larger maps where O(1) lookup matters, collect into a `HashMap`:
///
/// ```ignore
/// use std::collections::HashMap;
/// let index: HashMap<&str, &str> = view.labels.into_iter().collect();
/// ```
///
/// Duplicate keys resolve last-write-wins in the collected map (matching
/// proto map semantics), since `HashMap::from_iter` keeps the last value.
///
/// # Allocation
///
/// Like [`RepeatedView`], the `Vec` backing store requires allocation.
/// The individual keys and values borrow from the input buffer where possible
/// (string keys as `&'a str`, bytes values as `&'a [u8]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapView<'a, K, V> {
    entries: alloc::vec::Vec<(K, V)>,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a, K, V> MapView<'a, K, V> {
    /// Construct from a `Vec` of entries, for [`ViewEncode`] use.
    ///
    /// Duplicate keys are kept and all encoded — valid protobuf wire data
    /// (decoders apply last-write-wins). Mirrors [`RepeatedView::new`].
    pub fn new(entries: alloc::vec::Vec<(K, V)>) -> Self {
        Self {
            entries,
            _marker: core::marker::PhantomData,
        }
    }

    /// Returns the number of entries (including duplicates).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Append a key-value pair (used by generated `decode_view` code).
    #[doc(hidden)]
    pub fn push(&mut self, key: K, value: V) {
        self.entries.push((key, value));
    }

    /// Iterate over all entries in wire order.
    ///
    /// If duplicate keys exist, all occurrences are yielded.
    pub fn iter(&self) -> core::slice::Iter<'_, (K, V)> {
        self.entries.iter()
    }

    /// Iterate over all keys in wire order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|(k, _)| k)
    }

    /// Iterate over all values in wire order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }

    /// Look up a value by key, returning the last occurrence (last-write-wins).
    ///
    /// Accepts any type that `K` can borrow as, so `map.get("key")` works
    /// when `K` is `&str`. O(n) scan.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: core::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.entries
            .iter()
            .rev()
            .find(|(k, _)| k.borrow() == key)
            .map(|(_, v)| v)
    }

    /// Returns `true` if an entry with the given key exists.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: core::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.entries.iter().any(|(k, _)| k.borrow() == key)
    }

    /// Iterate over key-value pairs with duplicate keys removed.
    ///
    /// Each distinct key is yielded **exactly once, at the position of its
    /// last wire occurrence, carrying that occurrence's value** — i.e.
    /// last-write-wins, mirroring the merge semantics that an owned
    /// `HashMap` decode applies. Callers that need first-occurrence position
    /// should use [`iter`](Self::iter) and filter themselves.
    ///
    /// Used by the generated view `Serialize` impl: a JSON object cannot
    /// hold duplicate keys, but `MapView` preserves all wire entries
    /// (including malformed duplicates), so the JSON encode path must
    /// deduplicate. The implementation is allocation-free and O(n²) — for
    /// each entry, scan the remaining entries for a later occurrence of the
    /// same key. Duplicate map keys are invalid per the protobuf encoding
    /// spec and only arise in adversarial or conformance-test wire data, so
    /// `n` is effectively always small.
    pub fn iter_unique(&self) -> impl Iterator<Item = &(K, V)>
    where
        K: PartialEq,
    {
        let entries = &self.entries;
        entries.iter().enumerate().filter_map(move |(i, entry)| {
            if entries[i + 1..]
                .iter()
                .any(|(later_k, _)| *later_k == entry.0)
            {
                None
            } else {
                Some(entry)
            }
        })
    }

    /// Count of distinct keys (`iter_unique().count()`).
    pub fn len_unique(&self) -> usize
    where
        K: PartialEq,
    {
        self.iter_unique().count()
    }
}

impl<'a, K, V> From<alloc::vec::Vec<(K, V)>> for MapView<'a, K, V> {
    fn from(entries: alloc::vec::Vec<(K, V)>) -> Self {
        Self::new(entries)
    }
}

/// Duplicate keys are kept and all encoded; see [`MapView::new`].
impl<'a, K, V> FromIterator<(K, V)> for MapView<'a, K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<'a, K, V> Default for MapView<'a, K, V> {
    fn default() -> Self {
        Self {
            entries: alloc::vec::Vec::new(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'b, 'a, K, V> IntoIterator for &'b MapView<'a, K, V> {
    type Item = &'b (K, V);
    type IntoIter = core::slice::Iter<'b, (K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl<'a, K, V> IntoIterator for MapView<'a, K, V> {
    type Item = (K, V);
    type IntoIter = alloc::vec::IntoIter<(K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

/// A borrowed view of unknown fields.
///
/// Stores raw byte slices from the input buffer rather than decoded values,
/// enabling zero-copy round-tripping of unknown fields. Each stored span
/// holds **one or more consecutive** complete `(tag, value)` records:
/// adjacent unknown fields are coalesced into a single span, so a long run
/// of unknown fields costs one `Vec` slot rather than one per field.
#[derive(Clone, Default)]
pub struct UnknownFieldsView<'a> {
    /// Raw byte spans from the input buffer, each one or more complete
    /// `(tag, value)` records.
    raw_spans: alloc::vec::Vec<&'a [u8]>,
    /// The input-buffer tail starting at the first byte of the last span,
    /// kept so [`push_record`](Self::push_record) can extend that span over
    /// an adjacent record by re-slicing `last_tail` — never by widening the
    /// narrowed span reference, which would be provenance-unsound.
    last_tail: Option<&'a [u8]>,
    /// The unknown-field allowance remaining when this view's first record
    /// was pushed — the budget [`to_owned`](Self::to_owned) re-materializes
    /// under, so a tight decode-time limit carries through conversion.
    to_owned_allowance: Option<usize>,
}

// Manual impl: `last_tail` is an internal coalescing cursor that extends to
// the end of the input buffer — deriving Debug would dump the remaining
// message bytes on every `{:?}` print.
impl core::fmt::Debug for UnknownFieldsView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UnknownFieldsView")
            .field("raw_spans", &self.raw_spans)
            .finish_non_exhaustive()
    }
}

impl<'a> UnknownFieldsView<'a> {
    /// Creates an empty view.
    pub fn new() -> Self {
        Self::default()
    }

    #[doc(hidden)]
    pub fn push_raw(&mut self, span: &'a [u8]) {
        self.raw_spans.push(span);
        // A manually pushed span has no known position in the input buffer,
        // so coalescing must not extend it.
        self.last_tail = None;
    }

    /// Record one unknown wire record of `span_len` bytes starting at the
    /// head of `tail`, where `tail` extends from the record's first byte to
    /// the end of the input buffer.
    ///
    /// If the record starts exactly where the previous one ended, the
    /// previous span is extended in place (no allocation, no slot consumed);
    /// otherwise a new span is pushed and one slot of `ctx`'s unknown-field
    /// allowance is consumed.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::UnknownFieldLimitExceeded`] when a new span is
    /// needed but the allowance is exhausted, or
    /// [`DecodeError::UnexpectedEof`] if `span_len` exceeds `tail`.
    #[doc(hidden)]
    pub fn push_record(
        &mut self,
        tail: &'a [u8],
        span_len: usize,
        ctx: crate::DecodeContext<'_>,
    ) -> Result<(), crate::DecodeError> {
        if span_len > tail.len() {
            return Err(crate::DecodeError::UnexpectedEof);
        }
        if self.to_owned_allowance.is_none() {
            self.to_owned_allowance = Some(ctx.remaining_unknown_fields());
        }
        if let (Some(last), Some(prev_tail)) = (self.raw_spans.last_mut(), self.last_tail) {
            let prev_len = last.len();
            // Contiguous if the new record begins exactly one past the end
            // of the previous span. Both checks are plain pointer/length
            // comparisons; the extension below re-slices `prev_tail`, whose
            // provenance covers the combined range.
            if prev_tail.len() >= prev_len + span_len
                && core::ptr::eq(prev_tail[prev_len..].as_ptr(), tail.as_ptr())
            {
                *last = &prev_tail[..prev_len + span_len];
                return Ok(());
            }
        }
        ctx.register_unknown_field()?;
        self.raw_spans.push(&tail[..span_len]);
        self.last_tail = Some(tail);
        Ok(())
    }

    /// Returns `true` if no unknown fields were recorded.
    pub fn is_empty(&self) -> bool {
        self.raw_spans.is_empty()
    }

    /// Total byte length of all unknown field data.
    pub fn encoded_len(&self) -> usize {
        self.raw_spans.iter().map(|s| s.len()).sum()
    }

    /// Write all unknown-field bytes verbatim. Each span holds one or more
    /// complete `(tag, value)` records as they appeared on the wire, so
    /// concatenating the spans produces a valid encoding.
    pub fn write_to(&self, buf: &mut impl BufMut) {
        for span in &self.raw_spans {
            buf.put_slice(span);
        }
    }

    /// Convert to an owned [`UnknownFields`](crate::UnknownFields) by parsing all stored raw byte spans.
    ///
    /// Each span holds one or more consecutive (tag + value) records as they
    /// appeared on the wire. Parsing uses
    /// [`crate::encoding::decode_unknown_field`] with the full recursion
    /// limit so deeply nested group fields are handled correctly, and the
    /// unknown-field allowance that remained when this view recorded its
    /// first unknown field — so a tight decode-time limit carries through
    /// conversion. Views built manually (via [`push_raw`](Self::push_raw))
    /// fall back to
    /// [`DEFAULT_UNKNOWN_FIELD_LIMIT`](crate::DEFAULT_UNKNOWN_FIELD_LIMIT).
    /// A coalesced span re-materializes one owned `UnknownField` per
    /// record, so this conversion is where a long run of unknown fields
    /// actually allocates — and where the limit is enforced per field.
    ///
    /// # Errors
    ///
    /// Returns `Err` if any stored span is malformed — which should not occur
    /// when the view was produced by `decode_view` from valid wire data.
    pub fn to_owned(&self) -> Result<crate::UnknownFields, crate::DecodeError> {
        use crate::encoding::{decode_unknown_field, Tag};

        let limit = core::cell::Cell::new(
            self.to_owned_allowance
                .unwrap_or(crate::DEFAULT_UNKNOWN_FIELD_LIMIT),
        );
        let ctx = crate::DecodeContext::new(crate::RECURSION_LIMIT, &limit);
        let mut out = crate::UnknownFields::new();
        for span in &self.raw_spans {
            let mut cur: &[u8] = span;
            while !cur.is_empty() {
                let tag = Tag::decode(&mut cur)?;
                let field = decode_unknown_field(tag, &mut cur, ctx)?;
                out.push(field);
            }
        }
        Ok(out)
    }
}

/// An owned, `'static` container for a decoded message view.
///
/// `OwnedView` holds a [`Bytes`] buffer alongside the decoded view, ensuring
/// the view's borrows remain valid for the container's lifetime. The inner
/// view is reached through [`reborrow()`](OwnedView::reborrow), which returns
/// it with a lifetime tied to `&self`.
///
/// This type is `Send + Sync + 'static`, making it suitable for use across
/// async boundaries, in tower services, and anywhere a `'static` bound is
/// required.
///
/// # When to use
///
/// Use `OwnedView` when you need a zero-copy view that outlives the scope
/// where the buffer was received — for example, in an RPC handler where the
/// framework requires `'static` types:
///
/// ```rust,ignore
/// use buffa::view::OwnedView;
/// use bytes::Bytes;
///
/// let bytes: Bytes = receive_request_body().await;
/// let view = OwnedView::<PersonView>::decode(bytes)?;
///
/// // Field access through reborrow — the borrow is tied to `view`.
/// let person = view.reborrow();
/// println!("name: {}", person.name);
/// println!("id: {}", person.id);
///
/// // Convert to owned if you need to store or mutate
/// let owned: Person = view.to_owned_message();
/// ```
///
/// Generated code additionally provides a per-message `FooOwnedView` wrapper
/// around `OwnedView<FooView<'static>>` with per-field accessor methods
/// (`owned.name()`, `owned.id()`, …), so most handler code never touches
/// `OwnedView` or `reborrow` directly.
///
/// For scoped access where the buffer's lifetime is known, use
/// [`MessageView::decode_view`] directly — it has zero overhead beyond the
/// decode itself.
///
/// # Why field access goes through `reborrow`
///
/// `OwnedView` stores `V = FooView<'static>`: the view's borrows really point
/// into `self`'s [`Bytes`] buffer, and the `'static` is a synthetic lifetime
/// established by the constructor. Exposing `&V` directly (for example via a
/// `Deref` impl) would let borrowed fields *appear* `'static` to the
/// compiler and escape the `OwnedView`'s scope, dangling once it drops.
/// [`reborrow()`](OwnedView::reborrow) narrows the synthetic `'static` down
/// to the `OwnedView`'s real lifetime, so the borrow checker enforces the
/// actual validity of every field borrow:
///
/// ```no_run
/// # use buffa::view::OwnedView;
/// # use buffa::__doctest_fixtures::PersonView;
/// // Inline reads: reborrow once, then use plain field access.
/// fn log(owned: &OwnedView<PersonView<'static>>) {
///     let person = owned.reborrow();
///     println!("{}", person.name);
/// }
///
/// // Returning a borrowed field: the result is tied to the OwnedView.
/// fn name<'a>(owned: &'a OwnedView<PersonView<'static>>) -> &'a str {
///     owned.reborrow().name  // &'a str tied to the OwnedView's lifetime
/// }
/// ```
///
/// View fields are not reachable directly on the handle — this fails to
/// compile rather than handing out a `'static` borrow into the buffer:
///
/// ```compile_fail,E0609
/// # use buffa::view::OwnedView;
/// # use buffa::__doctest_fixtures::PersonView;
/// fn field(owned: &OwnedView<PersonView<'static>>) -> &'static str {
///     owned.name // error[E0609]: no field `name` on type `&OwnedView<...>`
/// }
/// ```
///
/// # Safety
///
/// Internally, `OwnedView` extends the view's lifetime to `'static` via
/// `transmute` in its constructors. This is sound because:
///
/// 1. [`Bytes`] is reference-counted — its heap data pointer is stable across
///    moves. The view's borrows always point into valid memory.
/// 2. [`Bytes`] is immutable — the underlying data cannot be modified while
///    borrowed.
/// 3. A manual [`Drop`] impl explicitly drops the view before the bytes,
///    ensuring no dangling references during cleanup. The view field uses
///    [`ManuallyDrop`](core::mem::ManuallyDrop) to prevent the automatic
///    drop from running out of order.
///
/// [`reborrow`](OwnedView::reborrow) is a plain Rust subtype coercion (no
/// `unsafe`, no pointer cast): the [`ViewReborrow`] trait method coerces
/// `&'b FooView<'static>` into `&'b FooView<'b>` via standard lifetime
/// variance for covariant view types. See [`ViewReborrow`]'s docs for the
/// soundness argument.
pub struct OwnedView<V> {
    // INVARIANT: `view` borrows from `bytes`. The `Drop` impl ensures
    // `view` is dropped before `bytes`. `ManuallyDrop` prevents the compiler
    // from dropping `view` automatically — our `Drop` impl handles it.
    //
    // CONSTRUCTORS: any constructor added here MUST ensure the view's
    // borrows point into `self.bytes` (not into caller-owned memory).
    // The auto-`Send`/`Sync` derivation is only sound under that invariant
    // — there is no longer a `V: 'static` bound on `Send` to act as a
    // second gate. See the comment block above `send_sync_assertions` below.
    view: core::mem::ManuallyDrop<V>,
    bytes: Bytes,
}

impl<V> Drop for OwnedView<V> {
    fn drop(&mut self) {
        // SAFETY: `view` borrows from `bytes`. We must drop the view before
        // bytes is dropped. `ManuallyDrop::drop` runs V's destructor in place
        // without moving it. After this, `bytes` drops automatically via the
        // compiler-generated drop glue.
        unsafe {
            core::mem::ManuallyDrop::drop(&mut self.view);
        }
    }
}

impl<V> OwnedView<V>
where
    V: MessageView<'static>,
{
    /// Decode a view from a [`Bytes`] buffer.
    ///
    /// The view borrows directly from the buffer's data. Because [`Bytes`] is
    /// reference-counted and its data pointer is stable across moves, the
    /// view's borrows remain valid for the lifetime of this `OwnedView`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the buffer contains invalid protobuf data.
    pub fn decode(bytes: Bytes) -> Result<Self, DecodeError> {
        // SAFETY: `Bytes` is StableDeref — its heap data never moves or is
        // freed while we hold the `Bytes` value. We hold it in `self.bytes`,
        // and drop order guarantees `view` drops first.
        let view = unsafe {
            let slice: &'static [u8] = core::mem::transmute::<&[u8], &'static [u8]>(&bytes);
            V::decode_view(slice)?
        };
        Ok(Self {
            view: core::mem::ManuallyDrop::new(view),
            bytes,
        })
    }

    /// Decode a view with custom [`DecodeOptions`](crate::DecodeOptions)
    /// (recursion limit, max message size).
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the buffer is invalid or exceeds the
    /// configured limits.
    pub fn decode_with_options(
        bytes: Bytes,
        opts: &crate::DecodeOptions,
    ) -> Result<Self, DecodeError> {
        // SAFETY: Same invariants as `decode` — see above.
        let view = unsafe {
            let slice: &'static [u8] = core::mem::transmute::<&[u8], &'static [u8]>(&bytes);
            opts.decode_view::<V>(slice)?
        };
        Ok(Self {
            view: core::mem::ManuallyDrop::new(view),
            bytes,
        })
    }

    /// Create an `OwnedView` from an owned message by encoding then decoding.
    ///
    /// This performs a full **encode → decode** round-trip: the message is
    /// serialized to protobuf bytes, then a zero-copy view is decoded from
    /// those bytes. This is useful when the original wire bytes are not
    /// available (e.g., after JSON deserialization or programmatic construction),
    /// but note the cost: one allocation + O(n) encode + O(n) decode.
    ///
    /// For the common case where you already have wire bytes, prefer
    /// [`decode`](Self::decode) instead.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the re-encoded bytes are somehow invalid
    /// (should not happen for well-formed messages).
    pub fn from_owned(msg: &V::Owned) -> Result<Self, DecodeError> {
        let bytes = Bytes::from(msg.encode_to_vec());
        Self::decode(bytes)
    }

    /// Convert the view to the corresponding owned message type.
    ///
    /// `bytes::Bytes`-typed fields are produced via [`Bytes::slice_ref`]
    /// into the retained buffer (zero-copy); other borrowed fields are
    /// allocated and copied.
    ///
    /// # Errors
    ///
    /// Returns an error if re-materializing preserved unknown fields fails
    /// (see [`MessageView::to_owned_message`]).
    pub fn to_owned_message(&self) -> Result<V::Owned, DecodeError> {
        self.view.to_owned_from_source(Some(&self.bytes))
    }

    /// Get a reference to the underlying bytes buffer.
    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Create an `OwnedView` from a buffer and a pre-decoded view.
    ///
    /// This avoids re-decoding when you already hold a decoded view and want
    /// to wrap it for `'static` use.
    ///
    /// # Safety
    ///
    /// The caller must ensure that **all** borrows in `view` point into the
    /// data region of `bytes`. In practice, `view` must have been decoded
    /// from `bytes` (or a sub-slice that `bytes` fully contains). Violating
    /// this invariant causes undefined behavior (dangling references).
    pub unsafe fn from_parts(bytes: Bytes, view: V) -> Self {
        Self {
            view: core::mem::ManuallyDrop::new(view),
            bytes,
        }
    }

    /// Consume the `OwnedView`, returning the underlying [`Bytes`] buffer.
    ///
    /// The view is dropped before the buffer is returned.
    pub fn into_bytes(mut self) -> Bytes {
        // SAFETY: Drop the view first (while bytes data is still alive),
        // then read bytes out via ptr::read, then forget self to prevent
        // the Drop impl from double-dropping the view.
        unsafe {
            core::mem::ManuallyDrop::drop(&mut self.view);
            let bytes = core::ptr::read(&self.bytes);
            core::mem::forget(self);
            bytes
        }
    }

    /// Reborrow the view with a lifetime tied to `&'b self`.
    ///
    /// `OwnedView<V>` stores `V` with a `'static` lifetime — the actual borrows
    /// point into `self`'s internal [`Bytes`] buffer and are only valid while
    /// `self` is alive. `reborrow` makes that real lifetime visible to the borrow
    /// checker: the returned `&'b V::Reborrowed<'b>` cannot outlive `&'b self`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use buffa::view::OwnedView;
    /// # use buffa::__doctest_fixtures::PersonView;
    /// fn handler<'a>(req: &'a OwnedView<PersonView<'static>>) -> &'a str {
    ///     // The explicit annotation is for emphasis; inference works without it.
    ///     // If you need to name the lifetime, store the reborrow in a `let` first.
    ///     let req_view: &PersonView<'a> = req.reborrow();
    ///     req_view.name  // zero-copy from the OwnedView's buffer
    /// }
    /// ```
    ///
    /// The returned reference is tied to `&'b self` — the borrow checker
    /// prevents the reborrowed view from outliving the `OwnedView`:
    ///
    /// ```compile_fail,E0597
    /// # use buffa::view::OwnedView;
    /// # use buffa::__doctest_fixtures::PersonView;
    /// let name: &str;
    /// {
    ///     // SAFETY: empty Bytes, no borrows — safe to construct directly.
    ///     let owned = unsafe {
    ///         OwnedView::<PersonView<'static>>::from_parts(
    ///             ::buffa::bytes::Bytes::new(),
    ///             PersonView::default(),
    ///         )
    ///     };
    ///     name = owned.reborrow().name; // error[E0597]: `owned` does not live long enough
    /// }
    /// println!("{name}"); // name is dangling — borrow checker rejects this
    /// ```
    ///
    /// # How it works
    ///
    /// The trait method [`ViewReborrow::reborrow`] is a plain Rust subtype
    /// coercion: `&'b V` (where `V = FooView<'static>`) flows into the
    /// return slot `&'b V::Reborrowed<'b>` (= `&'b FooView<'b>`). Variance
    /// makes this safe — covariant view types narrow `'static` down to
    /// `'b` automatically. No `unsafe`, no pointer cast, no layout
    /// assertions. `OwnedView`'s own invariant (every borrow in `view`
    /// points into `self.bytes`, established by `decode` or upheld by the
    /// `unsafe from_parts` caller) guarantees the pointed-to data lives
    /// at least as long as `'b`.
    #[must_use = "reborrow returns a tied-lifetime view; discarding it is a no-op"]
    pub fn reborrow<'b>(&'b self) -> &'b V::Reborrowed<'b>
    where
        V: ViewReborrow,
    {
        V::reborrow(&self.view)
    }
}

// Deliberately NO `Deref<Target = V>` impl: `V` is `FooView<'static>`, so a
// `&V` return would expose the synthetic `'static` on every borrowed field
// and let it escape the OwnedView's scope (dangling once the buffer drops).
// All access goes through `reborrow()`, which ties the borrow to `&self`.

impl<V> core::fmt::Debug for OwnedView<V>
where
    V: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (*self.view).fmt(f)
    }
}

impl<V> Clone for OwnedView<V>
where
    V: Clone,
{
    fn clone(&self) -> Self {
        // SAFETY: `Bytes::clone()` is a refcount bump — both the original and
        // the clone share the same backing heap allocation. The cloned view's
        // `'static` references remain valid because they point into data that
        // is now kept alive by the cloned `Bytes` handle. This would be
        // unsound if `Bytes::clone()` performed a deep copy to a new address.
        Self {
            view: self.view.clone(),
            bytes: self.bytes.clone(),
        }
    }
}

impl<V> PartialEq for OwnedView<V>
where
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        *self.view == *other.view
    }
}

impl<V> Eq for OwnedView<V> where V: Eq {}

/// Serialize an `OwnedView<V>` by delegating to the inner view's `Serialize`
/// impl.
///
/// Equivalent to serializing `owned_view.reborrow()` directly, so
/// `serde_json::to_string(&owned_view)` works on the handle itself. When
/// `V` is a buffa-generated view with `generate_json` enabled, this produces
/// protobuf JSON; the impl itself just forwards to whatever `V::serialize`
/// does.
///
/// Only available when the `json` feature is enabled.
#[cfg(feature = "json")]
impl<V: ::serde::Serialize> ::serde::Serialize for OwnedView<V> {
    fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ::serde::Serialize::serialize(&*self.view, s)
    }
}

// `OwnedView<V>` is auto-`Send`/`Sync` when `V` is — `ManuallyDrop<V>` and
// `Bytes` both forward auto-traits. No manual `unsafe impl` is needed, and
// adding one with a `V: 'static` bound is actively harmful: it is precisely
// what triggers E0477 when `async fn` is used in a trait impl against an
// RPITIT `+ Send` return type (rust-lang/rust#128095). The RPITIT desugaring
// introduces a fresh lifetime for the `'static` in `FooView<'static>`, and
// then cannot prove that fresh lifetime satisfies `'static` to discharge the
// manual impl's bound.
//
// The bound was defensive — intended to prevent `OwnedView<FooView<'short>>`
// from being `Send` when the view borrows from something outside `self.bytes`.
// But that type is already unconstructible: `::decode()` and
// `::decode_with_options()` are gated on `V: MessageView<'static>`, and the
// fields are private. The short-lifetime case the bound guards against cannot
// exist in safe code.
//
// Auto-trait soundness: `Bytes` is `Send + Sync`. The view's `&'static [u8]`
// borrows point into `Bytes`'s heap allocation, which is immutable,
// `StableDeref`, and moves with the struct. Sending the whole pair to another
// thread preserves the invariant. The `'static` in `V` being a lie is about
// *where* the reference points, not about thread safety.
#[cfg(test)]
mod send_sync_assertions {
    use super::*;
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    // Any `V: Send + Sync` suffices — generated `FooView<'static>` types are
    // auto-`Send + Sync` via their `&'static str` / `&'static [u8]` fields.
    #[allow(dead_code)]
    fn owned_view_is_send_sync<V: Send + Sync>() {
        assert_send::<OwnedView<V>>();
        assert_sync::<OwnedView<V>>();
    }

    // Concrete-type regression: `TinyView` is declared in the `tests` module
    // below and has the same shape as generated view types (one `&'a str`).
    #[allow(dead_code)]
    fn owned_tiny_view_is_send_sync() {
        assert_send::<OwnedView<super::tests::TinyView<'static>>>();
        assert_sync::<OwnedView<super::tests::TinyView<'static>>>();
    }

    // `ViewReborrow::Reborrowed<'b>` must also be Send + Sync so that a
    // reborrowed view can be passed across threads (e.g. into a Tokio task).
    #[allow(dead_code)]
    fn reborrowed_view_is_send_sync<V>()
    where
        V: ViewReborrow + Send + Sync,
        for<'b> V::Reborrowed<'b>: Send + Sync,
    {
        assert_send::<OwnedView<V>>();
        assert_sync::<OwnedView<V>>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MessageFieldView ─────────────────────────────────────────────────

    #[test]
    fn message_field_view_default_is_unset() {
        let v: MessageFieldView<i32> = MessageFieldView::default();
        assert!(v.is_unset());
        assert!(!v.is_set());
        assert_eq!(v.as_option(), None);
    }

    #[test]
    fn message_field_view_set_value() {
        let v = MessageFieldView::set(42);
        assert!(v.is_set());
        assert!(!v.is_unset());
        assert_eq!(v.as_option(), Some(&42));
    }

    #[test]
    fn message_field_view_with_non_copy_type() {
        let v = MessageFieldView::set(alloc::string::String::from("hello"));
        assert!(v.is_set());
        assert_eq!(v.as_option().map(|s| s.as_str()), Some("hello"));

        let unset: MessageFieldView<alloc::string::String> = MessageFieldView::unset();
        assert!(unset.is_unset());
        assert_eq!(unset.as_option(), None);
    }

    // ── MessageFieldView Deref ─────────────────────────────────────────

    /// A trivial view type for testing MessageFieldView Deref.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub(super) struct TinyView<'a> {
        pub value: &'a str,
    }

    impl<'v> DefaultViewInstance for TinyView<'v> {
        fn default_view_instance<'a>() -> &'a Self
        where
            Self: 'a,
        {
            static INST: crate::__private::OnceBox<TinyView<'static>> =
                crate::__private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(<TinyView<'static>>::default()))
        }
    }

    #[test]
    fn message_field_view_deref_set() {
        let v = MessageFieldView::set(TinyView { value: "hello" });
        // Deref gives access to the inner view
        assert_eq!(v.value, "hello");
    }

    #[test]
    fn message_field_view_deref_unset_returns_default() {
        let v: MessageFieldView<TinyView<'_>> = MessageFieldView::unset();
        // Deref transparently returns the default instance
        assert_eq!(v.value, "");
    }

    #[test]
    fn message_field_view_deref_chained_access() {
        // Simulates accessing a nested field through an unset sub-message
        let v: MessageFieldView<TinyView<'_>> = MessageFieldView::unset();
        let len = v.value.len();
        assert_eq!(len, 0);
    }

    // ── MessageFieldView PartialEq (wire-equivalent) ───────────────────

    #[test]
    fn message_field_view_equality() {
        // None = Unset, Some(s) = Set(TinyView { value: s }).
        // TinyView::default() has value == "", so Some("") encodes Set(default).
        fn mk(c: Option<&str>) -> MessageFieldView<TinyView<'_>> {
            match c {
                None => MessageFieldView::unset(),
                Some(v) => MessageFieldView::set(TinyView { value: v }),
            }
        }

        #[rustfmt::skip]
        let cases: &[(Option<&str>, Option<&str>, bool)] = &[
            // Wire-equivalent semantics: Unset == Set(default), matching
            // MessageField::PartialEq on the owned side.
            (None,      None,        true ),  // Unset == Unset
            (None,      Some(""),    true ),  // Unset == Set(default)
            (Some(""),  None,        true ),  // Set(default) == Unset (symmetric)
            (Some(""),  Some(""),    true ),  // Set(default) == Set(default)
            (None,      Some("x"),   false),  // Unset != Set(nondefault)
            (Some("x"), None,        false),  // Set(nondefault) != Unset (symmetric)
            (Some("x"), Some("x"),   true ),  // Set == Set (same)
            (Some("x"), Some("y"),   false),  // Set != Set (different)
        ];

        for &(l, r, expect) in cases {
            assert_eq!(
                mk(l) == mk(r),
                expect,
                "({l:?} == {r:?}) should be {expect}"
            );
        }
    }

    // ── RepeatedView ─────────────────────────────────────────────────────

    #[test]
    fn repeated_view_new_and_accessors() {
        let rv = RepeatedView::new(alloc::vec![10, 20, 30]);
        assert_eq!(rv.len(), 3);
        assert!(!rv.is_empty());
        assert_eq!(&*rv, &[10, 20, 30]);
    }

    #[test]
    fn repeated_view_default_is_empty() {
        let rv: RepeatedView<'_, u8> = RepeatedView::default();
        assert!(rv.is_empty());
        assert_eq!(rv.len(), 0);
    }

    #[test]
    fn repeated_view_push_and_iter() {
        let mut rv = RepeatedView::<i32>::default();
        rv.push(1);
        rv.push(2);
        let collected: alloc::vec::Vec<_> = rv.iter().copied().collect();
        assert_eq!(collected, alloc::vec![1, 2]);
    }

    // ── UnknownFieldsView::push_record (coalescing + limit) ────────────

    /// A test context at full depth with `n` unknown-field slots, leaking
    /// the cell so the context can outlive this helper.
    fn record_ctx(n: usize) -> crate::DecodeContext<'static> {
        let limit = alloc::boxed::Box::leak(alloc::boxed::Box::new(core::cell::Cell::new(n)));
        crate::DecodeContext::new(crate::RECURSION_LIMIT, limit)
    }

    #[test]
    fn push_record_coalesces_adjacent_records() {
        // Buffer holds three consecutive 2-byte records.
        let buf: &[u8] = &[0x08, 0x00, 0x08, 0x01, 0x08, 0x02];
        let ctx = record_ctx(1); // one slot is enough for a contiguous run
        let mut ufv = UnknownFieldsView::new();
        ufv.push_record(&buf[0..], 2, ctx).unwrap();
        ufv.push_record(&buf[2..], 2, ctx).unwrap();
        ufv.push_record(&buf[4..], 2, ctx).unwrap();
        assert_eq!(ufv.encoded_len(), 6);
        let mut out = alloc::vec::Vec::new();
        ufv.write_to(&mut out);
        assert_eq!(out, buf);
        assert_eq!(ctx.remaining_unknown_fields(), 0, "single slot consumed");
    }

    #[test]
    fn push_record_non_adjacent_records_use_separate_slots() {
        let buf: &[u8] = &[0x08, 0x00, 0xFF, 0x08, 0x01];
        let ctx = record_ctx(2);
        let mut ufv = UnknownFieldsView::new();
        ufv.push_record(&buf[0..], 2, ctx).unwrap();
        // Skip buf[2] — the next record is not adjacent to the previous one.
        ufv.push_record(&buf[3..], 2, ctx).unwrap();
        assert_eq!(ufv.encoded_len(), 4);
        assert_eq!(ctx.remaining_unknown_fields(), 0, "two slots consumed");
    }

    #[test]
    fn push_record_enforces_limit_for_new_spans() {
        let buf: &[u8] = &[0x08, 0x00, 0xFF, 0x08, 0x01];
        let ctx = record_ctx(1);
        let mut ufv = UnknownFieldsView::new();
        ufv.push_record(&buf[0..], 2, ctx).unwrap();
        assert_eq!(
            ufv.push_record(&buf[3..], 2, ctx),
            Err(crate::DecodeError::UnknownFieldLimitExceeded)
        );
        // Extending the existing span never needs a slot — even at zero
        // remaining, an adjacent record still coalesces.
        ufv.push_record(&buf[2..], 1, ctx)
            .expect("adjacent record coalesces without a slot");
    }

    #[test]
    fn push_raw_disables_coalescing_for_next_record() {
        let buf: &[u8] = &[0x08, 0x00, 0x08, 0x01];
        let ctx = record_ctx(2);
        let mut ufv = UnknownFieldsView::new();
        ufv.push_raw(&buf[0..2]);
        // Adjacent on the wire, but push_raw cleared the tail, so this must
        // open a fresh span (a manual span has no trusted buffer position).
        ufv.push_record(&buf[2..], 2, ctx).unwrap();
        assert_eq!(ctx.remaining_unknown_fields(), 1);
        assert_eq!(ufv.encoded_len(), 4);
    }

    #[test]
    fn push_record_rejects_span_past_tail_end() {
        let buf: &[u8] = &[0x08, 0x00];
        let ctx = record_ctx(1);
        let mut ufv = UnknownFieldsView::new();
        assert_eq!(
            ufv.push_record(buf, 3, ctx),
            Err(crate::DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn coalesced_span_to_owned_parses_every_record() {
        let buf: &[u8] = &[0x08, 0x00, 0x08, 0x01, 0x08, 0x02];
        let ctx = record_ctx(3);
        let mut ufv = UnknownFieldsView::new();
        for i in 0..3 {
            ufv.push_record(&buf[2 * i..], 2, ctx).unwrap();
        }
        let owned = ufv.to_owned().unwrap();
        assert_eq!(owned.iter().count(), 3, "all records parsed");
    }

    #[test]
    fn to_owned_enforces_decode_time_allowance() {
        // Decoded under an allowance of 1: the coalesced span holds three
        // records, so materializing them as owned fields must fail — the
        // decode-time limit carries through conversion.
        let buf: &[u8] = &[0x08, 0x00, 0x08, 0x01, 0x08, 0x02];
        let ctx = record_ctx(1);
        let mut ufv = UnknownFieldsView::new();
        for i in 0..3 {
            ufv.push_record(&buf[2 * i..], 2, ctx).unwrap();
        }
        assert_eq!(
            ufv.to_owned(),
            Err(crate::DecodeError::UnknownFieldLimitExceeded)
        );
    }

    #[test]
    fn to_owned_of_manual_view_uses_default_allowance() {
        // push_raw leaves no captured allowance; to_owned falls back to the
        // default limit.
        let mut ufv = UnknownFieldsView::new();
        ufv.push_raw(&[0x08, 0x00]);
        let owned = ufv.to_owned().unwrap();
        assert_eq!(owned.iter().count(), 1);
    }

    #[test]
    fn repeated_view_reserve_grows_capacity() {
        let mut rv = RepeatedView::<u32>::default();
        rv.reserve(64);
        assert!(rv.elements.capacity() >= 64);
        // Reserve must not produce visible elements.
        assert!(rv.is_empty());
        // reserve(0) is a no-op and must not panic.
        rv.reserve(0);
        // Reserve after pushes adds capacity above current len.
        rv.push(1);
        rv.push(2);
        rv.reserve(100);
        assert_eq!(rv.len(), 2);
        assert!(rv.elements.capacity() >= 102);
        // Subsequent reserve calls must not corrupt the existing elements.
        let collected: alloc::vec::Vec<_> = rv.iter().copied().collect();
        assert_eq!(collected, alloc::vec![1, 2]);
    }

    #[test]
    fn repeated_view_with_borrowed_str() {
        let data = alloc::string::String::from("hello world");
        let parts: alloc::vec::Vec<&str> = data.split_whitespace().collect();
        let rv = RepeatedView::new(parts);
        assert_eq!(rv.len(), 2);
        assert_eq!(rv[0], "hello");
        assert_eq!(rv[1], "world");
    }

    #[test]
    fn repeated_view_into_iter_by_ref() {
        let rv = RepeatedView::new(alloc::vec![1, 2, 3]);
        let sum: i32 = (&rv).into_iter().sum();
        assert_eq!(sum, 6);
        // `for x in &rv` syntax works:
        let mut count = 0;
        for _ in &rv {
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn repeated_view_into_iter_by_value() {
        let rv = RepeatedView::new(alloc::vec![
            alloc::string::String::from("a"),
            alloc::string::String::from("b"),
        ]);
        let collected: alloc::vec::Vec<_> = rv.into_iter().collect();
        assert_eq!(collected, alloc::vec!["a".to_string(), "b".to_string()]);
    }

    // ── MapView ──────────────────────────────────────────────────────────

    #[test]
    fn map_view_get_with_borrow() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("apples", 3);
        mv.push("bananas", 5);

        // Ergonomic: get("key") works via Borrow<str> on &str
        assert_eq!(mv.get("apples"), Some(&3));
        assert_eq!(mv.get("bananas"), Some(&5));
        assert_eq!(mv.get("oranges"), None);

        // Old style still works
        assert_eq!(mv.get(&"apples"), Some(&3));
    }

    #[test]
    fn map_view_contains_key_with_borrow() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("key", 1);

        assert!(mv.contains_key("key"));
        assert!(!mv.contains_key("missing"));
    }

    #[test]
    fn map_view_get_last_write_wins() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("x", 1);
        mv.push("x", 2);
        assert_eq!(mv.get("x"), Some(&2));
    }

    #[test]
    fn map_view_iter_unique_dedups_last_write_wins() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("a", 1);
        mv.push("b", 2);
        mv.push("a", 3); // duplicate key — only this entry survives for "a"
        mv.push("c", 4);
        mv.push("b", 5); // duplicate key — only this entry survives for "b"

        assert_eq!(mv.len(), 5, "iter() preserves all wire entries");
        assert_eq!(mv.len_unique(), 3, "len_unique() counts distinct keys");

        let unique: alloc::vec::Vec<_> = mv.iter_unique().collect();
        assert_eq!(unique, [&("a", 3), &("c", 4), &("b", 5)]);
    }

    #[test]
    fn map_view_iter_unique_all_duplicates() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("a", 1);
        mv.push("a", 2);
        mv.push("a", 3);
        assert_eq!(mv.len_unique(), 1);
        assert_eq!(
            mv.iter_unique().collect::<alloc::vec::Vec<_>>(),
            [&("a", 3)]
        );
    }

    #[test]
    fn map_view_iter_unique_no_duplicates() {
        let mut mv = MapView::<i32, &str>::default();
        mv.push(1, "x");
        mv.push(2, "y");
        assert_eq!(mv.len_unique(), 2);
        assert_eq!(
            mv.iter_unique().collect::<alloc::vec::Vec<_>>(),
            [&(1, "x"), &(2, "y")]
        );
    }

    #[test]
    fn map_view_iter_unique_empty() {
        let mv = MapView::<&str, i32>::default();
        assert_eq!(mv.len_unique(), 0);
        assert_eq!(mv.iter_unique().count(), 0);
    }

    #[test]
    fn map_view_keys_and_values() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("a", 1);
        mv.push("b", 2);
        mv.push("c", 3);

        let keys: alloc::vec::Vec<_> = mv.keys().copied().collect();
        assert_eq!(keys, alloc::vec!["a", "b", "c"]);

        let values: alloc::vec::Vec<_> = mv.values().copied().collect();
        assert_eq!(values, alloc::vec![1, 2, 3]);
    }

    #[test]
    fn map_view_keys_and_values_empty() {
        let mv = MapView::<&str, i32>::default();
        assert_eq!(mv.keys().count(), 0);
        assert_eq!(mv.values().count(), 0);
    }

    #[test]
    fn map_view_into_iter_collect_to_hashmap() {
        let mut mv = MapView::<&str, i32>::default();
        mv.push("a", 1);
        mv.push("b", 2);
        mv.push("a", 3); // duplicate — last-write-wins on collect
        let m: crate::__private::HashMap<&str, i32> = mv.into_iter().collect();
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("a"), Some(&3)); // last value kept
        assert_eq!(m.get("b"), Some(&2));
    }

    // ── bytes_from_source ────────────────────────────────────────────────

    #[test]
    fn bytes_from_source_none_copies() {
        let data: &[u8] = b"hello";
        let out = bytes_from_source(None, data);
        assert_eq!(&out[..], data);
        assert_ne!(out.as_ptr(), data.as_ptr()); // distinct allocation
    }

    #[test]
    fn bytes_from_source_some_within_is_slice_ref() {
        let parent = Bytes::copy_from_slice(b"hello world");
        let slice = &parent[6..11];
        let out = bytes_from_source(Some(&parent), slice);
        assert_eq!(&out[..], b"world");
        // slice_ref shares the same backing storage — same pointer.
        assert_eq!(out.as_ptr(), slice.as_ptr());
    }

    #[test]
    fn bytes_from_source_some_outside_falls_back_to_copy() {
        let parent = Bytes::copy_from_slice(b"hello");
        let outside: &[u8] = b"world"; // static, not in `parent`
        let out = bytes_from_source(Some(&parent), outside);
        assert_eq!(&out[..], b"world");
        assert_ne!(out.as_ptr(), outside.as_ptr());
    }

    #[test]
    fn bytes_from_source_empty_returns_new() {
        let parent = Bytes::copy_from_slice(b"hello");
        assert!(bytes_from_source(Some(&parent), &[]).is_empty());
        assert!(bytes_from_source(None, &[]).is_empty());
    }

    #[test]
    fn bytes_from_source_full_range() {
        let parent = Bytes::copy_from_slice(b"hello");
        let out = bytes_from_source(Some(&parent), &parent[..]);
        assert_eq!(out.as_ptr(), parent.as_ptr());
        assert_eq!(out.len(), parent.len());
    }

    // ── UnknownFieldsView ────────────────────────────────────────────────

    #[test]
    fn unknown_fields_view_new_is_empty() {
        let uf = UnknownFieldsView::new();
        assert!(uf.is_empty());
        assert_eq!(uf.encoded_len(), 0);
    }

    #[test]
    fn unknown_fields_view_push_raw_and_encoded_len() {
        let mut uf = UnknownFieldsView::new();
        uf.push_raw(&[0x08, 0x01]); // field 1, varint 1
        uf.push_raw(&[0x10, 0x02]); // field 2, varint 2
        assert!(!uf.is_empty());
        assert_eq!(uf.encoded_len(), 4);
    }

    #[test]
    fn unknown_fields_view_to_owned_single_field() {
        // Build a valid unknown field: tag for field 99, varint wire type,
        // value 42.  Tag = (99 << 3) | 0 = 792 = varint bytes [0x98, 0x06].
        let span: &[u8] = &[0x98, 0x06, 0x2A];
        let mut uf = UnknownFieldsView::new();
        uf.push_raw(span);

        let owned = uf.to_owned().expect("valid wire data");
        assert_eq!(owned.len(), 1);
        let field = owned.iter().next().unwrap();
        assert_eq!(field.number, 99);
        assert_eq!(
            field.data,
            crate::unknown_fields::UnknownFieldData::Varint(42)
        );
    }

    #[test]
    fn unknown_fields_view_to_owned_multiple_fields() {
        let mut uf = UnknownFieldsView::new();
        // Field 1, varint, value 7:  tag = (1<<3)|0 = 0x08, value = 0x07
        uf.push_raw(&[0x08, 0x07]);
        // Field 2, fixed32, value 0x01020304:
        //   tag = (2<<3)|5 = 0x15, then 4 LE bytes
        uf.push_raw(&[0x15, 0x04, 0x03, 0x02, 0x01]);

        let owned = uf.to_owned().expect("valid wire data");
        assert_eq!(owned.len(), 2);

        let mut it = owned.iter();
        let f1 = it.next().unwrap();
        assert_eq!(f1.number, 1);
        assert_eq!(f1.data, crate::unknown_fields::UnknownFieldData::Varint(7));

        let f2 = it.next().unwrap();
        assert_eq!(f2.number, 2);
        assert_eq!(
            f2.data,
            crate::unknown_fields::UnknownFieldData::Fixed32(0x01020304)
        );
    }

    #[test]
    fn unknown_fields_view_to_owned_malformed_returns_error() {
        // A truncated tag (high continuation bit, then EOF).
        let mut uf = UnknownFieldsView::new();
        uf.push_raw(&[0x80]);
        assert!(uf.to_owned().is_err());
    }

    // ── OwnedView ──────────────────────────────────────────────────────

    // Minimal types to test OwnedView without depending on generated code.

    use crate::message::Message;

    /// A trivial "message" for the owned side of the view contract.
    #[derive(Clone, Debug, Default, PartialEq)]
    struct SimpleMessage {
        pub id: i32,
        pub name: alloc::string::String,
    }

    impl crate::DefaultInstance for SimpleMessage {
        fn default_instance() -> &'static Self {
            static INST: crate::__private::OnceBox<SimpleMessage> =
                crate::__private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(SimpleMessage::default()))
        }
    }

    impl crate::Message for SimpleMessage {
        fn compute_size(&self, _cache: &mut crate::SizeCache) -> u32 {
            let mut size = 0u32;
            if self.id != 0 {
                size += 1 + crate::types::int32_encoded_len(self.id) as u32;
            }
            if !self.name.is_empty() {
                size += 1 + crate::types::string_encoded_len(&self.name) as u32;
            }
            size
        }

        fn write_to(&self, _cache: &mut crate::SizeCache, buf: &mut impl bytes::BufMut) {
            if self.id != 0 {
                crate::encoding::Tag::new(1, crate::encoding::WireType::Varint).encode(buf);
                crate::types::encode_int32(self.id, buf);
            }
            if !self.name.is_empty() {
                crate::encoding::Tag::new(2, crate::encoding::WireType::LengthDelimited)
                    .encode(buf);
                crate::types::encode_string(&self.name, buf);
            }
        }

        fn merge_field(
            &mut self,
            tag: crate::encoding::Tag,
            buf: &mut impl bytes::Buf,
            _ctx: crate::DecodeContext<'_>,
        ) -> Result<(), DecodeError> {
            match tag.field_number() {
                1 => self.id = crate::types::decode_int32(buf)?,
                2 => crate::types::merge_string(&mut self.name, buf)?,
                _ => crate::encoding::skip_field(tag, buf)?,
            }
            Ok(())
        }

        fn clear(&mut self) {
            self.id = 0;
            self.name.clear();
        }
    }

    /// A zero-copy view of `SimpleMessage`. Borrows `name` as `&str`.
    #[derive(Clone, Debug, Default, PartialEq)]
    struct SimpleMessageView<'a> {
        pub id: i32,
        pub name: &'a str,
    }

    impl ViewReborrow for SimpleMessageView<'static> {
        type Reborrowed<'b> = SimpleMessageView<'b>;
        fn reborrow<'b>(this: &'b Self) -> &'b Self::Reborrowed<'b> {
            this
        }
    }

    impl<'a> MessageView<'a> for SimpleMessageView<'a> {
        type Owned = SimpleMessage;

        fn decode_view(buf: &'a [u8]) -> Result<Self, DecodeError> {
            let mut view = SimpleMessageView::default();
            let mut cursor: &'a [u8] = buf;
            while !cursor.is_empty() {
                let tag = crate::encoding::Tag::decode(&mut cursor)?;
                match tag.field_number() {
                    1 => view.id = crate::types::decode_int32(&mut cursor)?,
                    2 => view.name = crate::types::borrow_str(&mut cursor)?,
                    _ => crate::encoding::skip_field(tag, &mut cursor)?,
                }
            }
            Ok(view)
        }

        fn to_owned_message(&self) -> Result<SimpleMessage, DecodeError> {
            Ok(SimpleMessage {
                id: self.id,
                name: self.name.into(),
            })
        }
    }

    impl<'a> ViewEncode<'a> for SimpleMessageView<'a> {
        fn compute_size(&self, _cache: &mut crate::SizeCache) -> u32 {
            let mut size = 0u32;
            if self.id != 0 {
                size += 1 + crate::types::int32_encoded_len(self.id) as u32;
            }
            if !self.name.is_empty() {
                size += 1 + crate::types::string_encoded_len(self.name) as u32;
            }
            size
        }

        fn write_to(&self, _cache: &mut crate::SizeCache, buf: &mut impl bytes::BufMut) {
            if self.id != 0 {
                crate::encoding::Tag::new(1, crate::encoding::WireType::Varint).encode(buf);
                crate::types::encode_int32(self.id, buf);
            }
            if !self.name.is_empty() {
                crate::encoding::Tag::new(2, crate::encoding::WireType::LengthDelimited)
                    .encode(buf);
                crate::types::encode_string(self.name, buf);
            }
        }
    }

    /// Encode a SimpleMessage to Bytes for testing.
    fn encode_simple(id: i32, name: &str) -> Bytes {
        let msg = SimpleMessage {
            id,
            name: name.into(),
        };
        Bytes::from(msg.encode_to_vec())
    }

    #[test]
    fn owned_view_decode_and_reborrow() {
        let bytes = encode_simple(42, "hello");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();

        // Field access via reborrow — the borrow is tied to `view`.
        assert_eq!(view.reborrow().id, 42);
        assert_eq!(view.reborrow().name, "hello");
    }

    #[test]
    fn owned_view_to_owned_message() {
        let bytes = encode_simple(7, "world");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let owned = view.to_owned_message().unwrap();

        assert_eq!(owned.id, 7);
        assert_eq!(owned.name, "world");
    }

    #[test]
    fn owned_view_debug_delegates_to_view() {
        let bytes = encode_simple(1, "test");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let debug = alloc::format!("{:?}", view);
        assert!(debug.contains("test"));
        assert!(debug.contains("1"));
    }

    #[test]
    fn owned_view_bytes_accessor() {
        let bytes = encode_simple(5, "data");
        let original_len = bytes.len();
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();

        assert_eq!(view.bytes().len(), original_len);
    }

    #[test]
    fn owned_view_into_bytes_recovers_buffer() {
        let bytes = encode_simple(99, "recover");
        let expected = bytes.clone();
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let recovered = view.into_bytes();

        assert_eq!(recovered, expected);
    }

    #[test]
    fn owned_view_decode_invalid_data_returns_error() {
        // Truncated varint
        let bad = Bytes::from_static(&[0x08, 0x80]);
        let result = OwnedView::<SimpleMessageView<'static>>::decode(bad);
        assert!(result.is_err());
    }

    #[test]
    fn owned_view_empty_message() {
        let bytes = Bytes::from_static(&[]);
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        assert_eq!(view.reborrow().id, 0);
        assert_eq!(view.reborrow().name, "");
    }

    #[test]
    fn owned_view_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<OwnedView<SimpleMessageView<'static>>>();
    }

    #[test]
    fn owned_view_from_owned_roundtrips() {
        let msg = SimpleMessage {
            id: 99,
            name: "roundtrip".into(),
        };
        let view = OwnedView::<SimpleMessageView<'static>>::from_owned(&msg).expect("from_owned");
        assert_eq!(view.reborrow().id, 99);
        assert_eq!(view.reborrow().name, "roundtrip");

        let back = view.to_owned_message().unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn owned_view_decode_with_options() {
        let bytes = encode_simple(42, "opts");
        let opts = crate::DecodeOptions::new().with_max_message_size(1024);
        let view =
            OwnedView::<SimpleMessageView<'static>>::decode_with_options(bytes, &opts).unwrap();
        assert_eq!(view.reborrow().id, 42);
        assert_eq!(view.reborrow().name, "opts");
    }

    #[test]
    fn owned_view_decode_with_options_rejects_oversized() {
        let bytes = encode_simple(42, "too large");
        let opts = crate::DecodeOptions::new().with_max_message_size(2);
        let result = OwnedView::<SimpleMessageView<'static>>::decode_with_options(bytes, &opts);
        assert!(result.is_err());
    }

    #[test]
    fn owned_view_clone_survives_original_drop() {
        let bytes = encode_simple(42, "cloned");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let cloned = view.clone();
        drop(view); // drop original — clone must still be valid
        assert_eq!(cloned.reborrow().id, 42);
        assert_eq!(cloned.reborrow().name, "cloned");
    }

    #[test]
    fn owned_view_clone_equality() {
        let bytes = encode_simple(42, "eq");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let cloned = view.clone();
        assert_eq!(view, cloned);
    }

    #[test]
    fn owned_view_eq_same_data() {
        let a = OwnedView::<SimpleMessageView<'static>>::decode(encode_simple(1, "x")).unwrap();
        let b = OwnedView::<SimpleMessageView<'static>>::decode(encode_simple(1, "x")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn owned_view_ne_different_data() {
        let a = OwnedView::<SimpleMessageView<'static>>::decode(encode_simple(1, "x")).unwrap();
        let b = OwnedView::<SimpleMessageView<'static>>::decode(encode_simple(2, "y")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn owned_view_into_bytes_after_clone() {
        let bytes = encode_simple(42, "test");
        let expected = bytes.clone();
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let cloned = view.clone();
        drop(view); // drop original first
        let recovered = cloned.into_bytes();
        assert_eq!(recovered, expected);
    }

    #[test]
    fn owned_view_drop_count() {
        use core::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        /// A wrapper view that counts drops.
        struct DropCountingView<'a> {
            inner: SimpleMessageView<'a>,
        }

        impl Drop for DropCountingView<'_> {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        impl<'a> MessageView<'a> for DropCountingView<'a> {
            type Owned = SimpleMessage;

            fn decode_view(buf: &'a [u8]) -> Result<Self, DecodeError> {
                Ok(DropCountingView {
                    inner: SimpleMessageView::decode_view(buf)?,
                })
            }

            fn to_owned_message(&self) -> Result<SimpleMessage, DecodeError> {
                self.inner.to_owned_message()
            }
        }

        // Test normal drop: view drops exactly once.
        DROP_COUNT.store(0, Ordering::SeqCst);
        {
            let bytes = encode_simple(1, "drop");
            let _view = OwnedView::<DropCountingView<'static>>::decode(bytes).unwrap();
        }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1, "normal drop");

        // Test into_bytes: view drops exactly once.
        DROP_COUNT.store(0, Ordering::SeqCst);
        {
            let bytes = encode_simple(2, "into");
            let view = OwnedView::<DropCountingView<'static>>::decode(bytes).unwrap();
            let _bytes = view.into_bytes();
        }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1, "into_bytes drop");
    }

    #[test]
    fn owned_view_name_borrows_from_bytes_buffer() {
        let bytes = encode_simple(42, "borrowed");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let buf = view.bytes();
        let buf_start = buf.as_ptr() as usize;
        let buf_end = buf_start + buf.len();
        let name_ptr = view.reborrow().name.as_ptr() as usize;
        assert!(
            (buf_start..buf_end).contains(&name_ptr),
            "view name should point into the Bytes buffer"
        );
    }

    #[test]
    fn owned_view_concurrent_read() {
        use alloc::sync::Arc;

        let bytes = encode_simple(42, "concurrent");
        let view = Arc::new(OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap());
        let handles: alloc::vec::Vec<_> = (0..4)
            .map(|_| {
                let v = Arc::clone(&view);
                std::thread::spawn(move || {
                    assert_eq!(v.reborrow().id, 42);
                    assert_eq!(v.reborrow().name, "concurrent");
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn owned_view_from_parts_roundtrip() {
        let bytes = encode_simple(42, "parts");
        // Decode a view from the bytes, then wrap via from_parts.
        // SAFETY: `view` was decoded from `bytes`.
        let view = unsafe {
            let slice: &'static [u8] = core::mem::transmute::<&[u8], &'static [u8]>(&bytes);
            let decoded = SimpleMessageView::decode_view(slice).unwrap();
            OwnedView::<SimpleMessageView<'static>>::from_parts(bytes, decoded)
        };
        assert_eq!(view.reborrow().id, 42);
        assert_eq!(view.reborrow().name, "parts");
    }

    // ── ViewReborrow / OwnedView::reborrow ───────────────────────────────

    #[test]
    fn reborrow_fields_match_original() {
        let bytes = encode_simple(7, "hello");
        let owned = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let reborrowed: &SimpleMessageView<'_> = owned.reborrow();
        assert_eq!(reborrowed.id, 7);
        assert_eq!(reborrowed.name, "hello");
        // The reborrowed &str must point into the Bytes buffer, not a copy.
        let buf_start = owned.bytes().as_ptr() as usize;
        let buf_end = buf_start + owned.bytes().len();
        assert!((buf_start..buf_end).contains(&(reborrowed.name.as_ptr() as usize)));
    }

    #[test]
    fn reborrow_does_not_consume_owned_view() {
        let bytes = encode_simple(1, "world");
        let owned = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let r1: &SimpleMessageView<'_> = owned.reborrow();
        let r2: &SimpleMessageView<'_> = owned.reborrow();
        assert_eq!(r1.name, r2.name);
        // `owned` still usable here
        assert_eq!(owned.reborrow().name, "world");
    }
    // ── Lazy views ───────────────────────────────────────────────────────

    /// Hand-written lazy view of `SimpleMessage`, shaped like generated
    /// `FooLazyView` code: scalars borrowed, one flat scan, fragment merge.
    #[derive(Clone, Debug, Default, PartialEq)]
    struct SimpleLazyView<'a> {
        pub id: i32,
        pub name: &'a str,
    }

    impl<'a> LazyMessageView<'a> for SimpleLazyView<'a> {
        type Owned = SimpleMessage;

        fn decode_lazy(buf: &'a [u8]) -> Result<Self, DecodeError> {
            let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
            Self::decode_lazy_with_ctx(
                buf,
                crate::DecodeContext::new(crate::RECURSION_LIMIT, &limit),
            )
        }

        fn decode_lazy_with_ctx(
            buf: &'a [u8],
            ctx: crate::DecodeContext<'_>,
        ) -> Result<Self, DecodeError> {
            let mut view = SimpleLazyView::default();
            view.merge_lazy(buf, ctx)?;
            Ok(view)
        }

        fn merge_lazy(
            &mut self,
            buf: &'a [u8],
            ctx: crate::DecodeContext<'_>,
        ) -> Result<(), DecodeError> {
            let mut cursor: &'a [u8] = buf;
            while !cursor.is_empty() {
                let tag = crate::encoding::Tag::decode(&mut cursor)?;
                match tag.field_number() {
                    1 => self.id = crate::types::decode_int32(&mut cursor)?,
                    2 => self.name = crate::types::borrow_str(&mut cursor)?,
                    _ => crate::encoding::skip_field_depth(tag, &mut cursor, ctx.depth())?,
                }
            }
            Ok(())
        }

        fn to_owned_message(&self) -> Result<SimpleMessage, DecodeError> {
            Ok(SimpleMessage {
                id: self.id,
                name: self.name.into(),
            })
        }
    }

    fn full_budget_ctx(cell: &core::cell::Cell<usize>) -> crate::DecodeContext<'_> {
        crate::DecodeContext::new(crate::RECURSION_LIMIT, cell)
    }

    #[test]
    fn lazy_message_field_view_decodes_on_access() {
        let bytes = encode_simple(42, "lazy");
        let unset = LazyMessageFieldView::<SimpleLazyView<'_>>::unset();
        assert!(unset.is_unset());
        assert!(unset.fragments().is_empty());
        assert!(unset.get().unwrap().is_none());
        assert_eq!(unset.get_or_default().unwrap(), SimpleLazyView::default());

        let lazy = LazyMessageFieldView::<SimpleLazyView<'_>>::from_bytes(&bytes);
        assert!(lazy.is_set());
        assert_eq!(lazy.fragments(), &[&bytes[..]]);
        let v = lazy.get().unwrap().expect("set");
        assert_eq!((v.id, v.name), (42, "lazy"));
        // Re-decodes each call (no cache).
        assert_eq!(lazy.get().unwrap().unwrap().id, 42);
        assert_eq!(v.to_owned_message().unwrap().name, "lazy");
    }

    #[test]
    fn lazy_message_field_view_merges_fragments() {
        // A singular message field split across wire occurrences must merge:
        // fragment 1 sets `name`, fragment 2 sets `id`; the merged view has
        // both, matching the eager/owned decoders.
        let frag1 = encode_simple(0, "from-frag-1");
        let frag2 = encode_simple(7, "");
        let cell = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);

        let mut lazy = LazyMessageFieldView::<SimpleLazyView<'_>>::unset();
        lazy.push_fragment(&frag1, full_budget_ctx(&cell));
        assert_eq!(lazy.fragments().len(), 1);
        lazy.push_fragment(&frag2, full_budget_ctx(&cell));
        assert_eq!(lazy.fragments(), &[&frag1[..], &frag2[..]]);

        let v = lazy.get().unwrap().expect("set");
        assert_eq!((v.id, v.name), (7, "from-frag-1"));

        // Later fragments overwrite singular scalars (last-wins).
        let frag3 = encode_simple(9, "final");
        lazy.push_fragment(&frag3, full_budget_ctx(&cell));
        assert_eq!(lazy.fragments().len(), 3);
        let v = lazy.get().unwrap().expect("set");
        assert_eq!((v.id, v.name), (9, "final"));
    }

    #[test]
    fn lazy_message_field_view_records_smallest_budgets() {
        // Budgets recorded across pushes are min-combined, and the recorded
        // allowance replays per access (capture-then-replay).
        let bytes = encode_simple(1, "x");
        let cell_big = core::cell::Cell::new(500);
        let cell_small = core::cell::Cell::new(3);

        let mut lazy = LazyMessageFieldView::<SimpleLazyView<'_>>::unset();
        lazy.push_fragment(&bytes, crate::DecodeContext::new(80, &cell_big));
        lazy.push_fragment(&bytes, crate::DecodeContext::new(50, &cell_small));
        // Decoding succeeds — SimpleLazyView has no unknowns or nesting here;
        // the recorded budgets only bound what access may consume.
        assert!(lazy.get().unwrap().is_some());
    }

    #[test]
    fn lazy_message_field_view_clone_default_debug() {
        let bytes = encode_simple(1, "x");
        let cell = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let mut lazy = LazyMessageFieldView::<SimpleLazyView<'_>>::default();
        assert!(lazy.is_unset());
        lazy.push_fragment(&bytes, full_budget_ctx(&cell));
        lazy.push_fragment(&bytes, full_budget_ctx(&cell));
        let cloned = lazy.clone();
        assert_eq!(cloned.fragments(), lazy.fragments());
        let dbg = alloc::format!("{lazy:?}");
        assert!(dbg.contains("is_set: true"), "{dbg}");
        assert!(dbg.contains("fragments: 2"), "{dbg}");
    }

    #[test]
    fn lazy_message_field_view_malformed_errors_on_access() {
        // 0xFF starts a tag whose varint never terminates — invalid.
        let malformed = [0xFFu8; 3];
        let lazy = LazyMessageFieldView::<SimpleLazyView<'_>>::from_bytes(&malformed);
        // Deferred validation: construction succeeds, access fails.
        assert!(lazy.is_set());
        assert!(lazy.get().is_err());
    }

    #[test]
    fn lazy_repeated_view_decodes_per_element() {
        let b0 = encode_simple(1, "a");
        let b1 = encode_simple(2, "b");
        let cell = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let mut rep = LazyRepeatedView::<SimpleLazyView<'_>>::new();
        assert!(rep.is_empty());
        rep.push_bytes(&b0, full_budget_ctx(&cell));
        rep.push_bytes(&b1, full_budget_ctx(&cell));
        assert_eq!(rep.len(), 2);
        assert_eq!(rep.raw_elements(), &[&b0[..], &b1[..]]);
        assert_eq!(rep.get(0).unwrap().unwrap().name, "a");
        assert_eq!(rep.get(1).unwrap().unwrap().id, 2);
        assert!(rep.get(2).is_none());
    }

    #[test]
    fn lazy_repeated_view_iter() {
        let bufs: alloc::vec::Vec<Bytes> = (1..=3)
            .map(|i| encode_simple(i, core::str::from_utf8(&[b'a' + i as u8]).unwrap()))
            .collect();
        let cell = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let mut rep = LazyRepeatedView::<SimpleLazyView<'_>>::new();
        for b in &bufs {
            rep.push_bytes(b, full_budget_ctx(&cell));
        }

        let iter = rep.iter();
        assert_eq!(iter.len(), 3);
        let ids: alloc::vec::Vec<i32> = iter.map(|r| r.unwrap().id).collect();
        assert_eq!(ids, [1, 2, 3]);

        // IntoIterator for &LazyRepeatedView.
        let names: alloc::vec::Vec<&str> = (&rep).into_iter().map(|r| r.unwrap().name).collect();
        assert_eq!(names, ["b", "c", "d"]);

        // DoubleEndedIterator.
        let rev_ids: alloc::vec::Vec<i32> = rep.iter().rev().map(|r| r.unwrap().id).collect();
        assert_eq!(rev_ids, [3, 2, 1]);
    }

    #[test]
    fn lazy_repeated_view_iter_surfaces_element_errors() {
        let good = encode_simple(1, "ok");
        let malformed = [0xFFu8; 3];
        let cell = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let mut rep = LazyRepeatedView::<SimpleLazyView<'_>>::new();
        rep.push_bytes(&good, full_budget_ctx(&cell));
        rep.push_bytes(&malformed, full_budget_ctx(&cell));
        let results: alloc::vec::Vec<_> = rep.iter().collect();
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        let cloned = rep.clone();
        assert_eq!(cloned.len(), 2);
    }
}

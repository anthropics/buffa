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
//!     #[doc(hidden)] pub __buffa_cached_size: /* internal */,
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
    fn decode_view(buf: &'a [u8]) -> Result<Self, DecodeError>;

    /// Decode a view with a custom recursion depth limit.
    ///
    /// Used by [`DecodeOptions::decode_view`](crate::DecodeOptions::decode_view)
    /// to pass a non-default recursion budget. The default implementation
    /// delegates to [`decode_view`](Self::decode_view) (ignoring the limit);
    /// generated code overrides this to call `_decode_depth(buf, depth)`.
    fn decode_view_with_limit(buf: &'a [u8], _depth: u32) -> Result<Self, DecodeError> {
        Self::decode_view(buf)
    }

    /// Convert this view to the owned message type.
    ///
    /// This allocates and copies all borrowed fields.
    fn to_owned_message(&self) -> Self::Owned;
}

/// Serialize a [`MessageView`] directly from its borrowed fields.
///
/// Symmetric with [`Message`](crate::Message)'s two-pass
/// `compute_size` / `write_to` model, but the `&'a str` / `&'a [u8]` /
/// [`MapView`] / [`RepeatedView`] fields are written by borrow — no
/// owned-struct intermediary, no per-field `String`/`Vec<u8>` allocations.
///
/// Generated `*View<'a>` types implement this trait whenever views are
/// generated (`generate_views(true)`, the default). Each view struct
/// carries a `__buffa_cached_size` field for the cached-size pass — same
/// `AtomicU32`-backed [`CachedSize`](crate::__private::CachedSize) as
/// owned messages, so views remain `Send + Sync`.
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
    /// Compute and cache the encoded byte size of this view.
    ///
    /// Recursively computes sizes for sub-message views and stores them in
    /// each view's `CachedSize` field. Must be called before
    /// [`write_to`](Self::write_to).
    #[must_use = "compute_size has the side-effect of populating cached sizes; \
                  if you only need that, call encode() instead"]
    fn compute_size(&self) -> u32;

    /// Write this view's encoded bytes to a buffer.
    ///
    /// Assumes [`compute_size`](Self::compute_size) has already been called.
    /// Uses cached sizes for length-delimited sub-message headers.
    fn write_to(&self, buf: &mut impl BufMut);

    /// Return the size cached by the most recent [`compute_size`](Self::compute_size).
    #[must_use]
    fn cached_size(&self) -> u32;

    /// Convenience: compute size, then write. Primary view-encode entry point.
    fn encode(&self, buf: &mut impl BufMut) {
        let _ = self.compute_size();
        self.write_to(buf);
    }

    /// Encode this view as a length-delimited byte sequence.
    fn encode_length_delimited(&self, buf: &mut impl BufMut) {
        let len = self.compute_size();
        crate::encoding::encode_varint(len as u64, buf);
        self.write_to(buf);
    }

    /// Encode this view to a new `Vec<u8>`.
    #[must_use]
    fn encode_to_vec(&self) -> alloc::vec::Vec<u8> {
        let size = self.compute_size() as usize;
        let mut buf = alloc::vec::Vec::with_capacity(size);
        self.write_to(&mut buf);
        buf
    }

    /// Encode this view to a new [`bytes::Bytes`].
    #[must_use]
    fn encode_to_bytes(&self) -> Bytes {
        let size = self.compute_size() as usize;
        let mut buf = bytes::BytesMut::with_capacity(size);
        self.write_to(&mut buf);
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
/// (`&'a str`, `&'a [u8]`, etc.). A default view contains only `'static`
/// data (`""`, `&[]`, `0`), so a `&'static FooView<'static>` can be safely
/// reinterpreted as `&'static FooView<'a>` for any `'a` via covariance.
///
/// This trait is implemented for the `'static` instantiation (e.g.,
/// `FooView<'static>`). The [`MessageFieldView`] `Deref` impl serves it for
/// any `'a` via the covariance contract on the companion
/// [`HasDefaultViewInstance`] — which **is** an `unsafe trait`, since that
/// layout/covariance contract is what backs the lifetime cast.
pub trait DefaultViewInstance: Default + 'static {
    /// Return a reference to the single default view instance.
    fn default_view_instance() -> &'static Self;
}

/// A borrowed view of an optional message field.
///
/// Analogous to [`MessageField<T>`](crate::MessageField) but for the view
/// layer. Like `MessageField`, the inner view is **boxed** — recursive
/// message types (`Foo { NestedMessage { corecursive: Foo } }`) would
/// otherwise have infinite size. The box is API-transparent: `Deref`
/// returns `&V`, and `set()` takes `V` by value.
///
/// When `V` implements [`HasDefaultViewInstance`], this type implements
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
    pub fn compute_size(&self) -> u32 {
        self.inner.as_deref().map_or(0, V::compute_size)
    }

    /// Forward to the inner view's [`cached_size`](ViewEncode::cached_size),
    /// or `0` if unset.
    #[inline]
    pub fn cached_size(&self) -> u32 {
        self.inner.as_deref().map_or(0, V::cached_size)
    }

    /// Forward to the inner view's [`write_to`](ViewEncode::write_to);
    /// no-op if unset.
    #[inline]
    pub fn write_to(&self, buf: &mut impl BufMut) {
        if let Some(v) = self.inner.as_deref() {
            v.write_to(buf);
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

/// Marker trait linking a lifetime-parameterized view type `V` (e.g.,
/// `FooView<'a>`) to its `'static` instantiation that implements
/// [`DefaultViewInstance`]. Generated code implements this for every
/// view type.
///
/// The `default_view_ptr` method returns a raw pointer to avoid forcing
/// `Self: 'static` — view types have a lifetime parameter that may not
/// be `'static`, but the default instance only contains `'static` data
/// and the types are covariant, so the pointer cast is sound.
///
/// # Safety
///
/// `Self` must be layout-identical to `Self::Static` (i.e., the only
/// difference is the lifetime parameter), and `Self` must be covariant
/// in that lifetime.
pub unsafe trait HasDefaultViewInstance {
    /// The `'static` instantiation of this view type.
    type Static: DefaultViewInstance;

    /// Return a pointer to the static default instance, erasing the
    /// lifetime so it can be used for any `'a`.
    fn default_view_ptr() -> *const u8 {
        // Return as a thin `*const u8` to avoid Sized constraints on Self.
        Self::Static::default_view_instance() as *const Self::Static as *const u8
    }
}

impl<V: HasDefaultViewInstance> core::ops::Deref for MessageFieldView<V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &V {
        match &self.inner {
            Some(v) => v,
            // SAFETY: `default_view_ptr` returns a pointer to a `'static`
            // default instance. The `HasDefaultViewInstance` safety contract
            // guarantees the type is covariant, so the `'static` instance
            // is valid at any shorter lifetime. The pointer is non-null and
            // points to an initialized, immutable, `'static` value.
            None => unsafe { &*(V::default_view_ptr() as *const V) },
        }
    }
}

/// Wire-equivalent equality: `Unset` equals `Set(v)` when `v` equals the
/// default instance.
///
/// This matches [`MessageField::eq`](crate::MessageField) on the owned side,
/// so `view_a == view_b` agrees with
/// `view_a.to_owned_message() == view_b.to_owned_message()`.
///
/// The comparison against the default reuses the same covariant-lifetime
/// pointer cast already established by the [`Deref`](core::ops::Deref) impl.
impl<V: PartialEq + HasDefaultViewInstance> PartialEq for MessageFieldView<V> {
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

impl<V: Eq + HasDefaultViewInstance> Eq for MessageFieldView<V> {}

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
/// enabling zero-copy round-tripping of unknown fields.
#[derive(Clone, Debug, Default)]
pub struct UnknownFieldsView<'a> {
    /// Raw (tag, value) byte spans from the input buffer.
    raw_spans: alloc::vec::Vec<&'a [u8]>,
}

impl<'a> UnknownFieldsView<'a> {
    /// Creates an empty view.
    pub fn new() -> Self {
        Self::default()
    }

    #[doc(hidden)]
    pub fn push_raw(&mut self, span: &'a [u8]) {
        self.raw_spans.push(span);
    }

    /// Returns `true` if no unknown fields were recorded.
    pub fn is_empty(&self) -> bool {
        self.raw_spans.is_empty()
    }

    /// Total byte length of all unknown field data.
    pub fn encoded_len(&self) -> usize {
        self.raw_spans.iter().map(|s| s.len()).sum()
    }

    /// Write all unknown-field bytes verbatim. Each span is a complete
    /// `(tag, value)` record as it appeared on the wire, so concatenating
    /// them produces a valid encoding.
    pub fn write_to(&self, buf: &mut impl BufMut) {
        for span in &self.raw_spans {
            buf.put_slice(span);
        }
    }

    /// Convert to an owned [`UnknownFields`](crate::UnknownFields) by parsing all stored raw byte spans.
    ///
    /// Each span is a complete (tag + value) record as it appeared on the wire.
    /// Parsing uses [`crate::encoding::decode_unknown_field`] with the full
    /// recursion limit so deeply nested group fields are handled correctly.
    ///
    /// # Errors
    ///
    /// Returns `Err` if any stored span is malformed — which should not occur
    /// when the view was produced by `decode_view` from valid wire data.
    pub fn to_owned(&self) -> Result<crate::UnknownFields, crate::DecodeError> {
        use crate::encoding::{decode_unknown_field, Tag};

        let mut out = crate::UnknownFields::new();
        for span in &self.raw_spans {
            let mut cur: &[u8] = span;
            let tag = Tag::decode(&mut cur)?;
            let field = decode_unknown_field(tag, &mut cur, crate::RECURSION_LIMIT)?;
            out.push(field);
        }
        Ok(out)
    }
}

/// An owned, `'static` container for a decoded message view.
///
/// `OwnedView` holds a [`Bytes`] buffer alongside the decoded view, ensuring
/// the view's borrows remain valid for the container's lifetime. It implements
/// [`Deref<Target = V>`](core::ops::Deref), so view fields are accessed
/// directly — no `.get()` or unwrapping needed.
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
/// // Direct field access via Deref — no .get() needed
/// println!("name: {}", view.name);
/// println!("id: {}", view.id);
///
/// // Convert to owned if you need to store or mutate
/// let owned: Person = view.to_owned_message();
/// ```
///
/// For scoped access where the buffer's lifetime is known, use
/// [`MessageView::decode_view`] directly — it has zero overhead beyond the
/// decode itself.
///
/// # Safety
///
/// Internally, `OwnedView` extends the view's lifetime to `'static` via
/// `transmute`. This is sound because:
///
/// 1. [`Bytes`] is reference-counted — its heap data pointer is stable across
///    moves. The view's borrows always point into valid memory.
/// 2. [`Bytes`] is immutable — the underlying data cannot be modified while
///    borrowed.
/// 3. A manual [`Drop`] impl explicitly drops the view before the bytes,
///    ensuring no dangling references during cleanup. The view field uses
///    [`ManuallyDrop`](core::mem::ManuallyDrop) to prevent the automatic
///    drop from running out of order.
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
    /// This allocates and copies all borrowed fields.
    pub fn to_owned_message(&self) -> V::Owned {
        self.view.to_owned_message()
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
}

impl<V> core::ops::Deref for OwnedView<V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &V {
        &self.view // Deref through ManuallyDrop is transparent
    }
}

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

    impl DefaultViewInstance for TinyView<'static> {
        fn default_view_instance() -> &'static Self {
            static INST: crate::__private::OnceBox<TinyView<'static>> =
                crate::__private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(TinyView::default()))
        }
    }

    // SAFETY: TinyView is covariant in 'a (only contains &'a str).
    unsafe impl<'a> HasDefaultViewInstance for TinyView<'a> {
        type Static = TinyView<'static>;
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
        fn compute_size(&self) -> u32 {
            let mut size = 0u32;
            if self.id != 0 {
                size += 1 + crate::types::int32_encoded_len(self.id) as u32;
            }
            if !self.name.is_empty() {
                size += 1 + crate::types::string_encoded_len(&self.name) as u32;
            }
            size
        }

        fn write_to(&self, buf: &mut impl bytes::BufMut) {
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
            _depth: u32,
        ) -> Result<(), DecodeError> {
            match tag.field_number() {
                1 => self.id = crate::types::decode_int32(buf)?,
                2 => crate::types::merge_string(&mut self.name, buf)?,
                _ => crate::encoding::skip_field(tag, buf)?,
            }
            Ok(())
        }

        fn cached_size(&self) -> u32 {
            0
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

        fn to_owned_message(&self) -> SimpleMessage {
            SimpleMessage {
                id: self.id,
                name: self.name.into(),
            }
        }
    }

    impl<'a> ViewEncode<'a> for SimpleMessageView<'a> {
        fn compute_size(&self) -> u32 {
            let mut size = 0u32;
            if self.id != 0 {
                size += 1 + crate::types::int32_encoded_len(self.id) as u32;
            }
            if !self.name.is_empty() {
                size += 1 + crate::types::string_encoded_len(self.name) as u32;
            }
            size
        }

        fn write_to(&self, buf: &mut impl bytes::BufMut) {
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

        fn cached_size(&self) -> u32 {
            // Test-only stub: SimpleMessageView has no nested messages,
            // so nothing reads cached_size. Real impls return the value
            // stored by compute_size.
            0
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
    fn owned_view_decode_and_deref() {
        let bytes = encode_simple(42, "hello");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();

        // Access via Deref — no .get() needed
        assert_eq!(view.id, 42);
        assert_eq!(view.name, "hello");
    }

    #[test]
    fn owned_view_to_owned_message() {
        let bytes = encode_simple(7, "world");
        let view = OwnedView::<SimpleMessageView<'static>>::decode(bytes).unwrap();
        let owned = view.to_owned_message();

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
        assert_eq!(view.id, 0);
        assert_eq!(view.name, "");
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
        assert_eq!(view.id, 99);
        assert_eq!(view.name, "roundtrip");

        let back = view.to_owned_message();
        assert_eq!(back, msg);
    }

    #[test]
    fn owned_view_decode_with_options() {
        let bytes = encode_simple(42, "opts");
        let opts = crate::DecodeOptions::new().with_max_message_size(1024);
        let view =
            OwnedView::<SimpleMessageView<'static>>::decode_with_options(bytes, &opts).unwrap();
        assert_eq!(view.id, 42);
        assert_eq!(view.name, "opts");
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
        assert_eq!(cloned.id, 42);
        assert_eq!(cloned.name, "cloned");
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

            fn to_owned_message(&self) -> SimpleMessage {
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
        let name_ptr = view.name.as_ptr() as usize;
        assert!(
            (buf_start..buf_end).contains(&name_ptr),
            "view.name should point into the Bytes buffer"
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
                    assert_eq!(v.id, 42);
                    assert_eq!(v.name, "concurrent");
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
        assert_eq!(view.id, 42);
        assert_eq!(view.name, "parts");
    }
}

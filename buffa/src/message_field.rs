//! Optional message field wrapper that provides ergonomic access.
//!
//! `MessageField<T>` replaces `Option<Box<T>>` for optional/singular message
//! fields. It dereferences to a default instance when unset, avoiding the
//! `Option<Box<M>>` unwrapping ceremony that plagues prost-generated code.

use alloc::boxed::Box;
use core::fmt;
use core::ops::{Deref, DerefMut};

/// Provides access to a lazily-initialized, immutable default instance of a
/// type.
///
/// Types that implement this trait can be used as the target of
/// [`MessageField<T>`] dereferences when the field is unset. It is also a
/// supertrait of [`Message`](crate::Message), so every generated message type
/// must implement it.
///
/// **Codegen implements this automatically** for every generated message
/// type. You only need to implement it by hand when manually implementing
/// [`Message`](crate::Message) for a custom type.
///
/// # Recommended implementation
///
/// The pattern codegen uses — and the recommended pattern for manual
/// implementations — stores the instance in a `static`
/// [`once_cell::race::OnceBox`] (re-exported as
/// `::buffa::__private::OnceBox`), which works in both `no_std + alloc` and
/// `std` environments:
///
/// ```rust,ignore
/// impl DefaultInstance for MyMessage {
///     fn default_instance() -> &'static Self {
///         static VALUE: ::buffa::__private::OnceBox<MyMessage>
///             = ::buffa::__private::OnceBox::new();
///         VALUE.get_or_init(|| ::alloc::boxed::Box::new(MyMessage::default()))
///     }
/// }
/// ```
///
/// In `std`-only environments [`std::sync::OnceLock`] is also available and
/// avoids the `alloc::boxed::Box` wrapping:
///
/// ```rust,ignore
/// impl DefaultInstance for MyMessage {
///     fn default_instance() -> &'static Self {
///         static VALUE: std::sync::OnceLock<MyMessage> = std::sync::OnceLock::new();
///         VALUE.get_or_init(MyMessage::default)
///     }
/// }
/// ```
pub trait DefaultInstance: Default + 'static {
    /// Return a reference to the single default instance of this type.
    fn default_instance() -> &'static Self;
}

/// Implement [`DefaultInstance`] for a message type via a lazily-initialized
/// `OnceBox` singleton.
///
/// Emitted by generated code (one invocation per message struct) so the
/// six-line singleton body lives here once instead of in every generated
/// impl. Hand-written `Message` types may also use it.
///
/// ```rust,ignore
/// buffa::impl_default_instance!(MyMessage);
/// ```
#[macro_export]
macro_rules! impl_default_instance {
    ($ty:ty) => {
        impl $crate::DefaultInstance for $ty {
            fn default_instance() -> &'static Self {
                static VALUE: $crate::__private::OnceBox<$ty> = $crate::__private::OnceBox::new();
                VALUE.get_or_init(|| {
                    $crate::alloc::boxed::Box::new(<$ty as ::core::default::Default>::default())
                })
            }
        }
    };
}

/// The owned smart pointer backing a singular message field inside
/// [`MessageField`].
///
/// The codegen default is [`Inline<T>`] — the message is stored directly in the
/// parent struct (no heap), with recursive fields kept on `Box<T>`
/// automatically. `buffa_build`'s [`box_type_in`][bti] selects [`Box<T>`] (or a
/// custom pointer) for fields where reserving `size_of::<T>()` in the parent is
/// wasteful. The `box_type_custom` knob substitutes any pointer that implements
/// `ProtoBox<T>` — for example a `smallbox`-style pointer that stores small
/// messages inline but spills large ones to the heap. The wire format is
/// unchanged; only the in-memory ownership of the boxed message changes, and
/// view types are unaffected.
///
/// There is intentionally no blanket impl — the built-in [`Box<T>`] and
/// [`Inline<T>`] impls below are the only ones buffa provides. Because
/// `ProtoBox` is buffa-owned, a *foreign* pointer cannot implement it directly
/// (orphan rule); wrap it in a crate-local newtype, like the `ProtoString`
/// newtype pattern.
///
/// [bti]: https://docs.rs/buffa-build/latest/buffa_build/struct.Config.html#method.box_type_in
///
/// # Why `DerefMut` (and why `Rc` / `Arc` are excluded)
///
/// The decoder merges a message field in place — it calls
/// [`get_or_insert_default`](MessageField::get_or_insert_default) to obtain a
/// `&mut T` and decodes into it. That requires `DerefMut`, which `Rc<T>` and
/// `Arc<T>` cannot provide while shared. So this knob selects an **allocation
/// strategy** (inline vs heap) for an exclusively-owned message, never a shared
/// or copy-on-write pointer. A custom pointer that is not exclusively owned will
/// fail to implement `ProtoBox` at its definition site, not silently misbehave.
///
/// # Thread-safety
///
/// `ProtoBox` deliberately does **not** require `Send`/`Sync` (unlike
/// `ProtoString`/`ProtoBytes`/`ProtoList`). `MessageField<T>` is the universal
/// message-field wrapper and has generic helpers; bounding the pointer
/// `Send + Sync` would tighten `T: Send + Sync` onto all of them. It is also
/// unnecessary: a generated message implements [`Message`](crate::Message),
/// whose `Send + Sync` supertraits already require every field — and thus the
/// pointer — to be `Send + Sync`, so a non-`Send` custom pointer is rejected at
/// the message's `impl Message`, just less locally.
///
/// # Caveat
///
/// A *custom* inline pointer (e.g. `SmallBox`) inflates the parent struct by
/// its inline-storage size for every such field. The built-in [`Inline<T>`]
/// default is recursion-aware and is the intended blanket; reserve custom
/// inline pointers for per-field or per-prefix overrides.
///
/// # Examples
///
/// A minimal crate-local newtype wrapping a foreign pointer (the `Clone` derive
/// is only needed if the enclosing message derives `Clone`):
///
/// ```rust,ignore
/// #[derive(Clone)]
/// pub struct SmallBox<T>(pub smallbox::SmallBox<T, smallbox::space::S4>);
///
/// impl<T> core::ops::Deref for SmallBox<T> {
///     type Target = T;
///     fn deref(&self) -> &T { &self.0 }
/// }
/// impl<T> core::ops::DerefMut for SmallBox<T> {
///     fn deref_mut(&mut self) -> &mut T { &mut self.0 }
/// }
/// impl<T> buffa::ProtoBox<T> for SmallBox<T> {
///     fn new(value: T) -> Self { SmallBox(smallbox::smallbox!(value)) }
///     fn into_inner(self) -> T { self.0.into_inner() }
/// }
/// ```
///
/// Then point a field at it: `box_type_custom("::my_crate::SmallBox<*>")`.
#[rustversion::attr(
    since(1.78),
    diagnostic::on_unimplemented(
        message = "`{Self}` cannot be used as a buffa custom box type",
        note = "buffa owns `ProtoBox`, so a foreign type can't implement it directly (orphan rule). \
                Wrap it in a crate-local newtype and implement `ProtoBox` on the newtype. \
                See the `custom-types` example in the buffa repository for a template."
    )
)]
pub trait ProtoBox<T>: Deref<Target = T> + DerefMut {
    /// Box a freshly-decoded or constructed message value.
    fn new(value: T) -> Self;

    /// Unwrap the pointer, returning the owned message value (the
    /// [`take`](MessageField::take) / [`into_option`](MessageField::into_option)
    /// path).
    fn into_inner(self) -> T;
}

impl<T> ProtoBox<T> for Box<T> {
    #[inline]
    fn new(value: T) -> Self {
        Box::new(value)
    }

    #[inline]
    fn into_inner(self) -> T {
        *self
    }
}

/// A [`ProtoBox`] that stores the message inline (no heap allocation).
///
/// `MessageField<T, Inline<T>>` is laid out as `Option<T>` — set values live
/// directly in the parent struct. This eliminates the per-field allocation of
/// the default `Box<T>` pointer at the cost of inflating the parent's size by
/// `size_of::<T>()` even when the field is unset.
///
/// Codegen uses this by default via [`PointerRepr::Inline`][pri], which is
/// recursion-aware: a field that would form an infinite-size cycle is silently
/// kept on `Box`. Opt out per-field with `box_type_in(PointerRepr::Box, …)`.
///
/// [pri]: https://docs.rs/buffa-codegen/latest/buffa_codegen/enum.PointerRepr.html#variant.Inline
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Inline<T>(pub T);

impl<T> Deref for Inline<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Inline<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> ProtoBox<T> for Inline<T> {
    #[inline]
    fn new(value: T) -> Self {
        Inline(value)
    }

    #[inline]
    fn into_inner(self) -> T {
        self.0
    }
}

/// Convenience mirror of `Box`'s `From<T>`; codegen and [`MessageField::some`]
/// use [`ProtoBox::new`] instead.
impl<T> From<T> for Inline<T> {
    #[inline]
    fn from(value: T) -> Self {
        Inline(value)
    }
}

// The built-in pointers must always satisfy the bound; freeze that invariant
// against future changes to the trait's supertraits.
const _: fn() = || {
    fn assert_proto_box<P: ProtoBox<T>, T>() {}
    assert_proto_box::<Box<u32>, u32>();
    assert_proto_box::<Inline<u32>, u32>();
};

/// A wrapper for optional message fields that provides transparent access
/// to a default instance when the field is not set.
///
/// This type is used for singular message fields in generated code. The pointer
/// type `P` is pluggable; see [`ProtoBox`]. Codegen emits [`Inline<T>`] by
/// default — the message is stored directly in the parent struct (laid out as
/// `Option<T>`), so `size_of::<T>()` is reserved whether set or not and there
/// is no per-field heap allocation. Recursive fields and explicit opt-outs use
/// `Box<T>` instead, which heap-allocates only when set. The struct's own
/// `P = Box<T>` type-parameter default applies only to a hand-written
/// `MessageField<T>` (e.g. in tests), not to generated fields.
/// Because `P` has a default, a *standalone* construction with no pinning
/// context needs a type annotation — write `let f: MessageField<Foo> =
/// MessageField::some(x);` (or `MessageField::<Foo>::some(x)`). In the common
/// cases — a struct-literal field (`Outer { inner: MessageField::some(x), .. }`)
/// or an assignment to a typed field — `P` is inferred from the target and no
/// annotation is needed.
///
/// # Access patterns
///
/// ```rust,ignore
/// // Reading through an unset field gives the default:
/// let msg = Outer::default();
/// assert_eq!(msg.inner.name, "");  // No unwrap needed, derefs to default
///
/// // Check if set:
/// if msg.inner.is_set() { ... }
///
/// // Set a value:
/// msg.inner = MessageField::some(Inner { name: "hello".into(), ..Default::default() });
///
/// // Clear:
/// msg.inner = MessageField::none();
/// ```
pub struct MessageField<T: Default, P = Box<T>> {
    inner: Option<P>,
    _marker: core::marker::PhantomData<T>,
}

impl<T: Default, P: ProtoBox<T>> MessageField<T, P> {
    /// Create a `MessageField` with no value set.
    #[inline]
    pub const fn none() -> Self {
        Self {
            inner: None,
            _marker: core::marker::PhantomData,
        }
    }

    /// Create a `MessageField` with a value.
    #[inline]
    pub fn some(value: T) -> Self {
        Self {
            inner: Some(<P as ProtoBox<T>>::new(value)),
            _marker: core::marker::PhantomData,
        }
    }

    /// Create a `MessageField` from an already-constructed pointer, without
    /// unwrapping and re-boxing the value.
    ///
    /// Prefer this over `MessageField::some(p.into_inner())` when you already
    /// hold a `P` — for an inline pointer (e.g. `SmallBox`) the latter would
    /// move the value out and re-store it, defeating the point. The generic
    /// counterpart to [`from_box`](Self::from_box) (which is `Box`-only).
    #[inline]
    pub fn from_pointer(value: P) -> Self {
        Self {
            inner: Some(value),
            _marker: core::marker::PhantomData,
        }
    }

    /// Returns `true` if the field has a value set.
    #[inline]
    pub fn is_set(&self) -> bool {
        self.inner.is_some()
    }

    /// Returns `true` if the field has no value set.
    #[inline]
    pub fn is_unset(&self) -> bool {
        self.inner.is_none()
    }

    /// Get a reference to the inner value, or `None` if unset.
    #[inline]
    pub fn as_option(&self) -> Option<&T> {
        self.inner.as_deref()
    }

    /// Get a mutable reference to the inner value, or `None` if unset.
    #[inline]
    pub fn as_option_mut(&mut self) -> Option<&mut T> {
        self.inner.as_deref_mut()
    }

    /// Take the inner value, leaving the field unset.
    #[inline]
    pub fn take(&mut self) -> Option<T> {
        self.inner.take().map(<P as ProtoBox<T>>::into_inner)
    }

    /// Get a mutable reference to the value, initializing to the default if unset.
    #[inline]
    pub fn get_or_insert_default(&mut self) -> &mut T {
        // `&mut P` coerces to `&mut T` via `P: DerefMut<Target = T>`. Uses
        // `ProtoBox::new` rather than `Option::get_or_insert_default` so no
        // `P: Default` bound is needed (a custom pointer need not be `Default`).
        self.inner
            .get_or_insert_with(|| <P as ProtoBox<T>>::new(T::default()))
    }

    /// Call `f` with a mutable reference to the inner value, initializing to
    /// the default if the field is currently unset.
    ///
    /// This is the ergonomic write counterpart to the transparent read
    /// provided by `Deref`. Instead of calling `get_or_insert_default` once
    /// per assignment:
    ///
    /// ```rust,ignore
    /// msg.address.get_or_insert_default().street = "123 Main St".into();
    /// msg.address.get_or_insert_default().city   = "Springfield".into();
    /// ```
    ///
    /// use `modify` to initialize the field once and set all sub-fields in
    /// the closure:
    ///
    /// ```rust,ignore
    /// msg.address.modify(|a| {
    ///     a.street = "123 Main St".into();
    ///     a.city   = "Springfield".into();
    /// });
    /// ```
    #[inline]
    pub fn modify<F: FnOnce(&mut T)>(&mut self, f: F) {
        f(self.get_or_insert_default());
    }

    /// Consume the field, returning `Some(T)` if set or `None` if unset.
    ///
    /// This unboxes the inner value. For in-place extraction that leaves the
    /// field unset without consuming the enclosing struct, see [`take`](Self::take).
    #[inline]
    pub fn into_option(self) -> Option<T> {
        self.inner.map(<P as ProtoBox<T>>::into_inner)
    }

    /// Consume the field, returning the inner value.
    ///
    /// Equivalent in effect to `into_option().unwrap()`, with a clearer
    /// panic message. Prefer [`ok_or`](Self::ok_or) /
    /// [`ok_or_else`](Self::ok_or_else) when an unset field should produce
    /// an error rather than a panic.
    ///
    /// # Panics
    ///
    /// Panics if the field is unset.
    #[inline]
    #[track_caller]
    pub fn unwrap(self) -> T {
        match self.inner {
            Some(b) => <P as ProtoBox<T>>::into_inner(b),
            None => panic!("called `MessageField::unwrap()` on an unset field"),
        }
    }

    /// Consume the field, returning the inner value, with a custom panic
    /// message if unset.
    ///
    /// Mirrors [`Option::expect`]. Prefer [`ok_or`](Self::ok_or) /
    /// [`ok_or_else`](Self::ok_or_else) when an unset field should produce
    /// an error rather than a panic.
    ///
    /// # Panics
    ///
    /// Panics with `msg` if the field is unset.
    #[inline]
    #[track_caller]
    pub fn expect(self, msg: &str) -> T {
        match self.inner {
            Some(b) => <P as ProtoBox<T>>::into_inner(b),
            None => panic!("{msg}"),
        }
    }

    /// Consume the field, returning `Ok(T)` if set or `Err(err)` if unset.
    ///
    /// Mirrors [`Option::ok_or`]. Useful for enforcing presence of
    /// semantically-required fields that the proto schema leaves optional:
    ///
    /// ```rust,ignore
    /// let cmd = request.normalized_command.ok_or(Error::MissingCommand)?;
    /// ```
    #[inline]
    pub fn ok_or<E>(self, err: E) -> Result<T, E> {
        match self.inner {
            Some(b) => Ok(<P as ProtoBox<T>>::into_inner(b)),
            None => Err(err),
        }
    }

    /// Consume the field, returning `Ok(T)` if set or `Err(err())` if unset.
    ///
    /// Mirrors [`Option::ok_or_else`]. The closure is only called if the
    /// field is unset, so use this over [`ok_or`](Self::ok_or) when
    /// constructing the error is non-trivial:
    ///
    /// ```rust,ignore
    /// let cmd = request.normalized_command.ok_or_else(|| {
    ///     ConnectError::invalid_argument("missing normalized_command in request")
    /// })?;
    /// ```
    #[inline]
    pub fn ok_or_else<E, F: FnOnce() -> E>(self, err: F) -> Result<T, E> {
        match self.inner {
            Some(b) => Ok(<P as ProtoBox<T>>::into_inner(b)),
            None => Err(err()),
        }
    }
}

impl<T: Default> MessageField<T, Box<T>> {
    /// Create a `MessageField` from a boxed value.
    ///
    /// Specific to `Box<T>` (the struct's `P = Box<T>` type-parameter default,
    /// used for recursive fields and explicit opt-outs); inline-backed
    /// generated fields use [`some`](Self::some) or
    /// [`from_pointer`](Self::from_pointer) instead.
    #[inline]
    pub fn from_box(value: Box<T>) -> Self {
        Self {
            inner: Some(value),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<T: Default, P: ProtoBox<T>> Default for MessageField<T, P> {
    #[inline]
    fn default() -> Self {
        Self::none()
    }
}

impl<T: DefaultInstance, P: ProtoBox<T>> Deref for MessageField<T, P> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        match &self.inner {
            // `&P` coerces to `&T` via `P: Deref<Target = T>`.
            Some(value) => value,
            None => T::default_instance(),
        }
    }
}

impl<T: Default + Clone, P: ProtoBox<T> + Clone> Clone for MessageField<T, P> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<T: DefaultInstance + PartialEq, P: ProtoBox<T>> PartialEq for MessageField<T, P> {
    fn eq(&self, other: &Self) -> bool {
        // Compare the pointed-to `T` values (via `**`), not the pointers, so no
        // `P: PartialEq` bound is needed and a set-to-default field equals an
        // unset one.
        match (&self.inner, &other.inner) {
            (Some(a), Some(b)) => **a == **b,
            (None, None) => true,
            // An unset field equals a set-to-default field. Use default_instance()
            // to avoid allocating a temporary value for the comparison.
            (Some(a), None) => **a == *T::default_instance(),
            (None, Some(b)) => *T::default_instance() == **b,
        }
    }
}

impl<T: DefaultInstance + Eq + PartialEq, P: ProtoBox<T>> Eq for MessageField<T, P> {}

impl<T: Default + fmt::Debug, P: ProtoBox<T>> fmt::Debug for MessageField<T, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            // Format the pointed-to `T` (via `&**`), so no `P: Debug` bound.
            Some(value) => f.debug_tuple("MessageField::Set").field(&**value).finish(),
            None => f.write_str("MessageField::Unset"),
        }
    }
}

impl<T: Default, P: ProtoBox<T>> From<Option<T>> for MessageField<T, P> {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => Self::some(v),
            None => Self::none(),
        }
    }
}

impl<T: Default, P: ProtoBox<T>> From<MessageField<T, P>> for Option<T> {
    /// Unbox a `MessageField` into an `Option<T>`; equivalent to
    /// [`MessageField::into_option`].
    #[inline]
    fn from(field: MessageField<T, P>) -> Self {
        field.into_option()
    }
}

impl<T: Default, P: ProtoBox<T>> From<T> for MessageField<T, P> {
    fn from(value: T) -> Self {
        Self::some(value)
    }
}

#[cfg(feature = "json")]
impl<T: Default + serde::Serialize, P: ProtoBox<T>> serde::Serialize for MessageField<T, P> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.inner.as_deref() {
            Some(v) => s.serialize_some(v),
            None => s.serialize_none(),
        }
    }
}

#[cfg(feature = "json")]
impl<'de, T: Default + serde::Deserialize<'de>, P: ProtoBox<T>> serde::Deserialize<'de>
    for MessageField<T, P>
{
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(d).map(|opt| match opt {
            Some(v) => Self::some(v),
            None => Self::none(),
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, T: Default + arbitrary::Arbitrary<'a>, P: ProtoBox<T>> arbitrary::Arbitrary<'a>
    for MessageField<T, P>
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(if bool::arbitrary(u)? {
            MessageField::some(T::arbitrary(u)?)
        } else {
            MessageField::none()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Default, PartialEq)]
    struct Inner {
        value: i32,
        name: alloc::string::String,
    }

    // Via the exported macro, which doubles as its unit test (hygiene and
    // `$crate` path resolution).
    crate::impl_default_instance!(Inner);

    #[test]
    fn inline_message_field_is_option_layout() {
        // `Inline<T>` is `repr(transparent)`, so `MessageField<T, Inline<T>>`
        // (which is `Option<Inline<T>>`) is laid out as `Option<T>`.
        use core::mem::size_of;
        assert_eq!(
            size_of::<MessageField<Inner, Inline<Inner>>>(),
            size_of::<Option<Inner>>()
        );
        // And the `ProtoBox` surface round-trips.
        let mut f: MessageField<Inner, Inline<Inner>> = MessageField::some(Inner {
            value: 7,
            ..Default::default()
        });
        assert_eq!(f.value, 7);
        f.get_or_insert_default().value = 9;
        assert_eq!(f.take().map(|i| i.value), Some(9));
        assert!(f.is_unset());
    }

    #[test]
    fn impl_default_instance_macro_returns_singleton() {
        let a: &'static Inner = Inner::default_instance();
        let b: &'static Inner = Inner::default_instance();
        assert!(core::ptr::eq(a, b), "singleton must be a single allocation");
        assert_eq!(a, &Inner::default());
    }

    #[test]
    fn test_unset_derefs_to_default() {
        let field: MessageField<Inner> = MessageField::none();
        assert!(!field.is_set());
        assert_eq!(field.value, 0);
        assert_eq!(field.name, "");
    }

    #[test]
    fn test_set_derefs_to_value() {
        let field: MessageField<Inner> = MessageField::some(Inner {
            value: 42,
            name: "hello".into(),
        });
        assert!(field.is_set());
        assert_eq!(field.value, 42);
        assert_eq!(field.name, "hello");
    }

    #[test]
    fn test_get_or_insert_default() {
        let mut field: MessageField<Inner> = MessageField::none();
        assert!(!field.is_set());
        field.get_or_insert_default().value = 10;
        assert!(field.is_set());
        assert_eq!(field.value, 10);
    }

    #[test]
    fn test_equality() {
        let a: MessageField<Inner> = MessageField::none();
        let b: MessageField<Inner> = MessageField::some(Inner::default());
        // An unset field and a set-to-default field are equal.
        assert_eq!(a, b);
    }

    #[test]
    fn test_unwrap_set() {
        let field: MessageField<Inner> = MessageField::some(Inner {
            value: 7,
            name: "x".into(),
        });
        let inner = field.unwrap();
        assert_eq!(inner.value, 7);
    }

    #[test]
    #[should_panic(expected = "called `MessageField::unwrap()` on an unset field")]
    fn test_unwrap_unset_panics() {
        let field: MessageField<Inner> = MessageField::none();
        let _ = field.unwrap();
    }

    #[test]
    #[should_panic(expected = "address is required")]
    fn test_expect_unset_panics_with_message() {
        let field: MessageField<Inner> = MessageField::none();
        let _ = field.expect("address is required");
    }

    #[test]
    fn test_from_message_field_for_option_roundtrips() {
        let inner = Inner {
            value: 3,
            name: "rt".into(),
        };
        let field: MessageField<Inner> = Some(inner.clone()).into();
        let back: Option<Inner> = field.into();
        assert_eq!(back, Some(inner));

        let none_field: MessageField<Inner> = MessageField::none();
        let none_back: Option<Inner> = none_field.into();
        assert_eq!(none_back, None);
    }

    #[test]
    fn test_take() {
        let mut field: MessageField<Inner> = MessageField::some(Inner {
            value: 7,
            name: "taken".into(),
        });
        let taken = field.take();
        assert!(field.is_unset());
        assert_eq!(taken.unwrap().value, 7);
    }

    #[test]
    fn test_clone() {
        let field: MessageField<Inner> = MessageField::some(Inner {
            value: 99,
            name: "clone".into(),
        });
        let cloned = field.clone();
        assert_eq!(field, cloned);
    }

    #[test]
    fn test_modify_initializes_unset_field() {
        let mut field: MessageField<Inner> = MessageField::none();
        field.modify(|inner| {
            inner.value = 42;
            inner.name = "hello".into();
        });
        assert!(field.is_set());
        assert_eq!(field.value, 42);
        assert_eq!(field.name, "hello");
    }

    #[test]
    fn test_modify_updates_already_set_field() {
        let mut field: MessageField<Inner> = MessageField::some(Inner {
            value: 1,
            name: "original".into(),
        });
        field.modify(|inner| {
            inner.value = 99;
        });
        // Existing value is mutated in place, not reset to default.
        assert_eq!(field.value, 99);
        assert_eq!(field.name, "original");
    }

    #[test]
    fn test_modify_multiple_fields_in_one_call() {
        // Demonstrates the primary motivation: one initialization, many assignments.
        let mut field: MessageField<Inner> = MessageField::none();
        field.modify(|inner| {
            inner.value = 10;
            inner.name = "multi".into();
        });
        assert_eq!(field.value, 10);
        assert_eq!(field.name, "multi");
    }

    #[test]
    fn test_modify_noop_still_initializes_unset_field() {
        // Even a no-op closure causes the field to become set (to the default).
        let mut field: MessageField<Inner> = MessageField::none();
        field.modify(|_| {});
        assert!(field.is_set());
        assert_eq!(field.value, 0);
        assert_eq!(field.name, "");
    }

    #[test]
    fn test_modify_closure_can_move_captured_values() {
        // Verifies that the FnOnce bound is in effect: a closure that moves a
        // captured value (String) into the field compiles and works correctly.
        let mut field: MessageField<Inner> = MessageField::none();
        let name = alloc::string::String::from("moved");
        field.modify(|inner| {
            inner.name = name; // moves `name` -- only valid with FnOnce
        });
        assert_eq!(field.name, "moved");
    }

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        #[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
        struct Msg {
            value: i32,
        }

        #[test]
        fn unset_serializes_as_null() {
            let f: MessageField<Msg> = MessageField::none();
            assert_eq!(serde_json::to_string(&f).unwrap(), "null");
        }

        #[test]
        fn set_serializes_as_inner_json() {
            let f: MessageField<Msg> = MessageField::some(Msg { value: 42 });
            let json = serde_json::to_string(&f).unwrap();
            assert_eq!(json, r#"{"value":42}"#);
        }

        #[test]
        fn null_deserializes_as_unset() {
            let f: MessageField<Msg> = serde_json::from_str("null").unwrap();
            assert!(f.is_unset());
        }

        #[test]
        fn object_deserializes_as_set() {
            let f: MessageField<Msg> = serde_json::from_str(r#"{"value":7}"#).unwrap();
            assert!(f.is_set());
            assert_eq!(f.as_option().unwrap().value, 7);
        }

        #[test]
        fn round_trip_set_field() {
            let original: MessageField<Msg> = MessageField::some(Msg { value: 99 });
            let json = serde_json::to_string(&original).unwrap();
            let recovered: MessageField<Msg> = serde_json::from_str(&json).unwrap();
            assert_eq!(
                original.as_option().unwrap().value,
                recovered.as_option().unwrap().value
            );
        }
    }

    #[test]
    fn test_into_option_unboxes() {
        let field: MessageField<Inner> = MessageField::some(Inner {
            value: 5,
            name: "x".into(),
        });
        // Type annotation asserts this is Option<Inner>, not Option<Box<Inner>>.
        let opt: Option<Inner> = field.into_option();
        assert_eq!(opt.unwrap().value, 5);

        let field: MessageField<Inner> = MessageField::none();
        assert!(field.into_option().is_none());
    }

    #[test]
    fn test_ok_or() {
        let set: MessageField<Inner> = MessageField::some(Inner {
            value: 1,
            name: "set".into(),
        });
        let r: Result<Inner, &str> = set.ok_or("missing");
        assert_eq!(r.unwrap().value, 1);

        let unset: MessageField<Inner> = MessageField::none();
        assert_eq!(unset.ok_or("missing"), Err("missing"));
    }

    #[test]
    fn test_ok_or_else() {
        let set: MessageField<Inner> = MessageField::some(Inner {
            value: 2,
            name: "set".into(),
        });
        assert_eq!(set.ok_or_else(|| "unreachable").unwrap().value, 2);

        let unset: MessageField<Inner> = MessageField::none();
        assert_eq!(unset.ok_or_else(|| "missing"), Err("missing"));
    }

    #[test]
    fn test_ok_or_else_closure_not_called_when_set() {
        let set: MessageField<Inner> = MessageField::some(Inner::default());
        let _ = set.ok_or_else(|| -> &str { panic!("closure must not run") });
    }

    #[test]
    fn test_ok_or_else_partial_move() {
        // The motivating pattern: destructure several required fields out of
        // an owned request struct, one ok_or_else per field. Each is a partial
        // move; the remaining fields stay accessible.
        #[derive(Default)]
        struct Request {
            a: MessageField<Inner>,
            b: MessageField<Inner>,
        }
        let req = Request {
            a: MessageField::some(Inner {
                value: 1,
                ..Default::default()
            }),
            b: MessageField::some(Inner {
                value: 2,
                ..Default::default()
            }),
        };
        let a = req.a.ok_or_else(|| "missing a").unwrap();
        let b = req.b.ok_or_else(|| "missing b").unwrap();
        assert_eq!(a.value, 1);
        assert_eq!(b.value, 2);
    }

    #[test]
    fn test_from_conversions() {
        let field: MessageField<Inner> = Inner {
            value: 1,
            name: "from".into(),
        }
        .into();
        assert!(field.is_set());

        let field: MessageField<Inner> = None.into();
        assert!(field.is_unset());

        let field: MessageField<Inner> = Some(Inner {
            value: 2,
            name: "some".into(),
        })
        .into();
        assert!(field.is_set());
    }
}

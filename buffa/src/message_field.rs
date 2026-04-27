//! Optional message field wrapper that provides ergonomic access.
//!
//! `MessageField<T>` replaces `Option<Box<T>>` for optional/singular message
//! fields. It dereferences to a default instance when unset, avoiding the
//! `Option<Box<M>>` unwrapping ceremony that plagues prost-generated code.

use alloc::boxed::Box;
use core::fmt;
use core::ops::Deref;

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

/// A wrapper for optional message fields that provides transparent access
/// to a default instance when the field is not set.
///
/// This type is used for singular message fields in generated code. It avoids
/// the ergonomic pain of `Option<Box<M>>` while still being heap-allocated
/// only when set (no allocation for unset fields).
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
pub struct MessageField<T: Default> {
    inner: Option<Box<T>>,
}

impl<T: Default> MessageField<T> {
    /// Create a `MessageField` with no value set.
    #[inline]
    pub const fn none() -> Self {
        Self { inner: None }
    }

    /// Create a `MessageField` with a value.
    #[inline]
    pub fn some(value: T) -> Self {
        Self {
            inner: Some(Box::new(value)),
        }
    }

    /// Create a `MessageField` from a boxed value.
    #[inline]
    pub fn from_box(value: Box<T>) -> Self {
        Self { inner: Some(value) }
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
        self.inner.take().map(|b| *b)
    }

    /// Get a mutable reference to the value, initializing to the default if unset.
    #[inline]
    pub fn get_or_insert_default(&mut self) -> &mut T {
        self.inner.get_or_insert_default()
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
        self.inner.map(|b| *b)
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
            Some(b) => Ok(*b),
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
            Some(b) => Ok(*b),
            None => Err(err()),
        }
    }
}

impl<T: Default> Default for MessageField<T> {
    #[inline]
    fn default() -> Self {
        Self::none()
    }
}

impl<T: DefaultInstance> Deref for MessageField<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        match &self.inner {
            Some(value) => value,
            None => T::default_instance(),
        }
    }
}

impl<T: Default + Clone> Clone for MessageField<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: DefaultInstance + PartialEq> PartialEq for MessageField<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.inner, &other.inner) {
            (Some(a), Some(b)) => a == b,
            (None, None) => true,
            // An unset field equals a set-to-default field. Use default_instance()
            // to avoid allocating a temporary value for the comparison.
            (Some(a), None) => **a == *T::default_instance(),
            (None, Some(b)) => *T::default_instance() == **b,
        }
    }
}

impl<T: DefaultInstance + Eq + PartialEq> Eq for MessageField<T> {}

impl<T: Default + fmt::Debug> fmt::Debug for MessageField<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            Some(value) => f.debug_tuple("MessageField::Set").field(value).finish(),
            None => f.write_str("MessageField::Unset"),
        }
    }
}

impl<T: Default> From<Option<T>> for MessageField<T> {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => Self::some(v),
            None => Self::none(),
        }
    }
}

impl<T: Default> From<T> for MessageField<T> {
    fn from(value: T) -> Self {
        Self::some(value)
    }
}

#[cfg(feature = "json")]
impl<T: Default + serde::Serialize> serde::Serialize for MessageField<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.inner.as_deref() {
            Some(v) => s.serialize_some(v),
            None => s.serialize_none(),
        }
    }
}

#[cfg(feature = "json")]
impl<'de, T: Default + serde::Deserialize<'de>> serde::Deserialize<'de> for MessageField<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(d).map(|opt| match opt {
            Some(v) => Self::some(v),
            None => Self::none(),
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, T: Default + arbitrary::Arbitrary<'a>> arbitrary::Arbitrary<'a> for MessageField<T> {
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

    impl DefaultInstance for Inner {
        fn default_instance() -> &'static Self {
            static VALUE: crate::__private::OnceBox<Inner> = crate::__private::OnceBox::new();
            VALUE.get_or_init(|| alloc::boxed::Box::new(Inner::default()))
        }
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
        let field = MessageField::some(Inner {
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
    fn test_take() {
        let mut field = MessageField::some(Inner {
            value: 7,
            name: "taken".into(),
        });
        let taken = field.take();
        assert!(field.is_unset());
        assert_eq!(taken.unwrap().value, 7);
    }

    #[test]
    fn test_clone() {
        let field = MessageField::some(Inner {
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
        let mut field = MessageField::some(Inner {
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
            let f = MessageField::some(Msg { value: 42 });
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
            let original = MessageField::some(Msg { value: 99 });
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
        let field = MessageField::some(Inner {
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
        let set = MessageField::some(Inner {
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
        let set = MessageField::some(Inner {
            value: 2,
            name: "set".into(),
        });
        assert_eq!(set.ok_or_else(|| "unreachable").unwrap().value, 2);

        let unset: MessageField<Inner> = MessageField::none();
        assert_eq!(unset.ok_or_else(|| "missing"), Err("missing"));
    }

    #[test]
    fn test_ok_or_else_closure_not_called_when_set() {
        let set = MessageField::some(Inner::default());
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

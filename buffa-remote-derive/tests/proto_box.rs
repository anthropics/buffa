use buffa::ProtoBox;
use buffa_remote_derive::ProtoBox as DeriveProtoBox;

#[derive(Clone, PartialEq, Debug, DeriveProtoBox)]
#[buffa(remote = smallbox::SmallBox<T, smallbox::space::S4>)]
struct MyBox<T>(pub smallbox::SmallBox<T, smallbox::space::S4>);

#[test]
fn new_and_into_inner_round_trip() {
    let boxed = MyBox::new(42i64);
    assert_eq!(*boxed, 42);
    assert_eq!(boxed.into_inner(), 42);
}

#[test]
fn deref_mut_writes_through() {
    let mut boxed = MyBox::new(String::from("hi"));
    boxed.push_str(" there");
    assert_eq!(boxed.into_inner(), "hi there");
}

// A pointer with non-conventional method names, exercising the
// `new`/`into_inner` attribute overrides instead of the defaults.
struct Holder<T>(T);
impl<T> Holder<T> {
    fn wrap(value: T) -> Self {
        Self(value)
    }
    fn unwrap(self) -> T {
        self.0
    }
}
impl<T> core::ops::Deref for Holder<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T> core::ops::DerefMut for Holder<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[derive(DeriveProtoBox)]
#[buffa(remote = Holder<T>, new = Holder::wrap, into_inner = Holder::unwrap)]
struct MyHolder<T>(pub Holder<T>);

#[test]
fn overridden_method_names_are_used() {
    let h = MyHolder::new(7i32);
    assert_eq!(*h, 7);
    assert_eq!(h.into_inner(), 7);
}

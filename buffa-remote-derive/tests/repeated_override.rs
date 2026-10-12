//! An override key repeated with the same path is accepted. Two `cfg_attr`s
//! whose conditions both hold reach the derive as the key written twice.

use buffa::ProtoBox;
use buffa_remote_derive::ProtoBox as DeriveProtoBox;

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
#[buffa(remote = Holder<T>, into_inner = Holder::unwrap)]
#[cfg_attr(all(), buffa(new = Holder::wrap))]
#[cfg_attr(not(any()), buffa(new = Holder::wrap))]
struct MyHolder<T>(pub Holder<T>);

#[test]
fn override_given_by_two_cfg_attrs_that_both_hold_is_used() {
    let holder = MyHolder::new(7i32);
    assert_eq!(*holder, 7);
    assert_eq!(holder.into_inner(), 7);
}

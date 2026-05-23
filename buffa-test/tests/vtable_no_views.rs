//! Owned-message vtable reflection without view generation.
//!
//! `vtable_no_views.proto` is built with `generate_views(false)` +
//! `reflect_mode(VTable)`. The owned `impl ReflectMessage` is self-contained
//! (no view types involved), so `reflect()` borrows `self` and reads owned
//! fields directly — proving vtable mode does not require views.

use buffa_descriptor::reflect::{ReflectCow, Reflectable, ValueRef};
use buffa_test::vtable_no_views::Simple;

#[test]
fn owned_vtable_reflects_without_views() {
    let msg = Simple {
        id: 7,
        name: "x".into(),
        tags: vec!["a".into(), "b".into()],
        ..Default::default()
    };

    // Vtable mode: reflect() borrows self.
    let r = msg.reflect();
    assert!(matches!(msg.reflect(), ReflectCow::Borrowed(_)));

    let md = r.message_descriptor();
    assert!(matches!(r.get(md.field(1).unwrap()), ValueRef::I32(7)));
    assert!(matches!(r.get(md.field(2).unwrap()), ValueRef::String("x")));
    let ValueRef::List(tags) = r.get(md.field(3).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(tags.len(), 2);
}

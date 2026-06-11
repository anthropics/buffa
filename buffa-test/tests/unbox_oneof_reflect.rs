//! Owned vtable reflection over an inline (unboxed) oneof variant.
//!
//! `unbox_oneof.proto` is built with `unbox_oneof_in` +
//! `reflect_mode(VTable)`. The generated owned `ReflectMessage` oneof arm
//! binds the variant payload as `&Small` (inline) rather than `&Box<Small>`,
//! so `get`/`has` must borrow it without the double deref used for boxed
//! variants.

use buffa_descriptor::reflect::{Reflectable, ValueRef};
use buffa_test::unbox_oneof::__buffa::oneof::envelope::Body;
use buffa_test::unbox_oneof::{Envelope, Large, Small};

#[test]
fn owned_vtable_reflects_inline_oneof_variant() {
    let msg = Envelope {
        body: Some(Body::Small(Small {
            value: 21,
            ..Default::default()
        })),
        ..Default::default()
    };

    let r = msg.reflect();
    let md = r.message_descriptor();
    let small_field = md.field(1).unwrap();
    assert!(r.has(small_field));
    let ValueRef::Message(cow) = r.get(small_field) else {
        panic!("expected Message for the inline variant")
    };
    let small_md = cow.message_descriptor();
    assert!(matches!(
        cow.get(small_md.field(1).unwrap()),
        ValueRef::I32(21)
    ));
}

#[test]
fn owned_vtable_reflects_boxed_sibling_variant() {
    let msg = Envelope {
        body: Some(Body::Large(Box::new(Large {
            label: "big".into(),
            ..Default::default()
        }))),
        ..Default::default()
    };

    let r = msg.reflect();
    let md = r.message_descriptor();
    let ValueRef::Message(cow) = r.get(md.field(2).unwrap()) else {
        panic!("expected Message for the boxed variant")
    };
    let large_md = cow.message_descriptor();
    assert!(matches!(
        cow.get(large_md.field(1).unwrap()),
        ValueRef::String("big")
    ));
}

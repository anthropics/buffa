//! Vtable reflection over a message generated with `string_type(SmolStr)`.
//!
//! The repeated-string field is `Vec<SmolStr>` in the owned struct, so its
//! reflective `get()` (`ValueRef::List(&self.items)`) relies on
//! `ReflectElement for SmolStr` in `buffa-descriptor` (feature `smol_str`).
//! Singular string fields reflect via deref regardless of the repr, and map
//! string keys/values stay `String`.

use buffa_descriptor::reflect::{Reflectable, ValueRef};
use buffa_test::vtable_string_repr::Labels;

#[test]
fn smol_str_repeated_field_reflects() {
    let labels = Labels {
        name: "svc".into(),
        items: vec!["a".into(), "bb".into(), "ccc".into()],
        ..Default::default()
    };

    let r = labels.reflect();
    let md = r.message_descriptor();

    // Singular SmolStr string (field 1) — reflects via deref.
    assert!(matches!(
        r.get(md.field(1).unwrap()),
        ValueRef::String("svc")
    ));

    // Repeated SmolStr (field 2) — the element path through ReflectElement.
    let ValueRef::List(items) = r.get(md.field(2).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(items.len(), 3);
    assert!(matches!(items.get(0), Some(ValueRef::String("a"))));
    assert!(matches!(items.get(2), Some(ValueRef::String("ccc"))));

    let mut collected = Vec::new();
    items.for_each(&mut |v| {
        if let ValueRef::String(s) = v {
            collected.push(s.to_string());
        }
    });
    assert_eq!(collected, vec!["a", "bb", "ccc"]);
}

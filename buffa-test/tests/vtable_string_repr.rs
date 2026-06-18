//! Vtable reflection + JSON over a message generated with a custom string type.
//!
//! `vtable_string_repr.proto` is compiled with a crate-local newtype
//! (`crate::vtable_string_repr::LocalStr`) as the string representation. The
//! repeated field is `Vec<LocalStr>`, so both the reflective `get()`
//! (`ValueRef::List(&self.items)`) and the repeated-string JSON path need the
//! codegen-emitted `ReflectElement` / `ProtoElemJson` impls — which compile
//! only because the type is local to this crate (a foreign type would violate
//! the orphan rule). Singular string fields reflect via deref regardless of the
//! repr, and map string keys/values stay `String`.

use buffa_descriptor::reflect::{Reflectable, ValueRef};
use buffa_test::vtable_string_repr::Labels;

#[test]
fn custom_repeated_field_reflects() {
    let labels = Labels {
        name: "svc".into(),
        items: vec!["a".into(), "bb".into(), "ccc".into()],
        ..Default::default()
    };

    let r = labels.reflect();
    let md = r.message_descriptor();

    // Singular custom string (field 1) — reflects via deref.
    assert!(matches!(
        r.get(md.field(1).unwrap()),
        ValueRef::String("svc")
    ));

    // Repeated custom string (field 2) — the element path through the emitted
    // `ReflectElement`.
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

#[test]
fn custom_repeated_field_json_roundtrip() {
    use buffa::Message;

    let labels = Labels {
        name: "svc".into(),
        items: vec!["a".into(), "bb".into(), "ccc".into()],
        ..Default::default()
    };

    // Repeated custom strings serialize as a JSON array; the singular field uses
    // the `proto_string` with-module. Both round-trip back to the same value.
    let json = serde_json::to_string(&labels).expect("serialize");
    assert!(json.contains(r#""name":"svc""#), "{json}");
    assert!(json.contains(r#""items":["a","bb","ccc"]"#), "{json}");
    let back: Labels = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, labels);

    // Wire format is representation-independent.
    let wire = labels.encode_to_vec();
    assert_eq!(
        Labels::decode(&mut wire.as_slice()).expect("decode"),
        labels
    );
}

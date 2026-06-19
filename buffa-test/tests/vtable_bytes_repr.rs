//! Custom-bytes element using a crate-local newtype (`LocalBytes`) under vtable
//! reflection + JSON. The bytes-side mirror of `vtable_string_repr`: the
//! `chunks` field is `Vec<LocalBytes>` and the `tagged` field is
//! `HashMap<String, LocalBytes>`, exercising the codegen-emitted `ReflectElement`
//! (so the reflective `get` returns `ValueRef::List` / `ValueRef::Map`) and the
//! emitted base64 `ProtoElemJson` impl (proto3 JSON renders `bytes` as base64).
//! The type is local so those emitted impls clear the orphan rule.

use buffa::Message;
use buffa_descriptor::reflect::{Reflectable, ValueRef};
use buffa_test::vtable_bytes_repr::{Blob, LocalBytes};

fn sample() -> Blob {
    Blob {
        head: LocalBytes(b"hi".to_vec()),
        chunks: vec![
            LocalBytes(b"a".to_vec()),
            LocalBytes(b"bb".to_vec()),
            LocalBytes(b"ccc".to_vec()),
        ],
        tagged: [("k".to_string(), LocalBytes(b"z".to_vec()))]
            .into_iter()
            .collect(),
        ..Default::default()
    }
}

#[test]
fn custom_repeated_bytes_field_types_and_binary_roundtrip() {
    let m = sample();
    let _: &LocalBytes = &m.head;
    let _: &::buffa::alloc::vec::Vec<LocalBytes> = &m.chunks;

    let wire = m.encode_to_vec();
    let back = Blob::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(back, m);
    // Wire format is representation-independent.
    assert_eq!(back.encode_to_vec(), wire);
}

#[test]
fn custom_repeated_bytes_json_roundtrip_base64() {
    let m = sample();
    // proto3 JSON renders bytes as base64, via the emitted `ProtoElemJson` for
    // the repeated element and the `bytes` with-module for the singular field.
    let json = serde_json::to_string(&m).expect("serialize");
    assert!(json.contains(r#""head":"aGk=""#), "{json}");
    assert!(
        json.contains(r#""chunks":["YQ==","YmI=","Y2Nj"]"#),
        "{json}"
    );
    // Map value is also base64 (custom bytes map value via proto_map).
    assert!(json.contains(r#""tagged":{"k":"eg=="}"#), "{json}");
    let back: Blob = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, m);
}

#[test]
fn custom_bytes_map_value_text_roundtrip() {
    let m = sample();
    let text = buffa::text::encode_to_string(&m);
    let back: Blob = buffa::text::decode_from_str(&text).expect("parse text");
    assert_eq!(back, m);
}

#[test]
fn custom_repeated_bytes_vtable_reflect() {
    let m = sample();
    let r = m.reflect();
    let md = r.message_descriptor();

    // Repeated `chunks` (field 2) reflects as a list of byte slices, dispatching
    // through the emitted `ReflectElement for LocalBytes`.
    match r.get(md.field(2).expect("field 2")) {
        ValueRef::List(list) => {
            assert_eq!(list.len(), 3);
            let mut got: ::buffa::alloc::vec::Vec<::buffa::alloc::vec::Vec<u8>> =
                ::buffa::alloc::vec::Vec::new();
            list.for_each(&mut |v| match v {
                ValueRef::Bytes(b) => got.push(b.to_vec()),
                other => panic!("expected Bytes element, got {other:?}"),
            });
            assert_eq!(got, [b"a".to_vec(), b"bb".to_vec(), b"ccc".to_vec()]);
        }
        other => panic!("expected List, got {other:?}"),
    }

    // Singular `head` (field 1) reflects as bytes via `Deref<[u8]>`.
    match r.get(md.field(1).expect("field 1")) {
        ValueRef::Bytes(b) => assert_eq!(b, b"hi"),
        other => panic!("expected Bytes, got {other:?}"),
    }

    // Map `tagged` (field 3) reflects as a map whose values dispatch through the
    // emitted `ReflectElement for LocalBytes` (HashMap<_, V>: ReflectMap where
    // V: ReflectElement).
    match r.get(md.field(3).expect("field 3")) {
        ValueRef::Map(map) => {
            assert_eq!(map.len(), 1);
            match map.get_str("k") {
                Some(ValueRef::Bytes(b)) => assert_eq!(b, b"z"),
                other => panic!("expected Bytes map value, got {other:?}"),
            }
        }
        other => panic!("expected Map, got {other:?}"),
    }
}

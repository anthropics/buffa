//! use_bytes_type(): bytes::Bytes instead of Vec<u8>, including views.

// Regression: use_bytes_type() produced uncompilable code for decode/merge
// (merge_bytes wants &mut Vec<u8>, struct field was bytes::Bytes).
// The module above compiling is most of the test; these verify correctness.
use crate::basic_bytes::Person;
use buffa::Message;

#[test]
fn test_bytes_type_singular_roundtrip() {
    let msg = Person {
        id: 1,
        avatar: bytes::Bytes::from_static(&[0xDE, 0xAD, 0xBE, 0xEF]),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let decoded = Person::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(&decoded.avatar[..], &[0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn test_bytes_type_wire_compatible_with_vec() {
    // The wire format must be identical regardless of Rust type.
    let vec_msg = crate::basic::Person {
        id: 42,
        avatar: vec![1, 2, 3],
        ..Default::default()
    };
    let vec_wire = vec_msg.encode_to_vec();
    // Decode the Vec<u8>-encoded bytes with the Bytes-typed struct.
    let bytes_decoded = Person::decode(&mut vec_wire.as_slice()).expect("decode");
    assert_eq!(bytes_decoded.id, 42);
    assert_eq!(&bytes_decoded.avatar[..], &[1, 2, 3]);
}

#[test]
fn test_bytes_type_clear() {
    let mut msg = Person {
        avatar: bytes::Bytes::from_static(b"hello"),
        ..Default::default()
    };
    assert!(!msg.avatar.is_empty());
    msg.clear();
    assert!(msg.avatar.is_empty(), "clear must reset Bytes field");
}

#[test]
fn test_bytes_type_view_to_owned() {
    // Views borrow &[u8]; to_owned_message must produce Bytes (not Vec<u8>)
    // when use_bytes_type() is active. Previously this emitted .to_vec()
    // unconditionally, failing to compile.
    use crate::basic_bytes::__buffa::view::PersonView;
    use buffa::MessageView;
    let msg = Person {
        id: 7,
        avatar: bytes::Bytes::from_static(&[0xCA, 0xFE]),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = PersonView::decode_view(&wire).expect("decode_view");
    assert_eq!(view.avatar, &[0xCA, 0xFE][..]);
    // to_owned_message produces Bytes.
    let owned: Person = view.to_owned_message();
    assert_eq!(&owned.avatar[..], &[0xCA, 0xFE]);
    assert_eq!(owned.encode_to_vec(), wire);
}

// ── BytesContexts: repeated + oneof + optional + map ────────────────────
//
// These tests disprove a static-analysis false positive that claimed
// `bytes_to_owned(..., quote!{*v})` generates `copy_from_slice(*v)` where
// *v: [u8] (unsized). The analysis missed that the surrounding generated
// code double-references the binding:
//
//   oneof:    self.f.as_ref().map(|v| match v { Variant(v) => ... })
//             → outer v: &ViewEnum, match ergonomics → inner v: &&[u8]
//   repeated: Vec<&[u8]>::iter().map(|b| ...)
//             → b: &&[u8]
//
// So `*v` / `*b` yield `&[u8]` (Sized), and `copy_from_slice(*v)` is fine.
//
// The bytes_variant build block compiles BytesContexts with use_bytes_type()
// + generate_views=true; compilation alone is the primary assertion.

use crate::basic_bytes::__buffa::oneof::bytes_contexts::Choice as ChoiceOneof;
use crate::basic_bytes::__buffa::view::BytesContextsView;
use crate::basic_bytes::BytesContexts;

#[test]
fn test_bytes_type_repeated_view_to_owned() {
    use buffa::MessageView;
    let msg = BytesContexts {
        many: vec![
            bytes::Bytes::from_static(b"a"),
            bytes::Bytes::from_static(b"bc"),
            bytes::Bytes::from_static(b""),
        ],
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = BytesContextsView::decode_view(&wire).expect("decode_view");
    let borrowed: Vec<&[u8]> = view.many.iter().copied().collect();
    assert_eq!(borrowed, vec![&b"a"[..], &b"bc"[..], &b""[..]]);

    // to_owned_message: Vec<&[u8]> → Vec<bytes::Bytes>.
    // Generated: self.many.iter().map(|b| bytes_from_source(__buffa_src, b)).collect()
    // where b: &&[u8]; the &[u8] arg auto-derefs.
    let owned: BytesContexts = view.to_owned_message();
    assert_eq!(owned.many.len(), 3);
    assert_eq!(&owned.many[0][..], b"a");
    assert_eq!(&owned.many[1][..], b"bc");
    assert_eq!(&owned.many[2][..], b"");
    assert_eq!(owned.encode_to_vec(), wire);
}

#[test]
fn test_bytes_type_oneof_view_to_owned() {
    use buffa::MessageView;
    let msg = BytesContexts {
        choice: Some(ChoiceOneof::Raw(bytes::Bytes::from_static(&[
            0x00, 0xFF, 0x7F,
        ]))),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = BytesContextsView::decode_view(&wire).expect("decode_view");

    // to_owned_message: view oneof ChoiceView::Raw(&[u8]) → owned Choice::Raw(Bytes).
    // Generated: self.choice.as_ref().map(|v| match v {
    //     ChoiceView::Raw(v) => Choice::Raw(bytes_from_source(__buffa_src, v)), ... })
    // Match ergonomics: v in the arm is &&[u8]; the &[u8] arg auto-derefs.
    let owned: BytesContexts = view.to_owned_message();
    match &owned.choice {
        Some(ChoiceOneof::Raw(b)) => assert_eq!(&b[..], &[0x00, 0xFF, 0x7F]),
        other => panic!("expected Choice::Raw, got {other:?}"),
    }
    assert_eq!(owned.encode_to_vec(), wire);
}

#[test]
fn test_bytes_type_optional_view_to_owned() {
    use buffa::MessageView;
    // Both Some and None round-trips.
    #[rustfmt::skip]
    let cases: &[(Option<&[u8]>, &str)] = &[
        (Some(b"present"), "Some"),
        (None,             "None"),
    ];
    for &(input, label) in cases {
        let msg = BytesContexts {
            maybe: input.map(bytes::Bytes::copy_from_slice),
            ..Default::default()
        };
        let wire = msg.encode_to_vec();
        let view = BytesContextsView::decode_view(&wire).expect("decode_view");
        let owned: BytesContexts = view.to_owned_message();
        assert_eq!(
            owned.maybe.as_deref(),
            input,
            "optional bytes {label} round-trip"
        );
    }
}

#[test]
fn test_bytes_type_view_to_owned_from_source_zero_copy() {
    // Issue #52: to_owned_from_source(Some(&buf)) must slice_ref into the
    // source buffer for singular/optional/repeated/oneof bytes_fields.
    use buffa::MessageView;
    let msg = BytesContexts {
        many: vec![bytes::Bytes::from_static(b"aaaa"), bytes::Bytes::new()],
        maybe: Some(bytes::Bytes::from_static(b"bbbb")),
        choice: Some(ChoiceOneof::Raw(bytes::Bytes::from_static(b"cccc"))),
        ..Default::default()
    };
    let buf = bytes::Bytes::from(msg.encode_to_vec());
    let in_buf = |p: *const u8| {
        let r = buf.as_ptr() as usize..buf.as_ptr() as usize + buf.len();
        r.contains(&(p as usize))
    };

    let view = BytesContextsView::decode_view(&buf).expect("decode_view");
    let owned = view.to_owned_from_source(Some(&buf));

    assert_eq!(&owned.many[0][..], b"aaaa");
    assert!(
        in_buf(owned.many[0].as_ptr()),
        "repeated[0] should slice_ref"
    );
    assert!(owned.many[1].is_empty());
    assert_eq!(owned.maybe.as_deref(), Some(&b"bbbb"[..]));
    assert!(
        in_buf(owned.maybe.as_ref().unwrap().as_ptr()),
        "optional should slice_ref"
    );
    match &owned.choice {
        Some(ChoiceOneof::Raw(b)) => {
            assert_eq!(&b[..], b"cccc");
            assert!(in_buf(b.as_ptr()), "oneof should slice_ref");
        }
        other => panic!("expected Choice::Raw, got {other:?}"),
    }
    assert_eq!(owned.encode_to_vec(), buf);
}

#[test]
fn test_bytes_type_nested_to_owned_from_source_zero_copy() {
    // Issue #52: __buffa_src must thread through nested-message recursion.
    use crate::basic_bytes::__buffa::view::BytesNestedView;
    use crate::basic_bytes::BytesNested;
    use buffa::MessageView;
    let msg = BytesNested {
        inner: buffa::MessageField::some(BytesContexts {
            singular: bytes::Bytes::from_static(b"nested-payload"),
            ..Default::default()
        }),
        ..Default::default()
    };
    let buf = bytes::Bytes::from(msg.encode_to_vec());
    let view = BytesNestedView::decode_view(&buf).expect("decode_view");
    let owned = view.to_owned_from_source(Some(&buf));
    let inner_bytes = &owned.inner.singular;
    assert_eq!(&inner_bytes[..], b"nested-payload");
    let r = buf.as_ptr() as usize..buf.as_ptr() as usize + buf.len();
    assert!(
        r.contains(&(inner_bytes.as_ptr() as usize)),
        "nested bytes field should slice_ref into parent buf"
    );
}

// ── JSON: use_bytes_type() + generate_json(true) ─────────────────────────
//
// The bytes_variant build enables both. Runtime support comes from:
//   - json_helpers::bytes::serialize takes &[u8] → Bytes deref-coerces
//   - json_helpers::bytes::deserialize is generic over T: From<Vec<u8>>;
//     the field type pins T at the serde call site (Vec<u8> or Bytes)
//   - json_helpers::opt_bytes generic over AsRef<[u8]> / From<Vec<u8>>
//   - ProtoElemJson for bytes::Bytes (for repeated bytes → proto_seq)
// No codegen shims — type inference does the work.

#[test]
fn test_bytes_type_json_all_contexts_roundtrip() {
    let msg = BytesContexts {
        singular: bytes::Bytes::from_static(&[0x01]),
        maybe: Some(bytes::Bytes::from_static(&[0x02, 0x03])),
        many: vec![
            bytes::Bytes::from_static(&[0x04]),
            bytes::Bytes::from_static(b""),
        ],
        choice: Some(ChoiceOneof::Raw(bytes::Bytes::from_static(&[0xDE, 0xAD]))),
        by_key: [("k".to_string(), vec![0x05])].into_iter().collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");

    // Spot-check proto3-JSON bytes → base64 in each context.
    assert!(json.contains(r#""singular":"AQ==""#), "singular: {json}");
    assert!(json.contains(r#""maybe":"AgM=""#), "optional: {json}");
    assert!(json.contains(r#""many":["BA==",""]"#), "repeated: {json}");
    assert!(json.contains(r#""raw":"3q0=""#), "oneof: {json}");
    assert!(json.contains(r#""k":"BQ==""#), "map value: {json}");

    let back: BytesContexts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(&back.singular[..], &[0x01]);
    assert_eq!(back.maybe.as_deref(), Some(&[0x02, 0x03][..]));
    assert_eq!(back.many, msg.many);
    match &back.choice {
        Some(ChoiceOneof::Raw(b)) => assert_eq!(&b[..], &[0xDE, 0xAD]),
        other => panic!("expected Choice::Raw, got {other:?}"),
    }
    assert_eq!(back.by_key, msg.by_key);
}

#[test]
fn test_bytes_type_json_nulls_and_absence() {
    // null → default/None/empty in each context.
    let json = r#"{"singular":null,"maybe":null,"many":null,"raw":null}"#;
    let back: BytesContexts = serde_json::from_str(json).expect("deserialize nulls");
    assert!(back.singular.is_empty(), "singular null → empty Bytes");
    assert!(back.maybe.is_none(), "optional null → None");
    assert!(back.many.is_empty(), "repeated null → empty Vec");
    assert!(back.choice.is_none(), "oneof bytes null → variant not set");

    // Absent → defaults (proto3 JSON: absent ≡ default).
    let back: BytesContexts = serde_json::from_str("{}").expect("deserialize empty");
    assert!(back.singular.is_empty());
    assert!(back.maybe.is_none());
}

#[test]
fn test_bytes_type_json_cross_decodes_external_json() {
    // A receiver with use_bytes_type() must accept proto3-JSON produced by
    // any conformant sender (Go protojson, Java JsonFormat, etc.). That means
    // correct base64 handling regardless of the backing Rust type. The
    // round-trip test above proves self-consistency; this proves conformance
    // to the wire format by decoding hand-constructed canonical JSON.
    let external = r#"{
        "singular": "yv66vg==",
        "maybe":    "AAECAwQFBgcICQ==",
        "many":     ["", "QQ=="],
        "raw":      "AP9/"
    }"#;
    let back: BytesContexts = serde_json::from_str(external).expect("deserialize external");
    assert_eq!(&back.singular[..], &[0xCA, 0xFE, 0xBA, 0xBE]);
    assert_eq!(
        back.maybe.as_deref(),
        Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9][..])
    );
    assert_eq!(back.many.len(), 2);
    assert!(back.many[0].is_empty());
    assert_eq!(&back.many[1][..], b"A");
    match &back.choice {
        Some(ChoiceOneof::Raw(b)) => assert_eq!(&b[..], &[0x00, 0xFF, 0x7F]),
        other => panic!("expected Choice::Raw, got {other:?}"),
    }
}

#[test]
fn test_bytes_type_map_value_stays_vec() {
    // bytes_fields config does NOT propagate into map key/value types
    // (map_rust_type_from_entry → scalar_rust_type hardcodes Vec<u8>).
    // map_to_owned_expr correspondingly uses .to_vec(), not bytes_to_owned().
    // This test pins that agreement: if one side changes, compilation breaks.
    use buffa::MessageView;
    let msg = BytesContexts {
        by_key: [("k".to_string(), b"v".to_vec())].into_iter().collect(),
        ..Default::default()
    };
    // Type assertion: map value is Vec<u8>, not bytes::Bytes, even under
    // use_bytes_type().
    let _: &std::collections::HashMap<String, Vec<u8>> = &msg.by_key;

    let wire = msg.encode_to_vec();
    let view = BytesContextsView::decode_view(&wire).expect("decode_view");
    let owned: BytesContexts = view.to_owned_message();
    assert_eq!(owned.by_key.get("k").map(Vec::as_slice), Some(&b"v"[..]));
}

#[test]
fn test_bytes_type_view_encode_roundtrip() {
    // ViewEncode × use_bytes_type: view-side bytes fields are `&[u8]` while
    // owned-side are `bytes::Bytes`. The encode-stmt builders duck-type
    // through `encode_bytes(&self.#ident, buf)` (AsRef<[u8]>) so this should
    // be wire-identical to the owned encode across all bytes-field shapes.
    use buffa::{MessageView, ViewEncode};
    let msg = BytesContexts {
        many: vec![
            bytes::Bytes::from_static(b"a"),
            bytes::Bytes::from_static(b"bc"),
        ],
        maybe: Some(bytes::Bytes::from_static(&[0x00, 0xFF])),
        choice: Some(ChoiceOneof::Raw(bytes::Bytes::from_static(b"o"))),
        by_key: [("k".to_string(), b"v".to_vec())].into_iter().collect(),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = BytesContextsView::decode_view(&wire).expect("decode_view");
    let view_wire = view.encode_to_vec();
    // Decode-then-compare rather than byte-equality (map iteration order is
    // hash-seed dependent on the owned side).
    let back = BytesContexts::decode_from_slice(&view_wire).expect("decode");
    assert_eq!(back, msg);
    // Singular bytes field via Person.avatar (no map → byte-exact).
    use crate::basic_bytes::__buffa::view::PersonView;
    let p = Person {
        id: 1,
        avatar: bytes::Bytes::from_static(&[0xCA, 0xFE, 0xBE, 0xEF]),
        ..Default::default()
    };
    let p_wire = p.encode_to_vec();
    let p_view = PersonView::decode_view(&p_wire).expect("decode_view");
    assert_eq!(p_view.encode_to_vec(), p_wire);
}

//! Generated `FooOwnedView` wrapper types: per-field accessors over an
//! `OwnedView<FooView<'static>>`, with all borrows tied to `&self`.

use buffa::Message;

// Natural-path re-export: the wrapper is reachable at the package root, like
// the view struct (canonical path: `__buffa::view::PersonOwnedView`).
use crate::basic::{Address, Person, PersonOwnedView};

fn sample_person() -> Person {
    let mut msg = Person::default();
    msg.id = 42;
    msg.name = "Alice".into();
    msg.avatar = vec![0xDE, 0xAD];
    msg.tags = vec!["x".into(), "y".into()];
    msg.address.get_or_insert_default().street = "1 Main St".into();
    msg
}

#[test]
fn test_owned_view_wrapper_field_accessors() {
    let msg = sample_person();
    let bytes = bytes::Bytes::from(msg.encode_to_vec());
    let owned = PersonOwnedView::decode(bytes).expect("decode");

    assert_eq!(owned.id(), 42);
    assert_eq!(owned.name(), "Alice");
    assert_eq!(owned.avatar(), &[0xDE, 0xAD]);

    let tags: Vec<&str> = owned.tags().iter().copied().collect();
    assert_eq!(tags, vec!["x", "y"]);

    let address = owned.address().as_option().expect("address set");
    assert_eq!(address.street, "1 Main St");

    // Zero-copy: the &str returned by the accessor points into the wrapper's
    // retained Bytes buffer, not into a copy.
    let buf_range = owned.bytes().as_ptr_range();
    assert!(buf_range.contains(&owned.name().as_ptr()));
}

#[test]
fn test_owned_view_wrapper_view_escape_hatch() {
    let bytes = bytes::Bytes::from(sample_person().encode_to_vec());
    let owned = PersonOwnedView::decode(bytes).expect("decode");

    // `view()` exposes the full reborrowed view; field access there agrees
    // with the accessor methods.
    let view = owned.view();
    assert_eq!(view.name, owned.name());
    assert_eq!(view.id, owned.id());
}

#[test]
fn test_owned_view_wrapper_owned_roundtrip() {
    let msg = sample_person();
    let owned = PersonOwnedView::from_owned(&msg).expect("from_owned");
    let back: Person = owned.to_owned_message();
    assert_eq!(back, msg);
}

#[test]
fn test_owned_view_wrapper_is_send_and_static() {
    let bytes = bytes::Bytes::from(sample_person().encode_to_vec());
    let owned = PersonOwnedView::decode(bytes).expect("decode");

    // The wrapper is 'static + Send: it can move into a spawned thread and be
    // read there.
    let handle = std::thread::spawn(move || owned.name().to_owned());
    assert_eq!(handle.join().expect("join"), "Alice");
}

#[test]
fn test_owned_view_wrapper_conversions_and_bytes() {
    let encoded = sample_person().encode_to_vec();
    let bytes = bytes::Bytes::from(encoded.clone());

    // Wrapper ⇄ raw OwnedView conversions.
    let raw =
        buffa::OwnedView::<crate::basic::__buffa::view::PersonView<'static>>::decode(bytes.clone())
            .expect("decode raw");
    let wrapped = PersonOwnedView::from(raw);
    assert_eq!(wrapped.name(), "Alice");
    let raw_again: buffa::OwnedView<crate::basic::__buffa::view::PersonView<'static>> =
        wrapped.into();
    assert_eq!(raw_again.reborrow().name, "Alice");

    // bytes() / into_bytes() expose the retained buffer.
    let owned = PersonOwnedView::decode(bytes).expect("decode");
    assert_eq!(owned.bytes().as_ref(), encoded.as_slice());
    assert_eq!(owned.into_bytes().as_ref(), encoded.as_slice());
}

#[test]
fn test_owned_view_wrapper_decode_with_options_limit() {
    let bytes = bytes::Bytes::from(sample_person().encode_to_vec());
    let opts = buffa::DecodeOptions::new().with_max_message_size(2);
    assert!(PersonOwnedView::decode_with_options(bytes, &opts).is_err());
}

#[test]
fn test_owned_view_wrapper_default_message_field_unset() {
    let msg = Person {
        name: "no-address".into(),
        ..Default::default()
    };
    let owned = PersonOwnedView::from_owned(&msg).expect("from_owned");
    assert!(owned.address().as_option().is_none());
    // Unrelated check: an unset Address still derefs to the default instance.
    assert_eq!(owned.address().street, Address::default().street);
}

mod view_json_types {
    use crate::view_json::__buffa::oneof::with_oneof::Value as ValueOneof;
    use crate::view_json::__buffa::view::oneof::with_oneof::Value as ValueViewOneof;
    use crate::view_json::{
        Scalars, ScalarsOwnedView, WithMaps, WithMapsOwnedView, WithOneof, WithOneofOwnedView,
    };

    #[test]
    fn oneof_accessor_returns_active_variant() {
        let msg = WithOneof {
            value: Some(ValueOneof::Text("hello".into())),
            ..Default::default()
        };
        let owned = WithOneofOwnedView::from_owned(&msg).expect("from_owned");
        match owned.value() {
            Some(ValueViewOneof::Text(s)) => assert_eq!(*s, "hello"),
            other => panic!("expected Text variant, got {other:?}"),
        }
    }

    #[test]
    fn oneof_accessor_unset_is_none() {
        let owned = WithOneofOwnedView::from_owned(&WithOneof::default()).expect("from_owned");
        assert!(owned.value().is_none());
    }

    #[test]
    fn map_accessor_exposes_entries() {
        let msg = WithMaps {
            labels: [
                ("env".into(), "prod".into()),
                ("region".into(), "us-east".into()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let owned = WithMapsOwnedView::from_owned(&msg).expect("from_owned");
        assert_eq!(owned.labels().len(), 2);
        assert!(owned.by_id().is_empty());
    }

    #[test]
    fn wrapper_json_matches_owned_json() {
        let msg = Scalars {
            i32: -42,
            s: "hello world".into(),
            by: vec![0xDE, 0xAD],
            ..Default::default()
        };
        let owned = ScalarsOwnedView::from_owned(&msg).expect("from_owned");
        let json_wrapper = serde_json::to_string(&owned).expect("serialize wrapper");
        let json_owned = serde_json::to_string(&msg).expect("serialize owned");
        assert_eq!(json_wrapper, json_owned);
    }
}

mod view_family {
    use super::*;

    /// Generic over the owned message via `HasMessageView`: decode into the
    /// handle, then reach the reborrowed view, the buffer, and the owned
    /// message through the trait's structural bounds only — no concrete type
    /// names beyond the call site's turbofish.
    fn decode_via_family<M>(bytes: bytes::Bytes) -> (M, usize)
    where
        M: buffa::HasMessageView,
        M::View<'static>: buffa::ViewReborrow,
    {
        let handle = M::decode_view_handle(bytes).expect("decode");
        let raw = handle.as_ref();
        let _view = raw.reborrow();
        let len = raw.bytes().len();
        (raw.to_owned_message(), len)
    }

    #[test]
    fn test_has_message_view_generic_roundtrip() {
        let msg = sample_person();
        let encoded = msg.encode_to_vec();
        let (decoded, len): (Person, usize) =
            decode_via_family(bytes::Bytes::from(encoded.clone()));
        assert_eq!(decoded, msg);
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_has_message_view_generic_roundtrip_with_oneof() {
        use crate::view_json::__buffa::oneof::with_oneof::Value as ValueOneof;
        use crate::view_json::WithOneof;

        let msg = WithOneof {
            value: Some(ValueOneof::Text("family".into())),
            ..Default::default()
        };
        let (decoded, _len): (WithOneof, usize) =
            decode_via_family(bytes::Bytes::from(msg.encode_to_vec()));
        assert_eq!(decoded, msg);
    }

    #[test]
    fn test_has_message_view_names_the_generated_wrapper() {
        // The `ViewHandle` associated type resolves to the generated wrapper.
        fn assert_wrapper<M: buffa::HasMessageView<ViewHandle = W>, W>() {}
        assert_wrapper::<Person, PersonOwnedView>();
    }

    #[test]
    fn test_has_message_view_handle_and_view_are_send_sync() {
        // The async-motivated thread-safety contract, asserted generically.
        fn assert_send_sync<M: buffa::HasMessageView>() {
            fn check<T: Send + Sync>() {}
            check::<M::ViewHandle>();
            check::<M::View<'static>>();
        }
        assert_send_sync::<Person>();
    }
}

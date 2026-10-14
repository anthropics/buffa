//! Nested-types modules that get a trailing `_` because `snake_case(Name)`
//! is taken in their scope.
//!
//! `modcollide*.proto` and `modrace*.proto` cover a sub-package of that name
//! (issue #135): `package modcollide` has `message Oof { message Inner {} }`,
//! and `package modcollide.oof` is also `mod oof`. The nested-types module is
//! `oof_`; the struct `Oof` and the sub-package module `oof` keep their names.
//!
//! `nested_module_names.proto` covers a generated type of that name
//! (`message item` declares `struct item`) and the reserved names (`crate`,
//! `arbitrary`, `oneof`).
//!
//! Compiling the fixtures is the primary assertion; the tests pin the paths.

use buffa::Message;

#[test]
fn test_nested_module_deconflicted_from_subpackage() {
    // Nested type `Inner` lives under the deconflicted `oof_` module.
    let msg = crate::modcollide::Oof {
        inner: buffa::MessageField::some(crate::modcollide::oof_::Inner {
            x: 7,
            ..Default::default()
        }),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let back = crate::modcollide::Oof::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(back.inner.as_option().map(|i| i.x), Some(7));
}

#[test]
fn test_subpackage_message_keeps_natural_path() {
    // The sub-package message lives at the natural `modcollide::oof::Thing`,
    // unaffected by the nested-module deconfliction.
    let t = crate::modcollide::oof::Thing {
        y: 9,
        // The sub-package message's OWN nested type is emitted normally under
        // `oof::thing::Detail` — deconfliction does not leak into sub-packages.
        detail: buffa::MessageField::some(crate::modcollide::oof::thing::Detail {
            z: 3,
            ..Default::default()
        }),
        ..Default::default()
    };
    let wire = t.encode_to_vec();
    let back = crate::modcollide::oof::Thing::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(back.y, 9);
    assert_eq!(back.detail.as_option().map(|d| d.z), Some(3));
}

#[test]
fn test_multi_message_race_distinct_modules() {
    // `Oof` and `Oof_` both collide with sub-packages `oof` and `oof_`. Their
    // nested-types modules deconflict to distinct names (`oof__`, `oof___`), and
    // the two sub-packages keep their natural names — all four coexist. These
    // path references resolving is the assertion.
    let a = crate::modrace::Oof {
        inner: buffa::MessageField::some(crate::modrace::oof__::Inner {
            x: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    let b = crate::modrace::Oof_ {
        inner: buffa::MessageField::some(crate::modrace::oof___::Inner {
            x: 2,
            ..Default::default()
        }),
        ..Default::default()
    };
    let _thing = crate::modrace::oof::Thing::default();
    let _widget = crate::modrace::oof_::Widget::default();

    assert_eq!(a.inner.as_option().map(|i| i.x), Some(1));
    assert_eq!(b.inner.as_option().map(|i| i.x), Some(2));
    // Round-trips confirm the distinct modules are wired through encode/decode.
    assert_eq!(
        crate::modrace::Oof::decode(&mut a.encode_to_vec().as_slice())
            .unwrap()
            .inner
            .as_option()
            .map(|i| i.x),
        Some(1)
    );
}

#[test]
fn test_subpackage_deconflicted_module_has_the_same_name_in_the_view_tree() {
    // `Oof.InnerView` is a message, so `oof_::InnerView` is its struct, and
    // the view of `Oof.Inner` has its `__buffa::view` path only.
    use crate::modcollide::{self, __buffa::view};
    use buffa::MessageView;

    let msg = modcollide::Oof {
        inner: buffa::MessageField::some(modcollide::oof_::Inner {
            x: 5,
            ..Default::default()
        }),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let decoded = view::OofView::decode_view(&wire).expect("decode view");
    let inner: &view::oof_::InnerView<'_> = decoded.inner.as_option().expect("inner is set");
    assert_eq!(inner.x, 5);
    assert_eq!(modcollide::oof_::InnerView::default().y, 0);
}

#[test]
fn test_module_beside_a_type_of_its_name_gets_a_suffix() {
    use crate::nested_module_names::{self as nm, __buffa::oneof};
    use buffa::{EnumValue, MessageField};

    let item = nm::item {
        inner: MessageField::some(nm::item_::Inner {
            n: 1,
            ..Default::default()
        }),
        mode: EnumValue::Known(nm::item_::Mode::MODE_ONE),
        pick: Some(oneof::item_::Pick::Name("n".into())),
        ..Default::default()
    };
    let outer = nm::Outer {
        l: MessageField::some(nm::outer::leaf {
            deep: MessageField::some(nm::outer::leaf_::Deep {
                n: 2,
                ..Default::default()
            }),
            ..Default::default()
        }),
        m: MessageField::some(nm::outer::Mode {
            detail: MessageField::some(nm::outer::mode_::Detail {
                n: 3,
                ..Default::default()
            }),
            ..Default::default()
        }),
        e: EnumValue::Known(nm::outer::mode::MODE_ONE),
        a: MessageField::some(nm::outer::Arbitrary {
            pick: Some(oneof::outer::arbitrary_::Pick::A(4)),
            ..Default::default()
        }),
        s: MessageField::some(nm::outer::self_ {
            deep: MessageField::some(nm::outer::self__::Deep {
                n: 9,
                ..Default::default()
            }),
            ..Default::default()
        }),
        kind: Some(oneof::outer::Kind::X(5)),
        ..Default::default()
    };
    let msg = nm::Holder {
        item: MessageField::some(item),
        item_mode: EnumValue::Known(nm::item_::Mode::MODE_ONE),
        outer: MessageField::some(outer),
        deep: MessageField::some(nm::outer::leaf_::Deep {
            n: 6,
            ..Default::default()
        }),
        details: [(
            "k".to_string(),
            nm::outer::mode_::Detail {
                n: 7,
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        pick: Some(oneof::holder::Pick::PickItem(Box::new(nm::item_::Inner {
            n: 8,
            ..Default::default()
        }))),
        ..Default::default()
    };

    let wire = msg.encode_to_vec();
    assert_eq!(
        nm::Holder::decode(&mut wire.as_slice()).expect("decode"),
        msg
    );
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: nm::Holder = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn test_module_named_after_a_reserved_name_gets_a_suffix() {
    use crate::nested_module_names::{self as nm, __buffa::view};
    use buffa::{MessageField, MessageName, MessageView};

    let msg = nm::Holder {
        crate_inner: MessageField::some(nm::crate_::Inner {
            n: 1,
            ..Default::default()
        }),
        super_inner: MessageField::some(nm::super_::Inner {
            n: 2,
            ..Default::default()
        }),
        self_inner: MessageField::some(nm::self_::Inner {
            n: 3,
            ..Default::default()
        }),
        arbitrary_inners: vec![nm::arbitrary_::Inner {
            n: 4,
            ..Default::default()
        }],
        oneof_inner: MessageField::some(nm::oneof_::Inner {
            n: 5,
            ..Default::default()
        }),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    assert_eq!(
        nm::Holder::decode(&mut wire.as_slice()).expect("decode"),
        msg
    );

    // The `__buffa` trees use the same module names, and the natural paths
    // re-export from them.
    let decoded = view::HolderView::decode_view(&wire).expect("decode view");
    let inner: &view::crate_::InnerView<'_> = decoded.crate_inner.as_option().expect("set");
    assert_eq!(inner.n, 1);
    let inner: &nm::self_::InnerView<'_> = decoded.self_inner.as_option().expect("set");
    assert_eq!(inner.n, 3);
    // `oneof_` is the view module of `Oneof` and, under
    // `__buffa::view::oneof`, the module of its view-oneof enum.
    let inner: &view::oneof_::InnerView<'_> = decoded.oneof_inner.as_option().expect("set");
    assert_eq!(inner.n, 5);
    let _: Option<view::oneof::oneof_::Pick<'_>> = None;

    // The proto names are unchanged.
    assert_eq!(
        nm::crate_::Inner::FULL_NAME,
        "test.nested_module_names.Crate.Inner"
    );
    assert_eq!(
        nm::item_::Inner::FULL_NAME,
        "test.nested_module_names.item.Inner"
    );
}

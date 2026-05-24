//! Issue #135: a message whose snake_case module name collides with a sibling
//! sub-package module.
//!
//! `package modcollide` has `message Oof { message Inner {} }` — `Oof`'s nested
//! types normally live in `mod oof`. `package modcollide.oof` also maps to
//! `mod oof`. Both land in `mod modcollide`, which previously produced an E0428
//! "module `oof` redefined" error. Codegen now deconflicts the nested-types
//! module to `oof_`, leaving the message struct (`Oof`) and the sub-package
//! module (`oof`) at their natural names. Compiling this file is the primary
//! assertion; the checks below pin the resulting paths.

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

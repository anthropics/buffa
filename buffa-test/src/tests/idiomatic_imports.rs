//! Runtime tests for `idiomatic_imports` codegen (file_per_package mode).
//!
//! The primary assertion is that `crate::idiomatic` compiles at all — the
//! `use`-backed short names must resolve and no import may go unreferenced.
//! These tests verify the shortened types behave identically to default
//! codegen at runtime: binary and JSON round-trips through every shortened
//! field shape, including the parent-qualified collision case and the
//! qualified nested-message scope.

use super::round_trip;
use crate::idiomatic::test::idiomatic::{self, Clash, Holder, Outer};
use crate::idiomatic::test::idiomatic_other::{Clash as OtherClash, Dep, DepKind};

fn sample_holder() -> Holder {
    Holder {
        dep: buffa::MessageField::some(Dep {
            label: "dep".into(),
            ..Default::default()
        }),
        other_clash: buffa::MessageField::some(OtherClash {
            n: 7,
            ..Default::default()
        }),
        local_clash: buffa::MessageField::some(Clash {
            m: 8,
            ..Default::default()
        }),
        at: buffa::MessageField::some(buffa_types::google::protobuf::Timestamp {
            seconds: 1_700_000_000,
            nanos: 42,
            ..Default::default()
        }),
        name: "short names".into(),
        data: vec![1, 2, 3],
        maybe: Some(-5),
        attrs: [("k".to_string(), 9_i64)].into_iter().collect(),
        kind: buffa::EnumValue::Known(DepKind::DEP_KIND_PRIMARY),
        more_deps: vec![Dep {
            label: "more".into(),
            ..Default::default()
        }],
        choice: Some(idiomatic::__buffa::oneof::holder::Choice::PickName(
            "picked".into(),
        )),
        ..Default::default()
    }
}

#[test]
fn binary_round_trip_through_shortened_types() {
    let msg = sample_holder();
    let decoded = round_trip(&msg);
    assert_eq!(decoded, msg);
    assert_eq!(decoded.dep.label, "dep");
    assert_eq!(decoded.other_clash.n, 7);
    assert_eq!(decoded.at.seconds, 1_700_000_000);
    assert_eq!(decoded.maybe, Some(-5));
    assert_eq!(decoded.attrs["k"], 9);
}

#[test]
fn json_round_trip_through_shortened_types() {
    let msg = sample_holder();
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: Holder = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn nested_scope_keeps_working_alongside_shortened_root() {
    let msg = Outer {
        inner: buffa::MessageField::some(idiomatic::outer::Inner {
            d: buffa::MessageField::some(Dep {
                label: "nested".into(),
                ..Default::default()
            }),
            s: "inner".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.inner.d.label, "nested");
    assert_eq!(decoded.inner.s, "inner");
}

//! Mixed-mode reflection: a vtable-mode parent embedding bridge-mode messages.
//!
//! `mixed_reflect.proto` (vtable) holds `mixedref.dep.Inner` (bridge, via
//! extern_path) in every message-typed position. The vtable accessors route
//! through `Inner`'s own `Reflectable::reflect()` / `ReflectElement` impls,
//! so each boundary degrades to an owned `DynamicMessage` snapshot instead of
//! failing to compile — the behavior `docs/investigations/reflection.md`
//! designs for. The parent itself stays zero-cost (`ReflectCow::Borrowed`).

use buffa_descriptor::reflect::{ReflectCow, ReflectMessage, Reflectable, ValueRef};
use buffa_test::mixed_reflect_dep::Inner;
use buffa_test::mixed_reflect_parent::__buffa::oneof::outer::Kind;
use buffa_test::mixed_reflect_parent::Outer;

fn inner(value: i32, label: &str) -> Inner {
    Inner {
        value,
        label: label.to_string(),
        ..Default::default()
    }
}

#[test]
fn parent_is_vtable_grade_bridge_field_degrades_to_owned() {
    let msg = Outer {
        single: buffa::MessageField::some(inner(7, "seven")),
        ..Default::default()
    };

    let r = msg.reflect();
    assert!(
        matches!(r, ReflectCow::Borrowed(_)),
        "vtable parent must borrow, not round-trip"
    );

    let md = r.message_descriptor();
    let fd = md.field(1).unwrap();
    assert!(r.has(fd));
    let ValueRef::Message(cow) = r.get(fd) else {
        panic!("expected Message for the bridge-grade field")
    };
    assert!(
        matches!(cow, ReflectCow::Owned(_)),
        "bridge-grade field must degrade to an owned snapshot at the boundary"
    );
    let imd = cow.message_descriptor();
    assert!(matches!(cow.get(imd.field(1).unwrap()), ValueRef::I32(7)));
    assert!(matches!(
        cow.get(imd.field(2).unwrap()),
        ValueRef::String("seven")
    ));
}

#[test]
fn absent_bridge_field_reflects_as_default_instance() {
    let msg = Outer::default();
    let r = msg.reflect();
    let md = r.message_descriptor();
    let fd = md.field(1).unwrap();
    assert!(!r.has(fd));
    // Absent singular message fields read as the (degraded) default instance.
    let ValueRef::Message(cow) = r.get(fd) else {
        panic!("expected Message for the absent field")
    };
    let imd = cow.message_descriptor();
    assert!(matches!(cow.get(imd.field(1).unwrap()), ValueRef::I32(0)));
}

#[test]
fn repeated_bridge_elements_reflect_through_list() {
    let msg = Outer {
        many: vec![inner(1, "a"), inner(2, "b")],
        ..Default::default()
    };
    let r = msg.reflect();
    let md = r.message_descriptor();
    let ValueRef::List(list) = r.get(md.field(2).unwrap()) else {
        panic!("expected List")
    };
    assert_eq!(list.len(), 2);
    let ValueRef::Message(cow) = list.get(1).unwrap() else {
        panic!("expected Message element")
    };
    assert!(matches!(cow, ReflectCow::Owned(_)));
    let imd = cow.message_descriptor();
    assert!(matches!(cow.get(imd.field(1).unwrap()), ValueRef::I32(2)));
}

#[test]
fn map_bridge_values_reflect_through_map() {
    let msg = Outer {
        by_name: [("k".to_string(), inner(5, "five"))].into_iter().collect(),
        ..Default::default()
    };
    let r = msg.reflect();
    let md = r.message_descriptor();
    let ValueRef::Map(map) = r.get(md.field(3).unwrap()) else {
        panic!("expected Map")
    };
    assert_eq!(map.len(), 1);
    let ValueRef::Message(cow) = map.get_str("k").unwrap() else {
        panic!("expected Message value")
    };
    let imd = cow.message_descriptor();
    assert!(matches!(cow.get(imd.field(1).unwrap()), ValueRef::I32(5)));
}

#[test]
fn oneof_bridge_variant_degrades_and_default_arm_works() {
    let msg = Outer {
        kind: Some(Kind::Alt(Box::new(inner(9, "nine")))),
        ..Default::default()
    };
    let r = msg.reflect();
    let md = r.message_descriptor();

    let alt_fd = md.field(4).unwrap();
    assert!(r.has(alt_fd));
    let ValueRef::Message(cow) = r.get(alt_fd) else {
        panic!("expected Message for the active oneof variant")
    };
    assert!(matches!(cow, ReflectCow::Owned(_)));
    let imd = cow.message_descriptor();
    assert!(matches!(cow.get(imd.field(1).unwrap()), ValueRef::I32(9)));

    // With the oneof set to a different member, the message variant's get()
    // takes the default arm — also degraded through reflect().
    let other = Outer {
        kind: Some(Kind::Num(3)),
        ..Default::default()
    };
    let r2 = other.reflect();
    assert!(!r2.has(alt_fd));
    let ValueRef::Message(cow2) = r2.get(alt_fd) else {
        panic!("expected Message default for the inactive variant")
    };
    let imd2 = cow2.message_descriptor();
    assert!(matches!(cow2.get(imd2.field(1).unwrap()), ValueRef::I32(0)));
}

#[test]
fn for_each_set_crosses_the_mode_boundary() {
    let msg = Outer {
        single: buffa::MessageField::some(inner(7, "seven")),
        many: vec![inner(1, "a")],
        ..Default::default()
    };
    let r = msg.reflect();
    let mut seen = Vec::new();
    r.for_each_set(&mut |fd, _| seen.push(fd.number()));
    seen.sort_unstable();
    assert_eq!(seen, vec![1, 2]);
}

#[test]
fn which_oneof_resolves_bridge_member() {
    let msg = Outer {
        kind: Some(Kind::Alt(Box::new(inner(9, "nine")))),
        ..Default::default()
    };
    let r = msg.reflect();
    let md = r.message_descriptor();
    let oneof = &md.oneofs()[0];
    let active = r.which_oneof(oneof).expect("oneof is set");
    assert_eq!(active.number(), 4);
}

#[test]
fn to_dynamic_round_trips_the_mixed_tree() {
    // to_dynamic takes the wire round-trip path through the parent's pool
    // (whose embedded FileDescriptorSet includes the extern-path import),
    // not the per-field degradation — exercise it separately.
    let msg = Outer {
        single: buffa::MessageField::some(inner(7, "seven")),
        many: vec![inner(1, "a")],
        by_name: [("k".to_string(), inner(5, "five"))].into_iter().collect(),
        kind: Some(Kind::Alt(Box::new(inner(9, "nine")))),
        ..Default::default()
    };
    let dynamic = msg.reflect().to_dynamic();
    let md = dynamic.message_descriptor();
    let ValueRef::Message(single) = dynamic.get(md.field(1).unwrap()) else {
        panic!("expected Message for single")
    };
    let imd = single.message_descriptor();
    assert!(matches!(
        single.get(imd.field(1).unwrap()),
        ValueRef::I32(7)
    ));
    let ValueRef::List(many) = dynamic.get(md.field(2).unwrap()) else {
        panic!("expected List for many")
    };
    assert_eq!(many.len(), 1);
    let ValueRef::Map(by_name) = dynamic.get(md.field(3).unwrap()) else {
        panic!("expected Map for by_name")
    };
    assert_eq!(by_name.len(), 1);
    assert!(dynamic.has(md.field(4).unwrap()));
}

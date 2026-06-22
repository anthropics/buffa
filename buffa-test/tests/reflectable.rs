//! `Reflectable` trait integration test against codegen output.
//!
//! `basic.proto` is generated with `generate_reflection(true)`, so `Person`
//! and `Address` already implement `Reflectable` via the bridge mode. This
//! test verifies the codegen-emitted impls — the per-message
//! `impl Reflectable` and the per-package `__buffa::reflect::descriptor_pool()`
//! — produce a working `&dyn ReflectMessage` surface.
//!
//! It also locks in the call-site contract: `foo.reflect().get(fd)` works
//! through `ReflectCow`'s `Deref`, which is what makes bridge → vtable mode
//! switching a zero-diff change for consumers.

use buffa::MessageField;
use buffa_descriptor::reflect::{ReflectMessage, Reflectable, ValueRef};
use buffa_test::basic::{Address, Person};

#[test]
fn reflectable_call_site_works_through_deref() {
    let person = Person {
        id: 7,
        name: "Grace".into(),
        address: MessageField::some(Address {
            street: "100 Lab Ave".into(),
            zip_code: 90210,
            ..Default::default()
        }),
        ..Default::default()
    };

    // The contract: this call site is identical for bridge and vtable
    // mode. `reflect()` returns a `ReflectCow`, `Deref` gives `&dyn
    // ReflectMessage`, and from there it's descriptor-keyed accessors.
    let r = person.reflect();
    let md = r.message_descriptor();
    assert_eq!(md.full_name(), "basic.Person");

    let id = r.get(md.field(1).unwrap());
    assert!(matches!(id, ValueRef::I32(7)));

    let name = r.get(md.field(2).unwrap());
    assert!(matches!(name, ValueRef::String("Grace")));

    // Nested message — accessed through the same trait surface.
    let addr_ref = r.get(md.field(7).unwrap());
    let ValueRef::Message(addr_cow) = addr_ref else {
        panic!("expected Message")
    };
    let addr_md = addr_cow.message_descriptor();
    assert_eq!(addr_md.full_name(), "basic.Address");
    assert!(matches!(
        addr_cow.get(addr_md.field(3).unwrap()),
        ValueRef::U32(90210)
    ));
}

#[test]
fn generic_function_over_dyn_reflect_message() {
    /// A generic interceptor — the connect-rust use case. It takes any
    /// reflectable message and reads a field by name without knowing the
    /// concrete type.
    fn read_string_field(msg: &dyn ReflectMessage, field_name: &str) -> Option<String> {
        let md = msg.message_descriptor();
        let fd = md.field_by_name(field_name)?;
        if !msg.has(fd) {
            return None;
        }
        match msg.get(fd) {
            ValueRef::String(s) => Some(s.to_string()),
            _ => None,
        }
    }

    let person = Person {
        name: "Alan".into(),
        ..Default::default()
    };
    let address = Address {
        street: "1 Compute St".into(),
        ..Default::default()
    };

    // Call the generic function with two different types — the dyn
    // dispatch is the contract that makes interceptors work.
    let r1 = person.reflect();
    let r2 = address.reflect();
    assert_eq!(read_string_field(&*r1, "name"), Some("Alan".into()));
    assert_eq!(
        read_string_field(&*r2, "street"),
        Some("1 Compute St".into())
    );
    assert_eq!(read_string_field(&*r1, "street"), None); // no such field
    assert_eq!(read_string_field(&*r2, "city"), None); // present descriptor, unset
}

#[test]
fn for_each_set_visits_set_fields() {
    let person = Person {
        id: 1,
        verified: true,
        score: 9.5,
        ..Default::default()
    };
    let r = person.reflect();
    let mut seen = Vec::new();
    r.for_each_set(&mut |fd, _| seen.push(fd.name().to_string()));
    seen.sort();
    assert_eq!(seen, vec!["id", "score", "verified"]);
}

#[test]
fn owned_vtable_matches_bridge_for_every_field() {
    use buffa_descriptor::reflect::DynamicMessage;
    use buffa_test::basic::{person, Inventory, Status};
    use std::sync::Arc;

    // A Person exercising every owned-vtable arm: scalars, string, bytes,
    // bool, double, enum, nested message, repeated string + repeated scalar,
    // explicit-presence optional, and a oneof.
    let person = Person {
        id: 42,
        name: "Ada".into(),
        avatar: vec![0xDE, 0xAD],
        verified: true,
        score: 9.5,
        status: Status::ACTIVE.into(),
        address: MessageField::some(Address {
            street: "1 Main".into(),
            zip_code: 12345,
            ..Default::default()
        }),
        tags: vec!["x".into(), "y".into()],
        lucky_numbers: vec![7, 11, 13],
        maybe_age: Some(30),
        contact: Some(person::Contact::Email("ada@example.com".into())),
        ..Default::default()
    };

    // Compare the vtable reflection (reflect() → Borrowed(self)) against the
    // bridge round-trip, field by field. `ValueRef` has no `PartialEq` (trait
    // objects), so compare the owned `Value` snapshots.
    let vt = person.reflect();
    let md = vt.message_descriptor();
    let pool = Arc::clone(vt.pool());
    let idx = pool.message_index("basic.Person").unwrap();
    let bridge = DynamicMessage::from_message(&person, Arc::clone(&pool), idx);

    for fd in md.fields() {
        assert_eq!(
            vt.has(fd),
            bridge.has(fd),
            "has() mismatch on field {}",
            fd.number()
        );
        if vt.has(fd) {
            assert_eq!(
                vt.get(fd).to_owned(),
                bridge.get(fd).to_owned(),
                "get() mismatch on field {}",
                fd.number()
            );
        }
    }

    // Map fields (Inventory.stock: map<string, int32>) — separate message.
    let mut stock = buffa::Map::default();
    stock.insert("apples".to_string(), 3);
    stock.insert("oranges".to_string(), 7);
    let inv = Inventory {
        stock,
        ..Default::default()
    };
    let inv_vt = inv.reflect();
    let inv_md = inv_vt.message_descriptor();
    let inv_idx = pool.message_index("basic.Inventory").unwrap();
    let inv_bridge = DynamicMessage::from_message(&inv, Arc::clone(&pool), inv_idx);
    let stock_fd = inv_md
        .fields()
        .iter()
        .find(|f| f.name() == "stock")
        .unwrap();
    assert_eq!(
        inv_vt.get(stock_fd).to_owned(),
        inv_bridge.get(stock_fd).to_owned(),
        "map field mismatch"
    );
}

#[test]
fn reflect_borrows_in_vtable_mode() {
    use buffa_descriptor::reflect::ReflectCow;
    // basic.proto is generated with vtable mode, so `reflect()` borrows `self`
    // directly (no `DynamicMessage` round-trip / heap allocation).
    let person = Person {
        id: 1,
        ..Default::default()
    };
    assert!(matches!(person.reflect(), ReflectCow::Borrowed(_)));
}

#[test]
fn descriptor_pool_is_built_once() {
    // The pool is `OnceLock`-backed; multiple `reflect()` calls share it.
    let p1 = buffa_test::basic::descriptor_pool();
    let p2 = buffa_test::basic::descriptor_pool();
    assert!(std::sync::Arc::ptr_eq(p1, p2));
    // The pool resolves both `basic.*` types and the WKT imports.
    assert!(p1.message_index("basic.Person").is_some());
    assert!(p1.message_index("basic.Address").is_some());
}

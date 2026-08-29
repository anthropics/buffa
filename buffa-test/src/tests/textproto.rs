//! Textproto (text format) integration tests via generated `impl TextFormat`.
//!
//! Exercises the codegen output in `buffa-codegen/src/impl_text.rs` against
//! the runtime in `buffa/src/text/*`. The runtime itself has 73 unit tests
//! against a hand-rolled impl; these tests verify generated code produces
//! matching output and accepts the same inputs.

use crate::basic::*;
use buffa::text::{
    decode_from_str, decode_from_str_with_element_memory_limit, encode_to_string,
    encode_to_string_pretty, ParseErrorKind,
};
use buffa::DEFAULT_ELEMENT_MEMORY_LIMIT;
use buffa::{EnumValue, MessageField};

// ── scalars ─────────────────────────────────────────────────────────────────

#[test]
fn all_scalars_golden() {
    // Every numeric scalar type. Implicit presence: zero values suppressed.
    let msg = AllScalars {
        f_int32: -7,
        f_int64: 9_000_000_000,
        f_uint32: 42,
        f_uint64: 18_000_000_000_000_000_000,
        f_sint32: -100,
        f_sint64: -200,
        f_fixed32: 0xDEAD_BEEF,
        f_fixed64: 0xCAFE_BABE,
        f_sfixed32: -42,
        f_sfixed64: -84,
        f_float: 1.5,
        f_double: 2.5,
        f_bool: true,
        ..Default::default()
    };
    let text = encode_to_string(&msg);
    assert_eq!(
        text,
        "f_int32: -7 f_int64: 9000000000 f_uint32: 42 f_uint64: 18000000000000000000 \
         f_sint32: -100 f_sint64: -200 f_fixed32: 3735928559 f_fixed64: 3405691582 \
         f_sfixed32: -42 f_sfixed64: -84 f_float: 1.5 f_double: 2.5 f_bool: true"
    );
    let back: AllScalars = decode_from_str(&text).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn default_encodes_to_empty() {
    // Implicit presence: all-zero → nothing emitted.
    assert_eq!(encode_to_string(&AllScalars::default()), "");
    assert_eq!(encode_to_string(&Empty::default()), "");
}

// ── presence ────────────────────────────────────────────────────────────────

#[test]
fn presence_forms() {
    // | shape              | set? | expect in output |
    // | implicit scalar    | no   | absent           |
    // | implicit scalar    | yes  | present          |
    // | Option<T>          | None | absent           |
    // | Option<T>          | Some | present (even 0) |
    // | MessageField<T>    | unset| absent           |
    // | MessageField<T>    | set  | present (even empty) |
    let mut p = Person::default();
    p.maybe_age = Some(0); // explicit presence: Some(0) IS emitted
    p.maybe_nickname = Some(String::new()); // Some("") likewise
    p.address = MessageField::some(Address::default()); // set-but-empty
    let text = encode_to_string(&p);
    assert_eq!(text, r#"address {} maybe_age: 0 maybe_nickname: """#);

    let back: Person = decode_from_str(&text).unwrap();
    assert_eq!(back.maybe_age, Some(0));
    assert_eq!(back.maybe_nickname.as_deref(), Some(""));
    assert!(back.address.is_set());
}

// ── full Person roundtrip ───────────────────────────────────────────────────

#[test]
fn person_roundtrip() {
    let mut p = Person::default();
    p.id = 42;
    p.name = "Alice".into();
    p.avatar = vec![0xDE, 0xAD];
    p.verified = true;
    p.score = 9.5;
    p.status = EnumValue::Known(Status::ACTIVE);
    p.address = MessageField::some(Address {
        street: "1 High St".into(),
        city: "London".into(),
        zip_code: 12345,
        ..Default::default()
    });
    p.tags = vec!["a".into(), "b".into()];
    p.lucky_numbers = vec![7, 13, 42];
    p.addresses = vec![Address {
        city: "Paris".into(),
        ..Default::default()
    }];
    p.maybe_age = Some(30);
    p.contact = Some(crate::basic::__buffa::oneof::person::Contact::Email(
        "alice@example.com".into(),
    ));

    let text = encode_to_string(&p);
    let back: Person = decode_from_str(&text).unwrap();
    assert_eq!(back, p);
}

#[test]
fn person_pretty_output() {
    let mut p = Person::default();
    p.id = 1;
    p.address = MessageField::some(Address {
        city: "London".into(),
        ..Default::default()
    });
    let text = encode_to_string_pretty(&p);
    assert_eq!(text, "id: 1\naddress {\n  city: \"London\"\n}\n");
}

// ── enum ────────────────────────────────────────────────────────────────────

#[test]
fn open_enum_known_and_unknown() {
    let mut p = Person::default();
    p.status = EnumValue::Known(Status::INACTIVE);
    assert_eq!(encode_to_string(&p), "status: INACTIVE");

    p.status = EnumValue::Unknown(99);
    assert_eq!(encode_to_string(&p), "status: 99");

    // Decode: name → Known, number → from (known or Unknown).
    let p: Person = decode_from_str("status: ACTIVE").unwrap();
    assert_eq!(p.status, EnumValue::Known(Status::ACTIVE));

    let p: Person = decode_from_str("status: 99").unwrap();
    assert_eq!(p.status, EnumValue::Unknown(99));
}

#[test]
fn closed_enum_encode_decode() {
    // proto2 closed enum: stored as bare E, not EnumValue<E>.
    use crate::proto2::{Priority, RequiredDefaults};
    let mut r = RequiredDefaults::default();
    r.level = Priority::HIGH;
    let text = encode_to_string(&r);
    // RequiredDefaults has many required fields; just check level is in there.
    assert!(text.contains("level: HIGH"), "got: {text}");

    // Decode back — only set `level` via merge on top of defaults.
    let mut r2 = RequiredDefaults::default();
    buffa::text::merge_from_str(&mut r2, "level: HIGH").unwrap();
    assert_eq!(r2.level, Priority::HIGH);
}

// ── oneof ───────────────────────────────────────────────────────────────────

#[test]
fn oneof_variants() {
    let mut p = Person::default();
    p.contact = Some(crate::basic::__buffa::oneof::person::Contact::Phone(
        "555-1234".into(),
    ));
    assert_eq!(encode_to_string(&p), r#"phone: "555-1234""#);

    let p: Person = decode_from_str(r#"email: "x@y.com""#).unwrap();
    assert_eq!(
        p.contact,
        Some(crate::basic::__buffa::oneof::person::Contact::Email(
            "x@y.com".into()
        ))
    );

    // Last-wins when both variants appear (textproto merge semantics).
    let p: Person = decode_from_str(r#"email: "a" phone: "b""#).unwrap();
    assert_eq!(
        p.contact,
        Some(crate::basic::__buffa::oneof::person::Contact::Phone(
            "b".into()
        ))
    );
}

// ── repeated ────────────────────────────────────────────────────────────────

#[test]
fn repeated_list_and_one_per_line() {
    // Encode always uses one-per-line form; decode accepts both.
    let mut p = Person::default();
    p.lucky_numbers = vec![1, 2, 3];
    assert_eq!(
        encode_to_string(&p),
        "lucky_numbers: 1 lucky_numbers: 2 lucky_numbers: 3"
    );

    let p: Person = decode_from_str("lucky_numbers: [1, 2, 3]").unwrap();
    assert_eq!(p.lucky_numbers, vec![1, 2, 3]);

    let p: Person =
        decode_from_str("lucky_numbers: 1 lucky_numbers: [2, 3] lucky_numbers: 4").unwrap();
    assert_eq!(p.lucky_numbers, vec![1, 2, 3, 4]);
}

#[test]
fn repeated_message() {
    let p: Person = decode_from_str(r#"addresses { city: "A" } addresses { city: "B" }"#).unwrap();
    assert_eq!(p.addresses.len(), 2);
    assert_eq!(p.addresses[0].city, "A");
    assert_eq!(p.addresses[1].city, "B");
}

// ── map ─────────────────────────────────────────────────────────────────────

#[test]
fn map_roundtrip() {
    let mut inv = Inventory::default();
    inv.stock.insert("apples".into(), 10);
    inv.stock.insert("oranges".into(), 5);
    let text = encode_to_string(&inv);
    let back: Inventory = decode_from_str(&text).unwrap();
    assert_eq!(back.stock.len(), 2);
    assert_eq!(back.stock["apples"], 10);
    assert_eq!(back.stock["oranges"], 5);
}

#[test]
fn map_message_value() {
    let mut inv = Inventory::default();
    inv.locations.insert(
        "hq".into(),
        Address {
            city: "SF".into(),
            ..Default::default()
        },
    );
    let text = encode_to_string(&inv);
    let back: Inventory = decode_from_str(&text).unwrap();
    assert_eq!(back.locations["hq"].city, "SF");
}

#[test]
fn map_enum_value() {
    let mut inv = Inventory::default();
    inv.statuses
        .insert("k".into(), EnumValue::Known(Status::ACTIVE));
    let text = encode_to_string(&inv);
    assert!(text.contains("value: ACTIVE"), "got: {text}");
    let back: Inventory = decode_from_str(&text).unwrap();
    assert_eq!(back.statuses["k"], EnumValue::Known(Status::ACTIVE));
}

#[test]
fn map_decode_list_form() {
    let inv: Inventory =
        decode_from_str(r#"stock: [{key: "a" value: 1}, {key: "b" value: 2}]"#).unwrap();
    assert_eq!(inv.stock["a"], 1);
    assert_eq!(inv.stock["b"], 2);
}

#[test]
fn map_decode_missing_key_or_value_defaults() {
    // Absent key → "" (String default); absent value → 0.
    let inv: Inventory = decode_from_str(r#"stock { value: 7 }"#).unwrap();
    assert_eq!(inv.stock[""], 7);
    let inv: Inventory = decode_from_str(r#"stock { key: "x" }"#).unwrap();
    assert_eq!(inv.stock["x"], 0);
}

// ── unknown fields ──────────────────────────────────────────────────────────

#[test]
fn unknown_fields_skipped() {
    // Generated merge_text skips unknowns via skip_value.
    let p: Person =
        decode_from_str(r#"not_a_field: 42 id: 1 also_unknown { x: "y" } name: "ok""#).unwrap();
    assert_eq!(p.id, 1);
    assert_eq!(p.name, "ok");
}

// ── merge semantics ─────────────────────────────────────────────────────────

#[test]
fn merge_scalars_overwrite_repeated_append() {
    let mut p = Person {
        id: 1,
        tags: vec!["a".into()],
        ..Default::default()
    };
    buffa::text::merge_from_str(&mut p, r#"id: 2 tags: "b""#).unwrap();
    assert_eq!(p.id, 2);
    assert_eq!(p.tags, vec!["a", "b"]);
}

#[test]
fn merge_message_recurses() {
    let mut p = Person::default();
    p.address = MessageField::some(Address {
        street: "old".into(),
        zip_code: 111,
        ..Default::default()
    });
    buffa::text::merge_from_str(&mut p, r#"address { city: "new" }"#).unwrap();
    // street + zip survive, city merged in.
    assert_eq!(p.address.street, "old");
    assert_eq!(p.address.city, "new");
    assert_eq!(p.address.zip_code, 111);
}

// ── proto2 group naming ─────────────────────────────────────────────────────
//
// `optional group Data = N { ... }` creates a field `data` but text format
// uses the TYPE name. Encode emits `MyGroup { ... }`; decode accepts both
// the type name and the lowercase field name (matching protobuf-go).

#[test]
fn group_encode_uses_type_name() {
    use crate::proto2::with_groups::{Item, MyGroup};
    use crate::proto2::WithGroups;

    let msg = WithGroups {
        mygroup: MessageField::some(MyGroup {
            a: Some(7),
            ..Default::default()
        }),
        item: vec![Item {
            id: Some(1),
            ..Default::default()
        }],
        ..Default::default()
    };
    let text = encode_to_string(&msg);
    // Singular: `MyGroup {a: 7}`, repeated: `Item {id: 1}`. Type name,
    // not the lowercase field name.
    assert_eq!(text, "MyGroup {a: 7} Item {id: 1}");
}

#[test]
fn group_decode_accepts_both_names() {
    use crate::proto2::WithGroups;

    // Type name (what other encoders emit).
    let g: WithGroups = decode_from_str("MyGroup { a: 1 } Item { id: 2 }").unwrap();
    assert_eq!(g.mygroup.a, Some(1));
    assert_eq!(g.item[0].id, Some(2));

    // Lowercase field name (legacy compat).
    let g: WithGroups = decode_from_str("mygroup { a: 3 } item { id: 4 }").unwrap();
    assert_eq!(g.mygroup.a, Some(3));
    assert_eq!(g.item[0].id, Some(4));
}

#[test]
fn group_in_oneof_uses_type_name() {
    use crate::proto2::__buffa::oneof::view_coverage::Choice as ChoiceOneof;
    use crate::proto2::view_coverage::Payload;
    use crate::proto2::ViewCoverage;

    let mut v = ViewCoverage::default();
    v.choice = Some(ChoiceOneof::Payload(Box::new(Payload {
        x: Some(5),
        ..Default::default()
    })));
    let text = encode_to_string(&v);
    // `level` is required so it's always emitted; Payload follows.
    assert!(text.contains("Payload {x: 5}"), "got: {text}");

    // Decode accepts both forms.
    let mut v = ViewCoverage::default();
    buffa::text::merge_from_str(&mut v, "Payload { x: 10 }").unwrap();
    assert!(matches!(v.choice, Some(ChoiceOneof::Payload(ref p)) if p.x == Some(10)));

    let mut v = ViewCoverage::default();
    buffa::text::merge_from_str(&mut v, "payload { x: 11 }").unwrap();
    assert!(matches!(v.choice, Some(ChoiceOneof::Payload(ref p)) if p.x == Some(11)));
}

// ── Element-memory budget ──────────────────────────────────────────────────
//
// The binary decoder bounds what a decode materializes rather than what it
// reads; the text parser has the same amplification and a cheaper element
// (`{},` is three input bytes against `size_of::<Element>()` in the `Vec`),
// but `DecodeContext` never reaches it, so it carries its own budget.

/// `n` empty repeated-message elements: `addresses: [{},{},...]`.
fn empty_address_list(n: usize) -> String {
    let mut s = String::with_capacity(n * 3 + 16);
    s.push_str("addresses: [");
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{}");
    }
    s.push(']');
    s
}

#[test]
fn the_default_budget_rejects_an_amplifying_text_payload() {
    let n = 4 * DEFAULT_ELEMENT_MEMORY_LIMIT / core::mem::size_of::<Address>();
    let text = empty_address_list(n);

    let err = decode_from_str::<Person>(&text).expect_err("must not materialize");
    assert_eq!(err.kind, ParseErrorKind::ElementMemoryLimitExceeded);
}

#[test]
fn the_default_budget_admits_an_ordinary_text_message() {
    let text = empty_address_list(32);
    let p: Person = decode_from_str(&text).expect("an ordinary list parses");
    assert_eq!(p.addresses.len(), 32);
}

#[test]
fn the_text_element_budget_is_configurable_in_both_directions() {
    let text = empty_address_list(1000);

    let err = decode_from_str_with_element_memory_limit::<Person>(&text, 64)
        .expect_err("a tightened budget is enforced");
    assert_eq!(err.kind, ParseErrorKind::ElementMemoryLimitExceeded);

    let p = decode_from_str_with_element_memory_limit::<Person>(&text, usize::MAX)
        .expect("a lifted budget accepts the same input");
    assert_eq!(p.addresses.len(), 1000);
}

#[test]
fn text_map_entries_are_charged_against_the_same_budget() {
    // Map entries buffer into an intermediate `Vec<(K, V)>` before the
    // duplicate-key collapse, so all N are live at once even when they share
    // a key — which is exactly the shape that needs bounding.
    let entries = |n: usize| {
        let mut t = String::from("locations: [");
        for i in 0..n {
            if i > 0 {
                t.push(',');
            }
            t.push_str("{}");
        }
        t.push(']');
        t
    };

    let err = decode_from_str_with_element_memory_limit::<Inventory>(&entries(200_000), 4096)
        .expect_err("map entries must be charged");
    assert_eq!(err.kind, ParseErrorKind::ElementMemoryLimitExceeded);

    // And the budget is shared with the rest of the parse, not per field:
    // a count that fits on its own is refused once a repeated field in the
    // same message has already spent part of the budget.
    let entry_cost = core::mem::size_of::<(String, Address)>();
    let n = 64;
    let alone = entries(n);
    let budget = n * entry_cost + 8;
    assert!(
        decode_from_str_with_element_memory_limit::<Inventory>(&alone, budget).is_ok(),
        "{n} entries fit in a budget sized for exactly that many"
    );

    let mut shared = String::from("stock: [{},{},{}] ");
    shared.push_str(&alone);
    let err = decode_from_str_with_element_memory_limit::<Inventory>(&shared, budget)
        .expect_err("the earlier field's entries must consume the same pool");
    assert_eq!(err.kind, ParseErrorKind::ElementMemoryLimitExceeded);
}

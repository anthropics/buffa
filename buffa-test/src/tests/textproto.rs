//! Textproto (text format) integration tests via generated `impl TextFormat`.
//!
//! Exercises the codegen output in `buffa-codegen/src/impl_text.rs` against
//! the runtime in `buffa/src/text/*`. The runtime itself has 73 unit tests
//! against a hand-rolled impl; these tests verify generated code produces
//! matching output and accepts the same inputs.

use crate::basic::*;
use buffa::text::{decode_from_str, encode_to_string, encode_to_string_pretty};
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
    p.contact = Some(person::ContactOneof::Email("alice@example.com".into()));

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
    p.contact = Some(person::ContactOneof::Phone("555-1234".into()));
    assert_eq!(encode_to_string(&p), r#"phone: "555-1234""#);

    let p: Person = decode_from_str(r#"email: "x@y.com""#).unwrap();
    assert_eq!(
        p.contact,
        Some(person::ContactOneof::Email("x@y.com".into()))
    );

    // Last-wins when both variants appear (textproto merge semantics).
    let p: Person = decode_from_str(r#"email: "a" phone: "b""#).unwrap();
    assert_eq!(p.contact, Some(person::ContactOneof::Phone("b".into())));
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
    use crate::proto2::view_coverage::{ChoiceOneof, Payload};
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

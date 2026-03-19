//! Extension JSON registry integration: message/enum/repeated extension types
//! round-trip through `serde_json` via the generated `register_json` +
//! the runtime's `#[serde(flatten)]` wrapper.

use crate::extjson::{
    register_json, Ann, Carrier, Color, ANN, ANNS, BIGS, COLOR, COLORS, NUMS, WEIGHT,
};
use buffa::json_registry::{set_json_registry, JsonRegistry};
use buffa::{Enumeration, ExtensionSet};

/// Install the unified JSON registry once for the test process. Tests run in
/// parallel threads; `set_json_registry` leaks the old halves (see its doc)
/// so racing installs are safe, but `Once` makes intent explicit.
fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut reg = JsonRegistry::new();
        register_json(&mut reg);
        set_json_registry(reg);
    });
}

/// Serialize `carrier` to JSON, then deserialize back. Asserts the intermediate
/// JSON matches `expected`, and returns the round-tripped carrier.
fn json_roundtrip(carrier: &Carrier, expected: serde_json::Value) -> Carrier {
    setup();
    let json = serde_json::to_value(carrier).expect("serialize");
    assert_eq!(json, expected);
    serde_json::from_value(json).expect("deserialize")
}

#[test]
fn singular_scalar_still_works() {
    // Sanity check: the pre-existing singular-scalar path is unchanged.
    let mut c = Carrier::default();
    c.set_extension(&WEIGHT, -7);
    let back = json_roundtrip(&c, serde_json::json!({"[buffa.test.extjson.weight]": -7}));
    assert_eq!(back.extension(&WEIGHT), Some(-7));
}

#[test]
fn message_extension_json_roundtrip() {
    let mut c = Carrier::default();
    c.set_extension(
        &ANN,
        Ann {
            doc: Some("hello".into()),
            priority: Some(5),
            ..Default::default()
        },
    );
    let back = json_roundtrip(
        &c,
        serde_json::json!({
            "[buffa.test.extjson.ann]": {"doc": "hello", "priority": 5}
        }),
    );
    let got = back.extension(&ANN).expect("ANN present");
    assert_eq!(got.doc.as_deref(), Some("hello"));
    assert_eq!(got.priority, Some(5));
}

#[test]
fn enum_extension_json_emits_variant_name() {
    let mut c = Carrier::default();
    c.set_extension(&COLOR, Color::GREEN.to_i32());
    let back = json_roundtrip(
        &c,
        serde_json::json!({"[buffa.test.extjson.color]": "GREEN"}),
    );
    assert_eq!(back.extension(&COLOR), Some(1));
}

#[test]
fn enum_extension_json_accepts_numeric() {
    setup();
    let c: Carrier =
        serde_json::from_str(r#"{"[buffa.test.extjson.color]": 2}"#).expect("deserialize");
    assert_eq!(c.extension(&COLOR), Some(2));
}

#[test]
fn repeated_scalar_extension_json_roundtrip() {
    let mut c = Carrier::default();
    c.set_extension(&NUMS, vec![1, -2, 3]);
    let back = json_roundtrip(
        &c,
        serde_json::json!({"[buffa.test.extjson.nums]": [1, -2, 3]}),
    );
    assert_eq!(back.extension(&NUMS), vec![1, -2, 3]);
}

#[test]
fn repeated_int64_extension_json_stringifies_elements() {
    let mut c = Carrier::default();
    c.set_extension(&BIGS, vec![7_i64, 9_999_999_999]);
    let back = json_roundtrip(
        &c,
        serde_json::json!({"[buffa.test.extjson.bigs]": ["7", "9999999999"]}),
    );
    assert_eq!(back.extension(&BIGS), vec![7, 9_999_999_999]);
}

#[test]
fn repeated_message_extension_json_roundtrip() {
    let mut c = Carrier::default();
    c.set_extension(
        &ANNS,
        vec![
            Ann {
                doc: Some("a".into()),
                ..Default::default()
            },
            Ann {
                doc: Some("b".into()),
                ..Default::default()
            },
        ],
    );
    let back = json_roundtrip(
        &c,
        serde_json::json!({
            "[buffa.test.extjson.anns]": [{"doc": "a"}, {"doc": "b"}]
        }),
    );
    let got = back.extension(&ANNS);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].doc.as_deref(), Some("a"));
    assert_eq!(got[1].doc.as_deref(), Some("b"));
}

#[test]
fn repeated_enum_extension_json_roundtrip() {
    let mut c = Carrier::default();
    c.set_extension(&COLORS, vec![Color::RED.to_i32(), Color::BLUE.to_i32()]);
    let back = json_roundtrip(
        &c,
        serde_json::json!({"[buffa.test.extjson.colors]": ["RED", "BLUE"]}),
    );
    assert_eq!(back.extension(&COLORS), vec![0, 2]);
}

#[test]
fn multiple_extension_types_coexist_in_json() {
    let mut c = Carrier::default();
    c.x = Some(99);
    c.set_extension(&WEIGHT, 42);
    c.set_extension(&COLOR, Color::BLUE.to_i32());
    c.set_extension(&NUMS, vec![10, 20]);

    setup();
    let json = serde_json::to_value(&c).expect("serialize");
    // Key set, not order (serde_json object iteration order is deterministic
    // but tied to insertion; the flatten wrapper emits in unknown-field order).
    assert_eq!(json["x"], serde_json::json!(99));
    assert_eq!(json["[buffa.test.extjson.weight]"], serde_json::json!(42));
    assert_eq!(
        json["[buffa.test.extjson.color]"],
        serde_json::json!("BLUE")
    );
    assert_eq!(
        json["[buffa.test.extjson.nums]"],
        serde_json::json!([10, 20])
    );

    let back: Carrier = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back.x, Some(99));
    assert_eq!(back.extension(&WEIGHT), Some(42));
    assert_eq!(back.extension(&COLOR), Some(2));
    assert_eq!(back.extension(&NUMS), vec![10, 20]);
}

// ── Codegen-emitted Any entries via register_json ─────────────────────────
//
// `register_json` populates BOTH the extension registry and the Any registry;
// these tests verify the Any half. `setup()` above installed the unified
// registry once, so `with_any_registry` sees every message in ext_json.proto.

#[test]
fn any_entry_const_has_correct_shape() {
    // The `#[doc(hidden)]` const is stable enough to name directly in tests
    // — if codegen renames it, this test catches the drift.
    use crate::extjson::__CARRIER_ANY_ENTRY;
    assert_eq!(__CARRIER_ANY_ENTRY.type_url, Carrier::TYPE_URL);
    assert!(!__CARRIER_ANY_ENTRY.is_wkt, "user messages are not WKTs");
}

#[test]
fn register_json_populates_any_registry() {
    setup();
    buffa::any_registry::with_any_registry(|r| {
        let r = r.expect("registry installed by setup");
        // Every message in ext_json.proto: Carrier and Ann.
        assert!(r.lookup(Carrier::TYPE_URL).is_some());
        assert!(r.lookup(Ann::TYPE_URL).is_some());
        // Not WKTs — codegen always emits is_wkt: false.
        assert!(!r.lookup(Carrier::TYPE_URL).unwrap().is_wkt);
    });
}

#[test]
fn any_entry_roundtrips_message_through_json() {
    // Exercise the generated fn pointers directly: encode a message to wire
    // bytes, run it through the entry's to_json, verify the JSON shape, then
    // back through from_json and verify the bytes decode to the same message.
    // This is what Any's serde impl does under the hood.
    use crate::extjson::__ANN_ANY_ENTRY;
    use buffa::Message;

    let ann = Ann {
        doc: Some("via any entry".into()),
        priority: Some(3),
        ..Default::default()
    };
    let wire = ann.encode_to_vec();

    let json = (__ANN_ANY_ENTRY.to_json)(&wire).expect("to_json");
    assert_eq!(json["doc"], serde_json::json!("via any entry"));
    assert_eq!(json["priority"], serde_json::json!(3));

    let bytes_back = (__ANN_ANY_ENTRY.from_json)(json).expect("from_json");
    let ann_back = Ann::decode_from_slice(&bytes_back).expect("decode");
    assert_eq!(ann_back.doc.as_deref(), Some("via any entry"));
    assert_eq!(ann_back.priority, Some(3));
}

#[test]
fn register_extensions_still_works_for_back_compat() {
    // `register_extensions` is preserved (extension-only; a subset of
    // `register_json`). Pre-0.3 callers compile unchanged.
    use crate::extjson::register_extensions;
    let mut ext = buffa::extension_registry::ExtensionRegistry::new();
    register_extensions(&mut ext);
    assert!(ext.by_name("buffa.test.extjson.weight").is_some());
}

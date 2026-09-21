//! `deny_unknown_json_fields_in` (#444): generated JSON deserializers reject
//! unknown keys for the messages a rule names, and keep ignoring them for the
//! messages it does not.
//!
//! The option is path-scoped in `build.rs`, so `Strict*` and `Lenient*` come
//! from one codegen run over one proto file — the difference in behaviour is
//! the rule, not the configuration around it. Both codegen paths are covered:
//! `StrictPlain` gets `#[serde(deny_unknown_fields)]` on its derived
//! `Deserialize`, while `StrictOneof` and `StrictExt` get the strict terminal
//! arm in the hand-written visitor that oneofs and extension ranges force.

use crate::strictjson::__buffa::ext::LABEL;
use crate::strictjson::__buffa::register_types;
use crate::strictjson::{LenientOneof, LenientPlain, StrictExt, StrictOneof, StrictPlain};
use buffa::type_registry::{set_type_registry, TypeRegistry};
use buffa::ExtensionSet;

/// Install the type registry once so `"[buffa.test.strictjson.label]"` keys
/// resolve. Mirrors `extensions_json::setup`.
fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut reg = TypeRegistry::new();
        register_types(&mut reg);
        set_type_registry(reg);
    });
}

/// The `expected one of ...` tail of a serde unknown-field message.
///
/// Assertions run against this rather than the whole message: the rejected key
/// is quoted in the first half, and every key in these tests is a prefix of
/// its own typo (`value` of `valueTypo`), so a whole-message `contains` would
/// pass even if no list were emitted at all.
fn expected_list(msg: &str) -> &str {
    msg.split_once(", expected")
        .unwrap_or_else(|| panic!("no expected-key list in: {msg}"))
        .1
}

#[test]
fn derive_path_rejects_an_unknown_key() {
    let err = serde_json::from_str::<StrictPlain>(r#"{"valueTypo": 7}"#)
        .expect_err("unknown key must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("unknown field `valueTypo`"), "{msg}");
    // serde's own `unknown_field`, so the message also lists what is accepted.
    // Searched in the tail only: `valueTypo` contains `value`, so asserting
    // against the whole message would pass without any list at all.
    assert!(expected_list(&msg).contains("value"), "{msg}");
}

#[test]
fn oneof_path_rejects_an_unknown_key() {
    // The shape from the issue: a near-miss on a oneof variant name used to
    // parse into a message with the oneof unset.
    let err = serde_json::from_str::<StrictOneof>(r#"{"sensorNodeTypo": {"value": 1}}"#)
        .expect_err("unknown key must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("unknown field `sensorNodeTypo`"), "{msg}");
    // The visitor accepts both spellings of every variant, and says so. Tail
    // only, since the typo'd key itself contains `sensorNode`.
    let expected = expected_list(&msg);
    assert!(expected.contains("sensorNode"), "{msg}");
    assert!(expected.contains("sensor_node"), "{msg}");
}

#[test]
fn strictness_keeps_both_spellings_of_a_known_field() {
    for json in [r#"{"maxItems": 5}"#, r#"{"max_items": 5}"#] {
        let parsed: StrictPlain = serde_json::from_str(json).expect(json);
        assert_eq!(parsed.max_items, Some(5), "{json}");
    }
    for json in [
        r#"{"sensorNode": {"value": 1}}"#,
        r#"{"sensor_node": {"value": 1}}"#,
    ] {
        let parsed: StrictOneof = serde_json::from_str(json).expect(json);
        assert!(parsed.node.is_some(), "{json}");
    }
}

#[test]
fn a_nested_message_inherits_its_parents_rule() {
    // `StrictPlain.Nested` is not named by any rule; it is covered because a
    // rule naming a message covers the messages nested inside it.
    let err = serde_json::from_str::<crate::strictjson::strict_plain::Nested>(r#"{"nTypo": 1}"#)
        .expect_err("unknown key must be rejected");
    assert!(err.to_string().contains("unknown field `nTypo`"));
}

#[test]
fn messages_outside_the_rule_still_ignore_unknown_keys() {
    // The default is unchanged: same file, same codegen run, no rule.
    let parsed: LenientPlain =
        serde_json::from_str(r#"{"valueTypo": 7, "value": 3}"#).expect("lenient parse");
    assert_eq!(parsed.value, Some(3));

    let parsed: LenientOneof = serde_json::from_str(r#"{"aTypo": 1}"#).expect("lenient parse");
    assert!(parsed.node.is_none());
}

#[test]
fn extension_keys_survive_strictness() {
    setup();
    // `"[pkg.ext]"` keys are claimed by the extension arm before the terminal
    // arm, so strictness does not reach them.
    let parsed: StrictExt =
        serde_json::from_str(r#"{"x": 1, "[buffa.test.strictjson.label]": "hi"}"#)
            .expect("extension key must still parse");
    assert_eq!(parsed.x, Some(1));
    assert_eq!(
        parsed.extension(&LABEL).map(|s| s.to_string()),
        Some("hi".to_string())
    );

    // A plain unknown key on the same message is still rejected.
    let err = serde_json::from_str::<StrictExt>(r#"{"xTypo": 1}"#)
        .expect_err("unknown key must be rejected");
    assert!(err.to_string().contains("unknown field `xTypo`"));

    // And an *unregistered* extension key is still governed by
    // `JsonParseOptions::strict_extension_keys`, not by this option: it is
    // dropped by default even on a strict message.
    let parsed: StrictExt =
        serde_json::from_str(r#"{"x": 2, "[buffa.test.strictjson.nosuch]": 1}"#)
            .expect("unregistered extension key is dropped, not rejected");
    assert_eq!(parsed.x, Some(2));
}

/// A key that only *looks* bracketed is not an extension key — the registry
/// requires both brackets — so it must be rejected like any other unknown key
/// rather than slip through the extension arm.
#[test]
fn a_malformed_bracketed_key_is_not_mistaken_for_an_extension() {
    setup();
    for json in [r#"{"[oops": 1}"#, r#"{"oops]": 1}"#] {
        let err = serde_json::from_str::<StrictExt>(json)
            .expect_err("malformed bracketed key must be rejected");
        assert!(err.to_string().contains("unknown field"), "{json} -> {err}");
    }
}

/// Every field of a message on the hand-written-visitor path stays accepted,
/// in both spellings, whatever its shape — a synthetic oneof from proto3
/// `optional`, a map, a `repeated`, a `google.protobuf.Value`, a plain scalar,
/// and the real oneof's own variants. A field missing from the visitor's
/// accepted-key list would reject a valid key, so this pins the list.
#[test]
fn every_field_shape_stays_accepted_under_strictness() {
    use crate::strictjson3::Mixed;

    for json in [
        r#"{"optCount": 1}"#,
        r#"{"opt_count": 1}"#,
        r#"{"labelsByName": {"a": 1}}"#,
        r#"{"labels_by_name": {"a": 1}}"#,
        r#"{"tagList": ["x"]}"#,
        r#"{"tag_list": ["x"]}"#,
        r#"{"anyValue": 5}"#,
        r#"{"any_value": 5}"#,
        r#"{"maxItems": 2}"#,
        r#"{"max_items": 2}"#,
        r#"{"sensorNode": {"irEnabled": true}}"#,
        r#"{"sensor_node": {"ir_enabled": true}}"#,
        r#"{"plainNode": 3}"#,
        r#"{"plain_node": 3}"#,
    ] {
        serde_json::from_str::<Mixed>(json).unwrap_or_else(|e| panic!("{json} -> {e}"));
    }

    // And the diagnostic names all fourteen, so the list is neither short nor
    // padded with keys the arms do not actually accept.
    let err = serde_json::from_str::<Mixed>(r#"{"tagListTypo": []}"#)
        .expect_err("unknown key must be rejected");
    let msg = err.to_string();
    let expected = expected_list(&msg);
    for key in [
        "optCount",
        "opt_count",
        "labelsByName",
        "labels_by_name",
        "tagList",
        "tag_list",
        "anyValue",
        "any_value",
        "maxItems",
        "max_items",
        "sensorNode",
        "sensor_node",
        "plainNode",
        "plain_node",
    ] {
        assert!(expected.contains(key), "{key} missing from: {msg}");
    }
}

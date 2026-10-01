//! Generated messages read `serde_json`'s private `RawValue` key as data, in
//! every field shape that buffers a value before decoding it. See
//! `buffa::json_helpers::buffered` for what the key does. `raw_value` is a
//! dev-dependency feature of this crate, so these tests run with it on.

use crate::edenumjson::EnumJsonContexts;
use crate::extjson::__buffa::ext::WEIGHT;
use crate::extjson::Carrier;
use crate::json_types::{OptionalScalars, WithEnum};
use crate::p2json::ClosedEnumJson;
use buffa::json::{with_json_parse_options, JsonParseOptions};
use buffa::ExtensionSet;
use serde::de::DeserializeOwned;

use super::install_type_registry;

const RAW_VALUE_KEY: &str = "$serde_json::private::RawValue";

/// An object that `serde_json::Value` reads as `json` when `raw_value` is on.
fn hidden(json: &str) -> String {
    serde_json::json!({ RAW_VALUE_KEY: json }).to_string()
}

/// Asserts that `M` rejects `template` with `hidden(value)` in place of `@`,
/// and accepts it with `value` itself there, so the rejection is of the key.
#[track_caller]
fn assert_key_is_data<M: DeserializeOwned>(template: &str, value: &str) {
    let plain = template.replace('@', value);
    if let Err(e) = serde_json::from_str::<M>(&plain) {
        panic!("{plain} must decode: {e}");
    }
    let with_key = template.replace('@', &hidden(value));
    assert!(
        serde_json::from_str::<M>(&with_key).is_err(),
        "{with_key} must not decode as {plain}"
    );
}

#[test]
fn the_test_build_has_serde_json_raw_value_enabled() {
    let value: serde_json::Value = serde_json::from_str(&hidden("[1]")).unwrap();
    assert_eq!(value, serde_json::json!([1]));
}

#[test]
fn an_open_enum_is_not_read_from_the_private_key() {
    assert_key_is_data::<WithEnum>(r#"{"colors": [@]}"#, r#""RED""#);
    assert_key_is_data::<WithEnum>(r#"{"colors": ["GREEN", @]}"#, "1");
    assert_key_is_data::<OptionalScalars>(r#"{"oColor": @}"#, r#""RED""#);
    assert_key_is_data::<EnumJsonContexts>(r#"{"openByKey": {"k": @}}"#, "1");
    // Control: a singular enum field decodes without buffering.
    assert_key_is_data::<WithEnum>(r#"{"color": @}"#, r#""RED""#);
}

#[test]
fn a_closed_enum_is_not_read_from_the_private_key() {
    assert_key_is_data::<ClosedEnumJson>(r#"{"tier": @}"#, r#""PRO""#);
    assert_key_is_data::<ClosedEnumJson>(r#"{"tiers": [@]}"#, "1");
    assert_key_is_data::<ClosedEnumJson>(r#"{"byName": {"k": @}}"#, r#""PRO""#);
    assert_key_is_data::<EnumJsonContexts>(r#"{"byKey": {"k": @}}"#, "1");
}

#[test]
fn ignoring_unknown_enum_values_drops_the_private_key_as_an_unknown_value() {
    let lenient = JsonParseOptions::new().ignore_unknown_enum_values(true);
    with_json_parse_options(&lenient, || {
        let json = format!(r#"{{"colors": ["GREEN", {}]}}"#, hidden(r#""RED""#));
        let open: WithEnum = serde_json::from_str(&json).unwrap();
        let plain: WithEnum = serde_json::from_str(r#"{"colors": ["GREEN"]}"#).unwrap();
        assert_eq!(open.colors, plain.colors);

        let json = format!(
            r#"{{"tiers": [{}], "byName": {{"k": {}}}}}"#,
            hidden("1"),
            hidden(r#""PRO""#)
        );
        let closed: ClosedEnumJson = serde_json::from_str(&json).unwrap();
        assert!(closed.tiers.is_empty(), "{:?}", closed.tiers);
        assert!(closed.by_name.is_empty(), "{:?}", closed.by_name);
    });
}

#[test]
fn an_extension_is_not_read_from_the_private_key() {
    install_type_registry();
    let plain: Carrier = serde_json::from_str(r#"{"[buffa.test.extjson.weight]": -7}"#).unwrap();
    assert_eq!(plain.extension(&WEIGHT), Some(-7));
    assert_key_is_data::<Carrier>(r#"{"[buffa.test.extjson.weight]": @}"#, "-7");
}

#[test]
fn a_value_under_an_unregistered_extension_key_is_not_parsed_again() {
    install_type_registry();
    // Parsed as JSON, the string is 200 arrays deep and fails the recursion
    // limit. As data it is a string under a key that names no extension.
    let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    let json = format!(r#"{{"[no.such.extension]": {}}}"#, hidden(&deep));
    let carrier: Carrier = serde_json::from_str(&json).unwrap();
    assert_eq!(carrier, Carrier::default());
}

/// The error of decoding `template` as `M`, with `@` replaced by the private
/// key holding 200 nested arrays as a string.
#[track_caller]
fn error_of_deep_string<M: DeserializeOwned>(template: &str) -> String {
    let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    match serde_json::from_str::<M>(&template.replace('@', &hidden(&deep))) {
        Ok(_) => panic!("{template} decoded"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_string_under_the_private_key_is_not_parsed() {
    // Parsing the string fails the recursion limit. Reading it as data
    // fails because an object is not an enum value.
    for err in [
        error_of_deep_string::<WithEnum>(r#"{"colors": [@]}"#),
        error_of_deep_string::<OptionalScalars>(r#"{"oColor": @}"#),
        error_of_deep_string::<EnumJsonContexts>(r#"{"openByKey": {"k": @}}"#),
        error_of_deep_string::<ClosedEnumJson>(r#"{"tier": @}"#),
        error_of_deep_string::<ClosedEnumJson>(r#"{"tiers": [@]}"#),
        error_of_deep_string::<ClosedEnumJson>(r#"{"byName": {"k": @}}"#),
    ] {
        assert!(!err.contains("recursion limit"), "{err:.200}");
        assert!(err.contains("expected a protobuf enum"), "{err:.200}");
    }
}

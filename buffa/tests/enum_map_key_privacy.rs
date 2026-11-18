#![cfg(feature = "json")]

use buffa::{EnumValue, Enumeration, Map};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Color {
    #[default]
    Zero,
}
impl Enumeration for Color {
    fn from_i32(value: i32) -> Option<Self> {
        (value == 0).then_some(Self::Zero)
    }
    fn to_i32(&self) -> i32 {
        0
    }
    fn proto_name(&self) -> &'static str {
        "ZERO"
    }
    fn from_proto_name(name: &str) -> Option<Self> {
        (name == "ZERO").then_some(Self::Zero)
    }
}
#[derive(Debug, Deserialize)]
struct OpenInt {
    #[serde(with = "buffa::json_helpers::map_enum")]
    values: Map<i32, EnumValue<Color>>,
}
#[derive(Debug, Deserialize)]
struct ClosedInt {
    #[serde(with = "buffa::json_helpers::map_closed_enum")]
    values: Map<i32, Color>,
}
#[derive(Debug, Deserialize)]
struct OpenBool {
    #[serde(with = "buffa::json_helpers::map_enum")]
    values: Map<bool, EnumValue<Color>>,
}

#[test]
fn enum_map_key_conversion_errors_omit_input() {
    for ignore in [false, true] {
        buffa::json::with_json_parse_options(
            &buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore),
            || {
                for key in ["4111111111111111", "-4111111111111111", "2147483648"] {
                    let input = format!(r#"{{"values":{{"{key}":0}}}}"#);
                    let value: serde_json::Value = serde_json::from_str(&input).unwrap();
                    let errors = [
                        serde_json::from_str::<OpenInt>(&input).unwrap_err(),
                        serde_json::from_str::<ClosedInt>(&input).unwrap_err(),
                        serde_json::from_value::<OpenInt>(value.clone()).unwrap_err(),
                        serde_json::from_value::<ClosedInt>(value).unwrap_err(),
                    ];
                    for error in errors {
                        assert!(error.is_data());
                        assert!(
                            error.to_string().starts_with("invalid protobuf map key"),
                            "{error}"
                        );
                        assert!(!error.to_string().contains(key), "{error}");
                    }
                }
                let error =
                    serde_json::from_str::<OpenBool>(r#"{"values":{"marker-key":0}}"#).unwrap_err();
                assert!(
                    error.to_string().starts_with("invalid protobuf map key"),
                    "{error}"
                );
                assert!(!error.to_string().contains("marker-key"));
            },
        );
    }
}

#[test]
fn native_key_grammar_and_borrowing_are_preserved() {
    for input in [
        r#"{"0":0}"#,
        r#"{"-1":0}"#,
        r#"{"2147483647":0}"#,
        r#"{"\u0031":0}"#,
        r#"{"1\u0030":0}"#,
        r#"{" 1":0}"#,
        r#"{"1 ":0}"#,
        r#"{"01":0}"#,
        r#"{"+1":0}"#,
    ] {
        let native = serde_json::from_str::<std::collections::BTreeMap<i32, i32>>(input);
        let adapted = serde_json::from_str::<OpenInt>(&format!(r#"{{"values":{input}}}"#));
        assert_eq!(native.is_ok(), adapted.is_ok(), "{input}");
        if let (Err(native), Err(adapted)) = (native, adapted) {
            assert_eq!(native.classify(), adapted.classify(), "{input}");
        }
    }
    let open = serde_json::from_str::<OpenInt>(r#"{"values":{"-1":0}}"#).unwrap();
    assert_eq!(open.values[&-1], EnumValue::Known(Color::Zero));
    let closed = serde_json::from_str::<ClosedInt>(r#"{"values":{"1":0}}"#).unwrap();
    assert_eq!(closed.values[&1], Color::Zero);
    let boolean = serde_json::from_str::<OpenBool>(r#"{"values":{"true":0}}"#).unwrap();
    assert_eq!(boolean.values[&true], EnumValue::Known(Color::Zero));
    #[derive(Debug, Deserialize)]
    struct Borrowed<'a> {
        #[serde(borrow, with = "buffa::json_helpers::map_enum")]
        values: Map<&'a str, EnumValue<Color>>,
    }
    let input = r#"{"values":{"borrowed-key":0}}"#;
    let borrowed: Borrowed<'_> = serde_json::from_str(input).unwrap();
    let key = *borrowed.values.keys().next().unwrap();
    let source = input.find("borrowed-key").unwrap();
    assert_eq!(key.as_ptr(), input[source..].as_ptr());
}

#[test]
fn malformed_keys_and_reader_failures_keep_their_category() {
    use std::io::{self, Read};
    struct Interrupted<'a> {
        input: &'a [u8],
        cutoff: usize,
        offset: usize,
    }
    impl Read for Interrupted<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.offset >= self.cutoff {
                return Err(io::Error::other("synthetic reader failure"));
            }
            if out.is_empty() {
                return Ok(0);
            }
            if self.offset == self.input.len() {
                return Ok(0);
            }
            out[0] = self.input[self.offset];
            self.offset = self.offset.checked_add(1).unwrap();
            Ok(1)
        }
    }
    let input = br#"{"values":{"12":0}}"#;
    for cutoff in 0..input.len() {
        let error = serde_json::from_reader::<_, OpenInt>(Interrupted {
            input,
            cutoff,
            offset: 0,
        })
        .unwrap_err();
        assert!(error.is_io(), "cutoff {cutoff}: {error}");
        let error = serde_json::from_reader::<_, ClosedOptionalKey>(Interrupted {
            input,
            cutoff,
            offset: 0,
        })
        .unwrap_err();
        assert!(error.is_io(), "nested cutoff {cutoff}: {error}");
    }
    for input in [
        r#"{"values":{"\u00QQ":0}}"#,
        r#"{"values":{"12"#,
        r#"{"values":{"12":0"#,
    ] {
        let error = serde_json::from_str::<OpenInt>(input).unwrap_err();
        assert!(error.is_syntax() || error.is_eof(), "{error}");
        let error = serde_json::from_str::<ClosedOptionalKey>(input).unwrap_err();
        assert!(error.is_syntax() || error.is_eof(), "nested: {error}");
    }
}

#[test]
fn unit_enum_keys_and_custom_validation_are_redacted() {
    #[derive(Debug, Deserialize, PartialEq, Eq, Hash)]
    enum Key {
        Known,
    }
    #[derive(Debug, Deserialize)]
    struct Open {
        #[serde(with = "buffa::json_helpers::map_enum")]
        values: Map<Key, EnumValue<Color>>,
    }
    #[derive(Debug, Deserialize)]
    struct Closed {
        #[serde(with = "buffa::json_helpers::map_closed_enum")]
        values: Map<Key, Color>,
    }
    let input = r#"{"values":{"private-key":0}}"#;
    for error in [
        serde_json::from_str::<Open>(input).unwrap_err(),
        serde_json::from_str::<Closed>(input).unwrap_err(),
    ] {
        assert!(
            error.to_string().starts_with("invalid protobuf map key"),
            "{error}"
        );
        assert!(!error.to_string().contains("private-key"));
    }
    let open = serde_json::from_str::<Open>(r#"{"values":{"Known":0}}"#).unwrap();
    assert_eq!(open.values[&Key::Known], EnumValue::Known(Color::Zero));
    let closed = serde_json::from_str::<Closed>(r#"{"values":{"Known":0}}"#).unwrap();
    assert_eq!(closed.values[&Key::Known], Color::Zero);

    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Checked;
    impl<'de> Deserialize<'de> for Checked {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let key = String::deserialize(d)?;
            // Deliberately models an input-bearing consumer validation error.
            Err(serde::de::Error::custom(key))
        }
    }
    #[derive(Debug, Deserialize)]
    struct CheckedMap {
        #[serde(with = "buffa::json_helpers::map_enum")]
        values: Map<Checked, EnumValue<Color>>,
    }
    let error = serde_json::from_str::<CheckedMap>(input).unwrap_err();
    assert!(
        error.to_string().starts_with("invalid protobuf map key"),
        "{error}"
    );
    assert!(!error.to_string().contains("private-key"));
    // This path only tests rejection; read the field type to keep strict lints useful.
    let _: fn(CheckedMap) -> Map<Checked, EnumValue<Color>> = |map| map.values;
}

#[test]
fn try_from_key_error_context_is_never_formatted() {
    #[derive(Debug)]
    struct ForbiddenContext;
    impl core::fmt::Display for ForbiddenContext {
        fn fmt(&self, _: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            panic!("key conversion context must never be formatted")
        }
    }
    #[derive(Debug, Deserialize, PartialEq, Eq, Hash)]
    #[serde(try_from = "String")]
    struct Checked;
    impl TryFrom<String> for Checked {
        type Error = ForbiddenContext;
        fn try_from(_: String) -> Result<Self, Self::Error> {
            Err(ForbiddenContext)
        }
    }
    #[derive(Debug, Deserialize)]
    struct Holder {
        #[serde(with = "buffa::json_helpers::map_enum")]
        values: Map<Checked, EnumValue<Color>>,
    }
    let error = serde_json::from_str::<Holder>(r#"{"values":{"private-key":0}}"#).unwrap_err();
    assert!(
        error.to_string().starts_with("invalid protobuf map key"),
        "{error}"
    );
    let _: fn(Holder) -> Map<Checked, EnumValue<Color>> = |map| map.values;
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct NativeKey(i32);
#[derive(Debug, Deserialize)]
struct OpenNewtype {
    #[serde(with = "buffa::json_helpers::map_enum")]
    values: Map<NativeKey, EnumValue<Color>>,
}
#[derive(Debug, Deserialize)]
struct ClosedOptionalKey {
    #[serde(with = "buffa::json_helpers::map_closed_enum")]
    values: Map<Option<NativeKey>, Color>,
}

#[test]
fn nested_key_visitors_preserve_valid_keys_and_redact_conversion_errors() {
    let input = r#"{"values":{"-1":0}}"#;
    let open: OpenNewtype = serde_json::from_str(input).unwrap();
    assert_eq!(open.values[&NativeKey(-1)], EnumValue::Known(Color::Zero));
    let closed: ClosedOptionalKey = serde_json::from_str(input).unwrap();
    assert_eq!(closed.values[&Some(NativeKey(-1))], Color::Zero);
    let value: serde_json::Value = serde_json::from_str(input).unwrap();
    assert!(serde_json::from_value::<OpenNewtype>(value.clone()).is_ok());
    assert!(serde_json::from_value::<ClosedOptionalKey>(value).is_ok());
    for key in ["2147483648", "private-key"] {
        let input = format!(r#"{{"values":{{"{key}":0}}}}"#);
        let value: serde_json::Value = serde_json::from_str(&input).unwrap();
        let native_text = serde_json::from_str::<std::collections::BTreeMap<NativeKey, u32>>(
            &format!(r#"{{"{key}":0}}"#),
        )
        .unwrap_err();
        let native_value = serde_json::from_value::<std::collections::BTreeMap<NativeKey, u32>>(
            value["values"].clone(),
        )
        .unwrap_err();
        for (error, category) in [
            (
                serde_json::from_str::<OpenNewtype>(&input).unwrap_err(),
                native_text.classify(),
            ),
            (
                serde_json::from_str::<ClosedOptionalKey>(&input).unwrap_err(),
                native_text.classify(),
            ),
            (
                serde_json::from_value::<OpenNewtype>(value.clone()).unwrap_err(),
                native_value.classify(),
            ),
            (
                serde_json::from_value::<ClosedOptionalKey>(value).unwrap_err(),
                native_value.classify(),
            ),
        ] {
            assert_eq!(error.classify(), category, "{key}: {error}");
            if error.is_data() {
                assert!(error.to_string().starts_with("invalid protobuf map key"));
            }
            assert!(!error.to_string().contains(key));
        }
    }
}

#[test]
fn native_key_type_errors_do_not_format_consumer_expectations() {
    use core::sync::atomic::{AtomicUsize, Ordering};
    static EXPECTED_CALLS: AtomicUsize = AtomicUsize::new(0);
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct BoolKey(bool);
    impl<'de> Deserialize<'de> for BoolKey {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl serde::de::Visitor<'_> for V {
                type Value = BoolKey;
                fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    EXPECTED_CALLS.fetch_add(1, Ordering::SeqCst);
                    f.write_str("consumer-provided context")
                }
                fn visit_bool<E>(self, value: bool) -> Result<BoolKey, E> {
                    Ok(BoolKey(value))
                }
            }
            d.deserialize_bool(V)
        }
    }
    #[derive(Debug, Deserialize)]
    struct Values {
        #[serde(with = "buffa::json_helpers::map_enum")]
        values: Map<BoolKey, EnumValue<Color>>,
    }
    let valid: Values = serde_json::from_str(r#"{"values":{"true":0}}"#).unwrap();
    assert_eq!(valid.values[&BoolKey(true)], EnumValue::Known(Color::Zero));
    let error = serde_json::from_str::<Values>(r#"{"values":{"private-key":0}}"#).unwrap_err();
    assert!(error.is_data());
    assert!(error.to_string().starts_with("invalid protobuf map key"));
    // The adapter discards this expectation, so formatting it would only
    // expose consumer context to an allocation, side effect, or panic.
    assert_eq!(EXPECTED_CALLS.load(Ordering::SeqCst), 0);
}

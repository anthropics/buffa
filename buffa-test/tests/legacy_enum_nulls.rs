//! Frozen pre-change bindings exercise compatibility with the updated WKT.
//! Captured with generator revision ce3fd6e using json=true, views=true and
//! unknown_fields=false; the input schema is beside the frozen bindings. Keep
//! their field/view plumbing unchanged so this exercises existing consumers.
#[allow(clippy::derivable_impls)]
pub mod legacy {
    include!("fixtures/legacy_enum_nulls/legacy_enum_nulls.mod.rs");
}

#[test]
fn pre_change_generated_fields_and_views_retain_null_value_semantics() {
    use buffa::{Message, MessageView};
    use legacy::{Legacy, LegacyView};
    let input = r#"{"values":[null,99],"byId":{"7":"NULL_VALUE","8":99}}"#;
    let value: Legacy = serde_json::from_str(input).unwrap();
    let expected = serde_json::json!({"values":[null,99],"byId":{"7":"NULL_VALUE","8":99}});
    let bytes = value.encode_to_vec();
    let view = LegacyView::decode_view(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&value).unwrap(), expected);
    assert_eq!(serde_json::to_value(&view).unwrap(), expected);
}

#[test]
fn pre_change_optional_null_value_presence_is_unchanged() {
    let value = legacy::Legacy {
        optional_value: Some(buffa_types::google::protobuf::NullValue::NULL_VALUE.into()),
        ..Default::default()
    };
    use buffa::{Message, MessageView};
    let bytes = value.encode_to_vec();
    let view = legacy::LegacyView::decode_view(&bytes).unwrap();
    for json in [
        serde_json::to_string(&value).unwrap(),
        serde_json::to_string(&view).unwrap(),
    ] {
        assert!(json.contains("NULL_VALUE"));
        let decoded: legacy::Legacy = serde_json::from_str(&json).unwrap();
        assert!(decoded.optional_value.is_some());
    }
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
struct Closed(
    #[serde(with = "buffa::json_helpers::repeated_closed_enum")]
    Vec<buffa_types::google::protobuf::NullValue>,
);
#[test]
fn explicit_closed_null_enum_helper_serializes_canonical_null() {
    let value = Closed(vec![buffa_types::google::protobuf::NullValue::NULL_VALUE]);
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        serde_json::json!([null])
    );
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(serde_json::from_str::<Closed>(&json).unwrap(), value);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Ordinary {
    #[default]
    Unspecified,
}
impl buffa::Enumeration for Ordinary {
    fn from_i32(value: i32) -> Option<Self> {
        (value == 0).then_some(Self::Unspecified)
    }
    fn to_i32(&self) -> i32 {
        0
    }
    fn proto_name(&self) -> &'static str {
        "ORDINARY_UNSPECIFIED"
    }
    fn from_proto_name(value: &str) -> Option<Self> {
        (value == "ORDINARY_UNSPECIFIED").then_some(Self::Unspecified)
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
struct OrdinaryClosed(#[serde(with = "buffa::json_helpers::repeated_closed_enum")] Vec<Ordinary>);
#[test]
fn ordinary_closed_enum_nulls_stay_malformed_in_both_modes() {
    for ignore in [false, true] {
        buffa::json::with_json_parse_options(
            &buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore),
            || {
                assert!(serde_json::from_str::<OrdinaryClosed>("[null]").is_err());
                let value: OrdinaryClosed = serde_json::from_str("[0]").unwrap();
                assert_eq!(
                    serde_json::to_value(value).unwrap(),
                    serde_json::json!(["ORDINARY_UNSPECIFIED"])
                );
            },
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum InvalidMarker {
    #[default]
    Other,
}
impl buffa::Enumeration for InvalidMarker {
    const IS_NULL_VALUE: bool = true;
    fn from_i32(value: i32) -> Option<Self> {
        (value == 7).then_some(Self::Other)
    }
    fn to_i32(&self) -> i32 {
        7
    }
    fn proto_name(&self) -> &'static str {
        "OTHER"
    }
    fn from_proto_name(value: &str) -> Option<Self> {
        (value == "OTHER").then_some(Self::Other)
    }
}
#[derive(Debug, serde::Deserialize)]
#[serde(transparent)]
struct InvalidClosed(
    #[serde(with = "buffa::json_helpers::repeated_closed_enum")] Vec<InvalidMarker>,
);
#[test]
fn a_null_marker_without_known_zero_never_defaults_or_filters_the_element() {
    assert!(serde_json::from_str::<InvalidClosed>("[]")
        .unwrap()
        .0
        .is_empty());
    for ignore in [false, true] {
        buffa::json::with_json_parse_options(
            &buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore),
            || {
                let error = serde_json::from_str::<InvalidClosed>("[null]").unwrap_err();
                assert!(error
                    .to_string()
                    .starts_with("invalid NullValue enum marker"));
            },
        );
    }
}

//! Repeated enum nulls are rejected unless the type is canonical NullValue.
use buffa_test::enum_nulls::{Custom, Plain};

#[test]
fn ordinary_enums_reject_first_and_later_nulls_in_both_modes() {
    for ignore in [false, true] {
        let options = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore);
        buffa::json::with_json_parse_options(&options, || {
            for field in ["values", "impostors"] {
                for items in ["[null]", "[1,null,0]"] {
                    let input = format!(r#"{{"{field}":{items}}}"#);
                    assert!(serde_json::from_str::<Plain>(&input).is_err());
                    assert!(serde_json::from_str::<Custom>(&input).is_err());
                    let value: serde_json::Value = serde_json::from_str(&input).unwrap();
                    assert!(serde_json::from_value::<Plain>(value.clone()).is_err());
                    assert!(serde_json::from_value::<Custom>(value).is_err());
                }
            }
        });
    }
}

#[test]
fn whole_field_null_and_null_value_elements_remain_supported() {
    for ignore in [false, true] {
        let options = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore);
        buffa::json::with_json_parse_options(&options, || {
            for input in [
                r#"{"values":null,"nulls":null}"#,
                r#"{"values":[],"nulls":[]}"#,
            ] {
                assert_eq!(
                    serde_json::from_str::<Plain>(input).unwrap(),
                    Plain::default()
                );
                assert_eq!(
                    serde_json::from_str::<Custom>(input).unwrap(),
                    Custom::default()
                );
            }
            let input = r#"{"values":[1,0,99],"nulls":[null,0,"NULL_VALUE",99]}"#;
            let expected =
                serde_json::json!({"values":["READY","ZERO",99],"nulls":[null,null,null,99]});
            assert_eq!(
                serde_json::to_value(serde_json::from_str::<Plain>(input).unwrap()).unwrap(),
                expected
            );
            assert_eq!(
                serde_json::to_value(serde_json::from_str::<Custom>(input).unwrap()).unwrap(),
                expected
            );
        });
    }
}

#[test]
fn owned_and_borrowed_null_value_json_agree() {
    use buffa::{Message, MessageView};
    use buffa_test::enum_nulls::{CustomView, PlainView};
    let input = r#"{"values":[1,99],"nulls":[null,99]}"#;
    let plain: Plain = serde_json::from_str(input).unwrap();
    let custom: Custom = serde_json::from_str(input).unwrap();
    let plain_bytes = plain.encode_to_vec();
    let custom_bytes = custom.encode_to_vec();
    assert_eq!(
        serde_json::to_value(PlainView::decode_view(&plain_bytes).unwrap()).unwrap(),
        serde_json::to_value(plain).unwrap()
    );
    assert_eq!(
        serde_json::to_value(CustomView::decode_view(&custom_bytes).unwrap()).unwrap(),
        serde_json::to_value(custom).unwrap()
    );
}

//! Rust keywords as proto package/message/field/enum names.

use super::round_trip;

#[test]
fn test_keyword_message_type_round_trip() {
    use crate::keywords;
    let msg = keywords::Type {
        name: "int".into(),
        kind: buffa::EnumValue::Known(keywords::r#type::Kind::KIND_PRIMITIVE),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.name, "int");
}

#[test]
fn test_keyword_fields_round_trip() {
    use crate::keywords::KeywordFields;
    let msg = KeywordFields {
        r#type: "hello".into(),
        r#match: 42,
        r#async: true,
        self_: "me".into(),
        r#mod: vec![1, 2, 3],
        r#fn: 3.14,
        super_: "parent".into(),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.r#type, "hello");
    assert_eq!(decoded.r#match, 42);
    assert!(decoded.r#async);
    assert_eq!(decoded.self_, "me");
    assert_eq!(decoded.r#mod, vec![1, 2, 3]);
    assert!((decoded.r#fn - 3.14).abs() < 1e-10);
    assert_eq!(decoded.super_, "parent");
}

#[test]
fn test_keyword_expression_with_oneof() {
    use crate::keywords;
    let msg = keywords::Expression {
        result_type: buffa::MessageField::some(keywords::Type {
            name: "bool".into(),
            ..Default::default()
        }),
        match_mode: buffa::EnumValue::Known(keywords::Match::MATCH_EXACT),
        value: Some(keywords::__buffa::oneof::expression::Value::Literal(
            "42".into(),
        )),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.result_type.name, "bool");
    assert_eq!(
        decoded.value,
        Some(keywords::__buffa::oneof::expression::Value::Literal(
            "42".into()
        ))
    );
}

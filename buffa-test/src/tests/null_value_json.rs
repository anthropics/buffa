//! `google.protobuf.NullValue` fields outside a oneof. ProtoJSON writes each
//! value as `null` and reads `null` as a set `NULL_VALUE`, in owned and view
//! serialization alike. Fixtures: `protos/null_value_json*.proto`.

use buffa::{EnumValue, Message, MessageView};
use buffa_types::google::protobuf::NullValue;
use serde_json::{json, Value};

const NULL: EnumValue<NullValue> = EnumValue::Known(NullValue::NULL_VALUE);

/// Serialize `$owned` and its decoded `$view`, assert that both give
/// `$expected`, and assert that `$expected` parses back to `$owned`.
macro_rules! check {
    ($view:ty, $owned:expr, $expected:expr $(,)?) => {{
        let owned = $owned;
        let expected: Value = $expected;
        assert_eq!(serde_json::to_value(&owned).unwrap(), expected, "owned");
        let bytes = owned.encode_to_vec();
        let view = <$view>::decode_view(&bytes).expect("decode_view");
        assert_eq!(serde_json::to_value(&view).unwrap(), expected, "view");
        assert_eq!(serde_json::from_value(expected).ok(), Some(owned));
    }};
}

#[test]
fn proto3_fields_write_null() {
    use crate::null_json::__buffa::view::NullFieldsView;
    use crate::null_json::NullFields;

    let mut msg = NullFields {
        opt: Some(NULL),
        list: vec![NULL, NULL],
        ..Default::default()
    };
    msg.by_name.insert("k".into(), NULL);
    msg.by_id.insert(7, NULL);
    msg.by_flag.insert(true, NULL);
    check!(
        NullFieldsView,
        msg,
        json!({
            "opt": null,
            "list": [null, null],
            "byName": {"k": null},
            "byId": {"7": null},
            "byFlag": {"true": null},
        }),
    );

    // An unknown number in an implicit-presence field is still `null`.
    let msg = NullFields {
        implicit: EnumValue::from(3),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(&msg).unwrap(),
        json!({"implicit": null})
    );
    let bytes = msg.encode_to_vec();
    let view = NullFieldsView::decode_view(&bytes).unwrap();
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        json!({"implicit": null})
    );
}

#[test]
fn proto3_null_sets_the_value() {
    use crate::null_json::NullFields;

    for opt in [json!(null), json!("NULL_VALUE"), json!(0)] {
        let msg: NullFields = serde_json::from_value(json!({ "opt": opt })).unwrap();
        assert_eq!(msg.opt, Some(NULL), "{opt}");
        // Present on the wire: field 2, varint 0.
        assert_eq!(msg.encode_to_vec(), [0x10, 0x00], "{opt}");
    }
    let msg: NullFields = serde_json::from_str("{}").unwrap();
    assert_eq!(msg.opt, None);

    let msg: NullFields = serde_json::from_value(json!({
        "list": [null, "NULL_VALUE", 0],
        "byName": {"a": null},
        "byId": {"-1": null},
        "byFlag": {"false": null},
    }))
    .unwrap();
    assert_eq!(msg.list, [NULL, NULL, NULL]);
    assert_eq!(msg.by_name.get("a"), Some(&NULL));
    assert_eq!(msg.by_id.get(&-1), Some(&NULL));
    assert_eq!(msg.by_flag.get(&false), Some(&NULL));

    // `null` for a whole container is still the empty container.
    let msg: NullFields = serde_json::from_value(json!({"list": null, "byName": null})).unwrap();
    assert!(msg.list.is_empty() && msg.by_name.is_empty());
}

#[test]
fn unknown_names_dropped_when_lenient() {
    use crate::null_json::NullFields;

    let opts = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(true);
    buffa::json::with_json_parse_options(&opts, || {
        let msg: NullFields = serde_json::from_value(json!({
            "opt": "BOGUS",
            "list": [null, "BOGUS"],
            "byName": {"a": null, "b": "BOGUS"},
        }))
        .unwrap();
        assert_eq!(msg.opt, None);
        assert_eq!(msg.list, [NULL]);
        assert_eq!(msg.by_name.len(), 1);
    });
    assert!(serde_json::from_value::<NullFields>(json!({"list": ["BOGUS"]})).is_err());
}

#[test]
fn editions_explicit_presence_and_bytes_key() {
    use crate::null_json_ed::__buffa::view::NullFieldsView;
    use crate::null_json_ed::NullFields;

    let mut msg = NullFields {
        sing: Some(NULL),
        ..Default::default()
    };
    msg.raw_keys.insert(b"a".to_vec(), NULL);
    check!(
        NullFieldsView,
        msg,
        json!({"sing": null, "rawKeys": {"YQ==": null}})
    );

    let msg: NullFields = serde_json::from_str(r#"{"sing":null}"#).unwrap();
    assert_eq!(msg.sing, Some(NULL));
}

#[test]
fn proto2_optional_and_required() {
    use crate::null_json_p2::__buffa::view::NullFieldsView;
    use crate::null_json_p2::NullFields;

    check!(NullFieldsView, NullFields::default(), json!({"req": null}));
    check!(
        NullFieldsView,
        NullFields {
            opt: Some(NULL),
            ..Default::default()
        },
        json!({"opt": null, "req": null}),
    );
}

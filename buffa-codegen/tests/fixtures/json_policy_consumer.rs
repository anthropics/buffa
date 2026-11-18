use crate::pa::{Record, RecordLazyView, RecordOwnedView, RecordView};
use buffa::{LazyMessageView, Message, MessageView};

#[test]
fn foreign_policies_round_trip_owned_and_all_view_families() {
    let json = r#"{"single":"AQ==","optionalBlob":"Ag==","title":"~hello","optionalTitle":"~maybe","blobs":["Aw=="],"labels":["~label"],"byId":{"1":"BA=="},"byFlag":{"true":"BQ=="},"byKey":{"~key":"Bg=="},"byTag":{"tag":"Bw=="},"textPairs":{"~k":"~v"},"texts":{"2":"~two"},"blob":"CA=="}"#;
    let message: Record = serde_json::from_str(json).unwrap();
    let other: crate::pb::Record = serde_json::from_str(json).unwrap();
    let expected = serde_json::to_value(&message).unwrap();
    assert_eq!(serde_json::to_value(&other).unwrap(), expected);
    let wire: buffa::bytes::Bytes = message.encode_to_vec().into();
    assert_eq!(Record::decode_from_slice(&wire).unwrap(), message);
    assert_eq!(
        serde_json::to_value(RecordView::decode_view(&wire).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::to_value(RecordLazyView::decode_lazy(&wire).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::to_value(RecordOwnedView::decode(wire).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn null_and_invalid_inputs_keep_protobuf_field_rules() {
    for json in [
        r#"{"blobs":[null]}"#,
        r#"{"labels":[null]}"#,
        r#"{"byId":{"1":null}}"#,
        r#"{"byFlag":{"true":null}}"#,
        r#"{"byKey":{"~k":null}}"#,
        r#"{"byTag":{"tag":null}}"#,
        r#"{"textPairs":{"~k":null}}"#,
        r#"{"texts":{"1":null}}"#,
        r#"{"single":"%%%"}"#,
        r#"{"labels":["missing-marker"]}"#,
        r#"{"byId":{"2147483648":"AQ=="}}"#,
    ] {
        assert!(serde_json::from_str::<Record>(json).is_err(), "{json}");
    }
    let empty: Record = serde_json::from_str(r#"{"single":null,"optionalBlob":null,"title":null,"optionalTitle":null,"blobs":null,"labels":null,"byId":null,"byKey":null,"blob":null}"#).unwrap();
    assert_eq!(empty, Record::default());
}

#[test]
fn export_refusal_reaches_owned_eager_lazy_and_owned_views() {
    for json in [
        r#"{"single":"/w=="}"#,
        r#"{"optionalBlob":"/w=="}"#,
        r#"{"blobs":["/w=="]}"#,
        r#"{"byId":{"1":"/w=="}}"#,
        r#"{"byFlag":{"true":"/w=="}}"#,
        r#"{"byKey":{"~k":"/w=="}}"#,
        r#"{"byTag":{"k":"/w=="}}"#,
        r#"{"blob":"/w=="}"#,
        r#"{"title":"~deny"}"#,
        r#"{"optionalTitle":"~deny"}"#,
        r#"{"labels":["~deny"]}"#,
        r#"{"textPairs":{"~k":"~deny"}}"#,
        r#"{"texts":{"1":"~deny"}}"#,
        r#"{"text":"~deny"}"#,
    ] {
        let message: Record = serde_json::from_str(json).unwrap();
        let wire: buffa::bytes::Bytes = message.encode_to_vec().into();
        assert!(serde_json::to_vec(&message).is_err(), "{json}");
        assert!(
            serde_json::to_vec(&RecordView::decode_view(&wire).unwrap()).is_err(),
            "{json}"
        );
        assert!(
            serde_json::to_vec(&RecordLazyView::decode_lazy(&wire).unwrap()).is_err(),
            "{json}"
        );
        assert!(
            serde_json::to_vec(&RecordOwnedView::decode(wire).unwrap()).is_err(),
            "{json}"
        );
    }
}

#[test]
#[cfg(feature = "reflect")]
fn descriptor_json_does_not_apply_the_generated_policy() {
    use buffa_descriptor::reflect::Reflectable;
    let message: Record = serde_json::from_str(r#"{"single":"/w==","title":"~deny"}"#).unwrap();
    assert!(serde_json::to_value(&message).is_err());
    let reflected = serde_json::to_value(message.reflect().to_dynamic()).unwrap();
    assert_eq!(reflected["single"], "/w==");
    assert_eq!(reflected["title"], "deny");
}

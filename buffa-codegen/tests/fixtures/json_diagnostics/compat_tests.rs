use buffa_types::google::protobuf::{Duration, Empty, FieldMask, ListValue, Struct, Timestamp};
use diagnostics_consumer::{compat::*, Text};
fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + core::fmt::Debug + PartialEq>(
    v: T,
) {
    let bytes = postcard::to_allocvec(&v).unwrap();
    assert_eq!(postcard::from_bytes::<T>(&bytes).unwrap(), v);
}
#[test]
fn timestamp_postcard() {
    roundtrip(Timestamp {
        seconds: 42,
        ..Default::default()
    });
}
#[test]
fn duration_postcard() {
    roundtrip(Duration {
        seconds: 1,
        nanos: 7,
        ..Default::default()
    });
}
#[test]
fn field_mask_postcard() {
    roundtrip(FieldMask {
        paths: vec!["alpha.beta".into()],
        ..Default::default()
    });
}
#[test]
fn empty_postcard() {
    roundtrip(Empty::default());
}
#[test]
fn empty_struct_postcard() {
    roundtrip(Struct::default());
}
#[test]
fn empty_list_postcard() {
    roundtrip(ListValue::default());
}
#[test]
fn optional_native_postcard() {
    roundtrip(Optional {
        flag: Some(true),
        text: Some("payload".into()),
        ..Default::default()
    });
}
#[test]
fn custom_optional_roundtrip() {
    let v = CustomOptional {
        text: Some(Text("payload".into())),
        ..Default::default()
    };
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["text"], "prefix:payload");
    assert_eq!(serde_json::from_value::<CustomOptional>(json).unwrap(), v);
    assert!(serde_json::from_str::<CustomOptional>(r#"{"text":"unprefixed"}"#).is_err());
}
#[test]
fn custom_oneof_roundtrip() {
    let v = CustomOneof {
        choice: Some(custom_oneof::Choice::Text(Text("payload".into()))),
        ..Default::default()
    };
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["text"], "prefix:payload");
    assert_eq!(serde_json::from_value::<CustomOneof>(json).unwrap(), v);
    assert!(serde_json::from_str::<CustomOneof>(r#"{"text":"unprefixed"}"#).is_err());
}
#[test]
fn explicit_private_number_json_name() {
    let v = Renamed {
        choice: Some(renamed::Choice::Field("payload".into())),
        ..Default::default()
    };
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(serde_json::from_value::<Renamed>(json).unwrap(), v);
}

#[allow(non_camel_case_types, dead_code)]
mod primitive_names {
    type bool = ();
    type i64 = ();
    type u64 = ();
    type i128 = ();
    type u128 = ();
    type f64 = ();
    type str = ();
    type u8 = ();
    pub(super) struct Reject;
    impl serde::de::Visitor<'_> for Reject {
        type Value = ();
        fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("a unit")
        }
        buffa::json_helpers::reject_other_json_kinds!(bool, i64, u64, i128, u128, f64, str, bytes);
    }
}
#[test]
fn exported_macro_resolves_primitive_types_in_the_consuming_crate() {
    use serde::de::Deserializer;
    let value = serde::de::value::U128Deserializer::<serde::de::value::Error>::new(u128::MAX);
    let error = value.deserialize_any(primitive_names::Reject).unwrap_err();
    assert!(!error.to_string().contains(&u128::MAX.to_string()));
}

#[test]
fn empty_reflective_message_reads_postcard_map() {
    use buffa_descriptor::reflect::DynamicMessage;
    use buffa_descriptor::{DescriptorPool, DynamicMessageSeed};
    use serde::de::DeserializeSeed;
    use std::sync::Arc;
    let pool = Arc::new(DescriptorPool::decode(include_bytes!("../proto/reflect.fds")).unwrap());
    let index = pool.message_index("reflect.test.Inner").unwrap();
    let value = DynamicMessage::new(Arc::clone(&pool), index);
    // DynamicMessage's serializer historically lacks a known map length.
    // Test the previously supported seed input, independently of that codec.
    let bytes =
        postcard::to_allocvec(&std::collections::BTreeMap::<String, String>::new()).unwrap();
    let mut de = postcard::Deserializer::from_bytes(&bytes);
    assert_eq!(
        DynamicMessageSeed::new(pool, index)
            .deserialize(&mut de)
            .unwrap(),
        value
    );
}
#[test]
fn explicit_private_number_json_name_does_not_accept_a_scalar() {
    let number = "18446744073709551616";
    assert!(serde_json::from_str::<Renamed>(number).is_err());
    let value: serde_json::Value = serde_json::from_str(number).unwrap();
    assert!(serde_json::from_value::<Renamed>(value).is_err());
}
#[test]
fn previously_supported_binary_collection_inputs() {
    #[derive(serde::Serialize)]
    struct Source<T> {
        values: Option<T>,
    }
    let bools = vec![true, false];
    let strings = vec!["payload".to_owned()];
    let bool_map: std::collections::BTreeMap<_, _> =
        [("key".to_owned(), true)].into_iter().collect();
    let string_map: std::collections::BTreeMap<_, _> = [("key".to_owned(), "payload".to_owned())]
        .into_iter()
        .collect();
    let bytes = postcard::to_allocvec(&Source {
        values: Some(bools.clone()),
    })
    .unwrap();
    assert_eq!(
        postcard::from_bytes::<RepeatedBool>(&bytes).unwrap().values,
        bools
    );
    let bytes = postcard::to_allocvec(&Source {
        values: Some(strings.clone()),
    })
    .unwrap();
    assert_eq!(
        postcard::from_bytes::<RepeatedString>(&bytes)
            .unwrap()
            .values,
        strings
    );
    let bytes = postcard::to_allocvec(&Source {
        values: Some(bool_map),
    })
    .unwrap();
    assert!(postcard::from_bytes::<BoolMap>(&bytes).unwrap().values["key"]);
    let bytes = postcard::to_allocvec(&Source {
        values: Some(string_map),
    })
    .unwrap();
    assert_eq!(
        postcard::from_bytes::<StringMap>(&bytes).unwrap().values["key"],
        "payload"
    );
}

#[test]
fn yaml_tags_preserve_typed_payloads_and_wrong_kind_privacy() {
    use serde::de::DeserializeOwned;
    fn check<T: DeserializeOwned + Default + PartialEq + core::fmt::Debug>(tagged: &str) {
        for empty in ["", "# comment\n"] {
            assert_eq!(serde_norway::from_str::<T>(empty).unwrap(), T::default());
        }
        assert_eq!(serde_norway::from_str::<T>(tagged).unwrap(), T::default());
        // Explicit empty documents and JSON/YAML null share visit_unit.
        // Accepting those documents would also accept invalid JSON null.
        assert!(serde_norway::from_str::<T>("---\n").is_err());
        assert!(serde_norway::from_str::<T>("null").is_err());
        assert!(serde_json::from_str::<T>("null").is_err());
        let error = serde_norway::from_str::<T>("!Tag 4111111111111111").unwrap_err();
        assert!(!error.to_string().contains("4111111111111111"));
    }
    check::<CustomOneof>("!Tag {}");
    check::<Empty>("!Tag {}");
    check::<Struct>("!Tag {}");
    check::<ListValue>("!Tag []");
    assert_eq!(
        serde_norway::from_str::<Duration>("!Tag '1s'")
            .unwrap()
            .seconds,
        1
    );
    assert_eq!(
        serde_norway::from_str::<Timestamp>("!Tag '1970-01-01T00:00:42Z'")
            .unwrap()
            .seconds,
        42
    );
    assert_eq!(
        serde_norway::from_str::<FieldMask>("!Tag 'alphaBeta'")
            .unwrap()
            .paths,
        ["alpha_beta"]
    );
    assert_eq!(
        serde_norway::from_str::<RepeatedString>("values: !Tag [payload]")
            .unwrap()
            .values,
        ["payload"]
    );
    assert_eq!(
        serde_norway::from_str::<RepeatedBool>("values: !Tag [true]")
            .unwrap()
            .values,
        [true]
    );
    assert_eq!(
        serde_norway::from_str::<StringMap>("values: !Tag {key: payload}")
            .unwrap()
            .values["key"],
        "payload"
    );
    assert!(
        serde_norway::from_str::<BoolMap>("values: !Tag {key: true}")
            .unwrap()
            .values["key"]
    );
}

#[test]
fn reflective_yaml_empty_and_tagged_documents_preserve_json_null_boundary() {
    use buffa_descriptor::{DescriptorPool, DynamicMessageSeed};
    use serde::de::DeserializeSeed;
    use std::sync::Arc;
    let pool = Arc::new(DescriptorPool::decode(include_bytes!("../proto/wkt.fds")).unwrap());
    for name in [
        "reflect.wrongkind.Fields",
        "google.protobuf.Struct",
        "google.protobuf.ListValue",
        "google.protobuf.Empty",
    ] {
        let index = pool.message_index(name).unwrap();
        let expected = DynamicMessageSeed::new(Arc::clone(&pool), index)
            .parse_json(if name == "google.protobuf.ListValue" {
                "[]"
            } else {
                "{}"
            })
            .unwrap();
        for input in [
            "",
            "# comment\n",
            if name == "google.protobuf.ListValue" {
                "!Tag []"
            } else {
                "!Tag {}"
            },
        ] {
            let value = DynamicMessageSeed::new(Arc::clone(&pool), index)
                .deserialize(serde_norway::Deserializer::from_str(input));
            assert_eq!(value.unwrap(), expected, "{name} {input:?}");
        }
        for input in ["---\n", "null", "!Tag null", "!Tag 4111111111111111"] {
            let error = DynamicMessageSeed::new(Arc::clone(&pool), index)
                .deserialize(serde_norway::Deserializer::from_str(input))
                .unwrap_err();
            assert!(!error.to_string().contains("4111111111111111"));
        }
        assert!(DynamicMessageSeed::new(Arc::clone(&pool), index)
            .parse_json("null")
            .is_err());
    }
    // Field containers use the ordinary reflection fixture.
    let pool = Arc::new(DescriptorPool::decode(include_bytes!("../proto/reflect.fds")).unwrap());
    let index = pool.message_index("reflect.test.Containers").unwrap();
    for input in [
        "strings:\n",
        "tags:\n",
        "colors:\n",
        "!Tag {strings: !List [], tags: !Map {}}",
    ] {
        let value = DynamicMessageSeed::new(Arc::clone(&pool), index)
            .deserialize(serde_norway::Deserializer::from_str(input));
        assert!(value.is_ok(), "{input:?}: {value:?}");
    }
}

#[test]
fn tagged_null_cannot_bypass_bool_or_string_presence_and_container_rules() {
    assert!(serde_norway::from_str::<RepeatedBool>("values: [!Tag null]").is_err());
    assert!(serde_norway::from_str::<RepeatedString>("values: [!Tag null]").is_err());
    assert!(serde_norway::from_str::<BoolMap>("values: {key: !Tag null}").is_err());
    assert!(serde_norway::from_str::<StringMap>("values: {key: !Tag null}").is_err());
    assert!(serde_norway::from_str::<Optional>("flag: !Tag null").is_err());
    assert!(serde_norway::from_str::<Optional>("text: !Tag null").is_err());
    assert!(serde_norway::from_str::<CustomOneof>("flag: !Tag null").is_err());
}

#[test]
fn custom_collection_serde_envelopes_and_validation_are_preserved() {
    let json = serde_json::json!({
        "bools":{"values":[true]}, "strings":{"values":["payload"]},
        "boolMap":{"entries":{"key":true}}, "stringMap":{"entries":{"key":"payload"}}
    });
    let value: CustomCollections = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(&*value.bools, &[true]);
    assert_eq!(&*value.strings, &["payload"]);
    assert_eq!(serde_json::to_value(&value).unwrap(), json);
    assert!(serde_json::from_str::<CustomCollections>(r#"{"bools":[]}"#).is_err());
    assert!(serde_json::from_str::<CustomCollections>(r#"{"boolMap":{}}"#).is_err());
    let json =
        serde_json::json!({"bools":{"values":[true]}, "stringMap":{"entries":{"key":"payload"}}});
    let value: CustomCollectionsWithOneof = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(&*value.bools, &[true]);
    assert_eq!(serde_json::to_value(&value).unwrap(), json);
    assert!(serde_json::from_str::<CustomCollectionsWithOneof>(r#"{"bools":[]}"#).is_err());
    assert!(serde_json::from_str::<CustomCollectionsWithOneof>(r#"{"stringMap":{}}"#).is_err());
}

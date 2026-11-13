//! Exercise real generated code: source assertions alone cannot establish that
//! nested, map and oneof fields keep their guarded representation.

use crate::guarded::{
    credentials::Credential, Credentials, CredentialsLazyView, CredentialsOwnedView,
    CredentialsView,
};
use crate::guarded_repr::{SecretBytes, SecretString};
use buffa::{bytes::Buf, DecodeError, DecodeOptions, LazyMessageView, Message, MessageView};
use zeroize::Zeroize;

fn fixture() -> Credentials {
    let mut value = Credentials {
        key: b"private-key".to_vec().into(),
        identity: "identity-secret".to_owned().into(),
        token: Some(b"token".to_vec().into()),
        chain: vec![b"certificate".to_vec().into()],
        labels: vec!["label-secret".to_owned().into()],
        child: Some(Credentials {
            key: b"child-secret".to_vec().into(),
            ..Default::default()
        })
        .into(),
        children: vec![Credentials {
            identity: "nested-secret".to_owned().into(),
            ..Default::default()
        }],
        credential: Some(Credential::Raw(b"oneof-secret".to_vec().into())),
        ..Default::default()
    };
    value
        .keys
        .insert("key-name".to_owned().into(), b"map-secret".to_vec().into());
    value
        .identities
        .insert("name".to_owned().into(), "map-identity".to_owned().into());
    value.members.insert(
        "member".to_owned().into(),
        Credentials {
            key: b"member-secret".to_vec().into(),
            ..Default::default()
        },
    );
    value
}

#[test]
fn owned_binary_and_json_roundtrip() {
    let value = fixture();
    let wire = value.encode_to_vec();
    assert_eq!(Credentials::decode_from_slice(&wire).unwrap(), value);
    let json = serde_json::to_string(&value).unwrap();
    assert!(json.contains("cHJpdmF0ZS1rZXk="));
    assert_eq!(serde_json::from_str::<Credentials>(&json).unwrap(), value);
    let (first, rest) = wire.split_at(3);
    assert_eq!(Credentials::decode(&mut first.chain(rest)).unwrap(), value);
}

#[test]
fn clear_and_zeroize_reset_every_owned_shape() {
    let mut value = fixture();
    value.clear();
    assert_eq!(value, Credentials::default());
    let mut bytes = SecretBytes::from(b"secret".to_vec());
    bytes.zeroize();
    assert!(bytes.is_empty());
    let mut text = SecretString::from("secret".to_owned());
    text.zeroize();
    assert!(text.is_empty());
}

#[test]
fn singular_optional_oneof_and_map_replacements_preserve_wire_semantics() {
    let mut value = fixture();
    let replacement = Credentials {
        key: b"new".to_vec().into(),
        identity: "new-name".to_owned().into(),
        token: Some(b"new-token".to_vec().into()),
        credential: Some(Credential::Text("new-oneof".to_owned().into())),
        ..Default::default()
    };
    value
        .merge_from_slice(&replacement.encode_to_vec())
        .unwrap();
    assert_eq!(&*value.key, b"new");
    assert_eq!(&*value.identity, "new-name");
    assert_eq!(value.credential, replacement.credential);
    let decoded = serde_json::from_str::<Credentials>(
        r#"{"key":"b2xk","key":"bmV3","keys":{"same":"b2xk","same":"bmV3"}}"#,
    )
    .unwrap();
    assert_eq!(&*decoded.key, b"new");
    assert_eq!(&**decoded.keys.values().next().unwrap(), b"new");
}

#[test]
fn debug_redacts_owned_eager_lazy_oneof_and_owned_view_values() {
    let value = fixture();
    let bytes = value.encode_to_bytes();
    let view = CredentialsView::decode_view(&bytes).unwrap();
    let lazy = CredentialsLazyView::decode_lazy(&bytes).unwrap();
    let owned_view = CredentialsOwnedView::decode(bytes.clone()).unwrap();
    for output in [
        format!("{value:?}"),
        format!("{view:?}"),
        format!("{lazy:?}"),
        format!("{owned_view:?}"),
        format!("{:?}", value.credential),
        format!("{:?}", view.credential),
    ] {
        assert!(output.contains("REDACTED"));
        for secret in [
            "private",
            "identity-secret",
            "label-secret",
            "oneof-secret",
            "map-secret",
        ] {
            assert!(!output.contains(secret), "{output}");
        }
        assert!(!output.contains("112, 114, 105"));
    }
    assert_eq!(view.to_owned_message().unwrap(), value);
    assert_eq!(lazy.to_owned_message().unwrap(), value);
    assert_eq!(owned_view.to_owned_message(), value);
}

#[test]
fn rejects_malformed_and_oversized_binary_input() {
    for wire in [
        &[0x0a, 0x05, 0x01][..],
        &[0x12, 0x02, 0xff, 0xfe][..],
        &[
            0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02,
        ][..],
    ] {
        assert!(Credentials::decode_from_slice(wire).is_err());
        let (first, rest) = wire.split_at(2);
        assert!(Credentials::decode(&mut first.chain(rest)).is_err());
    }
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    let options = DecodeOptions::new()
        .with_max_message_size(MAX_BYTES)
        .with_recursion_limit(4)
        .with_unknown_field_limit(0);
    assert!(matches!(
        options.decode_from_slice::<Credentials>(&vec![0; MAX_BYTES + 1]),
        Err(DecodeError::MessageTooLarge)
    ));
    let mut value = Credentials::default();
    for _ in 0..8 {
        value = Credentials {
            child: Some(value).into(),
            ..Default::default()
        };
    }
    assert!(matches!(
        options.decode_from_slice::<Credentials>(&value.encode_to_vec()),
        Err(DecodeError::RecursionLimitExceeded)
    ));
}

#[test]
fn unknown_fields_are_skipped_without_retaining_payloads() {
    let wire = [0xa0, 0x06, 0x01];
    let value = Credentials::decode_from_slice(&wire).unwrap();
    assert_eq!(value.encode_to_vec(), Vec::<u8>::new());
    assert_eq!(
        CredentialsView::decode_view(&wire)
            .unwrap()
            .to_owned_message()
            .unwrap(),
        value
    );
    assert_eq!(
        CredentialsLazyView::decode_lazy(&wire)
            .unwrap()
            .to_owned_message()
            .unwrap(),
        value
    );
}

#[test]
fn malformed_json_cleans_up_earlier_fields_and_container_elements() {
    for input in [
        r#"{"key":"c2VjcmV0","identity":[]}"#,
        r#"{"chain":["c2VjcmV0","c2VjcmV0%%%%"]}"#,
        r#"{"labels":["secret",{}]}"#,
        r#"{"keys":{"secret":"c2VjcmV0","other":"%%%%"}}"#,
        r#"{"raw":"c2VjcmV0","text":"duplicate"}"#,
        r#"{"child":{"key":"c2VjcmV0","identity":[]}}"#,
    ] {
        assert!(
            serde_json::from_str::<Credentials>(input).is_err(),
            "{input}"
        );
    }
    for input in [
        r#"{"key":null,"token":null,"chain":null}"#,
        r#"{"key":"-_8"}"#,
        r#"{"key":"+/8="}"#,
    ] {
        assert!(
            serde_json::from_str::<Credentials>(input).is_ok(),
            "{input}"
        );
    }
}

#[test]
fn string_map_keys_support_borrowed_lookup_and_removal() {
    let mut value = fixture();
    assert!(value.keys.contains_key("key-name"));
    assert_eq!(&**value.keys.get("key-name").unwrap(), b"map-secret");
    assert_eq!(&*value.keys.remove("key-name").unwrap(), b"map-secret");
    assert!(!value.keys.contains_key("key-name"));
}

#[test]
fn cloning_remains_explicit_and_public_fields_keep_their_representation() {
    let value = fixture();
    let copy = value.clone();
    assert_eq!(value, copy);
    assert_ne!(value.key.as_ptr(), copy.key.as_ptr());
    let _: &String = &value.media_type;
}

#[test]
fn custom_bytes_preserve_protobuf_null_boundaries() {
    let value: Credentials =
        serde_json::from_str(r#"{"key":null,"token":null,"raw":null}"#).unwrap();
    assert!(value.key.is_empty());
    assert!(value.token.is_none());
    assert!(value.credential.is_none());
    for input in [r#"{"chain":[null]}"#, r#"{"keys":{"name":null}}"#] {
        assert!(serde_json::from_str::<Credentials>(input).is_err());
    }
}

#[test]
fn derived_json_visitor_uses_guarded_singular_and_optional_bytes() {
    use crate::guarded::DirectCredentials;
    let value = DirectCredentials {
        key: b"key".to_vec().into(),
        token: Some(b"token".to_vec().into()),
        ..Default::default()
    };
    let json = serde_json::to_string(&value).unwrap();
    assert!(json.contains("a2V5"));
    assert!(json.contains("dG9rZW4="));
    assert_eq!(
        serde_json::from_str::<DirectCredentials>(&json).unwrap(),
        value
    );
    let value: DirectCredentials = serde_json::from_str(r#"{"key":null,"token":null}"#).unwrap();
    assert!(value.key.is_empty());
    assert!(value.token.is_none());
    for input in [r#"{"key":"%%%%"}"#, r#"{"token":"%%%%"}"#] {
        assert!(serde_json::from_str::<DirectCredentials>(input).is_err());
    }
}

/// A fully populated `Sealed`, one value in every field shape.
fn sealed() -> crate::guarded::Sealed {
    let mut value = crate::guarded::Sealed {
        one: b"one".to_vec().into(),
        maybe: Some(b"maybe".to_vec().into()),
        many: vec![b"many".to_vec().into()],
        ..Default::default()
    };
    value
        .by_name
        .insert("name".to_owned(), b"by-name".to_vec().into());
    value
}

#[test]
fn json_policy_governs_owned_eager_lazy_and_owned_view_export() {
    use crate::guarded::{sealed::Choice, Sealed, SealedLazyView, SealedOwnedView, SealedView};

    let populated = [
        Sealed {
            one: b"one".to_vec().into(),
            ..Default::default()
        },
        Sealed {
            maybe: Some(b"maybe".to_vec().into()),
            ..Default::default()
        },
        Sealed {
            many: vec![b"many".to_vec().into()],
            ..Default::default()
        },
        {
            let mut value = Sealed::default();
            value
                .by_name
                .insert("name".to_owned(), b"by-name".to_vec().into());
            value
        },
        Sealed {
            choice: Some(Choice::Chosen(b"chosen".to_vec().into())),
            ..Default::default()
        },
        sealed(),
    ];
    for value in populated {
        let bytes = value.encode_to_bytes();
        let view = SealedView::decode_view(&bytes).unwrap();
        let lazy = SealedLazyView::decode_lazy(&bytes).unwrap();
        let owned_view = SealedOwnedView::decode(bytes.clone()).unwrap();
        for result in [
            serde_json::to_string(&value),
            serde_json::to_string(&view),
            serde_json::to_string(&lazy),
            serde_json::to_string(&owned_view),
        ] {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("sealed field cannot be exported"), "{error}");
        }
    }

    // Nothing to export: the policy is never asked.
    let empty = Sealed::default();
    let bytes = empty.encode_to_bytes();
    assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
    assert_eq!(
        serde_json::to_string(&SealedView::decode_view(&bytes).unwrap()).unwrap(),
        "{}"
    );
}

#[test]
fn json_policy_still_accepts_import_of_sealed_fields() {
    let value: crate::guarded::Sealed = serde_json::from_str(
        r#"{"one":"b25l","maybe":"bWF5YmU=","many":["bWFueQ=="],"byName":{"name":"YnktbmFtZQ=="}}"#,
    )
    .unwrap();
    assert_eq!(value, sealed());
    let null_fields: crate::guarded::Sealed =
        serde_json::from_str(r#"{"one":null,"maybe":null,"chosen":null,"many":null}"#).unwrap();
    assert_eq!(null_fields, crate::guarded::Sealed::default());
    for input in [r#"{"many":[null]}"#, r#"{"byName":{"name":null}}"#] {
        assert!(serde_json::from_str::<crate::guarded::Sealed>(input).is_err());
    }
}

#[test]
fn views_serialize_policy_fields_exactly_like_the_owned_message() {
    let value = fixture();
    let bytes = value.encode_to_bytes();
    let owned = serde_json::to_value(&value).unwrap();
    let view = CredentialsView::decode_view(&bytes).unwrap();
    let lazy = CredentialsLazyView::decode_lazy(&bytes).unwrap();
    let owned_view = CredentialsOwnedView::decode(bytes.clone()).unwrap();
    assert_eq!(serde_json::to_value(&view).unwrap(), owned);
    assert_eq!(serde_json::to_value(&lazy).unwrap(), owned);
    assert_eq!(serde_json::to_value(&owned_view).unwrap(), owned);
}

#[test]
fn string_policy_handles_null_and_rejects_null_elements() {
    let value: Credentials =
        serde_json::from_str(r#"{"identity":null,"text":null,"labels":null}"#).unwrap();
    assert!(value.identity.is_empty());
    assert!(value.credential.is_none());
    assert!(value.labels.is_empty());
    for input in [
        r#"{"labels":[null]}"#,
        r#"{"identities":{"name":null}}"#,
        r#"{"identity":7}"#,
    ] {
        assert!(
            serde_json::from_str::<Credentials>(input).is_err(),
            "{input}"
        );
    }
}

#[test]
fn guarded_repeated_floats_roundtrip_and_clear() {
    let mut value = Credentials::default();
    for index in 0..9u8 {
        buffa::ProtoList::push(&mut value.embedding, f32::from(index) / 2.0);
    }
    let wire = value.encode_to_vec();
    let decoded = Credentials::decode_from_slice(&wire).unwrap();
    assert_eq!(&*decoded.embedding, &*value.embedding);
    let (first, rest) = wire.split_at(4);
    assert_eq!(
        &*Credentials::decode(&mut first.chain(rest))
            .unwrap()
            .embedding,
        &*value.embedding
    );
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        &*serde_json::from_str::<Credentials>(&json)
            .unwrap()
            .embedding,
        &*value.embedding
    );
    assert!(!format!("{value:?}").contains("4.5"));
    buffa::ProtoList::clear(&mut value.embedding);
    assert!(value.embedding.is_empty());
}

//! A JSON policy type used in `repeated` and `map` value positions without any
//! trait impl in the code that uses it.
//!
//! `secret` stands in for a shared crate that defines the policy type and
//! implements buffa's traits for it, as only that crate can. `generated`
//! stands in for generated code in another crate: under the orphan rule it may
//! not implement a buffa trait for `Secret`, so it uses only buffa's
//! with-modules, and the test checks that `Secret` has no `ProtoElemJson` impl
//! for them to lean on.

#![cfg(feature = "json")]

mod secret {
    use buffa::json_helpers::{bytes, ProtoBytesJson};
    use buffa::{DecodeError, ProtoBytes, WirePayload};
    use serde::{Deserializer, Serializer};

    /// Bytes that export as standard base64 behind a `sealed:` prefix.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Secret(pub Vec<u8>);

    impl From<Vec<u8>> for Secret {
        fn from(value: Vec<u8>) -> Self {
            Self(value)
        }
    }
    impl core::ops::Deref for Secret {
        type Target = [u8];
        fn deref(&self) -> &[u8] {
            &self.0
        }
    }
    impl AsRef<[u8]> for Secret {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }
    impl ProtoBytes for Secret {
        fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
            Ok(Self(payload.as_slice().to_vec()))
        }
    }
    impl ProtoBytesJson for Secret {
        fn serialize_json<S: Serializer>(value: &[u8], s: S) -> Result<S::Ok, S::Error> {
            let encoded = base64::display::Base64Display::new(
                value,
                &base64::engine::general_purpose::STANDARD,
            );
            s.collect_str(&format_args!("sealed:{encoded}"))
        }
        fn deserialize_json<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let text = <String as serde::Deserialize>::deserialize(d)?;
            let encoded = text
                .strip_prefix("sealed:")
                .ok_or_else(|| serde::de::Error::custom("missing sealed: prefix"))?;
            let mut out = vec![0; bytes::max_decoded_len(encoded)];
            let len = bytes::decode_into::<D::Error>(encoded, &mut out)?;
            out.truncate(len);
            Ok(Self(out))
        }
    }
}

mod generated {
    use super::secret::Secret;
    // Match generated code's map type even when the runtime's `std` feature
    // is off; the test executable's own use of `std` does not enable it.
    use buffa::Map;
    use std::collections::BTreeMap;

    #[derive(serde::Serialize, serde::Deserialize, Debug, Default, PartialEq)]
    pub struct Message {
        #[serde(with = "::buffa::json_helpers::repeated_bytes_policy", default)]
        pub list: Vec<Secret>,
        #[serde(with = "::buffa::json_helpers::map_bytes_policy", default)]
        pub by_name: BTreeMap<String, Secret>,
        #[serde(with = "::buffa::json_helpers::map_bytes_policy", default)]
        pub by_id: Map<u32, Secret>,
        #[serde(with = "::buffa::json_helpers::map_bytes_policy_serde_key", default)]
        pub by_tag: Map<Tag, Secret>,
    }

    /// A custom string key type without a JSON policy, also from another
    /// crate, encoded through its own serde impls.
    #[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
    #[serde(transparent)]
    pub struct Tag(pub String);
}

use generated::Message;
use secret::Secret;

/// `<Probe<T>>::IS_ELEM` is the inherent `true` only when `T: ProtoElemJson`;
/// otherwise it resolves to the trait's `false`.
struct Probe<T>(core::marker::PhantomData<T>);
trait NotElem {
    const IS_ELEM: bool = false;
}
impl<T> NotElem for Probe<T> {}
impl<T: buffa::json_helpers::ProtoElemJson> Probe<T> {
    #[allow(dead_code)]
    const IS_ELEM: bool = true;
}

// Checked at compile time: the probe detects an impl, and `Secret` has none.
const _: () = {
    assert!(<Probe<Vec<u8>>>::IS_ELEM);
    assert!(!<Probe<Secret>>::IS_ELEM);
};

#[test]
fn repeated_and_map_values_round_trip_through_a_foreign_policy() {
    let mut message = Message {
        list: vec![Secret(vec![1, 2, 3]), Secret(vec![])],
        ..Default::default()
    };
    message.by_name.insert("a".into(), Secret(vec![0xff]));
    message.by_id.insert(9, Secret(vec![0xfb]));
    message
        .by_tag
        .insert(generated::Tag("t".into()), Secret(vec![0]));
    let json = serde_json::to_string(&message).unwrap();
    assert_eq!(
        json,
        concat!(
            r#"{"list":["sealed:AQID","sealed:"],"by_name":{"a":"sealed:/w=="},"#,
            r#""by_id":{"9":"sealed:+w=="},"by_tag":{"t":"sealed:AA=="}}"#
        )
    );
    assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), message);
    // The policy's own import accepts the URL-safe alphabet through decode_into.
    let url_safe: Message = serde_json::from_str(r#"{"list":["sealed:-w"]}"#).unwrap();
    assert_eq!(url_safe.list, [Secret(vec![0xfb])]);
}

#[test]
fn null_elements_and_values_are_rejected_before_the_policy() {
    for json in [
        r#"{"list":[null]}"#,
        r#"{"by_name":{"a":null}}"#,
        r#"{"by_id":{"1":null}}"#,
        r#"{"by_tag":{"t":null}}"#,
    ] {
        let err = serde_json::from_str::<Message>(json).unwrap_err();
        assert!(
            err.to_string().contains("null is not a valid"),
            "{json}: {err}"
        );
    }
    let empty: Message =
        serde_json::from_str(r#"{"list":null,"by_name":null,"by_id":null,"by_tag":null}"#).unwrap();
    assert_eq!(empty, Message::default());
    assert!(serde_json::from_str::<Message>(r#"{"list":["AQID"]}"#).is_err());
}

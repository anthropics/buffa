//! Generated-code plumbing for `ProtoBytesJson` / `ProtoStringJson`.

use super::*;
use crate::{DecodeError, ProtoBytes, ProtoString, WirePayload};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Bytes whose policy hex-encodes, distinguishable from buffa's base64.
#[derive(Clone, Debug, Default, PartialEq)]
struct Hex(Vec<u8>);
impl From<Vec<u8>> for Hex {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for Hex {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<[u8]> for Hex {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl ProtoBytes for Hex {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(Self(payload.as_slice().to_vec()))
    }
}
impl ProtoBytesJson for Hex {
    fn serialize_json<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let text: String = bytes.iter().map(|b| alloc::format!("{b:02x}")).collect();
        s.serialize_str(&text)
    }
    fn deserialize_json<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        (0..text.len())
            .step_by(2)
            .map(|i| {
                text.get(i..i + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or_else(|| serde::de::Error::custom("invalid hex"))
            })
            .collect::<Result<Vec<u8>, D::Error>>()
            .map(Self)
    }
}

/// Text whose policy upper-cases on export, distinguishable from a plain string.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Shout(String);
impl From<String> for Shout {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for Shout {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Shout {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl ProtoString for Shout {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(|text| Self(text.to_string()))
    }
}
impl ProtoStringJson for Shout {
    fn serialize_json<S: Serializer>(text: &str, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&text.to_uppercase())
    }
    fn deserialize_json<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|text| Self(text.to_lowercase()))
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, PartialEq)]
struct Fields {
    #[serde(with = "bytes_policy", default)]
    bytes: Hex,
    #[serde(with = "opt_bytes_policy", default)]
    opt_bytes: Option<Hex>,
    #[serde(with = "string_policy", default)]
    text: Shout,
    #[serde(with = "opt_string_policy", default)]
    opt_text: Option<Shout>,
    #[serde(with = "policy_str_key_map", default)]
    keyed: BTreeMap<Shout, i32>,
}

#[test]
fn with_modules_route_every_shape_through_the_policy() {
    let mut value = Fields {
        bytes: Hex(alloc::vec![0xab, 0x01]),
        opt_bytes: Some(Hex(alloc::vec![0x02])),
        text: Shout("quiet".to_string()),
        opt_text: Some(Shout("hush".to_string())),
        ..Default::default()
    };
    value.keyed.insert(Shout("key".to_string()), 7);
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        json,
        r#"{"bytes":"ab01","opt_bytes":"02","text":"QUIET","opt_text":"HUSH","keyed":{"KEY":7}}"#
    );
    assert_eq!(serde_json::from_str::<Fields>(&json).unwrap(), value);
}

#[test]
fn null_is_handled_before_the_policy_runs() {
    let value: Fields = serde_json::from_str(
        r#"{"bytes":null,"opt_bytes":null,"text":null,"opt_text":null,"keyed":null}"#,
    )
    .unwrap();
    assert_eq!(value, Fields::default());
    assert!(serde_json::from_str::<Fields>(r#"{"keyed":{"k":null}}"#).is_err());
    assert!(serde_json::from_str::<Fields>(r#"{"bytes":"zz"}"#).is_err());
}

#[test]
fn adapters_and_seeds_use_the_policy() {
    assert_eq!(
        serde_json::to_string(&BytesPolicyJson::<Hex>::new(&[0x0f])).unwrap(),
        r#""0f""#
    );
    assert_eq!(
        serde_json::to_string(&StringPolicyJson::<Shout>::new("a")).unwrap(),
        r#""A""#
    );
    let elements: [&[u8]; 2] = [&[0x01], &[0x02]];
    assert_eq!(
        serde_json::to_string(&BytesPolicySeqJson::<Hex>::new(&elements)).unwrap(),
        r#"["01","02"]"#
    );
    let texts = ["x", "y"];
    assert_eq!(
        serde_json::to_string(&StringPolicySeqJson::<Shout>::new(&texts)).unwrap(),
        r#"["X","Y"]"#
    );
    let mut de = serde_json::Deserializer::from_str(r#""ff""#);
    assert_eq!(
        BytesPolicySeed::<Hex>::default()
            .deserialize(&mut de)
            .unwrap(),
        Hex(alloc::vec![0xff])
    );
    let mut de = serde_json::Deserializer::from_str(r#""LOUD""#);
    assert_eq!(
        StringPolicySeed::<Shout>::new()
            .deserialize(&mut de)
            .unwrap(),
        Shout("loud".to_string())
    );
}

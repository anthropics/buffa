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

/// Text whose policy marks the export with a `~` prefix that import requires
/// and strips: distinguishable from a plain string, and injective and
/// round-tripping, as a map-key policy must be.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Tilde(String);
impl From<String> for Tilde {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for Tilde {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Tilde {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl ProtoString for Tilde {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(|text| Self(text.to_string()))
    }
}
impl ProtoStringJson for Tilde {
    fn serialize_json<S: Serializer>(text: &str, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&format_args!("~{text}"))
    }
    fn deserialize_json<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.strip_prefix('~')
            .map(|rest| Self(rest.to_string()))
            .ok_or_else(|| serde::de::Error::custom("missing ~ prefix"))
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, PartialEq)]
struct Fields {
    #[serde(with = "bytes_policy", default)]
    bytes: Hex,
    #[serde(with = "opt_bytes_policy", default)]
    opt_bytes: Option<Hex>,
    #[serde(with = "string_policy", default)]
    text: Tilde,
    #[serde(with = "opt_string_policy", default)]
    opt_text: Option<Tilde>,
    #[serde(with = "policy_str_key_map", default)]
    keyed: BTreeMap<Tilde, i32>,
}

#[test]
fn with_modules_route_every_shape_through_the_policy() {
    let mut value = Fields {
        bytes: Hex(alloc::vec![0xab, 0x01]),
        opt_bytes: Some(Hex(alloc::vec![0x02])),
        text: Tilde("quiet".to_string()),
        opt_text: Some(Tilde("hush".to_string())),
        ..Default::default()
    };
    value.keyed.insert(Tilde("key".to_string()), 7);
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        json,
        r#"{"bytes":"ab01","opt_bytes":"02","text":"~quiet","opt_text":"~hush","keyed":{"~key":7}}"#
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
    let err = serde_json::from_str::<Fields>(r#"{"keyed":{"~k":null}}"#).unwrap_err();
    assert!(err.to_string().contains("null is not a valid"), "{err}");
    assert!(serde_json::from_str::<Fields>(r#"{"bytes":"zz"}"#).is_err());
}

#[test]
fn adapters_and_seeds_use_the_policy() {
    assert_eq!(
        serde_json::to_string(&BytesPolicyJson::<Hex>::new(&[0x0f])).unwrap(),
        r#""0f""#
    );
    assert_eq!(
        serde_json::to_string(&StringPolicyJson::<Tilde>::new("a")).unwrap(),
        r#""~a""#
    );
    let elements: [&[u8]; 2] = [&[0x01], &[0x02]];
    assert_eq!(
        serde_json::to_string(&BytesPolicySeqJson::<Hex>::new(&elements)).unwrap(),
        r#"["01","02"]"#
    );
    let texts = ["x", "y"];
    assert_eq!(
        serde_json::to_string(&StringPolicySeqJson::<Tilde>::new(&texts)).unwrap(),
        r#"["~x","~y"]"#
    );
    let mut de = serde_json::Deserializer::from_str(r#""ff""#);
    assert_eq!(
        BytesPolicySeed::<Hex>::default()
            .deserialize(&mut de)
            .unwrap(),
        Hex(alloc::vec![0xff])
    );
    let mut de = serde_json::Deserializer::from_str(r#""~loud""#);
    assert_eq!(
        StringPolicySeed::<Tilde>::new()
            .deserialize(&mut de)
            .unwrap(),
        Tilde("loud".to_string())
    );
}

/// A custom string key without a JSON policy, serialized through serde.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
struct Name(String);

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, PartialEq)]
struct Containers {
    #[serde(with = "repeated_bytes_policy", default)]
    hexes: Vec<Hex>,
    #[serde(with = "repeated_string_policy", default)]
    texts: Vec<Tilde>,
    #[serde(with = "map_bytes_policy", default)]
    by_int: BTreeMap<i64, Hex>,
    #[serde(with = "map_bytes_policy", default)]
    by_u32: crate::__private::HashMap<u32, Hex>,
    #[serde(with = "map_bytes_policy", default)]
    by_bool: BTreeMap<bool, Hex>,
    #[serde(with = "map_string_policy", default)]
    by_string: BTreeMap<String, Tilde>,
    #[serde(with = "map_bytes_policy_serde_key", default)]
    by_name: BTreeMap<Name, Hex>,
    #[serde(with = "map_string_policy_policy_key", default)]
    by_policy_key: BTreeMap<Tilde, Tilde>,
    #[serde(with = "map_bytes_policy_bytes_key", default)]
    by_bytes: BTreeMap<Vec<u8>, Hex>,
}

fn sample_containers() -> Containers {
    let mut value = Containers {
        hexes: alloc::vec![Hex(alloc::vec![0x0a]), Hex(alloc::vec![])],
        texts: alloc::vec![Tilde("one".to_string())],
        ..Default::default()
    };
    value.by_int.insert(-7, Hex(alloc::vec![0xbe, 0xef]));
    value.by_u32.insert(u32::MAX, Hex(alloc::vec![0x03]));
    value.by_bool.insert(true, Hex(alloc::vec![0x01]));
    value
        .by_string
        .insert("Plain".to_string(), Tilde("v".to_string()));
    value
        .by_name
        .insert(Name("Mixed".to_string()), Hex(alloc::vec![0x04]));
    value
        .by_policy_key
        .insert(Tilde("k".to_string()), Tilde("w".to_string()));
    value
        .by_bytes
        .insert(alloc::vec![0xff], Hex(alloc::vec![0x02]));
    value
}

#[test]
fn container_modules_route_elements_and_map_values_through_the_policy() {
    let value = sample_containers();
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        json,
        concat!(
            r#"{"hexes":["0a",""],"texts":["~one"],"by_int":{"-7":"beef"},"#,
            r#""by_u32":{"4294967295":"03"},"by_bool":{"true":"01"},"#,
            r#""by_string":{"Plain":"~v"},"by_name":{"Mixed":"04"},"#,
            r#""by_policy_key":{"~k":"~w"},"by_bytes":{"/w==":"02"}}"#
        )
    );
    assert_eq!(serde_json::from_str::<Containers>(&json).unwrap(), value);
    // `serde_json::Value` has its own map-key serializer and deserializer.
    let tree = serde_json::to_value(&value).unwrap();
    assert_eq!(
        tree,
        serde_json::from_str::<serde_json::Value>(&json).unwrap()
    );
    assert_eq!(serde_json::from_value::<Containers>(tree).unwrap(), value);
}

#[test]
fn container_modules_reject_null_elements_and_accept_null_containers() {
    let empty: Containers = serde_json::from_str(concat!(
        r#"{"hexes":null,"texts":null,"by_int":null,"by_u32":null,"by_bool":null,"#,
        r#""by_string":null,"by_name":null,"by_policy_key":null,"by_bytes":null}"#
    ))
    .unwrap();
    assert_eq!(empty, Containers::default());
    for json in [
        r#"{"hexes":["0a",null]}"#,
        r#"{"texts":[null]}"#,
        r#"{"by_int":{"1":null}}"#,
        r#"{"by_bool":{"false":null}}"#,
        r#"{"by_string":{"k":null}}"#,
        r#"{"by_name":{"k":null}}"#,
        r#"{"by_policy_key":{"~k":null}}"#,
        r#"{"by_bytes":{"AA==":null}}"#,
    ] {
        let err = serde_json::from_str::<Containers>(json).unwrap_err();
        assert!(
            err.to_string().contains("null is not a valid"),
            "{json}: {err}"
        );
    }
    assert!(serde_json::from_str::<Containers>(r#"{"hexes":["zz"]}"#).is_err());
    assert!(serde_json::from_str::<Containers>(r#"{"by_u32":{"4294967296":"00"}}"#).is_err());
}

#[test]
fn container_map_keys_are_not_echoed_in_errors() {
    for (json, message) in [
        (r#"{"by_int":{"secret":"00"}}"#, "invalid map key at"),
        (r#"{"by_bool":{"secret":"00"}}"#, "invalid map key at"),
        (
            r#"{"by_bytes":{"secret!":"00"}}"#,
            "invalid base64 map key at",
        ),
    ] {
        let text = serde_json::from_str::<Containers>(json)
            .unwrap_err()
            .to_string();
        assert!(text.starts_with(message), "{json}: {text}");
        assert!(!text.contains("secret"), "{json}: {text}");
    }
}

/// A user-supplied parser can put its rejected key in its error's Display
/// output. The generic map helper must not forward that output to JSON errors.
#[test]
fn container_map_keys_do_not_forward_custom_parser_errors() {
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct RejectedKey;

    #[derive(Debug, thiserror::Error)]
    enum KeyErrorReason {
        #[error("private-map-key")]
        Rejected,
    }

    impl core::fmt::Display for RejectedKey {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("unused")
        }
    }

    impl core::str::FromStr for RejectedKey {
        type Err = KeyErrorReason;

        fn from_str(_: &str) -> Result<Self, Self::Err> {
            Err(KeyErrorReason::Rejected)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(r#"{"private-map-key":"00"}"#);
    let error = map_bytes_policy::deserialize::<BTreeMap<RejectedKey, Hex>, _>(&mut deserializer)
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("invalid map key"), "{error}");
    assert!(!error.contains("private-map-key"), "{error}");
}

#[test]
fn container_maps_keep_the_last_duplicate_key() {
    let value: Containers = serde_json::from_str(r#"{"by_int":{"1":"01","1":"02"}}"#).unwrap();
    assert_eq!(value.by_int.get(&1), Some(&Hex(alloc::vec![0x02])));
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, PartialEq)]
struct OtherKeyShapes {
    #[serde(with = "map_bytes_policy_policy_key", default)]
    bytes_by_policy_key: BTreeMap<Tilde, Hex>,
    #[serde(with = "map_string_policy_serde_key", default)]
    string_by_name: BTreeMap<Name, Tilde>,
    #[serde(with = "map_string_policy_bytes_key", default)]
    string_by_bytes: BTreeMap<::bytes::Bytes, Tilde>,
}

#[test]
fn remaining_key_shapes_round_trip() {
    let mut value = OtherKeyShapes::default();
    value
        .bytes_by_policy_key
        .insert(Tilde("p".to_string()), Hex(alloc::vec![0x05]));
    value
        .string_by_name
        .insert(Name("n".to_string()), Tilde("x".to_string()));
    value
        .string_by_bytes
        .insert(::bytes::Bytes::from_static(&[0xfb]), Tilde("y".to_string()));
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        json,
        r#"{"bytes_by_policy_key":{"~p":"05"},"string_by_name":{"n":"~x"},"string_by_bytes":{"+w==":"~y"}}"#
    );
    assert_eq!(
        serde_json::from_str::<OtherKeyShapes>(&json).unwrap(),
        value
    );
}

/// The policy-key path carries edge-case keys through intact, with the
/// fixture's injective, round-tripping key policy: keys that differ only in
/// case or in the policy's own marker stay distinct.
#[test]
fn policy_key_path_preserves_distinct_keys() {
    let keys = ["", "a", "A", "~", "~a", "a~", "\u{e9}", "line\nbreak"];
    let value = Fields {
        keyed: (0..)
            .zip(keys)
            .map(|(i, k)| (Tilde(k.to_string()), i))
            .collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&value).unwrap();
    let parsed = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    assert_eq!(
        parsed["keyed"].as_object().unwrap().len(),
        keys.len(),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<Fields>(&json).unwrap(), value);
}

#[test]
fn policy_key_errors_propagate_and_duplicates_keep_the_last_entry() {
    for json in [r#"{"keyed":{"k":1}}"#, r#"{"text":"plain"}"#] {
        let err = serde_json::from_str::<Fields>(json).unwrap_err();
        assert!(
            err.to_string().contains("missing ~ prefix"),
            "{json}: {err}"
        );
    }
    let value: Fields = serde_json::from_str(r#"{"keyed":{"~a":1,"~a":2}}"#).unwrap();
    assert_eq!(value.keyed.len(), 1);
    assert_eq!(value.keyed.get(&Tilde("a".to_string())), Some(&2));
}

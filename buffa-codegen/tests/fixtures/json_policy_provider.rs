//! Non-sensitive representations defined outside the generated consumer crate.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use buffa::json_helpers::{ProtoBytesJson, ProtoStringJson};
use buffa::{DecodeError, ProtoBytes, ProtoString, WirePayload};
use serde::{Deserialize, Deserializer, Serializer};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blob(pub Vec<u8>);
pub type SizedBlob<const N: usize> = Blob;
impl From<Vec<u8>> for Blob {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for Blob {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<[u8]> for Blob {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl ProtoBytes for Blob {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(Self(payload.as_slice().to_vec()))
    }
}
impl ProtoBytesJson for Blob {
    fn serialize_json<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        if bytes == [0xff] {
            return Err(serde::ser::Error::custom("export refused"));
        }
        buffa::json_helpers::bytes::serialize(bytes, serializer)
    }
    fn deserialize_json<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        buffa::json_helpers::bytes::deserialize(deserializer)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text(pub String);
impl From<String> for Text {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl ProtoString for Text {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(|text| Self(text.to_string()))
    }
}
impl ProtoStringJson for Text {
    fn serialize_json<S: Serializer>(text: &str, serializer: S) -> Result<S::Ok, S::Error> {
        if text == "deny" {
            return Err(serde::ser::Error::custom("export refused"));
        }
        serializer.collect_str(&format_args!("~{text}"))
    }
    fn deserialize_json<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.strip_prefix('~')
            .map(|text| Self(text.to_string()))
            .ok_or_else(|| serde::de::Error::custom("missing text marker"))
    }
}

// This key deliberately has native serde and no policy, unlike Text.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Tag(pub String);
impl From<String> for Tag {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl core::ops::Deref for Tag {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for Tag {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl ProtoString for Tag {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(|text| Self(text.to_string()))
    }
}

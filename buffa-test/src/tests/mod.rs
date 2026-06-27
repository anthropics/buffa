//! Integration tests grouped by proto file / feature area.
//!
//! All sub-modules share the generated code from `crate::basic`, `crate::proto2`,
//! etc. (included at the crate root via `include!`) and the helpers below.

#![allow(clippy::module_inception)]

use buffa::Message;

/// Round-trip encode → decode helper used across test modules.
pub(super) fn round_trip<M: Message>(msg: &M) -> M {
    let bytes = msg.encode_to_vec();
    M::decode(&mut bytes.as_slice()).expect("decode failed")
}

/// Encode field `num` as a varint with value `v` — used by closed-enum
/// unknown-value tests that need hand-built wire bytes.
pub(super) fn varint_field(num: u32, v: u64) -> Vec<u8> {
    use buffa::encoding::{encode_varint, Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(num, WireType::Varint).encode(&mut wire);
    encode_varint(v, &mut wire);
    wire
}

mod any_type_url;
mod arbitrary_bytes;
mod basic;
mod box_type;
mod bytes_type;
mod closed_enum;
mod collision;
mod cross_ref;
mod debug_redact;
mod edge_cases;
#[cfg(has_edition_2024)]
mod editions_2024;
mod editions_enum_json;
mod extensions;
mod extensions_json;
mod idiomatic_imports;
mod inline_field;
mod json;
mod keyword;
mod lazy_views;
mod map_type;
mod map_type_custom;
mod message_set;
mod mod_collision;
mod nesting;
mod nestpkg;
mod owned_view;
mod proto2;
mod proto3_semantics;
mod repeated_type;
mod string_type;
mod textproto;
mod type_prefix;
mod unbox_oneof;
mod utf8_validation;
mod view;
mod view_json;
mod with_setters;
mod wkt;

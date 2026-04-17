//! Unit tests for the codegen crate, organized by feature area.
//!
//! Shared descriptor-construction helpers live here; section-specific
//! helpers (proto2_file, json_config) live in their respective modules.

use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, MessageOptions, OneofDescriptorProto,
};
use crate::*;

pub(super) fn proto3_file(name: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.to_string()),
        syntax: Some("proto3".to_string()),
        ..Default::default()
    }
}

pub(super) fn enum_value(name: &str, number: i32) -> EnumValueDescriptorProto {
    EnumValueDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        ..Default::default()
    }
}

pub(super) fn make_field(name: &str, number: i32, label: Label, ty: Type) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(label),
        r#type: Some(ty),
        ..Default::default()
    }
}

/// Concatenate every sibling file's content for search-based assertions.
/// Under PR 1 each proto produces three files (owned, view, ext); tests
/// that look for a generated item need to search across all of them
/// since the content may live in any of the three streams.
pub(super) fn all_content(files: &[crate::GeneratedFile]) -> String {
    files.iter().map(|f| f.content.as_str()).collect()
}

mod comments;
mod custom_attributes;
mod generation;
mod json_codegen;
mod naming;
mod proto2;
mod view_codegen;

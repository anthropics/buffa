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

/// Concatenate all generated-file contents for snapshot-style assertions.
///
/// Each proto now emits 5 content files + 1 `.mod.rs`; tests that assert
/// "the output contains substring X" don't care which file it lands in.
pub(super) fn joined(files: &[GeneratedFile]) -> String {
    files
        .iter()
        .map(|f| f.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

mod comments;
mod custom_attributes;
mod generation;
mod json_codegen;
mod naming;
mod proto2;
mod view_codegen;

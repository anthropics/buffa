//! Inferring a field's `type` from its `type_name`.
//!
//! `FieldDescriptorProto.type` "need not be set" when `type_name` is
//! (`descriptor.proto`). protoc always sets it, but a descriptor set built by
//! another tool can leave it out, and every codegen path reads an unset
//! `type` as the enum's default, `TYPE_DOUBLE`. [`generate`](crate::generate)
//! fills it in once, before anything reads it, from the message or enum that
//! `type_name` names, as protoc's `CrossLinkField` and `DescriptorPool` do.

use std::collections::HashMap;

use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, FileDescriptorProto};
use crate::CodeGenError;

/// Returns a copy of `files` in which every field and extension with
/// `type_name` set and `type` unset has `TYPE_MESSAGE` or `TYPE_ENUM`, or
/// `None` when no field needs it, so the usual protoc output is not copied.
///
/// `type_name` is matched as a fully qualified name with its leading dot,
/// as codegen resolves every type reference. An empty `type_name` counts as
/// unset, as `DescriptorPool` treats it.
///
/// # Errors
///
/// Returns [`CodeGenError::Other`] naming the field when its `type_name`
/// names no message or enum in `files`.
pub(crate) fn infer_field_types(
    files: &[FileDescriptorProto],
) -> Result<Option<Vec<FileDescriptorProto>>, CodeGenError> {
    if !files.iter().any(|file| {
        file.extension.iter().any(is_untyped) || file.message_type.iter().any(needs_inference)
    }) {
        return Ok(None);
    }

    let mut kinds = HashMap::new();
    for file in files {
        let scope = package_scope(file);
        for message in &file.message_type {
            collect_kinds(&scope, message, &mut kinds);
        }
        for enumeration in &file.enum_type {
            let name = enumeration.name.as_deref().unwrap_or_default();
            kinds.insert(format!("{scope}.{name}"), Type::TYPE_ENUM);
        }
    }

    let mut files = files.to_vec();
    for file in &mut files {
        let scope = package_scope(file);
        infer(&scope, &mut file.extension, &kinds)?;
        for message in &mut file.message_type {
            infer_message(&scope, message, &kinds)?;
        }
    }
    Ok(Some(files))
}

fn is_untyped(field: &FieldDescriptorProto) -> bool {
    field.r#type.is_none()
        && field
            .type_name
            .as_deref()
            .is_some_and(|name| !name.is_empty())
}

fn needs_inference(message: &DescriptorProto) -> bool {
    message
        .field
        .iter()
        .chain(&message.extension)
        .any(is_untyped)
        || message.nested_type.iter().any(needs_inference)
}

/// `.pkg`, or the empty string for a file without a package.
fn package_scope(file: &FileDescriptorProto) -> String {
    match file.package.as_deref() {
        Some(package) if !package.is_empty() => format!(".{package}"),
        _ => String::new(),
    }
}

fn collect_kinds(scope: &str, message: &DescriptorProto, kinds: &mut HashMap<String, Type>) {
    let fqn = format!("{scope}.{}", message.name.as_deref().unwrap_or_default());
    for enumeration in &message.enum_type {
        let name = enumeration.name.as_deref().unwrap_or_default();
        kinds.insert(format!("{fqn}.{name}"), Type::TYPE_ENUM);
    }
    for nested in &message.nested_type {
        collect_kinds(&fqn, nested, kinds);
    }
    kinds.insert(fqn, Type::TYPE_MESSAGE);
}

fn infer_message(
    scope: &str,
    message: &mut DescriptorProto,
    kinds: &HashMap<String, Type>,
) -> Result<(), CodeGenError> {
    let fqn = format!("{scope}.{}", message.name.as_deref().unwrap_or_default());
    infer(&fqn, &mut message.field, kinds)?;
    infer(&fqn, &mut message.extension, kinds)?;
    for nested in &mut message.nested_type {
        infer_message(&fqn, nested, kinds)?;
    }
    Ok(())
}

fn infer(
    scope: &str,
    fields: &mut [FieldDescriptorProto],
    kinds: &HashMap<String, Type>,
) -> Result<(), CodeGenError> {
    for field in fields.iter_mut().filter(|field| is_untyped(field)) {
        let type_name = field.type_name.as_deref().unwrap_or_default();
        let Some(&kind) = kinds.get(type_name) else {
            let scope = scope.strip_prefix('.').unwrap_or(scope);
            let name = field.name.as_deref().unwrap_or_default();
            let field_fqn = if scope.is_empty() {
                name.to_string()
            } else {
                format!("{scope}.{name}")
            };
            let reason = if type_name.starts_with('.') {
                "names no message or enum in the descriptor set; the set must \
                 contain the file that declares it (protoc: --include_imports)"
            } else {
                "does not start with '.'; codegen needs the fully qualified name with \
                 its leading dot, as protoc writes it"
            };
            return Err(CodeGenError::Other(format!(
                "field '{field_fqn}' has type_name '{type_name}' but no type, and \
                 '{type_name}' {reason}"
            )));
        };
        field.r#type = Some(kind);
    }
    Ok(())
}

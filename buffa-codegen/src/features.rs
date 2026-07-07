//! Edition feature resolution for code generation.
//!
//! The shared core (file/message/enum/oneof feature resolution) lives in
//! `buffa-descriptor`'s [`features`](buffa_descriptor::features) module so the
//! runtime [`DescriptorPool`](buffa_descriptor::DescriptorPool) and codegen
//! resolve editions identically — a divergence between them would mean
//! generated code and reflective code disagree on packed encoding, presence,
//! or enum openness.
//!
//! This module re-exports that core and adds the codegen-only
//! [`resolve_field`], which overlays the referenced enum's own `enum_type` and
//! applies codegen-only enum representation overrides. That overlay needs
//! [`CodeGenContext::is_enum_closed`], which is built during codegen and not
//! available to the runtime pool.

pub use buffa_descriptor::features::*;

use crate::context::CodeGenContext;
use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::generated::descriptor::FieldDescriptorProto;

/// Compute a field's resolved features, including enum closedness lookup and
/// `open_enums_in` override matching.
///
/// This is `resolve_child(parent, field_features(field))` plus a critical
/// fixup: for enum-typed fields, `enum_type` is overlaid with the
/// REFERENCED ENUM's own resolved `enum_type` (looked up from
/// `ctx.is_enum_closed`). protoc does not propagate enum-level `enum_type`
/// into field options, so without this lookup a per-enum
/// `option features.enum_type = CLOSED` would be ignored.
///
/// `field_fqn` is the public fully-qualified field path used by path-scoped
/// codegen options. For map values, pass the outer map field path; for oneof
/// fields, pass the direct oneof field path. When no field path is available,
/// `open_enums_in` can still match the referenced enum type FQN.
///
/// For extern_path enums (not in `ctx`), falls back to the field's own feature
/// chain, which is correct for proto2/proto3 where `enum_type` is file-level
/// anyway. `open_enums_in` may still force the field representation open.
pub fn resolve_field(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    parent: &ResolvedFeatures,
    field_fqn: Option<&str>,
) -> ResolvedFeatures {
    let mut resolved = resolve_child(parent, field_features(field));
    // Overlay the referenced enum's own enum_type.
    if field.r#type.unwrap_or_default() == Type::TYPE_ENUM {
        if let Some(fqn) = field.type_name.as_deref() {
            if let Some(closed) = ctx.is_enum_closed(fqn) {
                resolved.enum_type = if closed {
                    EnumType::Closed
                } else {
                    EnumType::Open
                };
            }
            if ctx.open_enum_override_matches(field_fqn, fqn) {
                resolved.enum_type = EnumType::Open;
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
    use crate::generated::descriptor::{
        DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    };
    use crate::CodeGenConfig;

    fn enum_field() -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some("e".into()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".p.E".into()),
            ..Default::default()
        }
    }

    fn file_with_closed_enum(field: FieldDescriptorProto) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("p".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("M".into()),
                field: vec![field],
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("E".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn resolve_with(config: &CodeGenConfig, field_fqn: Option<&str>) -> ResolvedFeatures {
        let field = enum_field();
        let files = [file_with_closed_enum(field.clone())];
        let ctx = CodeGenContext::new(&files, config, &config.extern_paths);
        resolve_field(&ctx, &field, &for_file(&files[0]), field_fqn)
    }

    #[test]
    fn resolve_field_applies_open_enums_in_after_closed_enum_lookup() {
        let config = CodeGenConfig::default();
        assert_eq!(
            resolve_with(&config, Some(".p.M.e")).enum_type,
            EnumType::Closed
        );

        let config = CodeGenConfig {
            open_enums_in: vec![".p.M.e".into()],
            ..Default::default()
        };
        assert_eq!(
            resolve_with(&config, Some(".p.M.e")).enum_type,
            EnumType::Open
        );
        assert_eq!(
            resolve_with(&config, Some(".p.M.other")).enum_type,
            EnumType::Closed
        );

        let config = CodeGenConfig {
            open_enums_in: vec![".p.E".into()],
            ..Default::default()
        };
        assert_eq!(resolve_with(&config, None).enum_type, EnumType::Open);
    }
}

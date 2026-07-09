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
//! [`resolve_field`], which overlays the referenced enum's own `enum_type`.
//! That overlay needs [`CodeGenContext::is_enum_closed`], which is built
//! during codegen and not available to the runtime pool.

pub use buffa_descriptor::features::*;

use crate::context::CodeGenContext;
use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::generated::descriptor::FieldDescriptorProto;

/// Compute a field's resolved features, including enum closedness lookup.
///
/// This is `resolve_child(parent, field_features(field))` plus a critical
/// fixup: for enum-typed fields, `enum_type` is overlaid with the
/// REFERENCED ENUM's own resolved `enum_type` (looked up from
/// `ctx.is_enum_closed`). protoc does not propagate enum-level `enum_type`
/// into field options, so without this lookup a per-enum
/// `option features.enum_type = CLOSED` would be ignored.
///
/// For extern_path enums (not in `ctx`), falls back to the field's own
/// feature chain, which is correct for proto2/proto3 where `enum_type`
/// is file-level anyway.
///
/// A field whose own `FeatureSet` sets `enum_type = OPEN` skips the overlay:
/// legal editions input can never carry a field-level `enum_type` (it targets
/// files and enums only, so protoc rejects it on a field), which makes the
/// slot the carrier for field-scoped
/// [`FeatureOverride::EnumType`](crate::FeatureOverride) rules injected
/// by the [`feature_overrides`](crate::feature_overrides) preprocessing pass
/// — those must win over the referenced enum's closedness. Only the `OPEN` value is
/// honored (the only value the pass injects); any other stray field-level
/// `enum_type` from a foreign descriptor set is ignored by the overlay,
/// preserving the referenced enum's semantics.
pub fn resolve_field(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    parent: &ResolvedFeatures,
) -> ResolvedFeatures {
    let mut resolved = resolve_child(parent, field_features(field));
    let field_opens_enum = field_features(field).and_then(|fs| fs.enum_type)
        == Some(crate::generated::descriptor::feature_set::EnumType::OPEN);
    // Overlay the referenced enum's own enum_type.
    if field.r#type.unwrap_or_default() == Type::TYPE_ENUM && !field_opens_enum {
        if let Some(fqn) = field.type_name.as_deref() {
            if let Some(closed) = ctx.is_enum_closed(fqn) {
                resolved.enum_type = if closed {
                    EnumType::Closed
                } else {
                    EnumType::Open
                };
            }
        }
    }
    resolved
}

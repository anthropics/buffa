//! Edition feature resolution for code generation.
//!
//! Protoc emits only *unresolved* (user-written) features in descriptor
//! options.  This module resolves them by walking the descriptor hierarchy
//! (file → message → field/enum/oneof) and merging overrides onto edition
//! defaults, matching the algorithm in protobuf's C++ `FeatureResolver`.

pub use buffa::editions::{
    EnumType, FieldPresence, JsonFormat, MessageEncoding, RepeatedFieldEncoding, ResolvedFeatures,
    Utf8Validation,
};

use crate::generated::descriptor::feature_set::{
    EnumType as FeatureSetEnumType, FieldPresence as FeatureSetFieldPresence,
    JsonFormat as FeatureSetJsonFormat, MessageEncoding as FeatureSetMessageEncoding,
    RepeatedFieldEncoding as FeatureSetRepeatedFieldEncoding,
    Utf8Validation as FeatureSetUtf8Validation,
};
use crate::generated::descriptor::{
    DescriptorProto, Edition, EnumDescriptorProto, FeatureSet, FieldDescriptorProto,
    FileDescriptorProto, OneofDescriptorProto,
};

/// Compute the file-level resolved features from a `FileDescriptorProto`.
///
/// For proto2/proto3 files (identified by the `syntax` field), returns the
/// corresponding legacy defaults.  For editions files, starts from the
/// edition defaults and merges any file-level feature overrides.
pub fn for_file(file: &FileDescriptorProto) -> ResolvedFeatures {
    match file.syntax.as_deref() {
        Some("proto3") => {
            let base = ResolvedFeatures::proto3_defaults();
            merge(base, file_features(file))
        }
        Some("editions") => {
            let edition = file.edition.unwrap_or(Edition::EDITION_UNKNOWN);
            let base = for_edition(edition);
            merge(base, file_features(file))
        }
        // proto2 or absent
        _ => {
            let base = ResolvedFeatures::proto2_defaults();
            merge(base, file_features(file))
        }
    }
}

/// Compute a child element's resolved features by merging the parent's
/// resolved features with the child's unresolved `FeatureSet` override.
///
/// If `child_features` is `None`, returns the parent unchanged.
pub fn resolve_child(
    parent: &ResolvedFeatures,
    child_features: Option<&FeatureSet>,
) -> ResolvedFeatures {
    merge(*parent, child_features)
}

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
pub fn resolve_field(
    ctx: &crate::context::CodeGenContext,
    field: &FieldDescriptorProto,
    parent: &ResolvedFeatures,
) -> ResolvedFeatures {
    let mut resolved = resolve_child(parent, field_features(field));
    // Overlay the referenced enum's own enum_type.
    if field.r#type.unwrap_or_default()
        == crate::generated::descriptor::field_descriptor_proto::Type::TYPE_ENUM
    {
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

/// Map an `Edition` value to the corresponding default features.
fn for_edition(edition: Edition) -> ResolvedFeatures {
    match edition {
        Edition::EDITION_PROTO2 | Edition::EDITION_LEGACY => ResolvedFeatures::proto2_defaults(),
        Edition::EDITION_PROTO3 => ResolvedFeatures::proto3_defaults(),
        // Edition 2024 is finalized: its only additions over 2023 are
        // source-retained features (enforce_naming_style, default_symbol_visibility)
        // which protoc enforces and strips before the descriptor reaches a plugin.
        // From a codegen perspective 2023 and 2024 are permanently equivalent.
        Edition::EDITION_2023 | Edition::EDITION_2024 => ResolvedFeatures::edition_2023_defaults(),
        // EDITION_UNSTABLE and the *_TEST_ONLY editions are intentionally not
        // supported; fall back to 2023 defaults so experimental descriptors
        // at least compile rather than panic.
        _ => ResolvedFeatures::edition_2023_defaults(),
    }
}

/// Merge an optional unresolved `FeatureSet` onto a resolved set.
/// Each field in the child that is set (not `None` / not `UNKNOWN`)
/// overrides the corresponding parent value.
fn merge(parent: ResolvedFeatures, features: Option<&FeatureSet>) -> ResolvedFeatures {
    let Some(fs) = features else {
        return parent;
    };
    ResolvedFeatures {
        field_presence: fs
            .field_presence
            .and_then(convert_field_presence)
            .unwrap_or(parent.field_presence),
        enum_type: fs
            .enum_type
            .and_then(convert_enum_type)
            .unwrap_or(parent.enum_type),
        repeated_field_encoding: fs
            .repeated_field_encoding
            .and_then(convert_repeated_field_encoding)
            .unwrap_or(parent.repeated_field_encoding),
        utf8_validation: fs
            .utf8_validation
            .and_then(convert_utf8_validation)
            .unwrap_or(parent.utf8_validation),
        message_encoding: fs
            .message_encoding
            .and_then(convert_message_encoding)
            .unwrap_or(parent.message_encoding),
        json_format: fs
            .json_format
            .and_then(convert_json_format)
            .unwrap_or(parent.json_format),
    }
}

// ── Feature extractors ──────────────────────────────────────────────────

/// Extract the unresolved `FeatureSet` from file options.
fn file_features(file: &FileDescriptorProto) -> Option<&FeatureSet> {
    file.options
        .as_option()
        .and_then(|o| o.features.as_option())
}

/// Extract the unresolved `FeatureSet` from message options.
pub fn message_features(msg: &DescriptorProto) -> Option<&FeatureSet> {
    msg.options.as_option().and_then(|o| o.features.as_option())
}

/// Extract the unresolved `FeatureSet` from field options.
pub fn field_features(field: &FieldDescriptorProto) -> Option<&FeatureSet> {
    field
        .options
        .as_option()
        .and_then(|o| o.features.as_option())
}

/// Extract the unresolved `FeatureSet` from enum options.
pub fn enum_features(e: &EnumDescriptorProto) -> Option<&FeatureSet> {
    e.options.as_option().and_then(|o| o.features.as_option())
}

/// Extract the unresolved `FeatureSet` from oneof options.
#[allow(dead_code)] // reserved for future per-oneof feature resolution
pub fn oneof_features(o: &OneofDescriptorProto) -> Option<&FeatureSet> {
    o.options.as_option().and_then(|o| o.features.as_option())
}

// ── Descriptor enum → runtime enum converters ───────────────────────────
//
// The generated descriptor enums (e.g. `FeatureSetFieldPresence`) have an
// `UNKNOWN = 0` variant that means "not set".  We convert to the runtime
// enums, returning `None` for unknown/unset so the merge uses the parent.

fn convert_field_presence(v: FeatureSetFieldPresence) -> Option<FieldPresence> {
    match v {
        FeatureSetFieldPresence::EXPLICIT => Some(FieldPresence::Explicit),
        FeatureSetFieldPresence::IMPLICIT => Some(FieldPresence::Implicit),
        FeatureSetFieldPresence::LEGACY_REQUIRED => Some(FieldPresence::LegacyRequired),
        FeatureSetFieldPresence::FIELD_PRESENCE_UNKNOWN => None,
    }
}

fn convert_enum_type(v: FeatureSetEnumType) -> Option<EnumType> {
    match v {
        FeatureSetEnumType::OPEN => Some(EnumType::Open),
        FeatureSetEnumType::CLOSED => Some(EnumType::Closed),
        FeatureSetEnumType::ENUM_TYPE_UNKNOWN => None,
    }
}

fn convert_repeated_field_encoding(
    v: FeatureSetRepeatedFieldEncoding,
) -> Option<RepeatedFieldEncoding> {
    match v {
        FeatureSetRepeatedFieldEncoding::PACKED => Some(RepeatedFieldEncoding::Packed),
        FeatureSetRepeatedFieldEncoding::EXPANDED => Some(RepeatedFieldEncoding::Expanded),
        FeatureSetRepeatedFieldEncoding::REPEATED_FIELD_ENCODING_UNKNOWN => None,
    }
}

fn convert_utf8_validation(v: FeatureSetUtf8Validation) -> Option<Utf8Validation> {
    match v {
        FeatureSetUtf8Validation::VERIFY => Some(Utf8Validation::Verify),
        FeatureSetUtf8Validation::NONE => Some(Utf8Validation::None),
        FeatureSetUtf8Validation::UTF8_VALIDATION_UNKNOWN => None,
    }
}

fn convert_message_encoding(v: FeatureSetMessageEncoding) -> Option<MessageEncoding> {
    match v {
        FeatureSetMessageEncoding::LENGTH_PREFIXED => Some(MessageEncoding::LengthPrefixed),
        FeatureSetMessageEncoding::DELIMITED => Some(MessageEncoding::Delimited),
        FeatureSetMessageEncoding::MESSAGE_ENCODING_UNKNOWN => None,
    }
}

fn convert_json_format(v: FeatureSetJsonFormat) -> Option<JsonFormat> {
    match v {
        FeatureSetJsonFormat::ALLOW => Some(JsonFormat::Allow),
        FeatureSetJsonFormat::LEGACY_BEST_EFFORT => Some(JsonFormat::LegacyBestEffort),
        FeatureSetJsonFormat::JSON_FORMAT_UNKNOWN => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::FeatureSetDefaults;

    #[test]
    fn proto2_file_returns_proto2_defaults() {
        let file = FileDescriptorProto {
            name: Some("test.proto".into()),
            ..Default::default()
        };
        let f = for_file(&file);
        assert_eq!(f, ResolvedFeatures::proto2_defaults());
    }

    #[test]
    fn proto3_file_returns_proto3_defaults() {
        let file = FileDescriptorProto {
            name: Some("test.proto".into()),
            syntax: Some("proto3".into()),
            ..Default::default()
        };
        let f = for_file(&file);
        assert_eq!(f, ResolvedFeatures::proto3_defaults());
    }

    #[test]
    fn editions_2023_file_returns_edition_defaults() {
        let file = FileDescriptorProto {
            name: Some("test.proto".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_2023),
            ..Default::default()
        };
        let f = for_file(&file);
        assert_eq!(f, ResolvedFeatures::edition_2023_defaults());
    }

    #[test]
    fn editions_2024_shares_2023_defaults() {
        let file = FileDescriptorProto {
            name: Some("test.proto".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_2024),
            ..Default::default()
        };
        let f = for_file(&file);
        assert_eq!(f, ResolvedFeatures::edition_2023_defaults());
    }

    #[test]
    fn edition_proto2_maps_to_proto2_defaults() {
        let file = FileDescriptorProto {
            name: Some("test.proto".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_PROTO2),
            ..Default::default()
        };
        let f = for_file(&file);
        assert_eq!(f, ResolvedFeatures::proto2_defaults());
    }

    #[test]
    fn edition_legacy_maps_to_proto2_defaults() {
        assert_eq!(
            for_edition(Edition::EDITION_LEGACY),
            ResolvedFeatures::proto2_defaults()
        );
    }

    #[test]
    fn child_inherits_parent_when_no_override() {
        let parent = ResolvedFeatures::proto2_defaults();
        let child = resolve_child(&parent, None);
        assert_eq!(parent, child);
    }

    #[test]
    fn child_partial_override() {
        let parent = ResolvedFeatures::proto3_defaults();
        let override_fs = FeatureSet {
            enum_type: Some(FeatureSetEnumType::CLOSED),
            ..Default::default()
        };
        let child = resolve_child(&parent, Some(&override_fs));
        assert_eq!(child.enum_type, EnumType::Closed);
        assert_eq!(child.field_presence, FieldPresence::Implicit);
        assert_eq!(child.repeated_field_encoding, RepeatedFieldEncoding::Packed);
    }

    #[test]
    fn child_full_override() {
        let parent = ResolvedFeatures::proto3_defaults();
        let override_fs = FeatureSet {
            field_presence: Some(FeatureSetFieldPresence::EXPLICIT),
            enum_type: Some(FeatureSetEnumType::CLOSED),
            repeated_field_encoding: Some(FeatureSetRepeatedFieldEncoding::EXPANDED),
            utf8_validation: Some(FeatureSetUtf8Validation::NONE),
            message_encoding: Some(FeatureSetMessageEncoding::DELIMITED),
            json_format: Some(FeatureSetJsonFormat::LEGACY_BEST_EFFORT),
            ..Default::default()
        };
        let child = resolve_child(&parent, Some(&override_fs));
        assert_eq!(child.field_presence, FieldPresence::Explicit);
        assert_eq!(child.enum_type, EnumType::Closed);
        assert_eq!(
            child.repeated_field_encoding,
            RepeatedFieldEncoding::Expanded
        );
        assert_eq!(child.utf8_validation, Utf8Validation::None);
        assert_eq!(child.message_encoding, MessageEncoding::Delimited);
        assert_eq!(child.json_format, JsonFormat::LegacyBestEffort);
    }

    #[test]
    fn unknown_enum_values_are_treated_as_unset() {
        let parent = ResolvedFeatures::edition_2023_defaults();
        let override_fs = FeatureSet {
            field_presence: Some(FeatureSetFieldPresence::FIELD_PRESENCE_UNKNOWN),
            enum_type: Some(FeatureSetEnumType::ENUM_TYPE_UNKNOWN),
            ..Default::default()
        };
        let child = resolve_child(&parent, Some(&override_fs));
        assert_eq!(child.field_presence, parent.field_presence);
        assert_eq!(child.enum_type, parent.enum_type);
    }

    #[test]
    fn editions_file_with_file_level_override() {
        let mut file = FileDescriptorProto {
            name: Some("test.proto".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_2023),
            ..Default::default()
        };
        file.options
            .get_or_insert_default()
            .features
            .get_or_insert_default()
            .enum_type = Some(FeatureSetEnumType::CLOSED);

        let f = for_file(&file);
        assert_eq!(f.enum_type, EnumType::Closed);
        assert_eq!(f.field_presence, FieldPresence::Explicit);
        assert_eq!(f.repeated_field_encoding, RepeatedFieldEncoding::Packed);
    }

    #[test]
    fn multi_level_hierarchy() {
        // File: edition 2023 defaults (open enums, packed, explicit presence)
        let file_features = for_edition(Edition::EDITION_2023);
        assert_eq!(file_features.enum_type, EnumType::Open);

        // Message: override to closed enums
        let msg_override = FeatureSet {
            enum_type: Some(FeatureSetEnumType::CLOSED),
            ..Default::default()
        };
        let msg_features = resolve_child(&file_features, Some(&msg_override));
        assert_eq!(msg_features.enum_type, EnumType::Closed);
        assert_eq!(msg_features.field_presence, FieldPresence::Explicit);

        // Field: override to implicit presence
        let field_override = FeatureSet {
            field_presence: Some(FeatureSetFieldPresence::IMPLICIT),
            ..Default::default()
        };
        let field_features = resolve_child(&msg_features, Some(&field_override));
        // Field-level override
        assert_eq!(field_features.field_presence, FieldPresence::Implicit);
        // Inherited from message
        assert_eq!(field_features.enum_type, EnumType::Closed);
        // Inherited from file/edition defaults
        assert_eq!(
            field_features.repeated_field_encoding,
            RepeatedFieldEncoding::Packed
        );
    }

    /// Resolve a `FeatureSet` from the protoc defaults into our
    /// `ResolvedFeatures`, merging both the fixed and overridable halves.
    ///
    /// The protoc defaults split features into `fixed_features` (cannot be
    /// overridden by users) and `overridable_features` (can be).  For the
    /// purpose of computing the edition's effective defaults, we merge both:
    /// start from fixed, then layer overridable on top.
    ///
    /// # Panics
    ///
    /// Panics if `field_presence` or `message_encoding` still hold
    /// sentinel values after merging, which indicates protoc did not set
    /// them in either half.  These two fields are checked because no
    /// real edition defaults to `LegacyRequired` or `Delimited`.
    fn resolve_protoc_defaults(
        fixed: Option<&FeatureSet>,
        overridable: Option<&FeatureSet>,
    ) -> ResolvedFeatures {
        // Merge both halves onto a deliberately distinguishable sentinel
        // base, then verify every field was overwritten.
        let sentinel = ResolvedFeatures {
            field_presence: FieldPresence::LegacyRequired,
            enum_type: EnumType::Closed,
            repeated_field_encoding: RepeatedFieldEncoding::Expanded,
            utf8_validation: Utf8Validation::None,
            message_encoding: MessageEncoding::Delimited,
            json_format: JsonFormat::LegacyBestEffort,
        };
        let after_fixed = merge(sentinel, fixed);
        let result = merge(after_fixed, overridable);

        // If any field still holds the sentinel value, protoc didn't set
        // it in either half — our assumptions are wrong.
        assert_ne!(
            result.field_presence,
            FieldPresence::LegacyRequired,
            "protoc did not set field_presence in either fixed or overridable features"
        );
        assert_ne!(
            result.message_encoding,
            MessageEncoding::Delimited,
            "protoc did not set message_encoding in either fixed or overridable features"
        );

        result
    }

    /// Verify that our hardcoded edition defaults match what protoc emits
    /// via `--edition_defaults_out`.
    ///
    /// The golden file `conformance/edition_defaults.binpb` is generated
    /// by running:
    /// ```text
    /// protoc --proto_path=<protobuf-src>/src \
    ///     --edition_defaults_out=conformance/edition_defaults.binpb \
    ///     google/protobuf/descriptor.proto
    /// ```
    /// It should be regenerated whenever the protoc/tools version changes.
    #[test]
    fn hardcoded_defaults_match_protoc_edition_defaults() {
        use buffa::Message;

        let binpb = include_bytes!("../../conformance/edition_defaults.binpb");
        let defaults = FeatureSetDefaults::decode_from_slice(binpb)
            .expect("failed to parse edition_defaults.binpb");

        // Map each entry's edition to the expected hardcoded defaults.
        // EDITION_2024 is included to verify it resolves to the same
        // defaults as 2023 (the golden file's maximum_edition is 2023,
        // so the lookup falls through to the 2023 entry).
        let expected_mappings: &[(Edition, ResolvedFeatures)] = &[
            (Edition::EDITION_LEGACY, ResolvedFeatures::proto2_defaults()),
            (Edition::EDITION_PROTO2, ResolvedFeatures::proto2_defaults()),
            (Edition::EDITION_PROTO3, ResolvedFeatures::proto3_defaults()),
            (
                Edition::EDITION_2023,
                ResolvedFeatures::edition_2023_defaults(),
            ),
            (
                Edition::EDITION_2024,
                ResolvedFeatures::edition_2023_defaults(),
            ),
        ];

        for &(target_edition, ref expected) in expected_mappings {
            // Find the highest defaults entry with edition ≤
            // target_edition, matching the algorithm from the protobuf
            // implementation guide.
            let entry = defaults
                .defaults
                .iter()
                .rfind(|d| {
                    d.edition
                        .is_some_and(|e| (e as i32) <= (target_edition as i32))
                })
                .unwrap_or_else(|| panic!("no defaults entry for edition {target_edition:?}"));

            let resolved = resolve_protoc_defaults(
                entry.fixed_features.as_option(),
                entry.overridable_features.as_option(),
            );

            assert_eq!(
                &resolved, expected,
                "defaults mismatch for edition {target_edition:?}: \
                 protoc emitted {resolved:?}, we hardcode {expected:?}"
            );
        }
    }
}

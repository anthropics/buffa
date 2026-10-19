//! Edition feature types and resolution.
//!
//! These types represent the protobuf edition features that control code
//! generation and runtime behavior. They are resolved at compile time by
//! `buffa-codegen` and baked into generated code.

/// The edition of a `.proto` file.
///
/// # Variant ordering
///
/// Variants are declared in chronological edition order.  The derived
/// `PartialOrd` / `Ord` implementations use declaration order, so new
/// editions **must be appended** — inserting a variant between existing
/// ones would silently break edition comparison logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Edition {
    /// Proto2 syntax (resolves to proto2 feature defaults).
    Proto2,
    /// Proto3 syntax (resolves to proto3 feature defaults).
    Proto3,
    /// Edition 2023.
    Edition2023,
    /// Edition 2024.
    Edition2024,
}

/// Controls whether field presence is tracked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldPresence {
    /// Presence is tracked explicitly (has-bit or Option).
    Explicit,
    /// No presence tracking; default value means "not set."
    Implicit,
    /// Legacy `required` behavior from proto2.
    LegacyRequired,
}

/// Controls how unknown enum values are handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnumType {
    /// Unknown values are preserved in the field (first-class `EnumValue<E>`).
    Open,
    /// Unknown values are stored in unknown fields.
    Closed,
}

/// Controls wire encoding of repeated scalar fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepeatedFieldEncoding {
    /// Packed encoding (length-delimited, all values concatenated).
    Packed,
    /// Expanded encoding (one tag per element).
    Expanded,
}

/// Controls UTF-8 validation for string fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Utf8Validation {
    /// Validate that string fields contain valid UTF-8.
    Verify,
    /// Skip UTF-8 validation.
    None,
}

/// Controls wire encoding of message fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageEncoding {
    /// Standard length-prefixed encoding.
    LengthPrefixed,
    /// Delimited (group-style) encoding.
    Delimited,
}

/// Controls JSON format support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonFormat {
    /// Full JSON support.
    Allow,
    /// Legacy best-effort JSON support.
    LegacyBestEffort,
}

/// The resolved set of edition features for a protobuf element.
///
/// # Forward compatibility
///
/// This struct is intentionally **not** `#[non_exhaustive]`. It is
/// constructed via struct literals by `buffa-codegen` (workspace sibling,
/// always released in lockstep with this crate). Adding a new feature field
/// here requires a coordinated change in `buffa-codegen::features::merge`.
///
/// User code should not construct `ResolvedFeatures` directly — use the
/// `*_defaults()` associated functions instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedFeatures {
    pub field_presence: FieldPresence,
    pub enum_type: EnumType,
    pub repeated_field_encoding: RepeatedFieldEncoding,
    pub utf8_validation: Utf8Validation,
    pub message_encoding: MessageEncoding,
    pub json_format: JsonFormat,
}

impl ResolvedFeatures {
    /// Default features for Edition 2023.
    pub fn edition_2023_defaults() -> Self {
        Self {
            field_presence: FieldPresence::Explicit,
            enum_type: EnumType::Open,
            repeated_field_encoding: RepeatedFieldEncoding::Packed,
            utf8_validation: Utf8Validation::Verify,
            message_encoding: MessageEncoding::LengthPrefixed,
            json_format: JsonFormat::Allow,
        }
    }

    /// Default features for Edition 2024.
    ///
    /// Edition 2024's new naming-style and symbol-visibility options are
    /// source-retained, so its runtime defaults match Edition 2023.
    pub fn edition_2024_defaults() -> Self {
        Self::edition_2023_defaults()
    }

    /// Features that replicate proto2 behavior.
    pub fn proto2_defaults() -> Self {
        Self {
            field_presence: FieldPresence::Explicit,
            enum_type: EnumType::Closed,
            repeated_field_encoding: RepeatedFieldEncoding::Expanded,
            utf8_validation: Utf8Validation::None,
            message_encoding: MessageEncoding::LengthPrefixed,
            json_format: JsonFormat::LegacyBestEffort,
        }
    }

    /// Features that replicate proto3 behavior.
    pub fn proto3_defaults() -> Self {
        Self {
            field_presence: FieldPresence::Implicit,
            enum_type: EnumType::Open,
            repeated_field_encoding: RepeatedFieldEncoding::Packed,
            utf8_validation: Utf8Validation::Verify,
            message_encoding: MessageEncoding::LengthPrefixed,
            json_format: JsonFormat::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edition_2024_runtime_defaults_match_2023() {
        let defaults = ResolvedFeatures::edition_2024_defaults();
        assert_eq!(defaults, ResolvedFeatures::edition_2023_defaults());
        assert_eq!(defaults.field_presence, FieldPresence::Explicit);
        assert_eq!(defaults.enum_type, EnumType::Open);
        assert_eq!(defaults.repeated_field_encoding, RepeatedFieldEncoding::Packed);
        assert_eq!(defaults.utf8_validation, Utf8Validation::Verify);
        assert_eq!(defaults.message_encoding, MessageEncoding::LengthPrefixed);
        assert_eq!(defaults.json_format, JsonFormat::Allow);
    }
}

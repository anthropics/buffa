//! Typed manifest for process-local adapters. No entry contains Rust source.

use std::io::Read;

use serde::Deserialize;

use super::{AdapterError, ErrorReason};

/// Maximum encoded size of an adapter manifest (1 MiB).
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// Opt-in, descriptor-checked adapter declarations.
///
/// This metadata contains names and paths only; never put keys or application
/// data in it. Unknown JSON members and duplicate members are rejected.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    /// Manifest format. Currently must be 1.
    pub version: u32,
    /// Closed selectors with checked wire conversions.
    #[serde(default)]
    pub selectors: Vec<ClosedSelector>,
    /// Copy payload selectors. Payloads name a declared descriptor field.
    #[serde(default)]
    pub payloads: Vec<PayloadSelector>,
    /// Borrowed enums; never implement a wire codec or own caller buffers.
    #[serde(default)]
    pub borrowed: Vec<BorrowedAdapter>,
    /// Process-local provider traits, explicitly declared by the application.
    #[serde(default)]
    pub ports: Vec<CapabilityPort>,
}

impl AdapterConfig {
    /// Decode a bounded manifest without exposing input in diagnostics.
    ///
    /// # Errors
    /// Returns a fixed reason for oversized, malformed or unsupported input.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AdapterError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ErrorReason::ManifestTooLarge.into());
        }
        let config: Self =
            serde_json::from_slice(bytes).map_err(|_| ErrorReason::InvalidManifest)?;
        if config.version != 1 {
            return Err(ErrorReason::UnsupportedVersion.into());
        }
        Ok(config)
    }

    /// Read a manifest with the same bound as [`Self::from_json`].
    ///
    /// # Errors
    /// Returns a fixed I/O, size or manifest error, without retaining paths.
    pub fn read_json(path: impl AsRef<std::path::Path>) -> Result<Self, AdapterError> {
        let limit = MAX_MANIFEST_BYTES
            .checked_add(1)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(ErrorReason::ManifestTooLarge)?;
        let file = std::fs::File::open(path).map_err(|_| ErrorReason::ManifestIo)?;
        let mut bytes = Vec::new();
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|_| ErrorReason::ManifestIo)?;
        Self::from_json(&bytes)
    }
}

/// Domain error construction using paths only, never expressions or messages.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorMapping {
    /// Domain error type, e.g. `crate::ContractError`.
    pub rust_type: String,
    /// Fixed enum variant or constant, converted with `Into` to `rust_type`.
    pub value: String,
}

/// Descriptor enum and its already-generated Rust wire type.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireEnum {
    /// Fully-qualified descriptor name, including its leading dot.
    pub source: String,
    /// Rust path to the corresponding wire enum.
    pub rust_type: String,
}

/// A nonzero canonical selector case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectorVariant {
    /// Exact descriptor value name, without the enclosing enum name.
    pub source: String,
    /// Public native variant name.
    pub name: String,
    /// Compatibility associated constants for this native variant.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Optional external protocol identity, distinct from the protobuf number.
    pub identifier: Option<i64>,
}

/// Explicit normalization of a legacy value to an existing canonical case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Normalization {
    /// Descriptor value name in this source enum.
    pub source: String,
    /// Canonical native variant name.
    pub target: String,
}

/// Additional legacy wire enum accepted by a closed selector.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacySelector {
    /// Descriptor enum and Rust type for the legacy input.
    pub wire: WireEnum,
    /// Every accepted nonzero legacy case.
    pub normalizations: Vec<Normalization>,
    /// Nonzero cases deliberately rejected, rather than silently omitted.
    #[serde(default)]
    pub rejected: Vec<String>,
}

/// Closed native enum; zero and unknown values always fail conversion.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedSelector {
    /// Bare sibling `.rs` filename, included explicitly by the application.
    pub file: String,
    /// Native Rust enum name.
    pub name: String,
    /// Authoritative wire enum.
    pub wire: WireEnum,
    /// Fixed error for unspecified, rejected and unknown inputs.
    pub error: ErrorMapping,
    /// Preserve an extensible public Rust matching surface.
    #[serde(default)]
    pub non_exhaustive: bool,
    /// Canonical cases in public `ALL` order.
    pub variants: Vec<SelectorVariant>,
    /// Deprecated inputs accepted as canonical values.
    #[serde(default)]
    pub normalizations: Vec<Normalization>,
    /// Nonzero cases intentionally unsupported by this adapter.
    #[serde(default)]
    pub rejected: Vec<String>,
    /// Additional checked conversions from legacy wire enums.
    #[serde(default)]
    pub legacy: Vec<LegacySelector>,
    /// Error for `from_identifier`; required if identities are supplied.
    pub identifier_error: Option<ErrorMapping>,
}

/// A declared schema field reached through singular message fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSource {
    /// Fully-qualified root message name.
    pub message: String,
    /// Nonempty field-name path, e.g. `["ecdsa_prehash", "digest"]`.
    pub path: Vec<String>,
}

/// Restricted representation of a payload field.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "PayloadTypeJson")]
pub enum PayloadType {
    /// Borrow the original bytes without copying or exposing `Debug` output.
    Bytes,
    /// An application-supplied representation of this descriptor field.
    ///
    /// The generator checks the descriptor shape, path syntax and generated
    /// `Copy` bound. It cannot resolve Rust type identity or inspect an opaque
    /// value's fields; the application must preserve the declared native-type,
    /// validation and ownership invariants. `Copy` alone permits references.
    /// The generated Eq/Hash/Debug bounds also apply to owning payload enums.
    Value {
        /// Fully-qualified descriptor enum or message represented by the type.
        source: String,
        /// Plain Rust path; generic arguments and reference syntax are rejected.
        /// For an enum field, the terminal identifier must match a configured
        /// selector's name for `source`; renamed aliases are not resolved.
        rust_type: String,
    },
}

// serde ignores `deny_unknown_fields` on internally tagged unit variants, so
// the JSON forms use empty struct variants: a members-free kind then rejects
// any member other than `kind`, as the protobuf option front end does.
#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum PayloadTypeJson {
    Bytes {},
    Value { source: String, rust_type: String },
}

impl From<PayloadTypeJson> for PayloadType {
    fn from(value: PayloadTypeJson) -> Self {
        match value {
            PayloadTypeJson::Bytes {} => Self::Bytes,
            PayloadTypeJson::Value { source, rust_type } => Self::Value { source, rust_type },
        }
    }
}

/// One payload field and its checked representation.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadField {
    /// Name used in a struct variant; also validated for tuple payloads.
    pub name: String,
    /// Canonical source field.
    pub source: FieldSource,
    /// Restricted field representation.
    pub representation: PayloadType,
}

/// Shape of one native enum alternative.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "PayloadJson")]
pub enum Payload {
    /// No payload.
    Unit,
    /// A single tuple field.
    Tuple { field: PayloadField },
    /// Named fields.
    Struct { fields: Vec<PayloadField> },
    /// A borrowed provider capability; never a portable field.
    Port { name: String },
}

// JSON form of `Payload`; see `PayloadTypeJson`.
#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum PayloadJson {
    Unit {},
    Tuple { field: PayloadField },
    Struct { fields: Vec<PayloadField> },
    Port { name: String },
}

impl From<PayloadJson> for Payload {
    fn from(value: PayloadJson) -> Self {
        match value {
            PayloadJson::Unit {} => Self::Unit,
            PayloadJson::Tuple { field } => Self::Tuple { field },
            PayloadJson::Struct { fields } => Self::Struct { fields },
            PayloadJson::Port { name } => Self::Port { name },
        }
    }
}

/// Native case covering one or more authoritative descriptor alternatives.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadVariant {
    /// Public Rust variant name.
    pub name: String,
    /// Descriptor enum value names, or oneof member names.
    pub cases: Vec<String>,
    /// Checked payload shape.
    pub payload: Payload,
}

/// Copy enum combining a selector with an explicitly validated native value.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadSelector {
    /// Bare sibling output filename.
    pub file: String,
    /// Public Rust enum name.
    pub name: String,
    /// Fully-qualified descriptor enum name.
    pub source: String,
    /// Complete nonzero case coverage.
    pub variants: Vec<PayloadVariant>,
}

/// Authoritative cases for a borrowed adapter.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BorrowedSource {
    /// All nonzero values of a descriptor enum.
    Enum { source: String },
    /// Every alternative of an explicit oneof. Its entire message must consist
    /// of that oneof, so unrelated wire fields cannot silently disappear.
    Oneof { message: String, name: String },
}

/// Lifetime-bearing adapter. It deliberately has no derived diagnostics.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorrowedAdapter {
    /// Bare sibling output filename.
    pub file: String,
    /// Public Rust enum name.
    pub name: String,
    /// Authoritative enum or oneof cases.
    pub source: BorrowedSource,
    /// Complete case and (for a oneof) payload-field coverage.
    pub variants: Vec<PayloadVariant>,
}

/// Process-local capability supplied by the application, never serialized.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPort {
    /// Manifest-local port name.
    pub name: String,
    /// Plain path to an existing object-safe Rust trait.
    pub rust_trait: String,
}

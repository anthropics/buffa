//! Opt-in, descriptor-checked native adapters. Generated adapters are separate
//! files: applications include them in the modules that own their native API.

mod config;
mod generate;
mod options;

pub use config::*;
pub use generate::generate_adapters;
pub(crate) use options::descriptor_options;

/// Deterministic, allocation-free reason for rejecting adapter metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorReason {
    /// An adapter-specific plugin option has an invalid or repeated value.
    InvalidPluginOption,
    /// JSON has unknown/duplicate members, invalid types or excessive nesting.
    InvalidManifest,
    /// The manifest version is not supported.
    UnsupportedVersion,
    /// The manifest exceeds the encoded size limit.
    ManifestTooLarge,
    /// Reading the manifest failed.
    ManifestIo,
    /// A Rust name is not a usable identifier.
    InvalidIdentifier,
    /// A Rust path contains unsupported syntax.
    InvalidRustPath,
    /// An output filename is not a bare `.rs` sibling, or its destination
    /// cannot be resolved safely.
    InvalidOutput,
    /// Two outputs have the same filename.
    DuplicateOutput,
    /// Native names, ports or variant names collide.
    DuplicateName,
    /// A named descriptor could not be found.
    MissingDescriptor,
    /// A descriptor has missing, ambiguous or invalid members.
    InvalidDescriptor,
    /// An adapter's authoritative source is outside this generation request.
    GenerationScope,
    /// A descriptor alternative was not explicitly mapped or rejected.
    UncoveredCase,
    /// An alternative is claimed more than once.
    DuplicateCase,
    /// A mapping names an alternative absent from the descriptor.
    UnknownCase,
    /// An unspecified zero value was mapped as an accepted alternative.
    UnspecifiedCase,
    /// An alias names no canonical variant or conflicts numerically.
    InvalidNormalization,
    /// External identities must be complete and unambiguous.
    InvalidIdentifiers,
    /// A payload has an incompatible type or capability.
    InvalidPayload,
    /// An owning payload attempted to borrow or own raw sensitive bytes.
    OwnershipMismatch,
    /// A oneof payload contains an unprojected field.
    UncoveredField,
    /// A payload claims a field outside its selected oneof alternative.
    UnexpectedField,
    /// Generated tokens did not form a Rust file.
    InvalidSyntax,
    /// Schema adapter options are enabled, but no requested file declares
    /// them and no explicit configuration was supplied.
    MissingSchemaAdapters,
}

/// Fixed adapter failure. Input names, paths and bytes are never retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("adapter generation failed: {reason:?}")]
pub struct AdapterError {
    /// Machine-readable failure reason.
    pub reason: ErrorReason,
}

impl From<ErrorReason> for AdapterError {
    fn from(reason: ErrorReason) -> Self {
        Self { reason }
    }
}

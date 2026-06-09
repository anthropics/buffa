//! Protobuf well-known types for buffa.
//!
//! This crate provides Rust types for Google's well-known `.proto` types:
//!
//! - [`google::protobuf::Timestamp`] — Unix timestamp with nanosecond precision
//! - [`google::protobuf::Duration`] — Signed duration with nanosecond precision
//! - [`google::protobuf::Any`] — Any value with an attached type URL
//! - [`google::protobuf::Struct`] / [`google::protobuf::Value`] / [`google::protobuf::ListValue`]
//!   — JSON-like dynamic values
//! - [`google::protobuf::FieldMask`] — Specifies a subset of fields referenced in a message
//! - [`google::protobuf::Empty`] — A generic empty message
//! - Wrapper types: [`google::protobuf::BoolValue`], [`google::protobuf::Int32Value`],
//!   [`google::protobuf::Int64Value`], [`google::protobuf::UInt32Value`],
//!   [`google::protobuf::UInt64Value`], [`google::protobuf::FloatValue`],
//!   [`google::protobuf::DoubleValue`], [`google::protobuf::StringValue`],
//!   [`google::protobuf::BytesValue`]
//!
//! # Usage
//!
//! ```rust,no_run
//! use buffa_types::google::protobuf::Timestamp;
//! use buffa::Message;
//!
//! let ts = Timestamp { seconds: 1_000_000_000, nanos: 0, ..Default::default() };
//! let bytes = ts.encode_to_vec();
//! let decoded = Timestamp::decode_from_slice(&bytes).unwrap();
//! assert_eq!(ts, decoded);
//! ```
//!
//! # Ergonomic helpers
//!
//! Common Rust type conversions are provided as trait impls:
//!
//! - `Timestamp` ↔ [`std::time::SystemTime`] (requires `std` feature)
//! - `Duration` ↔ [`std::time::Duration`] (requires `std` feature)
//! - `Timestamp` ↔ [`chrono::DateTime`] (requires `chrono` feature; any time
//!   zone in, `Utc` out)
//! - `Duration` ↔ [`chrono::TimeDelta`] (requires `chrono` feature)
//! - `Any::pack` / `Any::unpack` helpers
//! - `Value` constructors: [`Value::null`](google::protobuf::Value::null), `From<f64>`, `From<String>`, `From<bool>`, etc.
//! - Wrapper type `From`/`Into` impls
//!
//! # Cargo features
//!
//! - **`std`** (default) — standard-library integration (`SystemTime`/`Duration`
//!   conversions, `std::error::Error`). Without it the crate is `no_std` + `alloc`.
//! - **`json`** — proto3 canonical JSON serde for the WKTs.
//! - **`arbitrary`** — `arbitrary::Arbitrary` derives for fuzzing.
//! - **`chrono`** — `Timestamp` ↔ `chrono::DateTime` and `Duration` ↔
//!   `chrono::TimeDelta` conversions. `no_std`-compatible (`chrono` is pulled
//!   with `default-features = false`).
//! - **`reflect`** — runtime reflection: the WKT view types implement
//!   `buffa_descriptor::reflect::ReflectMessage`, so a message that has a WKT
//!   field can reflect over it. This pulls a `buffa-descriptor` dependency and
//!   requires `std` (the embedded descriptor pool uses `std::sync::OnceLock`).
//!   If you reach for `&view as &dyn ReflectMessage` on a WKT view and the
//!   compiler says `ReflectMessage` is not implemented, enable this feature.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
extern crate alloc;

// Extension modules (ergonomic helpers — hand-written, not generated).
mod any_ext;
mod duration_ext;
mod empty_ext;
mod field_mask_ext;
mod timestamp_ext;
mod value_ext;
#[cfg(feature = "json")]
mod view_serde_ext;
mod wrapper_ext;

#[cfg(feature = "chrono")]
mod duration_chrono;
#[cfg(feature = "chrono")]
mod timestamp_chrono;

// Well-known type Rust structs — generated once by `gen_wkt_types`, checked
// into src/generated/. These protos are Google-owned and frozen; regeneration
// is only needed when buffa-codegen's output format changes. See the
// `task gen-wkt-types` target and the `check-generated-code` CI job.
//
// The checked-in approach means consumers of buffa-types need only the
// `buffa` runtime — NOT protoc, NOT buffa-build, NOT buffa-codegen.
//
// The allow attributes suppress lints that fire on generated code:
//   derivable_impls      — enum Default impls are explicit rather than derived
//   match_single_binding — empty messages generate a single-arm wildcard merge
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod google {
    pub mod protobuf {
        include!("generated/google.protobuf.mod.rs");
    }
}

// Convenience re-exports of the most commonly-used well-known types.
// Full paths (`google::protobuf::*`) remain available for disambiguation.
// Wrapper types (Int32Value, etc.) are NOT re-exported to avoid name
// collisions with similarly-named types in user code.
pub use google::protobuf::{
    Any, Duration, Empty, FieldMask, ListValue, NullValue, Struct, Timestamp, Value,
};

// Re-export error types from extension modules (these are hand-written types
// in private modules, so re-exporting is the only way to make them accessible).
pub use duration_ext::DurationError;
pub use timestamp_ext::TimestampError;

#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
pub use duration_chrono::DurationChronoError;

// Re-export the WKT registry function for `Any` JSON + text support.
pub use any_ext::register_wkt_types;

#[cfg(test)]
mod full_name_tests {
    use super::google::protobuf::*;
    use buffa::MessageName;

    // Regression test: the WKT FQNs are baked into Any type-URLs, JSON
    // serialization, and the type registry. Codegen must keep emitting them
    // verbatim — these strings are observable on the wire.
    #[test]
    fn well_known_types_full_names_match_proto() {
        assert_eq!(Timestamp::FULL_NAME, "google.protobuf.Timestamp");
        assert_eq!(Duration::FULL_NAME, "google.protobuf.Duration");
        assert_eq!(Any::FULL_NAME, "google.protobuf.Any");
        assert_eq!(Empty::FULL_NAME, "google.protobuf.Empty");
        assert_eq!(FieldMask::FULL_NAME, "google.protobuf.FieldMask");
        assert_eq!(Struct::FULL_NAME, "google.protobuf.Struct");
        assert_eq!(Value::FULL_NAME, "google.protobuf.Value");
        assert_eq!(ListValue::FULL_NAME, "google.protobuf.ListValue");
    }
}

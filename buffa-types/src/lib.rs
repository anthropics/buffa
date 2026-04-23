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
//! - `Any::pack` / `Any::unpack` helpers
//! - `Value` constructors: [`Value::null`](google::protobuf::Value::null), `From<f64>`, `From<String>`, `From<bool>`, etc.
//! - Wrapper type `From`/`Into` impls

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(rustdoc::broken_intra_doc_links)]
extern crate alloc;

// Extension modules (ergonomic helpers — hand-written, not generated).
mod any_ext;
mod duration_ext;
mod empty_ext;
mod field_mask_ext;
mod timestamp_ext;
mod value_ext;
mod wrapper_ext;

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
        include!("generated/google.protobuf.any.rs");
        include!("generated/google.protobuf.duration.rs");
        include!("generated/google.protobuf.empty.rs");
        include!("generated/google.protobuf.field_mask.rs");
        include!("generated/google.protobuf.struct.rs");
        include!("generated/google.protobuf.timestamp.rs");
        include!("generated/google.protobuf.wrappers.rs");
        // All ancillary kind trees (view / ext / oneofs) live under a
        // single `pub mod __buffa { ... }` so they cannot collide with
        // proto-derived module names. See DESIGN.md → "Generated code
        // layout".
        #[allow(
            clippy::derivable_impls,
            clippy::match_single_binding,
            non_snake_case,
            non_camel_case_types,
            unused_imports,
            dead_code
        )]
        pub mod __buffa {
            // View tree — `view::<proto-path>::*View` for every message.
            // The inner `view::oneofs::<proto-path>::*` sub-module
            // carries view-of-oneof enums.
            pub mod view {
                include!("generated/google.protobuf.any.__view.rs");
                include!("generated/google.protobuf.duration.__view.rs");
                include!("generated/google.protobuf.empty.__view.rs");
                include!("generated/google.protobuf.field_mask.__view.rs");
                include!("generated/google.protobuf.struct.__view.rs");
                include!("generated/google.protobuf.timestamp.__view.rs");
                include!("generated/google.protobuf.wrappers.__view.rs");
                pub mod oneofs {
                    include!("generated/google.protobuf.any.__view_oneofs.rs");
                    include!("generated/google.protobuf.duration.__view_oneofs.rs");
                    include!("generated/google.protobuf.empty.__view_oneofs.rs");
                    include!("generated/google.protobuf.field_mask.__view_oneofs.rs");
                    include!("generated/google.protobuf.struct.__view_oneofs.rs");
                    include!("generated/google.protobuf.timestamp.__view_oneofs.rs");
                    include!("generated/google.protobuf.wrappers.__view_oneofs.rs");
                }
            }
            // WKTs have no file-level extensions, so the .__ext.rs files
            // are empty and register_types is suppressed. We still
            // include them to keep the sibling-file invariant.
            pub mod ext {
                include!("generated/google.protobuf.any.__ext.rs");
                include!("generated/google.protobuf.duration.__ext.rs");
                include!("generated/google.protobuf.empty.__ext.rs");
                include!("generated/google.protobuf.field_mask.__ext.rs");
                include!("generated/google.protobuf.struct.__ext.rs");
                include!("generated/google.protobuf.timestamp.__ext.rs");
                include!("generated/google.protobuf.wrappers.__ext.rs");
            }
            // Owned oneofs tree — `oneofs::<proto-path>::Kind` for every
            // oneof. Only Value has a `kind` oneof, which lands at
            // `oneofs::value::Kind` inside this module.
            pub mod oneofs {
                include!("generated/google.protobuf.any.__oneofs.rs");
                include!("generated/google.protobuf.duration.__oneofs.rs");
                include!("generated/google.protobuf.empty.__oneofs.rs");
                include!("generated/google.protobuf.field_mask.__oneofs.rs");
                include!("generated/google.protobuf.struct.__oneofs.rs");
                include!("generated/google.protobuf.timestamp.__oneofs.rs");
                include!("generated/google.protobuf.wrappers.__oneofs.rs");
            }
        }
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

// Re-export the WKT registry function for `Any` JSON + text support.
pub use any_ext::register_wkt_types;

//! Generated protobuf types for buffa benchmarks.
//!
//! Built per-message-isolated: `--no-default-features --features iso,<msg>`
//! emits only that message's codec (used by the per-message bench targets); the
//! default feature set emits all messages plus reflect + lazy views for the
//! combined `protobuf`/`reflect` benches.

#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh"
))]
#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
pub mod bench {
    buffa::include_proto!("bench");
}

#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
pub mod benchmarks {
    buffa::include_proto!("benchmarks");
}

#[cfg(feature = "google_message1")]
#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
pub mod proto3 {
    buffa::include_proto!("benchmarks.proto3");
}

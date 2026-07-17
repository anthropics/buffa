//! Generated protobuf types for buffa benchmarks (per-message isolated build).
#[allow(clippy::all, non_camel_case_types, unused_imports, dead_code)]
pub mod bench {
    #[cfg(feature = "api_response")]
    include!(concat!(env!("OUT_DIR"), "/api_response.rs"));
    #[cfg(feature = "log_record")]
    include!(concat!(env!("OUT_DIR"), "/log_record.rs"));
    #[cfg(feature = "analytics_event")]
    include!(concat!(env!("OUT_DIR"), "/analytics_event.rs"));
    #[cfg(feature = "google_message1")]
    include!(concat!(env!("OUT_DIR"), "/google_message1.rs"));
    #[cfg(feature = "media_frame")]
    include!(concat!(env!("OUT_DIR"), "/media_frame.rs"));
    #[cfg(feature = "packed_tile")]
    include!(concat!(env!("OUT_DIR"), "/packed_tile.rs"));
    #[cfg(feature = "mesh")]
    include!(concat!(env!("OUT_DIR"), "/mesh.rs"));
    #[cfg(feature = "column_batch")]
    include!(concat!(env!("OUT_DIR"), "/column_batch.rs"));
}
#[allow(clippy::all, non_camel_case_types, unused_imports, dead_code)]
pub mod benchmarks {
    include!(concat!(env!("OUT_DIR"), "/benchmarks.rs"));
}

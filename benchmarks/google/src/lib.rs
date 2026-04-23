//! Generated protobuf types for Google protobuf-v4 benchmarks.
//!
//! The protobuf v4 codegen produces a `generated.rs` entry point that wraps
//! each `.u.pb.rs` file in an internal module and re-exports all types.
//! This provides the `super::` paths that the generated code relies on.

// Include the generated entry point at the crate root.  The generated.rs
// file uses `#[path = "..."]` attributes with relative paths, so it must
// be included from its containing directory.
include!(concat!(env!("OUT_DIR"), "/protobuf_generated/generated.rs"));

//! Generated protobuf descriptor types.
//!
//! These types are generated from `google/protobuf/descriptor.proto` and
//! `google/protobuf/compiler/plugin.proto` using buffa-codegen itself.
//! This makes buffa fully self-hosted — no external protobuf library is
//! needed to decode descriptors — and gives direct access to edition
//! features (`FeatureSet`, `Edition`, etc.).
//!
//! To regenerate, run `task gen-bootstrap-types` from the repo root.

// Each `.proto` file contributes five sibling outputs to its package
// module (see `buffa-codegen::GeneratedFileKind` — owned, view, ext,
// oneofs, view_oneofs). Descriptor codegen runs with
// `generate_views=false` and has no file-level extensions / oneofs, so
// the ancillary `.__view*.rs` / `.__ext.rs` / `.__oneofs.rs` /
// `.__view_oneofs.rs` siblings are empty-bodied — we still `include!`
// them so the sibling-file invariant is visible in the source tree and
// so fresh clones don't surprise the reader.

#[allow(
    clippy::all,
    dead_code,
    missing_docs,
    unused_imports,
    unreachable_patterns,
    non_camel_case_types
)]
pub mod descriptor {
    // Re-export the buffa crate so `::buffa::` paths in generated code resolve.
    use buffa;
    include!("google.protobuf.descriptor.rs");
}

// Re-export the specific descriptor types referenced via `super::` from the
// compiler module (cross-package references in generated code).
#[allow(unused_imports)]
pub use descriptor::{FileDescriptorProto, GeneratedCodeInfo};

#[allow(
    clippy::all,
    dead_code,
    missing_docs,
    unused_imports,
    unreachable_patterns,
    non_camel_case_types
)]
pub mod compiler {
    // Re-export GeneratedCodeInfo so `super::GeneratedCodeInfo` resolves from
    // nested sub-modules (e.g. `code_generator_response::File`).
    #[allow(unused_imports)]
    pub use crate::generated::descriptor::GeneratedCodeInfo;

    use buffa;
    include!("google.protobuf.compiler.plugin.rs");
}

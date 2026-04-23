//! Generated protobuf descriptor types.
//!
//! These types are generated from `google/protobuf/descriptor.proto` and
//! `google/protobuf/compiler/plugin.proto` using buffa-codegen itself.
//! This makes buffa fully self-hosted — no external protobuf library is
//! needed to decode descriptors — and gives direct access to edition
//! features (`FeatureSet`, `Edition`, etc.).
//!
//! To regenerate, run `task gen-bootstrap-types` from the repo root.

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
    include!("google.protobuf.mod.rs");
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
    include!("google.protobuf.compiler.mod.rs");
}

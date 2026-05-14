//! Generated protobuf descriptor types.
//!
//! These types are generated from `google/protobuf/descriptor.proto` and
//! `google/protobuf/compiler/plugin.proto` using buffa-codegen itself.
//! This makes buffa fully self-hosted — no external protobuf library is
//! needed to decode descriptors — and gives direct access to edition
//! features (`FeatureSet`, `Edition`, etc.).
//!
//! To regenerate, run `task gen-bootstrap-types` from the repo root.
//!
//! The module tree mirrors the proto package nesting
//! (`google.protobuf.compiler` is a child of `google.protobuf`), so the
//! `super::*` cross-package references the codegen emits resolve without
//! re-export workarounds — `compiler::*` reaching for
//! `super::FileDescriptorProto` lands directly in the parent module, and
//! `compiler::__buffa::view::*` reaching for
//! `super::super::super::__buffa::view::FileDescriptorProtoView` does too.
//! The sibling-style `crate::generated::compiler::*` path used by
//! downstream consumers (and by `buffa-codegen`'s file-level extern routing
//! for `google/protobuf/compiler/plugin.proto`) is preserved with a
//! `pub use`.

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

    #[allow(
        clippy::all,
        dead_code,
        missing_docs,
        unused_imports,
        unreachable_patterns,
        non_camel_case_types
    )]
    pub mod compiler {
        use buffa;
        include!("google.protobuf.compiler.mod.rs");
    }
}

/// `google.protobuf.compiler` types — `CodeGeneratorRequest`,
/// `CodeGeneratorResponse`, `Version`. Re-exported here for the stable
/// `::buffa_descriptor::generated::compiler` path; the module proper lives
/// at [`descriptor::compiler`] so `super::*` cross-package references in the
/// generated code resolve through the proto package nesting.
pub use descriptor::compiler;

// Re-export the specific descriptor types referenced from `super::` by the
// compiler package — kept for backward compat with the previous flat module
// layout where `compiler` was a sibling of `descriptor`.
#[allow(unused_imports)]
pub use descriptor::{FileDescriptorProto, GeneratedCodeInfo};

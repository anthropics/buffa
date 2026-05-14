//! Hand-written module tree for `file_per_package=true` output.
//!
//! `buf generate` (see `buf.gen.yaml`) emits one self-contained `.rs` per
//! proto package, named after the dotted package (`example.v1.rs`). Wire
//! each in with a nested `pub mod` matching the package path. The
//! `#![allow(...)]` covers generated-code lints; without it `clippy -D
//! warnings` would flag generated style.
//!
//! The lint list is a hand-maintained copy of `buffa_codegen::ALLOW_LINTS`
//! (the canonical list at `buffa-codegen/src/lib.rs`). If a regen starts
//! firing a new lint under `clippy -D warnings`, re-copy from there. To
//! avoid maintaining this file at all, install `protoc-gen-buffa-packaging`
//! locally and add it as a second plugin in `buf.gen.yaml` (and drop the
//! `file_per_package=true` opt) — it generates the module tree and the
//! `#![allow(...)]` block for you.

#![allow(
    non_camel_case_types,
    dead_code,
    unused_imports,
    unused_qualifications,
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::uninlined_format_args,
    clippy::doc_lazy_continuation,
    clippy::module_inception
)]

pub mod example {
    pub mod v1 {
        include!("example.v1.rs");
    }
}

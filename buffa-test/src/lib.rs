// Wrap generated code in the package module so intra-file type references
// (e.g. `basic::Status`, `basic::Address`) resolve correctly.
//
// The clippy allows suppress lints that fire on generated code patterns:
// - derivable_impls: generated enum Default impls are explicit rather than derived
// - match_single_binding: empty messages generate a single-arm wildcard merge match
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod basic {
    buffa::include_proto!("basic");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto3sem {
    buffa::include_proto!("test.proto3sem");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod keywords {
    buffa::include_proto!("test.keywords");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod nested {
    buffa::include_proto!("test.nested");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod wkt {
    buffa::include_proto!("test.wkt");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross {
    buffa::include_proto!("test.cross");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross_syntax {
    buffa::include_proto!("test.cross_syntax");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod collisions {
    buffa::include_proto!("test.collisions");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod prelude_shadow {
    buffa::include_proto!("test.prelude_shadow");
}

// Nested-package pair, wrapped exactly the way `buffa-build`'s
// `_include.rs` would. The chain of `use super::*;` glob imports makes the
// outer package's `__buffa` reachable from `inner`'s scope, which is the
// only consumer layout where a bare `pub use __buffa::…;` import path is
// E0659-ambiguous against the locally-`include!`d `__buffa`. The natural
// re-exports must use `self::__buffa::…` / `super::__buffa::…` to compile
// here — see gh#80. Compilation is the assertion (`tests/nestpkg.rs` adds a
// type-resolution sanity check).
#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod nestpkg {
    #[allow(unused_imports)]
    use super::*;
    buffa::include_proto!("test.nestpkg");
    pub mod inner {
        #[allow(unused_imports)]
        use super::*;
        buffa::include_proto!("test.nestpkg.inner");
    }
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto2 {
    buffa::include_proto!("test.proto2");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod json_types {
    buffa::include_proto!("test.json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod view_json {
    buffa::include_proto!("test.viewjson");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod view_json_p2 {
    buffa::include_proto!("test.viewjson.p2");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod p2json {
    buffa::include_proto!("test.p2json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod utf8test {
    buffa::include_proto!("utf8test");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::wildcard_in_or_patterns,
    non_camel_case_types,
    dead_code
)]
pub mod edenumjson {
    buffa::include_proto!("test.edenumjson");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod edge {
    buffa::include_proto!("test.edge");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod custopts {
    buffa::include_proto!("buffa.test.options");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod extjson {
    buffa::include_proto!("buffa.test.extjson");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod groupext {
    buffa::include_proto!("buffa.test.groupext");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod msgset {
    buffa::include_proto!("buffa.test.messageset");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod with_setters {
    buffa::include_proto!("test.setters");
}

#[cfg(has_edition_2024)]
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod ed2024 {
    buffa::include_proto!("test.ed2024");
}

// Regression: use_bytes_type() previously produced uncompilable decode code.
// Compiling this module IS the test — if merge_bytes/decode_bytes mismatch
// the bytes::Bytes field type, the build fails.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod basic_bytes {
    include!(concat!(env!("OUT_DIR"), "/bytes_variant/basic.mod.rs"));
}

// Regression #88: bytes_fields + generate_arbitrary(true). Compilation is the
// primary assertion — all four bytes field shapes (singular, optional,
// repeated, oneof variant) must compile with the arbitrary shims in place.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod basic_arbitrary_bytes {
    include!(concat!(env!("OUT_DIR"), "/arbitrary_bytes/basic.mod.rs"));
}

// Views + preserve_unknown_fields=false: covers the else-branches in view
// codegen that omit the unknown-fields view field. Compilation IS the test.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod basic_no_uf {
    include!(concat!(env!("OUT_DIR"), "/no_unknown_views/basic.mod.rs"));
}

// These tests intentionally use the field-assignment style
// (`let mut m = T::default(); m.f = v;`) because it mirrors how protobuf
// messages are constructed in other languages and is what the docs show.
// `3.14` is a test value, not an attempt at PI.
#[allow(
    clippy::field_reassign_with_default,
    clippy::approx_constant,
    clippy::unnecessary_to_owned,
    clippy::assertions_on_constants
)]
#[cfg(test)]
mod tests;

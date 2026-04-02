// Wrap generated code in the package module so intra-file type references
// (e.g. `basic::Status`, `basic::Address`) resolve correctly.
//
// The clippy allows suppress lints that fire on generated code patterns:
// - derivable_impls: generated enum Default impls are explicit rather than derived
// - match_single_binding: empty messages generate a single-arm wildcard merge match
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod basic {
    include!(concat!(env!("OUT_DIR"), "/basic.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto3sem {
    include!(concat!(env!("OUT_DIR"), "/proto3_semantics.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod keywords {
    include!(concat!(env!("OUT_DIR"), "/keywords.rs"));
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod nested {
    include!(concat!(env!("OUT_DIR"), "/nested_deep.rs"));
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod wkt {
    include!(concat!(env!("OUT_DIR"), "/wkt_usage.rs"));
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross {
    include!(concat!(env!("OUT_DIR"), "/cross_package.rs"));
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross_syntax {
    include!(concat!(env!("OUT_DIR"), "/cross_syntax.rs"));
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod collisions {
    include!(concat!(env!("OUT_DIR"), "/name_collisions.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto2 {
    include!(concat!(env!("OUT_DIR"), "/proto2_defaults.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod json_types {
    include!(concat!(env!("OUT_DIR"), "/json_types.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod p2json {
    include!(concat!(env!("OUT_DIR"), "/proto2_json.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod utf8test {
    include!(concat!(env!("OUT_DIR"), "/utf8_validation.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::wildcard_in_or_patterns,
    non_camel_case_types,
    dead_code
)]
pub mod edenumjson {
    include!(concat!(env!("OUT_DIR"), "/editions_enum_json.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod edge {
    include!(concat!(env!("OUT_DIR"), "/edge_cases.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod custopts {
    include!(concat!(env!("OUT_DIR"), "/custom_options.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod extjson {
    include!(concat!(env!("OUT_DIR"), "/ext_json.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod groupext {
    include!(concat!(env!("OUT_DIR"), "/group_ext.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod msgset {
    include!(concat!(env!("OUT_DIR"), "/messageset.rs"));
}

#[cfg(has_edition_2024)]
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod ed2024 {
    include!(concat!(env!("OUT_DIR"), "/editions_2024.rs"));
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
    include!(concat!(env!("OUT_DIR"), "/bytes_variant/basic.rs"));
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
    include!(concat!(env!("OUT_DIR"), "/no_unknown_views/basic.rs"));
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

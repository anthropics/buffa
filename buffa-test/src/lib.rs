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

/// `[debug_redact = true]` — generated Debug impls print a placeholder
/// instead of the annotated field's value.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod debug_redact {
    buffa::include_proto!("debug_redact");
}

/// `string_type(SmolStr)` + vtable reflection — exercises `ReflectElement for
/// SmolStr` on the repeated-string element path.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod vtable_string_repr {
    buffa::include_proto!("vtable_string_repr");
}

/// `generate_views(false)` + vtable reflection — owned-only vtable, no views.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod vtable_no_views {
    buffa::include_proto!("vtable_no_views");
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

/// `lazy_views(true)` — the additive `FooLazyView` decode-on-access family.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod lazyviews {
    buffa::include_proto!("test.lazyviews");
}

/// `lazy_views(true)` + `preserve_unknown_fields(false)` — the lazy decode
/// loop without unknown-field capture, and an all-scalar lazy struct whose
/// lifetime is anchored by `PhantomData`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod lazyviewslean {
    buffa::include_proto!("test.lazyviewslean");
}

// unbox_oneof: `Envelope.body.small` is stored inline (opted out of Box),
// `large` stays boxed. Compiling this module exercises every boxing site for
// both shapes; runtime round-trips live in `tests/unbox_oneof.rs`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod unbox_oneof {
    buffa::include_proto!("unboxoneof");
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
pub mod cross_pertype {
    buffa::include_proto!("test.cross_pertype");
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

// Issue #135: message-nesting module vs sub-package module collision. The
// sub-package `modcollide.oof` is nested under `modcollide` as `pub mod oof`;
// `message Oof`'s nested-types module is deconflicted to `oof_`, so the two no
// longer redefine `mod oof`. Compiling this module is the regression guard.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod modcollide {
    buffa::include_proto!("modcollide");
    pub mod oof {
        buffa::include_proto!("modcollide.oof");
    }
}

// Issue #135, multi-message race: `Oof`/`Oof_` nested modules deconflict to
// `oof__`/`oof___` while sub-packages `oof`/`oof_` keep their names. Compiling
// this nested layout proves the four modules coexist.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod modrace {
    buffa::include_proto!("modrace");
    pub mod oof {
        buffa::include_proto!("modrace.oof");
    }
    pub mod oof_ {
        buffa::include_proto!("modrace.oof_");
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

// Mixed-mode reflection fixtures: bridge-mode dependency, vtable-mode parent
// referencing it via extern_path. See tests/reflect_mixed_mode.rs.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod mixed_reflect_dep {
    buffa::include_proto!("mixedref.dep");
}
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod mixed_reflect_parent {
    buffa::include_proto!("mixedref.parent");
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

// Idiomatic imports (file_per_package): package-root references emitted as
// `use`-backed short names. Compiling this module IS the primary test — the
// `use` directives must resolve, every import must be referenced, and no
// short name may shadow what sibling emissions reference bare. The index
// file reproduces the test::idiomatic / test::idiomatic_other sibling
// nesting the generated `super::` chains assume.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod idiomatic {
    include!(concat!(env!("OUT_DIR"), "/idiomatic_variant/_include.rs"));
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

// type_name_prefix (#46): basic.proto compiled with `.type_name_prefix("Rpc")`
// — every generated type is `Rpc*` (RpcPerson, RpcStatus, RpcPersonView, ...)
// while module names and the wire format stay unchanged. Compilation plus the
// runtime checks in `tests/type_prefix.rs` are the assertion.
// `clippy::manual_map`: the lazy-view oneof `to_owned` conversion always
// emits the match form (unlike the eager view, which uses `.map()` for
// scalar-only groups); basic.proto's bytes/string `choice` oneof is the
// first lazy-compiled proto to hit it. Generator follow-up tracked
// separately.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::manual_map,
    dead_code
)]
pub mod basic_prefixed {
    include!(concat!(env!("OUT_DIR"), "/prefix_variant/basic.mod.rs"));
}

// type_name_prefix + nested messages: nested_deep.proto compiled with
// `.type_name_prefix("Rpc")` and lazy views — the nested view / owned-view /
// lazy-view re-exports must reference the prefixed type names. Compile-only;
// no runtime tests.
#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod nested_prefixed {
    include!(concat!(
        env!("OUT_DIR"),
        "/prefix_nested_variant/test.nested.mod.rs"
    ));
}

// Carve-out (#76): utf8_validation.proto with a NONE-keyed `map<string, bytes>`,
// compiled with strict_utf8_mapping() + use_bytes_type(). The effective
// `map<bytes, bytes>` keeps `Vec<u8>` values; runtime checks live in
// `tests/bytes_type.rs`.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod utf8_bytes {
    include!(concat!(
        env!("OUT_DIR"),
        "/utf8_bytes_variant/utf8test.mod.rs"
    ));
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

// Configurable string_type: SmolStr default + CompactString/EcoString
// overrides, generate_json + arbitrary. Compiling this module exercises every
// string code path against the real crates; runtime checks live in
// `tests/string_type.rs`.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod string_types {
    include!(concat!(
        env!("OUT_DIR"),
        "/string_variant/stringtypes.mod.rs"
    ));
}

// proto2 `[default = "..."]` + string_type. Compiling this verifies the
// generated Default impl and clear() build the literal via the configured
// repr's From<String>.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod string_proto2 {
    include!(concat!(
        env!("OUT_DIR"),
        "/string_proto2_variant/stringproto2.mod.rs"
    ));
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

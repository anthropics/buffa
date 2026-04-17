// Wrap generated code in the package module so intra-file type references
// (e.g. `basic::Status`, `basic::Address`) resolve correctly.
//
// Each `.proto` file produces five sibling `.rs` outputs:
//   - `<stem>.rs`                — owned items (structs, enums, nested modules)
//   - `<stem>.__view.rs`         — view-tree contents (included inside `view::`)
//   - `<stem>.__ext.rs`          — extension-tree contents (included inside `ext::`)
//   - `<stem>.__oneofs.rs`       — owned oneof enums (inside `oneofs::`)
//   - `<stem>.__view_oneofs.rs`  — view oneof enums (inside `view::oneofs::`)
//
// Each pub mod below mirrors that layout: `include!` the owned file at
// the package root, and stitch the ancillary siblings inside
// `pub mod view { … pub mod oneofs { … } }`, `pub mod ext { … }`, and
// `pub mod oneofs { … }` per package.
//
// The clippy allows suppress lints that fire on generated code patterns:
// - derivable_impls: generated enum Default impls are explicit rather than derived
// - match_single_binding: empty messages generate a single-arm wildcard merge match

macro_rules! include_generated {
    ($stem:literal) => {
        include!(concat!(env!("OUT_DIR"), "/", $stem, ".rs"));
        #[allow(
            clippy::derivable_impls,
            clippy::match_single_binding,
            clippy::wildcard_in_or_patterns,
            non_camel_case_types,
            dead_code,
            unused_imports
        )]
        pub mod view {
            include!(concat!(env!("OUT_DIR"), "/", $stem, ".__view.rs"));
            #[allow(
                clippy::derivable_impls,
                clippy::match_single_binding,
                clippy::wildcard_in_or_patterns,
                non_camel_case_types,
                dead_code,
                unused_imports
            )]
            pub mod oneofs {
                include!(concat!(env!("OUT_DIR"), "/", $stem, ".__view_oneofs.rs"));
            }
        }
        #[allow(
            clippy::derivable_impls,
            clippy::match_single_binding,
            non_camel_case_types,
            dead_code,
            unused_imports
        )]
        pub mod ext {
            include!(concat!(env!("OUT_DIR"), "/", $stem, ".__ext.rs"));
        }
        #[allow(
            clippy::derivable_impls,
            clippy::match_single_binding,
            clippy::wildcard_in_or_patterns,
            non_camel_case_types,
            dead_code,
            unused_imports
        )]
        pub mod oneofs {
            include!(concat!(env!("OUT_DIR"), "/", $stem, ".__oneofs.rs"));
        }
    };
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod basic {
    include_generated!("basic");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto3sem {
    include_generated!("proto3_semantics");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod keywords {
    include_generated!("keywords");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod nested {
    include_generated!("nested_deep");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod wkt {
    include_generated!("wkt_usage");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross {
    include_generated!("cross_package");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross_syntax {
    include_generated!("cross_syntax");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod collisions {
    include_generated!("name_collisions");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod prelude_shadow {
    include!(concat!(env!("OUT_DIR"), "/prelude_shadow.rs"));
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto2 {
    include_generated!("proto2_defaults");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod json_types {
    include_generated!("json_types");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod p2json {
    include_generated!("proto2_json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod utf8test {
    include_generated!("utf8_validation");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::wildcard_in_or_patterns,
    non_camel_case_types,
    dead_code
)]
pub mod edenumjson {
    include_generated!("editions_enum_json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod edge {
    include_generated!("edge_cases");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod custopts {
    include_generated!("custom_options");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod extjson {
    include_generated!("ext_json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod groupext {
    include_generated!("group_ext");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod msgset {
    include_generated!("messageset");
}

#[cfg(has_edition_2024)]
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod ed2024 {
    include_generated!("editions_2024");
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
    #[allow(
        clippy::derivable_impls,
        clippy::match_single_binding,
        non_camel_case_types,
        dead_code,
        unused_imports
    )]
    pub mod view {
        include!(concat!(env!("OUT_DIR"), "/bytes_variant/basic.__view.rs"));
        #[allow(
            clippy::derivable_impls,
            clippy::match_single_binding,
            non_camel_case_types,
            dead_code,
            unused_imports
        )]
        pub mod oneofs {
            include!(concat!(
                env!("OUT_DIR"),
                "/bytes_variant/basic.__view_oneofs.rs"
            ));
        }
    }
    #[allow(
        clippy::derivable_impls,
        clippy::match_single_binding,
        non_camel_case_types,
        dead_code,
        unused_imports
    )]
    pub mod ext {
        include!(concat!(env!("OUT_DIR"), "/bytes_variant/basic.__ext.rs"));
    }
    #[allow(
        clippy::derivable_impls,
        clippy::match_single_binding,
        non_camel_case_types,
        dead_code,
        unused_imports
    )]
    pub mod oneofs {
        include!(concat!(env!("OUT_DIR"), "/bytes_variant/basic.__oneofs.rs"));
    }
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
    #[allow(
        clippy::derivable_impls,
        clippy::match_single_binding,
        non_camel_case_types,
        dead_code,
        unused_imports
    )]
    pub mod view {
        include!(concat!(
            env!("OUT_DIR"),
            "/no_unknown_views/basic.__view.rs"
        ));
        #[allow(
            clippy::derivable_impls,
            clippy::match_single_binding,
            non_camel_case_types,
            dead_code,
            unused_imports
        )]
        pub mod oneofs {
            include!(concat!(
                env!("OUT_DIR"),
                "/no_unknown_views/basic.__view_oneofs.rs"
            ));
        }
    }
    #[allow(
        clippy::derivable_impls,
        clippy::match_single_binding,
        non_camel_case_types,
        dead_code,
        unused_imports
    )]
    pub mod ext {
        include!(concat!(env!("OUT_DIR"), "/no_unknown_views/basic.__ext.rs"));
    }
    #[allow(
        clippy::derivable_impls,
        clippy::match_single_binding,
        non_camel_case_types,
        dead_code,
        unused_imports
    )]
    pub mod oneofs {
        include!(concat!(
            env!("OUT_DIR"),
            "/no_unknown_views/basic.__oneofs.rs"
        ));
    }
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

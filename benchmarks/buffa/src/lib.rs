//! Generated protobuf types for buffa benchmarks.

macro_rules! include_generated {
    ($stem:literal) => {
        include!(concat!(env!("OUT_DIR"), "/", $stem, ".rs"));
        #[allow(non_camel_case_types, unused_imports, dead_code)]
        pub mod view {
            include!(concat!(env!("OUT_DIR"), "/", $stem, ".__view.rs"));
            #[allow(non_camel_case_types, unused_imports, dead_code)]
            pub mod oneofs {
                include!(concat!(env!("OUT_DIR"), "/", $stem, ".__view_oneofs.rs"));
            }
        }
        #[allow(non_camel_case_types, unused_imports, dead_code)]
        pub mod ext {
            include!(concat!(env!("OUT_DIR"), "/", $stem, ".__ext.rs"));
        }
        #[allow(non_camel_case_types, unused_imports, dead_code)]
        pub mod oneofs {
            include!(concat!(env!("OUT_DIR"), "/", $stem, ".__oneofs.rs"));
        }
    };
}

#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
pub mod bench {
    include_generated!("bench_messages");
}

#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
pub mod benchmarks {
    include_generated!("benchmarks");
}

#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
pub mod proto3 {
    include_generated!("benchmark_message1_proto3");
}

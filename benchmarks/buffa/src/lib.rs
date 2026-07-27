//! Generated protobuf types for buffa benchmarks.
//!
//! Built per-message-isolated: `--no-default-features --features iso,<msg>`
//! emits only that message's codec (used by the per-message bench targets); the
//! default feature set emits all messages plus reflect + lazy views for the
//! combined `protobuf`/`reflect` benches.

#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "analytics_owned_types",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh",
    feature = "column_batch"
))]
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
    buffa::include_proto!("bench");
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
    buffa::include_proto!("benchmarks");
}

#[cfg(feature = "google_message1")]
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
    buffa::include_proto!("benchmarks.proto3");
}

#[cfg(feature = "analytics_owned_types")]
macro_rules! small_list {
    ($(#[$meta:meta])* $name:ident, $capacity:literal) => {
        $(#[$meta])*
        #[allow(clippy::derive_partial_eq_without_eq)]
        #[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
        #[repr(transparent)]
        #[serde(transparent)]
        pub struct $name<T>(pub smallvec::SmallVec<[T; $capacity]>);

        impl<T> Default for $name<T> {
            #[inline]
            fn default() -> Self {
                Self(smallvec::SmallVec::new())
            }
        }

        impl<T> core::ops::Deref for $name<T> {
            type Target = [T];

            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T> FromIterator<T> for $name<T> {
            #[inline]
            fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
                Self(smallvec::SmallVec::from_iter(iter))
            }
        }

        impl<T> From<Vec<T>> for $name<T> {
            #[inline]
            fn from(value: Vec<T>) -> Self {
                Self(smallvec::SmallVec::from_vec(value))
            }
        }

        impl<T> buffa::ProtoList<T> for $name<T>
        where
            T: Clone + PartialEq + core::fmt::Debug + Send + Sync,
        {
            #[inline]
            fn push(&mut self, value: T) {
                self.0.push(value);
            }

            #[inline]
            fn clear(&mut self) {
                self.0.clear();
            }
        }
    };
}

#[cfg(feature = "analytics_owned_types")]
// ponytail: benchmark-only wrappers share one implementation; use const
// generics if generated custom-type paths gain const-argument support.
small_list!(
    /// Four-element inline list used by the uniform owned-type benchmark.
    SmallList,
    4
);

#[cfg(feature = "analytics_owned_types")]
small_list!(
    /// Eight-element inline list used only for small-element repeated fields.
    SmallList8,
    8
);

#[cfg(feature = "analytics_owned_types")]
macro_rules! assert_transparent {
    ($outer:ty, $inner:ty) => {
        const _: () = {
            assert!(core::mem::size_of::<$outer>() == core::mem::size_of::<$inner>());
            assert!(core::mem::align_of::<$outer>() == core::mem::align_of::<$inner>());
        };
    };
}

#[cfg(feature = "analytics_owned_types")]
assert_transparent!(SmallList<u32>, smallvec::SmallVec<[u32; 4]>);
#[cfg(feature = "analytics_owned_types")]
assert_transparent!(SmallList8<u32>, smallvec::SmallVec<[u32; 8]>);

#[cfg(feature = "analytics_owned_types")]
pub mod analytics_smolstr {
    include!(concat!(env!("OUT_DIR"), "/analytics_smolstr/bench.mod.rs"));
}

#[cfg(feature = "analytics_owned_types")]
pub mod analytics_smallvec {
    include!(concat!(env!("OUT_DIR"), "/analytics_smallvec/bench.mod.rs"));
}

#[cfg(feature = "analytics_owned_types")]
pub mod analytics_smolstr_smallvec {
    include!(concat!(
        env!("OUT_DIR"),
        "/analytics_smolstr_smallvec/bench.mod.rs"
    ));
}

#[cfg(feature = "analytics_owned_types")]
pub mod analytics_selective_smallvec {
    include!(concat!(
        env!("OUT_DIR"),
        "/analytics_selective_smallvec/bench.mod.rs"
    ));
}

#[cfg(all(test, feature = "analytics_owned_types"))]
mod owned_type_tests {
    use buffa::Message;

    use super::{
        analytics_selective_smallvec, analytics_smallvec, analytics_smolstr,
        analytics_smolstr_smallvec, bench::AnalyticsEvent, benchmarks::BenchmarkDataset,
    };

    fn assert_small_list_shape(event: &analytics_smallvec::AnalyticsEvent) {
        let _: &super::SmallList<_> = &event.properties;
        let _: &super::SmallList<_> = &event.sections;
        if let Some(section) = event.sections.first() {
            let _: &super::SmallList<_> = &section.attributes;
            let _: &Vec<_> = &section.children;
        }
    }

    fn assert_selective_small_list_shape(event: &analytics_selective_smallvec::AnalyticsEvent) {
        let _: &super::SmallList8<_> = &event.properties;
        let _: &Vec<_> = &event.sections;
        if let Some(section) = event.sections.first() {
            let _: &super::SmallList8<_> = &section.attributes;
            let _: &Vec<_> = &section.children;
        }
    }

    #[test]
    fn analytics_owned_type_variants_preserve_wire_semantics() {
        let dataset = BenchmarkDataset::decode_from_slice(include_bytes!(
            "../../datasets/analytics_event.pb"
        ))
        .unwrap();

        for payload in dataset.payload {
            let expected = AnalyticsEvent::decode_from_slice(&payload).unwrap();

            let smol = analytics_smolstr::AnalyticsEvent::decode_from_slice(&payload).unwrap();
            let _: &buffa_smolstr::SmolStr = &smol.event_id;
            assert_eq!(
                AnalyticsEvent::decode_from_slice(&smol.encode_to_vec()).unwrap(),
                expected
            );

            let small = analytics_smallvec::AnalyticsEvent::decode_from_slice(&payload).unwrap();
            assert_small_list_shape(&small);
            assert_eq!(
                AnalyticsEvent::decode_from_slice(&small.encode_to_vec()).unwrap(),
                expected
            );

            let combined =
                analytics_smolstr_smallvec::AnalyticsEvent::decode_from_slice(&payload).unwrap();
            let _: &buffa_smolstr::SmolStr = &combined.event_id;
            let _: &super::SmallList<_> = &combined.properties;
            assert_eq!(
                AnalyticsEvent::decode_from_slice(&combined.encode_to_vec()).unwrap(),
                expected
            );

            let selective =
                analytics_selective_smallvec::AnalyticsEvent::decode_from_slice(&payload).unwrap();
            assert_selective_small_list_shape(&selective);
            assert_eq!(
                AnalyticsEvent::decode_from_slice(&selective.encode_to_vec()).unwrap(),
                expected
            );
        }
    }
}

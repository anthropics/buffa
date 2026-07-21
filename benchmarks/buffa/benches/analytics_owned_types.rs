// Owned-type comparison for `AnalyticsEvent`, kept separate from the isolated
// `analytics_event` history target so its binary layout remains comparable.
// Run with `--no-default-features --features iso,analytics_owned_types`.
//
// `smallvec4` measures one specific configuration, and that configuration is a
// poor fit for this data. Read it as such, not as a verdict on SmallVec.
//
// Inlining four elements costs `4 * size_of::<Element>()` in every enclosing
// message, whether or not the elements are there, and the two list fields have
// very different element sizes:
//
//     field       element   size   list length (median/max)   fits inline-4
//     properties  Property    72                    6.5/10             28%
//     sections    Nested     376                      4/5              72%
//
// (list lengths over the 50 payloads in analytics_event.pb.)
//
// So the configuration inlines the wrong field twice over. `sections` holds
// 376-byte `Nested` values, so its inline buffer alone is 4 * 376 + 16 = 1520
// bytes — about 79% of why `size_of::<AnalyticsEvent>()` goes from 128 bytes
// to 1904, a 14.9x blowup that every decode pays. Meanwhile `properties`, the
// field whose 72-byte elements are actually cheap to inline, is under-sized
// and spills to the heap roughly 72% of the time, where it costs a `Vec` plus
// a dead inline buffer.
//
// The useful follow-up is therefore NOT a uniformly larger capacity — inline-8
// would push `Nested` past 600 bytes and the message past 5 KB. It is to apply
// `SmallList` only to the small-element fields, sized to their length
// distribution, and leave message-typed lists on `Vec`. Tracked as a follow-up
// to #215.
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh",
    feature = "google_message1",
    feature = "column_batch",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("`analytics_owned_types` bench requires --no-default-features: another message/reflect/lazy feature is enabled");
include!("common.rs");
use bench_buffa::bench::AnalyticsEvent;
use bench_buffa::{analytics_smallvec, analytics_smolstr, analytics_smolstr_smallvec};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/analytics_event.pb");
    benchmark_decode::<AnalyticsEvent>(c, "buffa/analytics_owned_types/default", data);
    benchmark_json::<AnalyticsEvent>(c, "buffa/analytics_owned_types/default", data);
    benchmark_decode::<analytics_smolstr::AnalyticsEvent>(
        c,
        "buffa/analytics_owned_types/smolstr",
        data,
    );
    benchmark_json::<analytics_smolstr::AnalyticsEvent>(
        c,
        "buffa/analytics_owned_types/smolstr",
        data,
    );
    benchmark_decode::<analytics_smallvec::AnalyticsEvent>(
        c,
        "buffa/analytics_owned_types/smallvec4",
        data,
    );
    benchmark_json::<analytics_smallvec::AnalyticsEvent>(
        c,
        "buffa/analytics_owned_types/smallvec4",
        data,
    );
    benchmark_decode::<analytics_smolstr_smallvec::AnalyticsEvent>(
        c,
        "buffa/analytics_owned_types/smolstr_smallvec4",
        data,
    );
    benchmark_json::<analytics_smolstr_smallvec::AnalyticsEvent>(
        c,
        "buffa/analytics_owned_types/smolstr_smallvec4",
        data,
    );
}

criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

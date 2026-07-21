// Owned-type comparison for `AnalyticsEvent`, kept separate from the isolated
// `analytics_event` history target so its binary layout remains comparable.
// Run with `--no-default-features --features iso,analytics_owned_types`.
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

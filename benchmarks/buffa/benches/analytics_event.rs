// Isolated benchmark for the `analytics_event` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,analytics_event` (or `task bench-iso -- analytics_event`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh",
    feature = "google_message1",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `analytics_event` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::AnalyticsEventView, AnalyticsEvent};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/analytics_event.pb");
    benchmark_decode::<AnalyticsEvent>(c, "buffa/analytics_event", data);
    benchmark_json::<AnalyticsEvent>(c, "buffa/analytics_event", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/analytics_event");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(AnalyticsEventView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

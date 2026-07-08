// Isolated benchmark for the `google_message1` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,google_message1` (or `task bench-iso -- google_message1`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `google_message1` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::proto3::{__buffa::view::GoogleMessage1View, GoogleMessage1};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/google_message1_proto3.pb");
    benchmark_decode::<GoogleMessage1>(c, "buffa/google_message1_proto3", data);
    benchmark_json::<GoogleMessage1>(c, "buffa/google_message1_proto3", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/google_message1_proto3");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(GoogleMessage1View::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

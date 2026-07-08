// Isolated benchmark for the `api_response` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,api_response` (or `task bench-iso -- api_response`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "log_record",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh",
    feature = "google_message1",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `api_response` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::ApiResponseView, ApiResponse};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/api_response.pb");
    benchmark_decode::<ApiResponse>(c, "buffa/api_response", data);
    benchmark_json::<ApiResponse>(c, "buffa/api_response", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/api_response");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(ApiResponseView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

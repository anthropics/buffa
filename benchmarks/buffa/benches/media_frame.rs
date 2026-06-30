// Isolated benchmark for the `media_frame` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,media_frame` (or `task bench-iso -- media_frame`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "packed_tile",
    feature = "mesh",
    feature = "google_message1",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `media_frame` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::MediaFrameView, MediaFrame};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/media_frame.pb");
    benchmark_decode::<MediaFrame>(c, "buffa/media_frame", data);
    benchmark_json::<MediaFrame>(c, "buffa/media_frame", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/media_frame");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(MediaFrameView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

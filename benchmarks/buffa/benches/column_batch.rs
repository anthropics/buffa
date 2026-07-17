// Isolated benchmark for the `column_batch` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,column_batch` (or `task bench-iso -- column_batch`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "google_message1",
    feature = "mesh",
    feature = "packed_tile",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `column_batch` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::ColumnBatchView, ColumnBatch};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/column_batch.pb");
    benchmark_decode::<ColumnBatch>(c, "buffa/column_batch", data);
    benchmark_json::<ColumnBatch>(c, "buffa/column_batch", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/column_batch");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(ColumnBatchView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

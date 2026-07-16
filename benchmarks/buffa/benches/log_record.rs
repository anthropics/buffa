// Isolated benchmark for the `log_record` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,log_record` (or `task bench-iso -- log_record`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "mesh",
    feature = "google_message1",
    feature = "column_batch",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `log_record` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::LogRecordView, LogRecord};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/log_record.pb");
    benchmark_decode::<LogRecord>(c, "buffa/log_record", data);
    benchmark_json::<LogRecord>(c, "buffa/log_record", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/log_record");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(LogRecordView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

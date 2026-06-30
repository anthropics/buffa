// Isolated benchmark for the `packed_tile` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,packed_tile` (or `task bench-iso -- packed_tile`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "google_message1",
    feature = "mesh",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `packed_tile` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::PackedTileView, PackedTile};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/packed_tile.pb");
    benchmark_decode::<PackedTile>(c, "buffa/packed_tile", data);
    benchmark_json::<PackedTile>(c, "buffa/packed_tile", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/packed_tile");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(PackedTileView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

// Isolated benchmark for the `mesh` message: only its own decoder is compiled.
// Run with `--no-default-features --features iso,mesh` (or `task bench-iso -- mesh`).
// Guard: isolation is lost if another message (or reflect/lazy) is compiled in,
// which happens if you forget --no-default-features (the default set enables all).
#[cfg(any(
    feature = "api_response",
    feature = "log_record",
    feature = "analytics_event",
    feature = "media_frame",
    feature = "packed_tile",
    feature = "google_message1",
    feature = "reflect",
    feature = "lazy"
))]
compile_error!("isolated `mesh` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::TriMeshView, TriMesh};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/mesh.pb");
    benchmark_decode::<TriMesh>(c, "buffa/mesh", data);
    benchmark_json::<TriMesh>(c, "buffa/mesh", data);
    let ds = load_dataset(data);
    let bytes = total_payload_bytes(&ds);
    let mut g = c.benchmark_group("buffa/mesh");
    g.throughput(Throughput::Bytes(bytes));
    g.bench_function("decode_view", |b| {
        b.iter(|| {
            for p in &ds.payload {
                criterion::black_box(TriMeshView::decode_view(p).unwrap());
            }
        })
    });
    g.finish();
}
criterion::criterion_group!(grp, run);
criterion::criterion_main!(grp);

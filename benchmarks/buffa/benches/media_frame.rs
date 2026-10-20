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
    feature = "column_batch",
    feature = "reflect",
    feature = "lazy",
    feature = "analytics_owned_types"
))]
compile_error!("isolated `media_frame` bench requires --no-default-features: another message/reflect/lazy feature is enabled, which defeats per-message isolation");
include!("common.rs");
use bench_buffa::bench::{__buffa::view::MediaFrameView, MediaFrame};

fn run(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/media_frame.pb");
    benchmark_decode::<MediaFrame>(c, "buffa/media_frame", data);
    benchmark_json::<MediaFrame>(c, "buffa/media_frame", data);
    let ds = load_dataset(data);
    let many_small = MediaFrame {
        chunks: vec![vec![0xfb]; 1024],
        ..MediaFrame::default()
    };
    let many_small_standard = serde_json::to_string(&many_small).unwrap();
    assert_eq!(many_small_standard.matches("+w==").count(), 1024);
    let many_small_url_safe = many_small_standard.replace("+w==", "-w==");
    assert_eq!(many_small_standard.len(), many_small_url_safe.len());
    let decoded: MediaFrame = serde_json::from_str(&many_small_url_safe).unwrap();
    assert_eq!(decoded.chunks, many_small.chunks);
    let mut small_group =
        c.benchmark_group("buffa/media_frame/protojson_bytes_fallback/many_small_chunks");
    small_group.throughput(Throughput::Elements(many_small.chunks.len() as u64));
    small_group.bench_function("standard_base64", |b| {
        b.iter(|| {
            criterion::black_box(serde_json::from_str::<MediaFrame>(&many_small_standard).unwrap());
        })
    });
    small_group.bench_function("url_safe_padded", |b| {
        b.iter(|| {
            criterion::black_box(serde_json::from_str::<MediaFrame>(&many_small_url_safe).unwrap());
        })
    });
    small_group.finish();

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

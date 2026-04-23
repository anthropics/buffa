use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use prost::Message;

use bench_prost_bytes::bench::*;
use bench_prost_bytes::benchmarks::BenchmarkDataset;

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode(data).expect("failed to decode dataset")
}

fn total_payload_bytes(dataset: &BenchmarkDataset) -> u64 {
    dataset.payload.iter().map(|p| p.len() as u64).sum()
}

fn benchmark_decode<M: Message + Default>(c: &mut Criterion, name: &str, dataset_bytes: &[u8]) {
    let dataset = load_dataset(dataset_bytes);
    let bytes_total = total_payload_bytes(&dataset);

    // Pre-wrap every payload in a `bytes::Bytes` so the decoder sees a `Buf`
    // whose `copy_to_bytes` is a zero-copy ref-count slice. Prost's `&[u8]`
    // decode path copies into a fresh `Vec<u8>` / `Bytes` for each `bytes`
    // field; the `Bytes`-input path shares the input buffer by refcount.
    // `Bytes::copy_from_slice` is outside `b.iter()` so its allocation does
    // not pollute the measured decode cost; `payload.clone()` inside the
    // hot loop is a cheap atomic refcount bump. On schemas with no or few
    // `bytes` fields, that per-message refcount work can become a
    // meaningful fraction of decode cost — it's the reason this variant
    // can appear slightly slower than default `prost` on `ApiResponse` /
    // `LogRecord` / etc. where there's nothing to zero-copy.
    let payloads: Vec<Bytes> = dataset
        .payload
        .iter()
        .map(|p| Bytes::copy_from_slice(p))
        .collect();

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes_total));

    // Only `decode` / `merge` are measured here: `prost-build`'s
    // `.bytes(["."])` substitution affects only the decode path. The
    // `encode` and `encoded_len` numbers are expected to track default
    // `prost` within noise — see `benchmarks/prost/` for those benches.
    group.bench_function("decode", |b| {
        b.iter(|| {
            for payload in &payloads {
                let msg = M::decode(payload.clone()).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.bench_function("merge", |b| {
        let mut msg = M::default();
        b.iter(|| {
            for payload in &payloads {
                msg.clear();
                msg.merge(payload.clone()).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.finish();
}

fn bench_api_response(c: &mut Criterion) {
    benchmark_decode::<ApiResponse>(
        c,
        "prost-bytes/api_response",
        include_bytes!("../../datasets/api_response.pb"),
    );
}

fn bench_log_record(c: &mut Criterion) {
    benchmark_decode::<LogRecord>(
        c,
        "prost-bytes/log_record",
        include_bytes!("../../datasets/log_record.pb"),
    );
}

fn bench_analytics_event(c: &mut Criterion) {
    benchmark_decode::<AnalyticsEvent>(
        c,
        "prost-bytes/analytics_event",
        include_bytes!("../../datasets/analytics_event.pb"),
    );
}

fn bench_google_message1(c: &mut Criterion) {
    benchmark_decode::<bench_prost_bytes::benchmarks::proto3::GoogleMessage1>(
        c,
        "prost-bytes/google_message1_proto3",
        include_bytes!("../../datasets/google_message1_proto3.pb"),
    );
}

fn bench_media_frame(c: &mut Criterion) {
    benchmark_decode::<MediaFrame>(
        c,
        "prost-bytes/media_frame",
        include_bytes!("../../datasets/media_frame.pb"),
    );
}

criterion_group!(
    benches,
    bench_api_response,
    bench_log_record,
    bench_analytics_event,
    bench_google_message1,
    bench_media_frame,
);

criterion_main!(benches);

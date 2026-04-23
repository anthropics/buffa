use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use protobuf::{Parse, Serialize};

use bench_google::*;

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::parse(data).expect("failed to decode dataset")
}

fn total_payload_bytes(dataset: &BenchmarkDataset) -> u64 {
    dataset.payload().iter().map(|p| p.len() as u64).sum()
}

fn benchmark_decode<M: Parse + Serialize + Default>(
    c: &mut Criterion,
    name: &str,
    dataset_bytes: &[u8],
) {
    let dataset = load_dataset(dataset_bytes);
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode", |b| {
        b.iter(|| {
            for payload in dataset.payload() {
                let msg = M::parse(payload).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.bench_function("encode", |b| {
        let messages: Vec<M> = dataset
            .payload()
            .iter()
            .map(|p| M::parse(p).unwrap())
            .collect();
        b.iter(|| {
            for msg in &messages {
                let encoded = msg.serialize().unwrap();
                criterion::black_box(&encoded);
            }
        });
    });

    group.finish();
}

fn bench_api_response(c: &mut Criterion) {
    benchmark_decode::<ApiResponse>(
        c,
        "google/api_response",
        include_bytes!("../../datasets/api_response.pb"),
    );
}

fn bench_log_record(c: &mut Criterion) {
    benchmark_decode::<LogRecord>(
        c,
        "google/log_record",
        include_bytes!("../../datasets/log_record.pb"),
    );
}

fn bench_analytics_event(c: &mut Criterion) {
    benchmark_decode::<AnalyticsEvent>(
        c,
        "google/analytics_event",
        include_bytes!("../../datasets/analytics_event.pb"),
    );
}

fn bench_google_message1(c: &mut Criterion) {
    benchmark_decode::<GoogleMessage1>(
        c,
        "google/google_message1_proto3",
        include_bytes!("../../datasets/google_message1_proto3.pb"),
    );
}

fn bench_media_frame(c: &mut Criterion) {
    benchmark_decode::<MediaFrame>(
        c,
        "google/media_frame",
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

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use prost::Message;
use serde::{de::DeserializeOwned, Serialize};

use bench_prost::bench::*;
use bench_prost::benchmarks::BenchmarkDataset;

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode(data).expect("failed to decode dataset")
}

fn total_payload_bytes(dataset: &BenchmarkDataset) -> u64 {
    dataset.payload.iter().map(|p| p.len() as u64).sum()
}

fn benchmark_decode<M: Message + Default>(c: &mut Criterion, name: &str, dataset_bytes: &[u8]) {
    let dataset = load_dataset(dataset_bytes);
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let msg = M::decode(payload.as_slice()).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.bench_function("merge", |b| {
        let mut msg = M::default();
        b.iter(|| {
            for payload in &dataset.payload {
                msg.clear();
                msg.merge(payload.as_slice()).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.bench_function("encode", |b| {
        let messages: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode(p.as_slice()).unwrap())
            .collect();
        b.iter(|| {
            for msg in &messages {
                let encoded = msg.encode_to_vec();
                criterion::black_box(&encoded);
            }
        });
    });

    group.bench_function("encoded_len", |b| {
        let messages: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode(p.as_slice()).unwrap())
            .collect();
        b.iter(|| {
            for msg in &messages {
                let size = msg.encoded_len();
                criterion::black_box(size);
            }
        });
    });

    group.finish();
}

fn benchmark_json<M: Message + Default + Serialize + DeserializeOwned>(
    c: &mut Criterion,
    name: &str,
    dataset_bytes: &[u8],
) {
    let dataset = load_dataset(dataset_bytes);

    // Pre-decode binary payloads to owned messages.
    let messages: Vec<M> = dataset
        .payload
        .iter()
        .map(|p| M::decode(p.as_slice()).unwrap())
        .collect();

    // Pre-encode messages to JSON strings for decode benchmarks.
    let json_strings: Vec<String> = messages
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect();

    let json_bytes: u64 = json_strings.iter().map(|s| s.len() as u64).sum();

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(json_bytes));

    group.bench_function("json_encode", |b| {
        b.iter(|| {
            for msg in &messages {
                let json = serde_json::to_string(msg).unwrap();
                criterion::black_box(&json);
            }
        });
    });

    group.bench_function("json_decode", |b| {
        b.iter(|| {
            for json in &json_strings {
                let msg: M = serde_json::from_str(json).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.finish();
}

fn bench_api_response(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/api_response.pb");
    benchmark_decode::<ApiResponse>(c, "prost/api_response", data);
    benchmark_json::<ApiResponse>(c, "prost/api_response", data);
}

fn bench_log_record(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/log_record.pb");
    benchmark_decode::<LogRecord>(c, "prost/log_record", data);
    benchmark_json::<LogRecord>(c, "prost/log_record", data);
}

fn bench_analytics_event(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/analytics_event.pb");
    benchmark_decode::<AnalyticsEvent>(c, "prost/analytics_event", data);
    benchmark_json::<AnalyticsEvent>(c, "prost/analytics_event", data);
}

fn bench_google_message1(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/google_message1_proto3.pb");
    benchmark_decode::<bench_prost::benchmarks::proto3::GoogleMessage1>(
        c,
        "prost/google_message1_proto3",
        data,
    );
    benchmark_json::<bench_prost::benchmarks::proto3::GoogleMessage1>(
        c,
        "prost/google_message1_proto3",
        data,
    );
}

fn bench_media_frame(c: &mut Criterion) {
    let data = include_bytes!("../../datasets/media_frame.pb");
    benchmark_decode::<MediaFrame>(c, "prost/media_frame", data);
    benchmark_json::<MediaFrame>(c, "prost/media_frame", data);
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

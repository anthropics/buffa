// Shared helpers, textually included by each per-message isolated bench target.
use buffa::{Message, MessageView};
use criterion::{Criterion, Throughput};
use serde::{de::DeserializeOwned, Serialize};

use bench_buffa::benchmarks::BenchmarkDataset;

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode_from_slice(data).expect("failed to decode dataset")
}
fn total_payload_bytes(d: &BenchmarkDataset) -> u64 {
    d.payload.iter().map(|p| p.len() as u64).sum()
}

fn benchmark_decode<M: Message + Default>(c: &mut Criterion, name: &str, data: &[u8]) {
    let dataset = load_dataset(data);
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("decode", |b| {
        b.iter(|| {
            for p in &dataset.payload {
                criterion::black_box(M::decode_from_slice(p).unwrap());
            }
        })
    });
    group.bench_function("merge", |b| {
        let mut msg = M::default();
        b.iter(|| {
            for p in &dataset.payload {
                msg.clear();
                msg.merge_from_slice(p).unwrap();
                criterion::black_box(&msg);
            }
        })
    });
    group.bench_function("encode", |b| {
        let msgs: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode_from_slice(p).unwrap())
            .collect();
        b.iter(|| {
            for m in &msgs {
                criterion::black_box(m.encode_to_vec());
            }
        })
    });
    group.bench_function("compute_size", |b| {
        let msgs: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode_from_slice(p).unwrap())
            .collect();
        let mut cache = buffa::SizeCache::new();
        b.iter(|| {
            for m in &msgs {
                cache.clear();
                criterion::black_box(m.compute_size(&mut cache));
            }
        })
    });
    group.finish();
}

fn benchmark_json<M: Message + Default + Serialize + DeserializeOwned>(
    c: &mut Criterion,
    name: &str,
    data: &[u8],
) {
    let dataset = load_dataset(data);
    let messages: Vec<M> = dataset
        .payload
        .iter()
        .map(|p| M::decode_from_slice(p).unwrap())
        .collect();
    let json_strings: Vec<String> = messages
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect();
    let json_bytes: u64 = json_strings.iter().map(|s| s.len() as u64).sum();
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(json_bytes));
    group.bench_function("json_encode", |b| {
        b.iter(|| {
            for m in &messages {
                criterion::black_box(serde_json::to_string(m).unwrap());
            }
        })
    });
    group.bench_function("json_decode", |b| {
        b.iter(|| {
            for j in &json_strings {
                let m: M = serde_json::from_str(j).unwrap();
                criterion::black_box(m);
            }
        })
    });
    group.finish();
}

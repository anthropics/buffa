//! Benchmark `Any::clone()` across payload sizes.
//!
//! `Any` is commonly used as a cached carrier for encoded messages that get
//! cloned into `repeated google.protobuf.Any` response fields. With
//! `value: bytes::Bytes` the clone is a refcount bump (constant time) rather
//! than a payload memcpy (linear in `value.len()`).

use buffa_types::google::protobuf::Any;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn make_any(payload_len: usize) -> Any {
    Any {
        type_url: "type.googleapis.com/example.v1.Payload".into(),
        value: vec![0xAB; payload_len].into(),
        ..Default::default()
    }
}

fn bench_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("any_clone");
    for &len in &[64usize, 1024, 16 * 1024, 256 * 1024] {
        let any = make_any(len);
        group.throughput(Throughput::Bytes(len as u64));
        group.bench_with_input(BenchmarkId::from_parameter(len), &any, |b, any| {
            b.iter(|| black_box(any.clone()));
        });
    }
    group.finish();
}

fn bench_clone_into_vec(c: &mut Criterion) {
    let mut group = c.benchmark_group("any_clone_into_vec");
    for &(n, len) in &[(100usize, 1024usize), (1000, 1024)] {
        let src: Vec<Any> = (0..n).map(|_| make_any(len)).collect();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new(format!("{len}B"), n), &src, |b, src| {
            b.iter(|| {
                let out: Vec<Any> = src.to_vec();
                black_box(out);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_clone, bench_clone_into_vec);
criterion_main!(benches);

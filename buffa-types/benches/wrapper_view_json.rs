use buffa_types::google::protobuf::__buffa::view::{BytesValueView, StringValueView};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn bench_string_view_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_value_view_json");
    for len in [64usize, 1024, 16 * 1024, 256 * 1024] {
        let payload = "a".repeat(len);
        let view = StringValueView {
            value: &payload,
            ..Default::default()
        };
        let mut output = Vec::with_capacity(len + 2);
        group.throughput(Throughput::Bytes(len as u64));
        group.bench_function(BenchmarkId::from_parameter(len), |b| {
            b.iter(|| {
                output.clear();
                serde_json::to_writer(&mut output, black_box(&view)).unwrap();
                black_box(&output);
            });
        });
    }
    group.finish();
}

fn bench_bytes_view_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("bytes_value_view_json");
    for len in [64usize, 1024, 16 * 1024, 256 * 1024] {
        let payload = vec![0xAB; len];
        let view = BytesValueView {
            value: &payload,
            ..Default::default()
        };
        let mut output = Vec::with_capacity(len.div_ceil(3) * 4 + 2);
        group.throughput(Throughput::Bytes(len as u64));
        group.bench_function(BenchmarkId::from_parameter(len), |b| {
            b.iter(|| {
                output.clear();
                serde_json::to_writer(&mut output, black_box(&view)).unwrap();
                black_box(&output);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_string_view_json, bench_bytes_view_json);
criterion_main!(benches);

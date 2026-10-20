//! Opt-in comparison of the current small-bytes ProtoJSON path with
//! the previous temporary-String implementation.
use base64::Engine as _;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde::Serialize;

#[derive(Serialize)]
struct BuffaBytes<'a>(#[serde(with = "buffa::json_helpers::bytes")] &'a [u8]);

struct OriginalBytes<'a>(&'a [u8]);

impl Serialize for OriginalBytes<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(self.0))
    }
}

fn bench_json_base64(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_base64");
    for size in [8usize, 32, 96, 192, 193, 4096] {
        let input: Vec<u8> = (0..size)
            .map(|i| (i as u8).wrapping_mul(47).wrapping_add(199))
            .collect();
        let buffa = BuffaBytes(&input);
        let original = OriginalBytes(&input);
        assert_eq!(
            serde_json::to_vec(&buffa).unwrap(),
            serde_json::to_vec(&original).unwrap()
        );

        // Reuse the output buffer so only the original helper's intermediate
        // base64 String allocation differentiates the two code paths.
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_function(BenchmarkId::new("buffa", size), |b| {
            let mut output = Vec::with_capacity(size * 2 + 2);
            b.iter(|| {
                output.clear();
                serde_json::to_writer(&mut output, black_box(&buffa)).unwrap();
                black_box(&output);
            });
        });
        group.bench_function(BenchmarkId::new("original", size), |b| {
            let mut output = Vec::with_capacity(size * 2 + 2);
            b.iter(|| {
                output.clear();
                serde_json::to_writer(&mut output, black_box(&original)).unwrap();
                black_box(&output);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_json_base64);
criterion_main!(benches);

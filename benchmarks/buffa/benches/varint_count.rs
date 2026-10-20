// Focused microbench for the terminator counting used to reserve packed
// varint view storage. Run with:
// cargo bench --manifest-path benchmarks/buffa/Cargo.toml --features varint_count --bench varint_count
//
// Not part of the saved cross-implementation benchmark suite.
use buffa::encoding::count_varints;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn scalar_count(payload: &[u8]) -> usize {
    payload.iter().filter(|&&b| b < 0x80).count()
}

fn bench_count_varints(c: &mut Criterion) {
    let mut group = c.benchmark_group("packed_varint_count");
    for size in [32usize, 256, 4096, 65_536] {
        // One continuation byte per three bytes. Every continuation has a
        // following terminator, so the sequence is a valid packed payload.
        let mut payload: Vec<u8> = (0..size)
            .map(|i| if i % 3 == 0 { 0x80 } else { (i as u8) & 0x7f })
            .collect();
        if let Some(last) = payload.last_mut() {
            *last &= 0x7f;
        }

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("scalar", size),
            &payload,
            |b, payload| b.iter(|| black_box(scalar_count(black_box(payload)))),
        );
        group.bench_with_input(
            BenchmarkId::new("wordwise", size),
            &payload,
            |b, payload| b.iter(|| black_box(count_varints(black_box(payload)))),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_count_varints);
criterion_main!(benches);

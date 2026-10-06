use buffa::{EncodeSink, Rope, RopeBuf};
use bytes::{Buf, Bytes};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

fn buffer(segments: &[Bytes], offset: usize) -> RopeBuf {
    let mut rope = Rope::with_min_segment(1);
    for segment in segments {
        rope.put_shared(segment.clone());
    }
    let mut buf = RopeBuf::from(rope);
    buf.advance(offset);
    buf
}

fn run(c: &mut Criterion) {
    let mut group = c.benchmark_group("rope_buf/copy_to_bytes");
    for size in [32, 4096, 1_048_576] {
        let segments = [Bytes::from(vec![0x5A; size])];
        group.bench_with_input(BenchmarkId::new("whole_segment", size), &size, |b, &len| {
            b.iter_batched(
                || buffer(&segments, 0),
                |mut buf| black_box(buf.copy_to_bytes(black_box(len))),
                BatchSize::NumIterations(32),
            );
        });
        group.bench_with_input(
            BenchmarkId::new("partial_segment", size),
            &size,
            |b, &len| {
                b.iter_batched(
                    || buffer(&segments, len / 4),
                    |mut buf| black_box(buf.copy_to_bytes(black_box(len / 2))),
                    BatchSize::NumIterations(32),
                );
            },
        );
        let segments = [
            Bytes::from(vec![0x5A; size / 2]),
            Bytes::from(vec![0xA5; size / 2]),
        ];
        group.bench_with_input(BenchmarkId::new("cross_segment", size), &size, |b, &len| {
            b.iter_batched(
                || buffer(&segments, 0),
                |mut buf| black_box(buf.copy_to_bytes(black_box(len))),
                BatchSize::NumIterations(32),
            );
        });
    }
    group.finish();
}

criterion_group!(benches, run);
criterion_main!(benches);

// Reproduce unknown length-delimited field decode throughput across revisions.
//
// Not included in the default benchmark suite:
//   cargo bench --manifest-path benchmarks/buffa/Cargo.toml \
//     --features unknown_fields --bench unknown_fields -- --sample-size 30
//
// Compare baseline/main and candidate with the same Rust toolchain and
// Criterion configuration. Throughput is payload bytes, excluding framing.
use core::cell::Cell;

use buffa::bytes::Buf;
use buffa::encoding::{decode_unknown_field, encode_varint, Tag, WireType};
use buffa::DecodeContext;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

fn run(c: &mut Criterion) {
    for size in [32usize, 4 * 1024, 64 * 1024] {
        let payload: Vec<u8> = (0..size).map(|i| (i as u8).wrapping_mul(37)).collect();
        let mut wire = Vec::with_capacity(size + 16);
        Tag::new(21, WireType::LengthDelimited).encode(&mut wire);
        encode_varint(size as u64, &mut wire);
        wire.extend_from_slice(&payload);
        let split = wire.len() / 2;

        let mut group = c.benchmark_group(format!("unknown_fields/length_delimited/{size}"));
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_function("contiguous", |b| {
            b.iter(|| {
                let mut input = black_box(wire.as_slice());
                let tag = Tag::decode(&mut input).unwrap();
                let allowance = Cell::new(1);
                black_box(
                    decode_unknown_field(tag, &mut input, DecodeContext::new(100, &allowance))
                        .unwrap(),
                );
            });
        });
        group.bench_function("split_payload", |b| {
            b.iter(|| {
                let (left, right) = black_box(wire.as_slice()).split_at(split);
                let mut input = left.chain(right);
                let tag = Tag::decode(&mut input).unwrap();
                let allowance = Cell::new(1);
                black_box(
                    decode_unknown_field(tag, &mut input, DecodeContext::new(100, &allowance))
                        .unwrap(),
                );
            });
        });
        group.finish();
    }
}

criterion_group!(benches, run);
criterion_main!(benches);

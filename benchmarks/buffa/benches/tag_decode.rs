//! Isolated wire-tag decoding benchmarks.
//!
//! cargo bench --manifest-path benchmarks/buffa/Cargo.toml --bench tag_decode --features iso

use buffa::encoding::{Tag, WireType};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

fn benchmark_tag_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("buffa/tag_decode");
    let cases: &[(&str, &[u32])] = &[
        ("one_byte", &[1, 8, 15]),
        ("two_byte", &[16, 127, 2047]),
        ("three_byte", &[2048, 4096, 262_143]),
        ("mixed", &[1, 16, 127, 2048]),
    ];

    for &(name, fields) in cases {
        let mut payload = Vec::new();
        for &number in fields.iter().cycle().take(4096) {
            Tag::new(number, WireType::Varint).encode(&mut payload);
        }

        group.throughput(Throughput::Elements(4096));
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut input = black_box(payload.as_slice());
                let mut sum = 0u64;
                while !input.is_empty() {
                    let tag = Tag::decode(&mut input).unwrap();
                    sum = sum.wrapping_add(u64::from(tag.field_number()));
                }
                black_box(sum)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, benchmark_tag_decode);
criterion_main!(benches);

// Measure packed fixed-width extension decoding and packed scalar encoding.
//
// Run separately from the default benchmark suite to avoid changing saved
// benchmark IDs:
//
//   cd benchmarks/buffa
//   cargo bench --features packed_extension --bench packed_extension
//
// For a fair base-vs-branch comparison, run this same benchmark file against
// both implementations on the same machine and Rust toolchain.
use buffa::extension::codecs::{Fixed32, Fixed64, Int32, PackedRepeated, Repeated};
use buffa::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};
use buffa::ExtensionCodec;
use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};

const FIELD_NUMBER: u32 = 7;

fn packed_fixed32(count: usize) -> UnknownFields {
    let mut payload = Vec::with_capacity(count * 4);
    for i in 0..count {
        payload.extend_from_slice(&(i as u32).to_le_bytes());
    }
    let mut fields = UnknownFields::new();
    fields.push(UnknownField {
        number: FIELD_NUMBER,
        data: UnknownFieldData::LengthDelimited(payload),
    });
    fields
}

fn packed_fixed64(count: usize) -> UnknownFields {
    let mut payload = Vec::with_capacity(count * 8);
    for i in 0..count {
        payload.extend_from_slice(&(i as u64).to_le_bytes());
    }
    let mut fields = UnknownFields::new();
    fields.push(UnknownField {
        number: FIELD_NUMBER,
        data: UnknownFieldData::LengthDelimited(payload),
    });
    fields
}

fn packed_extension(c: &mut Criterion) {
    let mut group = c.benchmark_group("extension/decode_packed_fixed");
    for count in [1usize, 4, 8, 128, 4096, 16_384, 32_768, 65_536] {
        let fields32 = packed_fixed32(count);
        let fields64 = packed_fixed64(count);
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("fixed32", count),
            &fields32,
            |b, fields| {
                b.iter(|| black_box(Repeated::<Fixed32>::decode(FIELD_NUMBER, black_box(fields))));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("fixed64", count),
            &fields64,
            |b, fields| {
                b.iter(|| black_box(Repeated::<Fixed64>::decode(FIELD_NUMBER, black_box(fields))));
            },
        );
    }
    group.finish();
}

fn packed_extension_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("extension/encode_packed_scalar");
    for count in [8usize, 128, 4_096] {
        let fixed32: Vec<u32> = (0..count as u32).collect();
        let fixed64: Vec<u64> = (0..count as u64).collect();
        let varint: Vec<i32> = (0..count as i32).collect();
        let negative_varint: Vec<i32> = (0..count as i32).map(|value| -value - 1).collect();
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::new("fixed32", count), &fixed32, |b, values| {
            b.iter_batched(
                || (*values).clone(),
                |input| {
                    let mut fields = UnknownFields::new();
                    PackedRepeated::<Fixed32>::try_encode(
                        FIELD_NUMBER,
                        black_box(input),
                        &mut fields,
                    )
                    .unwrap();
                    black_box(fields);
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_with_input(BenchmarkId::new("fixed64", count), &fixed64, |b, values| {
            b.iter_batched(
                || (*values).clone(),
                |input| {
                    let mut fields = UnknownFields::new();
                    PackedRepeated::<Fixed64>::try_encode(
                        FIELD_NUMBER,
                        black_box(input),
                        &mut fields,
                    )
                    .unwrap();
                    black_box(fields);
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_with_input(BenchmarkId::new("varint", count), &varint, |b, values| {
            b.iter_batched(
                || (*values).clone(),
                |input| {
                    let mut fields = UnknownFields::new();
                    PackedRepeated::<Int32>::try_encode(
                        FIELD_NUMBER,
                        black_box(input),
                        &mut fields,
                    )
                    .unwrap();
                    black_box(fields);
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_with_input(
            BenchmarkId::new("int32_negative", count),
            &negative_varint,
            |b, values| {
                b.iter_batched(
                    || (*values).clone(),
                    |input| {
                        let mut fields = UnknownFields::new();
                        PackedRepeated::<Int32>::try_encode(
                            FIELD_NUMBER,
                            black_box(input),
                            &mut fields,
                        )
                        .unwrap();
                        black_box(fields);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(benches, packed_extension, packed_extension_encode);
criterion_main!(benches);

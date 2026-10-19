// Measure packed fixed-width extension decode into freshly allocated vectors.
//
// Run separately from the default benchmark suite to avoid changing saved
// benchmark IDs:
//
//   cd benchmarks/buffa
//   cargo bench --features packed_extension --bench packed_extension
//
// For a fair base-vs-branch comparison, run this same benchmark file against
// both implementations on the same machine and Rust toolchain.
use buffa::extension::codecs::{Fixed32, Fixed64, Repeated};
use buffa::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};
use buffa::ExtensionCodec;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

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

criterion_group!(benches, packed_extension);
criterion_main!(benches);

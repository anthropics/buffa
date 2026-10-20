//! Enum descriptor lookup and construction benchmarks.
//!
//! cargo bench --manifest-path benchmarks/buffa/Cargo.toml --bench enum_lookup --features iso

use buffa_descriptor::generated::descriptor::{
    EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::DescriptorPool;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

fn descriptor_set(size: usize) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("enum_lookup.proto".into()),
            package: Some("bench.enum_lookup".into()),
            syntax: Some("proto3".into()),
            enum_type: vec![EnumDescriptorProto {
                name: Some("Large".into()),
                value: (0..size)
                    .map(|i| EnumValueDescriptorProto {
                        name: Some(format!("VALUE_{i:04}")),
                        number: Some(i as i32),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn benchmark_enum_lookup(c: &mut Criterion) {
    let mut lookups = c.benchmark_group("buffa/enum_lookup");
    let sizes = [16, 127, 128, 129, 1024];

    for size in sizes {
        let set = descriptor_set(size);
        let pool = DescriptorPool::new(set.clone()).expect("valid benchmark descriptor");
        let enumeration = pool
            .enum_by_name("bench.enum_lookup.Large")
            .expect("benchmark enum exists");
        let hit_number = (size / 2) as i32;
        let hit_name = format!("VALUE_{:04}", size - 1);

        lookups.bench_function(BenchmarkId::new("number_hit", size), |b| {
            b.iter(|| black_box(enumeration.value(black_box(hit_number)).map(|v| v.number())))
        });
        lookups.bench_function(BenchmarkId::new("number_miss", size), |b| {
            b.iter(|| black_box(enumeration.value(black_box(-1)).map(|v| v.number())))
        });
        lookups.bench_function(BenchmarkId::new("name_hit", size), |b| {
            b.iter(|| {
                black_box(
                    enumeration
                        .value_by_name(black_box(&hit_name))
                        .map(|v| v.number()),
                )
            })
        });
        lookups.bench_function(BenchmarkId::new("name_miss", size), |b| {
            b.iter(|| {
                black_box(
                    enumeration
                        .value_by_name(black_box("VALUE_MISSING"))
                        .map(|v| v.number()),
                )
            })
        });
        lookups.bench_function(BenchmarkId::new("pool_build", size), |b| {
            b.iter_batched(
                || set.clone(),
                |set| DescriptorPool::new(set).expect("valid benchmark descriptor"),
                BatchSize::SmallInput,
            )
        });
    }

    lookups.finish();
}

criterion_group!(benches, benchmark_enum_lookup);
criterion_main!(benches);

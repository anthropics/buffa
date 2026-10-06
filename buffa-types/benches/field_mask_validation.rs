//! Benchmark FieldMask path validation and JSON serialization.

use buffa::json_helpers::wkt::field_mask_path_round_trips;
use buffa_types::FieldMask;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_mask_validation");
    for (name, path) in [
        ("short", "foo_bar"),
        ("nested", "user.profile.display_name"),
        ("leading_underscores", "_user._profile._display_name"),
        ("digits", "foo1.bar2.baz3_qux4"),
        ("invalid_underscore", "user.profile.display__name"),
        ("invalid_character", "user.profile.display-name"),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), path, |b, path| {
            b.iter(|| black_box(field_mask_path_round_trips(black_box(path))));
        });
    }
    group.finish();
}

fn bench_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_mask_json");
    for count in [1usize, 32] {
        let mask = FieldMask {
            paths: (0..count)
                .map(|i| format!("user{i}.profile.display_name"))
                .collect(),
            ..Default::default()
        };
        group.bench_with_input(BenchmarkId::from_parameter(count), &mask, |b, mask| {
            b.iter(|| black_box(serde_json::to_string(black_box(mask)).unwrap()));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_validation, bench_json);
criterion_main!(benches);

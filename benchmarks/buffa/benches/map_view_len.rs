//! Standalone MapView distinct-key counting microbenchmarks.
//!
//! Run with:
//! cargo bench --manifest-path benchmarks/buffa/Cargo.toml --bench map_view_len --features map_view_len

use buffa::MapView;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn map_view_len(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_view_len_unique");
    // Sizes on either side of the allocation-free cutoff, plus larger maps.
    // Both unique and duplicate-heavy inputs are important in practice.
    for (size, distinct) in [
        (1usize, 1usize),
        (6, 6),
        (6, 1),
        (7, 7),
        (32, 32),
        (32, 4),
        (512, 512),
        (512, 16),
    ] {
        let entries: Vec<_> = (0..size)
            .map(|i| (((i * 11) % distinct) as u32, i as u32))
            .collect();
        let map = MapView::new(entries);
        let label = format!("{size}_entries_{distinct}_keys");

        // Count-only path: the optimized API.
        group.bench_function(format!("{label}/len_unique"), |b| {
            b.iter(|| black_box(black_box(&map).len_unique()))
        });
        // Existing full-dedup path, equivalent to the old implementation of
        // len_unique. It also recovers wire order, which counting does not need.
        group.bench_function(format!("{label}/iter_unique_len"), |b| {
            b.iter(|| black_box(black_box(&map).iter_unique().len()))
        });
    }
    group.finish();
}

criterion_group!(benches, map_view_len);
criterion_main!(benches);

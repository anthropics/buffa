use buffa::{Inline, MessageField};
use criterion::{criterion_group, criterion_main, Criterion};

#[derive(Clone, Default)]
struct BenchMessage {
    _value: [u64; 2],
}

#[derive(Clone, Default)]
struct BenchParent {
    _child: MessageField<BenchMessage, Inline<BenchMessage>>,
}

#[derive(Clone, Default)]
struct OptionParent {
    _child: Option<Inline<BenchMessage>>,
}

fn bench_message_field_clone(c: &mut Criterion) {
    let set =
        MessageField::<BenchMessage, Inline<BenchMessage>>::from_pointer(Inline(BenchMessage {
            _value: [1, 2],
        }));
    let unset = MessageField::<BenchMessage, Inline<BenchMessage>>::none();
    let value = Inline(BenchMessage { _value: [1, 2] });
    let parent = BenchParent {
        _child: set.clone(),
    };
    let option_parent = OptionParent {
        _child: Some(value.clone()),
    };

    let mut group = c.benchmark_group("message_field_clone");
    group.bench_function("set_inline", |b| {
        b.iter(|| criterion::black_box(Clone::clone(criterion::black_box(&set))));
    });
    group.bench_function("unset_inline", |b| {
        b.iter(|| criterion::black_box(Clone::clone(criterion::black_box(&unset))));
    });
    group.bench_function("inline_payload", |b| {
        b.iter(|| criterion::black_box(Clone::clone(criterion::black_box(&value))));
    });
    group.bench_function("parent_set_inline", |b| {
        b.iter(|| criterion::black_box(Clone::clone(criterion::black_box(&parent))));
    });
    group.bench_function("option_parent_set_inline", |b| {
        b.iter(|| criterion::black_box(Clone::clone(criterion::black_box(&option_parent))));
    });
    group.finish();
}

criterion_group!(benches, bench_message_field_clone);
criterion_main!(benches);

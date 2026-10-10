use buffa::{Message, MessageView};
use buffa_types::google::protobuf::{__buffa::view::StructView, Struct, Value};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn nested_value(depth: usize, width: usize) -> Value {
    if depth == 0 {
        Value::from("payload")
    } else {
        Value::from(Struct::from_fields((0..width).map(|index| {
            (format!("field_{index}"), nested_value(depth - 1, width))
        })))
    }
}

fn document() -> Struct {
    Struct::from_fields([("root".to_owned(), nested_value(4, 8))])
}

fn bench_wkt_view_json(c: &mut Criterion) {
    let owned = document();
    let wire = owned.encode_to_vec();
    let view = StructView::decode_view(&wire).expect("decode view");
    assert_eq!(
        serde_json::to_value(&view).expect("serialize view"),
        serde_json::to_value(&owned).expect("serialize owned")
    );

    let mut group = c.benchmark_group("wkt_view_json");
    group.bench_function("nested_struct_view", |b| {
        b.iter(|| serde_json::to_vec(black_box(&view)).expect("serialize view"));
    });
    group.bench_function("nested_struct_owned", |b| {
        b.iter(|| serde_json::to_vec(black_box(&owned)).expect("serialize owned"));
    });
    group.finish();
}

criterion_group!(benches, bench_wkt_view_json);
criterion_main!(benches);

//! Reflection vs. generated codec performance comparison.
//!
//! Measures the cost of routing protobuf encode/decode through the
//! [`DynamicMessage`] reflection path against the generated typed codec.
//! Both paths are conformance-validated; this benchmark answers "how much
//! does the genericity cost?" so consumers (CEL evaluators, transcoding
//! gateways, generic interceptors) can budget for it.
//!
//! Measured per dataset:
//!
//! 1. **Decode** — `decode/generated` (`T::decode_from_slice`),
//!    `decode/reflect` (`DynamicMessage::decode`, descriptor-driven), and
//!    `decode/view` (`decode_view`, zero-copy — strings/bytes borrow from the
//!    input, so this is the floor, below even generated decode).
//! 2. **Encode** — `encode/generated` vs. `encode/reflect`.
//! 3. **Bridge round-trip** — `t.reflect()`: one full encode + decode + boxed
//!    `DynamicMessage`, the cost of the codegen-emitted `Reflectable` impl.
//! 4. **From wire bytes to reflective field reads** — the interceptor /
//!    field-mask workload, in `read_one` and `read_all` variants across three
//!    handle strategies: `vtable_*` (`decode_view` + borrow as
//!    `&dyn ReflectMessage`), `bridge_*` (`T::decode` then `.reflect()`), and
//!    `dynamic_*` (`DynamicMessage::decode`). Vtable reflection is dominated by
//!    the cheap zero-copy `decode_view`, so it lands well below both the bridge
//!    round-trip and pure `DynamicMessage` reflection.

use std::sync::Arc;

use buffa::{Message, MessageView};
use buffa_descriptor::reflect::{DynamicMessage, ReflectMessage, Reflectable};
use buffa_descriptor::{DescriptorPool, MessageIndex};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use bench_buffa::bench::{
    AnalyticsEvent, AnalyticsEventView, ApiResponse, ApiResponseView, LogRecord, LogRecordView,
};
use bench_buffa::benchmarks::BenchmarkDataset;
use bench_buffa::proto3::{GoogleMessage1, GoogleMessage1View};

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode_from_slice(data).expect("failed to decode dataset")
}

fn total_payload_bytes(dataset: &BenchmarkDataset) -> u64 {
    dataset.payload.iter().map(|p| p.len() as u64).sum()
}

/// Read the first declared field through a reflective handle (the field-mask
/// "extract one field" shape).
fn read_one(m: &dyn ReflectMessage) {
    if let Some(fd) = m.message_descriptor().fields().first() {
        criterion::black_box(m.get(fd));
    }
}

/// Visit every set field through a reflective handle (the "scan the whole
/// message" shape — a generic redactor or a full transcode).
fn read_all(m: &dyn ReflectMessage) {
    m.for_each_set(&mut |_fd, v| {
        criterion::black_box(v);
    });
}

/// Zero-copy decode floor: `decode_view(bytes)` and discard, no reflection.
/// Isolates the view-decode cost that the vtable reflection paths build on.
fn vtable_decode_only<'a, V>(payload: &'a [u8])
where
    V: MessageView<'a>,
{
    criterion::black_box(V::decode_view(payload).expect("decode_view"));
}

/// Vtable path: `decode_view(bytes)` then read the first field.
fn vtable_read_one<'a, V>(payload: &'a [u8])
where
    V: MessageView<'a> + ReflectMessage,
{
    let view = V::decode_view(payload).expect("decode_view");
    read_one(&view);
}

/// Vtable path: `decode_view(bytes)` then visit every set field.
fn vtable_read_all<'a, V>(payload: &'a [u8])
where
    V: MessageView<'a> + ReflectMessage,
{
    let view = V::decode_view(payload).expect("decode_view");
    read_all(&view);
}

// The view-path closures (decode, read-one, read-all) are passed individually
// because each call site monomorphizes them over a different concrete view
// type; bundling them would not reduce the real coupling.
#[allow(clippy::too_many_arguments)]
fn bench_message<M>(
    c: &mut Criterion,
    name: &str,
    full_name: &str,
    pool: &'static Arc<DescriptorPool>,
    dataset_bytes: &[u8],
    vt_decode: impl Fn(&[u8]),
    vt_read_one: impl Fn(&[u8]),
    vt_read_all: impl Fn(&[u8]),
) where
    M: Message + Default + Reflectable,
{
    let dataset = load_dataset(dataset_bytes);
    let bytes = total_payload_bytes(&dataset);
    // Decode the datasets up-front so the encode benches measure encode
    // only. The pool index is resolved once.
    let p = pool;
    let idx: MessageIndex = p
        .message_index(full_name)
        .expect("benchmark type registered in pool");
    let typed: Vec<M> = dataset
        .payload
        .iter()
        .map(|b| M::decode_from_slice(b).expect("dataset decodes via generated codec"))
        .collect();
    let reflective: Vec<DynamicMessage> = dataset
        .payload
        .iter()
        .map(|b| {
            DynamicMessage::decode(Arc::clone(p), idx, b).expect("dataset decodes via reflection")
        })
        .collect();

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode/generated", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let m = M::decode_from_slice(payload).expect("decode");
                criterion::black_box(&m);
            }
        });
    });

    group.bench_function("decode/reflect", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let m =
                    DynamicMessage::decode(Arc::clone(p), idx, payload).expect("reflect decode");
                criterion::black_box(&m);
            }
        });
    });

    // Zero-copy view decode (no reflection) — the floor the vtable reflection
    // paths build on. Strings/bytes borrow from the input instead of being
    // copied into owned `String`/`Vec`, so this undercuts even `decode/generated`.
    group.bench_function("decode/view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                vt_decode(payload);
            }
        });
    });

    group.bench_function("encode/generated", |b| {
        b.iter(|| {
            for m in &typed {
                criterion::black_box(m.encode_to_vec());
            }
        });
    });

    group.bench_function("encode/reflect", |b| {
        b.iter(|| {
            for m in &reflective {
                criterion::black_box(m.encode_to_vec());
            }
        });
    });

    // The bridge cost: codegen-emitted `Reflectable::reflect()` is one
    // full encode + decode + Box per call. This is what a generic
    // interceptor pays to get a `&dyn ReflectMessage` from a typed message.
    group.bench_function("reflect/bridge_round_trip", |b| {
        b.iter(|| {
            for m in &typed {
                criterion::black_box(m.reflect());
            }
        });
    });

    // ── From wire bytes to reflective field reads ─────────────────────────
    //
    // The interceptor / field-mask workload: given a wire payload, obtain a
    // reflective handle and read field(s). Three handle strategies, each in a
    // "read one field" and "read all set fields" variant:
    //
    //   vtable  — decode_view(bytes), borrow as &dyn ReflectMessage
    //   bridge  — M::decode(bytes) then .reflect() (encode + decode + Box)
    //   dynamic — DynamicMessage::decode(bytes) (pure reflection, no typed step)

    group.bench_function("reflect/vtable_read_one", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                vt_read_one(payload);
            }
        });
    });
    group.bench_function("reflect/vtable_read_all", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                vt_read_all(payload);
            }
        });
    });

    group.bench_function("reflect/bridge_read_one", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let m = M::decode_from_slice(payload).expect("decode");
                read_one(&*m.reflect());
            }
        });
    });
    group.bench_function("reflect/bridge_read_all", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let m = M::decode_from_slice(payload).expect("decode");
                read_all(&*m.reflect());
            }
        });
    });

    group.bench_function("reflect/dynamic_read_one", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let dm =
                    DynamicMessage::decode(Arc::clone(p), idx, payload).expect("reflect decode");
                read_one(&dm);
            }
        });
    });
    group.bench_function("reflect/dynamic_read_all", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let dm =
                    DynamicMessage::decode(Arc::clone(p), idx, payload).expect("reflect decode");
                read_all(&dm);
            }
        });
    });

    group.finish();
}

fn benchmark_api_response(c: &mut Criterion) {
    bench_message::<ApiResponse>(
        c,
        "reflect/ApiResponse",
        "bench.ApiResponse",
        bench_buffa::bench::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/api_response.pb"),
        |p| vtable_decode_only::<ApiResponseView<'_>>(p),
        |p| vtable_read_one::<ApiResponseView<'_>>(p),
        |p| vtable_read_all::<ApiResponseView<'_>>(p),
    );
}

fn benchmark_log_record(c: &mut Criterion) {
    bench_message::<LogRecord>(
        c,
        "reflect/LogRecord",
        "bench.LogRecord",
        bench_buffa::bench::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/log_record.pb"),
        |p| vtable_decode_only::<LogRecordView<'_>>(p),
        |p| vtable_read_one::<LogRecordView<'_>>(p),
        |p| vtable_read_all::<LogRecordView<'_>>(p),
    );
}

fn benchmark_analytics_event(c: &mut Criterion) {
    bench_message::<AnalyticsEvent>(
        c,
        "reflect/AnalyticsEvent",
        "bench.AnalyticsEvent",
        bench_buffa::bench::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/analytics_event.pb"),
        |p| vtable_decode_only::<AnalyticsEventView<'_>>(p),
        |p| vtable_read_one::<AnalyticsEventView<'_>>(p),
        |p| vtable_read_all::<AnalyticsEventView<'_>>(p),
    );
}

fn benchmark_google_message1(c: &mut Criterion) {
    bench_message::<GoogleMessage1>(
        c,
        "reflect/GoogleMessage1",
        "benchmarks.proto3.GoogleMessage1",
        bench_buffa::proto3::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/google_message1_proto3.pb"),
        |p| vtable_decode_only::<GoogleMessage1View<'_>>(p),
        |p| vtable_read_one::<GoogleMessage1View<'_>>(p),
        |p| vtable_read_all::<GoogleMessage1View<'_>>(p),
    );
}

criterion_group!(
    benches,
    benchmark_api_response,
    benchmark_log_record,
    benchmark_analytics_event,
    benchmark_google_message1,
);
criterion_main!(benches);

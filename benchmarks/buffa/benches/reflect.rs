//! Reflection vs. generated codec performance comparison.
//!
//! Measures the cost of routing protobuf encode/decode through the
//! [`DynamicMessage`] reflection path against the generated typed codec.
//! Both paths are conformance-validated; this benchmark answers "how much
//! does the genericity cost?" so consumers (CEL evaluators, transcoding
//! gateways, generic interceptors) can budget for it.
//!
//! Three things are measured per dataset:
//!
//! 1. **Generated decode** — `T::decode_from_slice(bytes)`. The baseline.
//! 2. **Reflective decode** — `DynamicMessage::decode(pool, idx, bytes)`.
//!    Same wire bytes, descriptor-driven field dispatch instead of
//!    generated match arms, `BTreeMap<u32, Value>` storage instead of
//!    struct fields.
//! 3. **Generated encode** vs. **Reflective encode** — `t.encode_to_vec()`
//!    on each.
//! 4. **Bridge round-trip** — `t.reflect()`. The cost a generic
//!    interceptor pays per message: one full encode + decode + boxed
//!    `DynamicMessage`. This is the headline number for the connect-rust
//!    interceptor use case and the codegen-emitted `Reflectable` impl.

use std::sync::Arc;

use buffa::Message;
use buffa_descriptor::reflect::{DynamicMessage, Reflectable};
use buffa_descriptor::{DescriptorPool, MessageIndex};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use bench_buffa::bench::{AnalyticsEvent, ApiResponse, LogRecord};
use bench_buffa::benchmarks::BenchmarkDataset;
use bench_buffa::proto3::GoogleMessage1;

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode_from_slice(data).expect("failed to decode dataset")
}

fn total_payload_bytes(dataset: &BenchmarkDataset) -> u64 {
    dataset.payload.iter().map(|p| p.len() as u64).sum()
}

fn bench_message<M>(
    c: &mut Criterion,
    name: &str,
    full_name: &str,
    pool: &'static Arc<DescriptorPool>,
    dataset_bytes: &[u8],
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

    group.finish();
}

fn benchmark_api_response(c: &mut Criterion) {
    bench_message::<ApiResponse>(
        c,
        "reflect/ApiResponse",
        "bench.ApiResponse",
        bench_buffa::bench::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/api_response.pb"),
    );
}

fn benchmark_log_record(c: &mut Criterion) {
    bench_message::<LogRecord>(
        c,
        "reflect/LogRecord",
        "bench.LogRecord",
        bench_buffa::bench::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/log_record.pb"),
    );
}

fn benchmark_analytics_event(c: &mut Criterion) {
    bench_message::<AnalyticsEvent>(
        c,
        "reflect/AnalyticsEvent",
        "bench.AnalyticsEvent",
        bench_buffa::bench::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/analytics_event.pb"),
    );
}

fn benchmark_google_message1(c: &mut Criterion) {
    bench_message::<GoogleMessage1>(
        c,
        "reflect/GoogleMessage1",
        "benchmarks.proto3.GoogleMessage1",
        bench_buffa::proto3::__buffa::reflect::descriptor_pool(),
        include_bytes!("../../datasets/google_message1_proto3.pb"),
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

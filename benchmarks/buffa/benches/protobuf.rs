use buffa::{Message, MessageView, ViewEncode};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use serde::{de::DeserializeOwned, Serialize};

use bench_buffa::bench::__buffa::view::*;
use bench_buffa::bench::*;
use bench_buffa::benchmarks::BenchmarkDataset;
use bench_buffa::proto3::__buffa::view::GoogleMessage1View;

fn load_dataset(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode_from_slice(data).expect("failed to decode dataset")
}

fn total_payload_bytes(dataset: &BenchmarkDataset) -> u64 {
    dataset.payload.iter().map(|p| p.len() as u64).sum()
}

fn benchmark_decode<M: Message + Default>(c: &mut Criterion, name: &str, dataset_bytes: &[u8]) {
    let dataset = load_dataset(dataset_bytes);
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let msg = M::decode_from_slice(payload).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.bench_function("merge", |b| {
        let mut msg = M::default();
        b.iter(|| {
            for payload in &dataset.payload {
                msg.clear();
                msg.merge_from_slice(payload).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.bench_function("encode", |b| {
        let messages: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode_from_slice(p).unwrap())
            .collect();
        b.iter(|| {
            for msg in &messages {
                let encoded = msg.encode_to_vec();
                criterion::black_box(&encoded);
            }
        });
    });

    group.bench_function("compute_size", |b| {
        let messages: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode_from_slice(p).unwrap())
            .collect();
        b.iter(|| {
            for msg in &messages {
                let size = msg.compute_size();
                criterion::black_box(size);
            }
        });
    });

    group.finish();
}

fn benchmark_json<M: Message + Default + Serialize + DeserializeOwned>(
    c: &mut Criterion,
    name: &str,
    dataset_bytes: &[u8],
) {
    let dataset = load_dataset(dataset_bytes);

    // Pre-decode binary payloads to owned messages.
    let messages: Vec<M> = dataset
        .payload
        .iter()
        .map(|p| M::decode_from_slice(p).unwrap())
        .collect();

    // Pre-encode messages to JSON strings for decode benchmarks.
    let json_strings: Vec<String> = messages
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect();

    let json_bytes: u64 = json_strings.iter().map(|s| s.len() as u64).sum();

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(json_bytes));

    group.bench_function("json_encode", |b| {
        b.iter(|| {
            for msg in &messages {
                let json = serde_json::to_string(msg).unwrap();
                criterion::black_box(&json);
            }
        });
    });

    group.bench_function("json_decode", |b| {
        b.iter(|| {
            for json in &json_strings {
                let msg: M = serde_json::from_str(json).unwrap();
                criterion::black_box(&msg);
            }
        });
    });

    group.finish();
}

// Per-message-type decode_view benchmarks using concrete view types.
fn bench_api_response_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/api_response.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/api_response");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = ApiResponseView::decode_view(payload).unwrap();
                criterion::black_box(&view);
            }
        });
    });

    group.finish();
}

fn bench_log_record_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/log_record.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/log_record");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = LogRecordView::decode_view(payload).unwrap();
                criterion::black_box(&view);
            }
        });
    });

    group.finish();
}

fn bench_analytics_event_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/analytics_event.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/analytics_event");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = AnalyticsEventView::decode_view(payload).unwrap();
                criterion::black_box(&view);
            }
        });
    });

    group.finish();
}

fn bench_google_message1_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/google_message1_proto3.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/google_message1_proto3");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = GoogleMessage1View::decode_view(payload).unwrap();
                criterion::black_box(&view);
            }
        });
    });

    group.finish();
}

fn bench_media_frame_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/media_frame.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/media_frame");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = MediaFrameView::decode_view(payload).unwrap();
                criterion::black_box(&view);
            }
        });
    });

    group.finish();
}

/// Add `encode_view` to a concrete per-dataset bench group: pre-decode
/// payloads into views, assert wire-compat against owned decode, then bench
/// re-encoding from the views' borrowed fields. The owned `encode` baseline
/// is in [`benchmark_decode`] — same group name, so throughputs sit side by
/// side.
///
/// Per-dataset functions are concrete (not generic over `V`) because the
/// views borrow from the locally-decoded `dataset.payload`; a `<'a, V>` fn
/// signature can't tie `'a` to a local. Same shape as `decode_view` above.
macro_rules! bench_view_encode {
    ($fn_name:ident, $owned:ty, $view:ty, $group:literal, $dataset:literal) => {
        fn $fn_name(c: &mut Criterion) {
            let dataset = load_dataset(include_bytes!($dataset));
            let bytes = total_payload_bytes(&dataset);
            let views: Vec<$view> = dataset
                .payload
                .iter()
                .map(|p| <$view>::decode_view(p).unwrap())
                .collect();
            for (v, p) in views.iter().zip(&dataset.payload) {
                let from_view = <$owned>::decode_from_slice(&v.encode_to_vec()).unwrap();
                let from_wire = <$owned>::decode_from_slice(p).unwrap();
                assert!(from_view == from_wire, "view-encode wire mismatch");
            }
            let mut group = c.benchmark_group($group);
            group.throughput(Throughput::Bytes(bytes));
            group.bench_function("encode_view", |b| {
                b.iter(|| {
                    for v in &views {
                        criterion::black_box(v.encode_to_vec());
                    }
                });
            });
            group.finish();
        }
    };
}

bench_view_encode!(
    bench_api_response_view_encode,
    ApiResponse,
    ApiResponseView,
    "buffa/api_response",
    "../../datasets/api_response.pb"
);
bench_view_encode!(
    bench_log_record_view_encode,
    LogRecord,
    LogRecordView,
    "buffa/log_record",
    "../../datasets/log_record.pb"
);
bench_view_encode!(
    bench_analytics_event_view_encode,
    AnalyticsEvent,
    AnalyticsEventView,
    "buffa/analytics_event",
    "../../datasets/analytics_event.pb"
);
bench_view_encode!(
    bench_google_message1_view_encode,
    bench_buffa::proto3::GoogleMessage1,
    bench_buffa::proto3::GoogleMessage1View,
    "buffa/google_message1_proto3",
    "../../datasets/google_message1_proto3.pb"
);

/// Build a `LogRecord` from borrowed source data and encode, vs build a
/// `LogRecordView` from the same borrows and encode. Unlike `encode` /
/// `encode_view` above (which serialize a pre-built struct), this includes
/// the build phase — the per-field `String`/`HashMap` allocs that the view
/// path avoids.
fn bench_log_record_build_encode(c: &mut Criterion) {
    let labels: Vec<(&str, &str)> = (0..15)
        .map(|i| {
            (
                Box::leak(format!("k8s.io/label-key-{i:02}").into_boxed_str()) as &str,
                Box::leak(format!("label-value-{i:04}").into_boxed_str()) as &str,
            )
        })
        .collect();
    let service = "inventory-service-2a";
    let msg = "GET /api/v1/items?tenant=acme-corp&warehouse=us-west-2a&page=1400 200 17ms";
    let mut group = c.benchmark_group("buffa/log_record");
    let probe = LogRecord {
        service_name: service.into(),
        message: msg.into(),
        labels: labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        ..Default::default()
    }
    .encode_to_vec();
    group.throughput(Throughput::Bytes(probe.len() as u64));

    group.bench_function("build_encode", |b| {
        b.iter(|| {
            let owned = LogRecord {
                service_name: service.into(),
                message: msg.into(),
                labels: labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
                ..Default::default()
            };
            criterion::black_box(owned.encode_to_vec())
        });
    });

    group.bench_function("build_encode_view", |b| {
        b.iter(|| {
            let view = LogRecordView {
                service_name: service,
                message: msg,
                labels: labels.iter().copied().collect(),
                ..Default::default()
            };
            criterion::black_box(view.encode_to_vec())
        });
    });

    group.finish();
}

fn bench_api_response(c: &mut Criterion) {
    benchmark_decode::<ApiResponse>(
        c,
        "buffa/api_response",
        include_bytes!("../../datasets/api_response.pb"),
    );
}

fn bench_log_record(c: &mut Criterion) {
    benchmark_decode::<LogRecord>(
        c,
        "buffa/log_record",
        include_bytes!("../../datasets/log_record.pb"),
    );
}

fn bench_analytics_event(c: &mut Criterion) {
    benchmark_decode::<AnalyticsEvent>(
        c,
        "buffa/analytics_event",
        include_bytes!("../../datasets/analytics_event.pb"),
    );
}

fn bench_google_message1(c: &mut Criterion) {
    benchmark_decode::<bench_buffa::proto3::GoogleMessage1>(
        c,
        "buffa/google_message1_proto3",
        include_bytes!("../../datasets/google_message1_proto3.pb"),
    );
}

fn bench_media_frame(c: &mut Criterion) {
    benchmark_decode::<MediaFrame>(
        c,
        "buffa/media_frame",
        include_bytes!("../../datasets/media_frame.pb"),
    );
}

fn bench_api_response_json(c: &mut Criterion) {
    benchmark_json::<ApiResponse>(
        c,
        "buffa/api_response",
        include_bytes!("../../datasets/api_response.pb"),
    );
}

fn bench_log_record_json(c: &mut Criterion) {
    benchmark_json::<LogRecord>(
        c,
        "buffa/log_record",
        include_bytes!("../../datasets/log_record.pb"),
    );
}

fn bench_analytics_event_json(c: &mut Criterion) {
    benchmark_json::<AnalyticsEvent>(
        c,
        "buffa/analytics_event",
        include_bytes!("../../datasets/analytics_event.pb"),
    );
}

fn bench_google_message1_json(c: &mut Criterion) {
    benchmark_json::<bench_buffa::proto3::GoogleMessage1>(
        c,
        "buffa/google_message1_proto3",
        include_bytes!("../../datasets/google_message1_proto3.pb"),
    );
}

fn bench_media_frame_json(c: &mut Criterion) {
    benchmark_json::<MediaFrame>(
        c,
        "buffa/media_frame",
        include_bytes!("../../datasets/media_frame.pb"),
    );
}

criterion_group!(
    owned,
    bench_api_response,
    bench_log_record,
    bench_analytics_event,
    bench_google_message1,
    bench_media_frame,
);

criterion_group!(
    views,
    bench_api_response_view,
    bench_log_record_view,
    bench_analytics_event_view,
    bench_google_message1_view,
    bench_media_frame_view,
    bench_api_response_view_encode,
    bench_log_record_view_encode,
    bench_analytics_event_view_encode,
    bench_google_message1_view_encode,
    bench_log_record_build_encode,
);

criterion_group!(
    json,
    bench_api_response_json,
    bench_log_record_json,
    bench_analytics_event_json,
    bench_google_message1_json,
    bench_media_frame_json,
);

criterion_main!(owned, views, json);

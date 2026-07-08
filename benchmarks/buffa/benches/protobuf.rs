use buffa::view::LazyMessageView;
use buffa::{Message, MessageView, ViewEncode};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use serde::{de::DeserializeOwned, Serialize};

use bench_buffa::bench::__buffa::lazy_view::{
    AnalyticsEventLazyView, ApiResponseLazyView, LogRecordLazyView, MediaFrameLazyView,
};
use bench_buffa::bench::__buffa::view::{
    analytics_event::PropertyView, AnalyticsEventView, ApiResponseView, LogRecordView,
    MediaFrameView, PackedSignedView, PackedTileView, TriMeshView,
};
use bench_buffa::bench::__buffa::{oneof, view::oneof as view_oneof};
use bench_buffa::bench::*;
use bench_buffa::benchmarks::BenchmarkDataset;
use bench_buffa::proto3::__buffa::lazy_view::GoogleMessage1LazyView;
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
                criterion::black_box(M::decode_from_slice(payload).unwrap());
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
                criterion::black_box(msg.encode_to_vec());
            }
        });
    });

    group.bench_function("compute_size", |b| {
        let messages: Vec<M> = dataset
            .payload
            .iter()
            .map(|p| M::decode_from_slice(p).unwrap())
            .collect();
        let mut cache = buffa::SizeCache::new();
        b.iter(|| {
            for msg in &messages {
                cache.clear();
                let size = msg.compute_size(&mut cache);
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
                criterion::black_box(serde_json::to_string(msg).unwrap());
            }
        });
    });

    group.bench_function("json_decode", |b| {
        b.iter(|| {
            for json in &json_strings {
                let msg: M = serde_json::from_str(json).unwrap();
                criterion::black_box(msg);
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

fn bench_packed_tile_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/packed_tile.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/packed_tile");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = PackedTileView::decode_view(payload).unwrap();
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
    GoogleMessage1View,
    "buffa/google_message1_proto3",
    "../../datasets/google_message1_proto3.pb"
);
bench_view_encode!(
    bench_media_frame_view_encode,
    MediaFrame,
    MediaFrameView,
    "buffa/media_frame",
    "../../datasets/media_frame.pb"
);

/// Lazy-view benches: `decode_lazy` is the single non-recursive scan that
/// records sub-message fields as undecoded byte ranges (the lazy analogue of
/// `decode_view`), and `encode_lazy` re-encodes from the lazy view, replaying
/// recorded fragments verbatim (the lazy analogue of `encode_view`). The
/// numbers bound the full-decode case; the lazy family's real advantage —
/// skipping untouched sub-trees on partial access — is workload-dependent
/// and not captured by a full-scan throughput chart. Before timing, both
/// paths are asserted against the owned decoder: `to_owned_message` for
/// decode parity, and a decode of the re-encoded bytes for encode parity
/// (decode-equivalence, as in `bench_view_encode!`).
macro_rules! bench_lazy_view {
    ($fn_name:ident, $owned:ty, $lazy:ty, $group:literal, $dataset:literal) => {
        fn $fn_name(c: &mut Criterion) {
            let dataset = load_dataset(include_bytes!($dataset));
            let bytes = total_payload_bytes(&dataset);
            for p in &dataset.payload {
                let lazy = <$lazy>::decode_lazy(p).unwrap();
                let from_lazy = lazy.to_owned_message().unwrap();
                let from_wire = <$owned>::decode_from_slice(p).unwrap();
                assert_eq!(from_lazy, from_wire, "lazy decode parity mismatch");
                let from_reencode = <$owned>::decode_from_slice(&lazy.encode_to_vec()).unwrap();
                assert_eq!(from_reencode, from_wire, "lazy re-encode wire mismatch");
            }
            let mut group = c.benchmark_group($group);
            group.throughput(Throughput::Bytes(bytes));
            group.bench_function("decode_lazy", |b| {
                b.iter(|| {
                    for payload in &dataset.payload {
                        let view = <$lazy>::decode_lazy(payload).unwrap();
                        criterion::black_box(&view);
                    }
                });
            });
            let views: Vec<$lazy> = dataset
                .payload
                .iter()
                .map(|p| <$lazy>::decode_lazy(p).unwrap())
                .collect();
            group.bench_function("encode_lazy", |b| {
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

bench_lazy_view!(
    bench_api_response_lazy,
    ApiResponse,
    ApiResponseLazyView,
    "buffa/api_response",
    "../../datasets/api_response.pb"
);
bench_lazy_view!(
    bench_log_record_lazy,
    LogRecord,
    LogRecordLazyView,
    "buffa/log_record",
    "../../datasets/log_record.pb"
);
bench_lazy_view!(
    bench_analytics_event_lazy,
    AnalyticsEvent,
    AnalyticsEventLazyView,
    "buffa/analytics_event",
    "../../datasets/analytics_event.pb"
);
bench_lazy_view!(
    bench_google_message1_lazy,
    bench_buffa::proto3::GoogleMessage1,
    GoogleMessage1LazyView,
    "buffa/google_message1_proto3",
    "../../datasets/google_message1_proto3.pb"
);
bench_lazy_view!(
    bench_media_frame_lazy,
    MediaFrame,
    MediaFrameLazyView,
    "buffa/media_frame",
    "../../datasets/media_frame.pb"
);

/// Build-then-encode benches: unlike `encode`/`encode_view` (which serialize
/// a pre-built struct), these include the cost of populating the message from
/// borrowed source — the per-field `String`/`Vec`/`HashMap` allocs that the
/// view path avoids. Each uses a synthetic fixture representative of the
/// message's shape; both paths populate identical fields, throughput is the
/// encoded length.
///
/// `bench_build_encode!(fn_name, group, OwnedTy, owned_expr, view_expr)` —
/// the two exprs share the source bindings declared above the macro call.
/// Asserts decode-equivalence (not byte-equality, since `HashMap` vs
/// `MapView` iteration order may differ on the wire).
macro_rules! bench_build_encode {
    ($fn_name:ident, $group:literal, $owned_ty:ty, $owned:expr, $view:expr $(,)?) => {
        fn $fn_name(c: &mut Criterion) {
            let probe = ($owned).encode_to_vec();
            let view_bytes = ($view).encode_to_vec();
            assert_eq!(probe.len(), view_bytes.len(), "fixture encode-len mismatch");
            assert_eq!(
                <$owned_ty>::decode_from_slice(&probe).unwrap(),
                <$owned_ty>::decode_from_slice(&view_bytes).unwrap(),
                "owned/view fixtures must decode-equal"
            );
            let mut group = c.benchmark_group($group);
            group.throughput(Throughput::Bytes(probe.len() as u64));
            group.bench_function("build_encode", |b| {
                b.iter(|| criterion::black_box(($owned).encode_to_vec()));
            });
            group.bench_function("build_encode_view", |b| {
                b.iter(|| criterion::black_box(($view).encode_to_vec()));
            });
            group.finish();
        }
    };
}

const TAGS: [&str; 5] = ["payments", "us-west-2a", "canary", "v3.14.2", "k8s"];

bench_build_encode!(
    bench_api_response_build_encode,
    "buffa/api_response",
    ApiResponse,
    ApiResponse {
        request_id: 9_001_234_567_890,
        status_code: 200,
        message: "transaction accepted".into(),
        latency_ms: 17.42,
        cached: true,
        trace_id: Some("4bf92f3577b34da6a3ce929d0e0e4736".into()),
        retry_after_ms: None,
        tags: TAGS.iter().map(|s| (*s).into()).collect(),
        ..Default::default()
    },
    ApiResponseView {
        request_id: 9_001_234_567_890,
        status_code: 200,
        message: "transaction accepted",
        latency_ms: 17.42,
        cached: true,
        trace_id: Some("4bf92f3577b34da6a3ce929d0e0e4736"),
        retry_after_ms: None,
        tags: TAGS.iter().copied().collect(),
        ..Default::default()
    },
);

const LABELS: [(&str, &str); 15] = [
    ("k8s.io/label-key-00", "label-value-0000"),
    ("k8s.io/label-key-01", "label-value-0001"),
    ("k8s.io/label-key-02", "label-value-0002"),
    ("k8s.io/label-key-03", "label-value-0003"),
    ("k8s.io/label-key-04", "label-value-0004"),
    ("k8s.io/label-key-05", "label-value-0005"),
    ("k8s.io/label-key-06", "label-value-0006"),
    ("k8s.io/label-key-07", "label-value-0007"),
    ("k8s.io/label-key-08", "label-value-0008"),
    ("k8s.io/label-key-09", "label-value-0009"),
    ("k8s.io/label-key-10", "label-value-0010"),
    ("k8s.io/label-key-11", "label-value-0011"),
    ("k8s.io/label-key-12", "label-value-0012"),
    ("k8s.io/label-key-13", "label-value-0013"),
    ("k8s.io/label-key-14", "label-value-0014"),
];
const LOG_SVC: &str = "inventory-service-2a";
const LOG_MSG: &str = "GET /api/v1/items?tenant=acme-corp&warehouse=us-west-2a&page=1400 200 17ms";

bench_build_encode!(
    bench_log_record_build_encode,
    "buffa/log_record",
    LogRecord,
    LogRecord {
        service_name: LOG_SVC.into(),
        message: LOG_MSG.into(),
        labels: LABELS
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        ..Default::default()
    },
    LogRecordView {
        service_name: LOG_SVC,
        message: LOG_MSG,
        labels: LABELS.iter().copied().collect(),
        ..Default::default()
    },
);

const PROPS: [(&str, &str); 8] = [
    ("page", "/checkout/confirm"),
    ("referrer", "https://example.com/cart"),
    ("session", "f0e1d2c3b4a59687"),
    ("variant", "control"),
    ("locale", "en-US"),
    ("device", "desktop"),
    ("browser", "firefox-125"),
    ("ab_bucket", "treatment-7"),
];

// `sections` (recursive Nested) omitted: building nested views means a
// `Box<NestedView>` per child — that conflates the alloc-avoidance signal
// with the documented `MessageFieldView` boxing follow-up.
bench_build_encode!(
    bench_analytics_event_build_encode,
    "buffa/analytics_event",
    AnalyticsEvent,
    AnalyticsEvent {
        event_id: "evt_01HW3K9QXAMPLE".into(),
        timestamp: 1_700_000_000_000,
        user_id: "usr_8f7e6d5c4b3a2910".into(),
        properties: PROPS
            .iter()
            .map(|(k, v)| analytics_event::Property {
                key: (*k).into(),
                value: Some(oneof::analytics_event::property::Value::StringValue(
                    (*v).into(),
                )),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    },
    AnalyticsEventView {
        event_id: "evt_01HW3K9QXAMPLE",
        timestamp: 1_700_000_000_000,
        user_id: "usr_8f7e6d5c4b3a2910",
        properties: PROPS
            .iter()
            .map(|(k, v)| PropertyView {
                key: k,
                value: Some(view_oneof::analytics_event::property::Value::StringValue(v)),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    },
);

bench_build_encode!(
    bench_google_message1_build_encode,
    "buffa/google_message1_proto3",
    bench_buffa::proto3::GoogleMessage1,
    bench_buffa::proto3::GoogleMessage1 {
        field1: "the quick brown fox".into(),
        field9: "jumps over the lazy dog".into(),
        field2: 42,
        field3: 17,
        field6: 9001,
        field22: 1_234_567_890_123,
        field12: true,
        field14: true,
        field100: 100,
        field101: 101,
        ..Default::default()
    },
    GoogleMessage1View {
        field1: "the quick brown fox",
        field9: "jumps over the lazy dog",
        field2: 42,
        field3: 17,
        field6: 9001,
        field22: 1_234_567_890_123,
        field12: true,
        field14: true,
        field100: 100,
        field101: 101,
        ..Default::default()
    },
);

static MF_BODY: [u8; 4096] = [0xAB; 4096];
static MF_CHUNKS: [[u8; 1024]; 4] = [[0xC0; 1024], [0xC1; 1024], [0xC2; 1024], [0xC3; 1024]];
static MF_ATT_A: [u8; 512] = [0xA0; 512];
static MF_ATT_B: [u8; 768] = [0xB0; 768];
const MF_ATTACH: [(&str, &[u8]); 2] = [("thumbnail", &MF_ATT_A), ("metadata", &MF_ATT_B)];

bench_build_encode!(
    bench_media_frame_build_encode,
    "buffa/media_frame",
    MediaFrame,
    MediaFrame {
        frame_id: "frame-001a2b3c".into(),
        timestamp_nanos: 1_700_000_000_000_000_000,
        content_type: "video/h264".into(),
        body: MF_BODY.to_vec(),
        chunks: MF_CHUNKS.iter().map(|c| c.to_vec()).collect(),
        attachments: MF_ATTACH
            .iter()
            .map(|(k, v)| ((*k).into(), v.to_vec()))
            .collect(),
        ..Default::default()
    },
    MediaFrameView {
        frame_id: "frame-001a2b3c",
        timestamp_nanos: 1_700_000_000_000_000_000,
        content_type: "video/h264",
        body: &MF_BODY,
        chunks: MF_CHUNKS.iter().map(|c| &c[..]).collect(),
        attachments: MF_ATTACH.iter().copied().collect(),
        ..Default::default()
    },
);

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

fn bench_packed_tile(c: &mut Criterion) {
    benchmark_decode::<PackedTile>(
        c,
        "buffa/packed_tile",
        include_bytes!("../../datasets/packed_tile.pb"),
    );
}

// Many small singular sub-messages per repeated element: the regression target
// for `PointerRepr::Inline` (issue #248). The owned decode/merge cost is
// dominated by per-submessage allocation under the `Box` default.
fn bench_mesh(c: &mut Criterion) {
    benchmark_decode::<TriMesh>(c, "buffa/mesh", include_bytes!("../../datasets/mesh.pb"));
}

fn bench_mesh_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/mesh.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/mesh");
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                criterion::black_box(TriMeshView::decode_view(payload).unwrap());
            }
        });
    });
    group.finish();
}

// Control for the packed-varint reservation: every element is a negative
// (10-byte) varint, the worst case for the old byte-length reserve.
fn bench_packed_signed(c: &mut Criterion) {
    benchmark_decode::<PackedSigned>(
        c,
        "buffa/packed_signed",
        include_bytes!("../../datasets/packed_signed.pb"),
    );
}

fn bench_packed_signed_view(c: &mut Criterion) {
    let dataset = load_dataset(include_bytes!("../../datasets/packed_signed.pb"));
    let bytes = total_payload_bytes(&dataset);
    let mut group = c.benchmark_group("buffa/packed_signed");
    group.throughput(Throughput::Bytes(bytes));

    group.bench_function("decode_view", |b| {
        b.iter(|| {
            for payload in &dataset.payload {
                let view = PackedSignedView::decode_view(payload).unwrap();
                criterion::black_box(&view);
            }
        });
    });

    group.finish();
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
    bench_packed_tile,
    bench_mesh,
    bench_packed_signed,
);

criterion_group!(
    views,
    bench_api_response_view,
    bench_log_record_view,
    bench_analytics_event_view,
    bench_google_message1_view,
    bench_media_frame_view,
    bench_packed_tile_view,
    bench_mesh_view,
    bench_packed_signed_view,
    bench_api_response_view_encode,
    bench_log_record_view_encode,
    bench_analytics_event_view_encode,
    bench_google_message1_view_encode,
    bench_media_frame_view_encode,
    bench_api_response_build_encode,
    bench_log_record_build_encode,
    bench_analytics_event_build_encode,
    bench_google_message1_build_encode,
    bench_media_frame_build_encode,
);

criterion_group!(
    lazy,
    bench_api_response_lazy,
    bench_log_record_lazy,
    bench_analytics_event_lazy,
    bench_google_message1_lazy,
    bench_media_frame_lazy,
);

criterion_group!(
    json,
    bench_api_response_json,
    bench_log_record_json,
    bench_analytics_event_json,
    bench_google_message1_json,
    bench_media_frame_json,
);

criterion_main!(owned, views, lazy, json);

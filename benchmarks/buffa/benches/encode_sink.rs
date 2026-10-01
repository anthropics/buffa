// Encode-sink comparison: the same messages written through `Vec<u8>` and
// `BytesMut`, and through the `encode_to_vec` / `encode_to_bytes` entry
// points.
//
// The encode entry points (`encode`, `encode_to_vec`, ...) compute the exact
// size, then write any `BufMut` through one shared pre-sized cursor (see the
// `buffa::encode_sink` module docs), so the `vec` and `bytesmut` rows are
// expected to track each other; this bench guards that. The `fresh` rows
// start every message with an unreserved sink, which stages messages larger
// than 64 bytes in a scratch buffer, and the `reused` rows write into a sink
// that already has room. `encode_to_bytes` encodes into a `Vec<u8>` and
// converts it (zero-copy).
//
// Not part of the default `task bench` run (gated behind the `encode_sink`
// feature so its IDs never enter the suite's saved baselines):
//
//   cargo bench --features encode_sink --bench encode_sink
//   cargo bench --features encode_sink --bench encode_sink --profile bench-nolto
//
// or `task bench-encode-sink`, which runs both and keeps their criterion
// results in separate directories.
use buffa::{Message, MessageView, ViewEncode};
use bytes::{BufMut, BytesMut};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use bench_buffa::bench::__buffa::view::LogRecordView;
use bench_buffa::bench::{ApiResponse, LogRecord, MediaFrame};
use bench_buffa::benchmarks::BenchmarkDataset;
use bench_buffa::proto3::GoogleMessage1;

fn load(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode_from_slice(data).expect("dataset")
}

/// Throughput is input payload bytes, as in every other `buffa/<shape>/*` row,
/// so MB/s is comparable across benches.
fn total_payload_bytes(d: &BenchmarkDataset) -> u64 {
    d.payload.iter().map(|p| p.len() as u64).sum()
}

fn sinks<M: Message + Default>(c: &mut Criterion, name: &str, data: &[u8]) {
    let ds = load(data);
    let msgs: Vec<M> = ds
        .payload
        .iter()
        .map(|p| M::decode_from_slice(p).unwrap())
        .collect();
    let sizes: Vec<usize> = msgs.iter().map(|m| m.encoded_len() as usize).collect();
    let max_size = sizes.iter().copied().max().unwrap_or(0);

    let mut g = c.benchmark_group(format!("encode_sink/{name}"));
    g.throughput(Throughput::Bytes(total_payload_bytes(&ds)));

    // The two library entry points. `encode_to_vec` is also the `encode` row
    // of `benches/protobuf.rs`; it is repeated here so one report carries the
    // reference next to `encode_to_bytes`.
    g.bench_function("encode_to_vec", |b| {
        b.iter(|| {
            for m in &msgs {
                black_box(m.encode_to_vec());
            }
        })
    });
    g.bench_function("encode_to_bytes", |b| {
        b.iter(|| {
            for m in &msgs {
                black_box(m.encode_to_bytes());
            }
        })
    });

    // `encode` (size pass + write pass) into one reused, pre-grown sink of
    // each type, so the rows differ only in the sink's `chunk_mut` and
    // `advance_mut` cost: no allocation inside the loop, and the size pass
    // is common to both.
    g.bench_function("encode_into_vec_reused", |b| {
        let mut buf: Vec<u8> = Vec::with_capacity(max_size);
        b.iter(|| {
            for m in &msgs {
                buf.clear();
                m.encode(&mut buf);
                black_box(&buf);
            }
        })
    });
    g.bench_function("encode_into_bytesmut_reused", |b| {
        let mut buf = BytesMut::with_capacity(max_size);
        b.iter(|| {
            for m in &msgs {
                buf.clear();
                m.encode(&mut buf);
                black_box(&buf);
            }
        })
    });

    // A fresh, unreserved sink per message, the usage the docs show: a message
    // larger than the 64 bytes an empty `Vec` or `BytesMut` offers is staged in
    // a scratch buffer and copied once.
    g.bench_function("encode_into_fresh_vec", |b| {
        b.iter(|| {
            for m in &msgs {
                let mut buf: Vec<u8> = Vec::new();
                m.encode(&mut buf);
                black_box(buf);
            }
        })
    });
    g.bench_function("encode_into_fresh_bytesmut", |b| {
        b.iter(|| {
            for m in &msgs {
                let mut buf = BytesMut::new();
                m.encode(&mut buf);
                black_box(buf);
            }
        })
    });

    // A caller that frames into its own `BytesMut` (a 5-byte envelope header
    // followed by the message, as an RPC codec does) reserves room for both
    // first; these rows compare encoding into that buffer with encoding to a
    // `Vec` first and copying once. Both write 5 bytes per message beyond the
    // payload throughput above.
    g.bench_function("frame_header_then_encode_into_bytesmut", |b| {
        b.iter(|| {
            for (m, &n) in msgs.iter().zip(&sizes) {
                let mut buf = BytesMut::with_capacity(5 + n);
                buf.put_u8(0);
                buf.put_u32(n as u32);
                m.encode(&mut buf);
                black_box(buf);
            }
        })
    });
    g.bench_function("frame_header_then_put_encoded_vec", |b| {
        b.iter(|| {
            for (m, &n) in msgs.iter().zip(&sizes) {
                let mut buf = BytesMut::with_capacity(5 + n);
                buf.put_u8(0);
                buf.put_u32(n as u32);
                buf.put_slice(&m.encode_to_vec());
                black_box(buf);
            }
        })
    });
    g.finish();
}

fn view_sinks(c: &mut Criterion) {
    let ds = load(include_bytes!("../../datasets/log_record.pb"));
    let views: Vec<LogRecordView<'_>> = ds
        .payload
        .iter()
        .map(|p| LogRecordView::decode_view(p).unwrap())
        .collect();
    let mut g = c.benchmark_group("encode_sink/log_record_view");
    g.throughput(Throughput::Bytes(total_payload_bytes(&ds)));
    g.bench_function("encode_to_vec", |b| {
        b.iter(|| {
            for v in &views {
                let out = v.encode_to_vec();
                debug_assert_eq!(out.len(), v.encoded_len() as usize);
                black_box(out);
            }
        })
    });
    g.bench_function("encode_to_bytes", |b| {
        b.iter(|| {
            for v in &views {
                black_box(v.encode_to_bytes());
            }
        })
    });
    g.finish();
}

fn run(c: &mut Criterion) {
    // Tag-dense shapes, where writes per byte of output are highest: string-heavy
    // (many short length-delimited fields), nested/mixed, and dense small
    // scalars.
    sinks::<LogRecord>(
        c,
        "log_record",
        include_bytes!("../../datasets/log_record.pb"),
    );
    sinks::<ApiResponse>(
        c,
        "api_response",
        include_bytes!("../../datasets/api_response.pb"),
    );
    sinks::<GoogleMessage1>(
        c,
        "google_message1_proto3",
        include_bytes!("../../datasets/google_message1_proto3.pb"),
    );
    // Bytes-heavy control: KB-scale `put_slice` calls dominate, so the
    // per-write cost is amortised and the encode-then-copy framing row buys
    // nothing here.
    sinks::<MediaFrame>(
        c,
        "media_frame",
        include_bytes!("../../datasets/media_frame.pb"),
    );
    view_sinks(c);
}

criterion_group!(grp, run);
criterion_main!(grp);

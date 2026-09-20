// Encode-sink comparison: the same messages written through `Vec<u8>` vs
// `BytesMut`, and through the `encode_to_vec` / `encode_to_bytes` entry
// points.
//
// Why this exists: `BufMut for BytesMut` does not mark `put_slice` (and hence
// the default `put_u8`) `#[inline]`, and LLVM folds `reserve_inner` into it, so
// every tag and varint byte written through a `BytesMut` is an out-of-line
// call — with or without fat LTO (quieted c7i.metal, both profiles:
// `encode_to_bytes` through `BytesMut` was 3.1–3.9x slower than
// `encode_to_vec` on the tag-dense shapes and 1.7x on bytes-heavy
// `media_frame`).
// `Vec<u8>`'s impl is inlined and compiles to a plain store. `encode_to_bytes`
// therefore encodes into a `Vec<u8>` and converts (zero-copy); this bench is
// the reproduction and the guard, and the `bytesmut` rows show what a caller
// encoding into their own `BytesMut` still pays.
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
    // each type, so the rows differ only in the sink's per-`put` cost: no
    // allocation inside the loop, and the size pass is common to both.
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

    // A caller that frames into its own `BytesMut` (a 5-byte envelope header
    // followed by the message, as an RPC codec does) keeps the slow sink even
    // after the `encode_to_bytes` change; these rows show what such a caller
    // pays and what encoding to a `Vec` first and copying once costs instead.
    // Both write 5 bytes per message beyond the payload throughput above.
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
    // Tag-dense shapes, where the per-byte sink cost dominates: string-heavy
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
    // out-of-line call is amortised (1.7x rather than 3-4x) and the
    // encode-then-copy framing row buys nothing here.
    sinks::<MediaFrame>(
        c,
        "media_frame",
        include_bytes!("../../datasets/media_frame.pb"),
    );
    view_sinks(c);
}

criterion_group!(grp, run);
criterion_main!(grp);

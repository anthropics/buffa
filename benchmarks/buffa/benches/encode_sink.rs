// Encode-sink comparison: the same messages written through `Vec<u8>` vs
// `BytesMut`, and through the `encode_to_vec` / `encode_to_bytes` entry
// points, for three payload shapes.
//
// Why this exists: `BufMut for BytesMut` does not mark `put_slice` (and hence
// the default `put_u8`) `#[inline]`, so without whole-program LTO every tag and
// varint byte written through a `BytesMut` is an out-of-line call. `Vec<u8>`'s
// impl is inlined and compiles to a plain store. `encode_to_bytes` therefore
// encodes into a `Vec<u8>` and converts (zero-copy) — this bench is the
// reproduction and the guard.
//
// Run it twice: once at the default bench profile (fat LTO — the gap mostly
// closes, which is why the other benches never showed it) and once the way a
// downstream crate without `[profile.release] lto` builds:
//
//   cargo bench --bench encode_sink
//   cargo bench --bench encode_sink --profile bench-nolto
//
// (or `task bench-encode-sink`, which runs both).
use buffa::{Message, MessageView, ViewEncode};
use bytes::{BufMut, BytesMut};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use bench_buffa::bench::__buffa::view::LogRecordView;
use bench_buffa::bench::{ApiResponse, LogRecord};
use bench_buffa::benchmarks::BenchmarkDataset;
use bench_buffa::proto3::GoogleMessage1;

fn load(data: &[u8]) -> BenchmarkDataset {
    BenchmarkDataset::decode_from_slice(data).expect("dataset")
}

fn sinks<M: Message + Default>(c: &mut Criterion, name: &str, data: &[u8]) {
    let ds = load(data);
    let msgs: Vec<M> = ds
        .payload
        .iter()
        .map(|p| M::decode_from_slice(p).unwrap())
        .collect();
    let sizes: Vec<usize> = msgs.iter().map(|m| m.encoded_len() as usize).collect();
    let total: u64 = sizes.iter().map(|&n| n as u64).sum();

    let mut g = c.benchmark_group(format!("encode_sink/{name}"));
    g.throughput(Throughput::Bytes(total));

    // The two library entry points.
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

    // The same write pass into a pre-sized sink of each type: isolates the
    // sink's per-`put` cost from allocation and size computation.
    g.bench_function("encode_into_vec_presized", |b| {
        b.iter(|| {
            for (m, &n) in msgs.iter().zip(&sizes) {
                let mut buf: Vec<u8> = Vec::with_capacity(n);
                m.encode(&mut buf);
                black_box(buf);
            }
        })
    });
    g.bench_function("encode_into_bytesmut_presized", |b| {
        b.iter(|| {
            for (m, &n) in msgs.iter().zip(&sizes) {
                let mut buf = BytesMut::with_capacity(n);
                m.encode(&mut buf);
                black_box(buf);
            }
        })
    });
    // A caller that frames into its own `BytesMut` (e.g. a 5-byte envelope
    // header followed by the message) keeps the slow path even after the
    // `encode_to_bytes` change; this row shows what such callers still pay
    // and what encoding to a `Vec` first and copying once would cost instead.
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
    let total: u64 = ds.payload.iter().map(|p| p.len() as u64).sum();
    let views: Vec<LogRecordView<'_>> = ds
        .payload
        .iter()
        .map(|p| LogRecordView::decode_view(p).unwrap())
        .collect();
    let mut g = c.benchmark_group("encode_sink/log_record_view");
    g.throughput(Throughput::Bytes(total));
    g.bench_function("encode_to_vec", |b| {
        b.iter(|| {
            for v in &views {
                black_box(v.encode_to_vec());
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
    // String-heavy (many short length-delimited fields), nested/mixed, and
    // dense small scalars respectively.
    sinks::<LogRecord>(c, "log_record", include_bytes!("../../datasets/log_record.pb"));
    sinks::<ApiResponse>(c, "api_response", include_bytes!("../../datasets/api_response.pb"));
    sinks::<GoogleMessage1>(
        c,
        "google_message1",
        include_bytes!("../../datasets/google_message1_proto3.pb"),
    );
    view_sinks(c);
}

criterion_group!(grp, run);
criterion_main!(grp);

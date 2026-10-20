// Opt-in textproto bytes escaping microbenchmark.
// cargo bench --manifest-path benchmarks/buffa/Cargo.toml --no-default-features \
//   --features text_escape_bytes --bench text_escape_bytes
use buffa::text::escape_bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

fn benchmark_text_escape_bytes(c: &mut Criterion) {
    let mut mixed = vec![b'a'; 4096];
    mixed[3072] = b'\n';
    mixed[4080] = b'"';

    let mut escaped = vec![b'a'; 256];
    for i in (0..escaped.len()).step_by(8) {
        escaped[i] = b'"';
    }

    let cases = [
        ("plain_16", vec![b'a'; 16]),
        ("plain_64", vec![b'a'; 64]),
        ("plain_256", vec![b'a'; 256]),
        ("plain_4096", vec![b'a'; 4096]),
        ("plain_65536", vec![b'a'; 65536]),
        ("mixed_4096", mixed),
        ("binary_4096", vec![0xff; 4096]),
        ("escaped_256", escaped),
    ];

    let mut group = c.benchmark_group("text_escape_bytes");
    for (name, input) in &cases {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function(*name, |b| {
            // Reuse the destination to isolate the escaping work from
            // allocation and capacity growth.
            let mut output = String::with_capacity(input.len() * 4 + 2);
            b.iter(|| {
                output.clear();
                escape_bytes(black_box(input.as_slice()), &mut output).unwrap();
                black_box(&output);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, benchmark_text_escape_bytes);
criterion_main!(benches);

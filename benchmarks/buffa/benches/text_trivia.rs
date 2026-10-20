// Opt-in textproto tokenizer benchmark: cargo bench --features text_trivia --bench text_trivia
// Baseline and optimized builds use the same input and measurement harness.
use buffa::text::{TokenKind, Tokenizer};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

fn tokenize(input: &str) -> usize {
    let mut t = Tokenizer::new(black_box(input));
    let mut count = 0;
    loop {
        let token = t.read().expect("benchmark input must be valid textproto");
        if token.kind == TokenKind::Eof {
            return count;
        }
        black_box(token.raw);
        count += 1;
    }
}

fn benchmark_trivia(c: &mut Criterion) {
    let cases = [
        ("dense_fields", "field: 1\n".repeat(512)),
        (
            "large_whitespace",
            format!("{}field: 1", " \t\r\n\x0b\x0c".repeat(10_923)),
        ),
        (
            "comment_lines",
            format!("{}field: 1", format!("#{}\n \t", "x".repeat(128)).repeat(256)),
        ),
    ];
    let mut group = c.benchmark_group("text_tokenizer_trivia");
    for (name, input) in &cases {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function(*name, |b| b.iter(|| black_box(tokenize(black_box(input)))));
    }
    group.finish();
}

criterion_group!(benches, benchmark_trivia);
criterion_main!(benches);

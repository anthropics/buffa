// Reproducible, dependency-free A/B benchmark for protobuf tag encoding.
// Run: cargo bench -p buffa --bench tag_encoding
// "before" exactly reproduces the original Tag::encode body.
// Both paths share one release build and an identically preallocated sink.
use buffa::encoding::{encode_varint, Tag, WireType, MAX_FIELD_NUMBER};
use buffa::EncodeSink;
use std::hint::black_box;
use std::time::Instant;

const VALUES: usize = 2048;
const BATCHES: usize = 2000;
const REPEATS: usize = 7;

#[inline]
fn before(tag: Tag, buf: &mut impl EncodeSink) {
    let value = ((tag.field_number() as u64) << 3) | tag.wire_type() as u64;
    encode_varint(value, buf);
}

#[inline]
fn after(tag: Tag, buf: &mut impl EncodeSink) {
    tag.encode(buf);
}

fn run(tags: &[Tag], f: impl Fn(Tag, &mut Vec<u8>)) -> f64 {
    let mut buf = Vec::with_capacity(tags.len() * 5);
    let started = Instant::now();
    for _ in 0..BATCHES {
        buf.clear();
        for &tag in tags {
            f(black_box(tag), &mut buf);
        }
        black_box(&buf);
    }
    started.elapsed().as_nanos() as f64 / (tags.len() * BATCHES) as f64
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn tags(fields: impl Iterator<Item = u32>) -> Vec<Tag> {
    fields
        .enumerate()
        .map(|(i, field)| {
            Tag::new(
                field,
                if i % 2 == 0 {
                    WireType::Varint
                } else {
                    WireType::LengthDelimited
                },
            )
        })
        .collect()
}

fn cases() -> Vec<(&'static str, Vec<Tag>)> {
    vec![
        ("1-byte", tags((0..VALUES).map(|i| 1 + (i % 15) as u32))),
        ("2-byte", tags((0..VALUES).map(|i| 16 + (i % 2032) as u32))),
        (
            "mixed-short",
            tags((0..VALUES).map(|i| {
                if i % 3 == 0 {
                    1 + (i % 15) as u32
                } else {
                    16 + (i % 2032) as u32
                }
            })),
        ),
        ("3-byte", tags((0..VALUES).map(|i| 2048 + (i * 13) as u32))),
        (
            "5-byte",
            tags((0..VALUES).map(|i| MAX_FIELD_NUMBER - i as u32)),
        ),
    ]
}

fn main() {
    println!(
        "Tag encoding A/B, {} tags/batch, {} batches/repeat, {} repeats",
        VALUES, BATCHES, REPEATS
    );
    println!("case | before ns/tag | after ns/tag | speedup");
    for (name, values) in cases() {
        let mut old = Vec::new();
        let mut new = Vec::new();
        for &tag in &values {
            before(tag, &mut old);
            tag.encode(&mut new);
        }
        assert_eq!(old, new, "{name} differs from baseline");

        let mut baseline = Vec::with_capacity(REPEATS);
        let mut optimized = Vec::with_capacity(REPEATS);
        black_box(run(&values, before));
        black_box(run(&values, after));
        for repeat in 0..REPEATS {
            if repeat % 2 == 0 {
                baseline.push(run(&values, before));
                optimized.push(run(&values, after));
            } else {
                optimized.push(run(&values, after));
                baseline.push(run(&values, before));
            }
        }
        let base = median(&mut baseline);
        let head = median(&mut optimized);
        println!("{name} | {base:.3} | {head:.3} | {:.2}x", base / head);
    }
}

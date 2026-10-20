//! Opt-in A/B benchmark for ProtoJSON 64-bit integer formatting.
//!
//! cargo run --release -p buffa --features json --example json_int64_bench
//!
//! The legacy wrappers reproduce the previous \`serialize_str(&value.to_string())\`
//! implementation. Both paths encode the same data using serde_json, with
//! preallocated, reused output buffers. No benchmark dependency is needed.

use serde::{Serialize, Serializer};
use std::time::Instant;

struct LegacyI64(i64);
struct CurrentI64(i64);
struct LegacyU64(u64);
struct CurrentU64(u64);

impl Serialize for LegacyI64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl Serialize for CurrentI64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::int64::serialize(&self.0, serializer)
    }
}

impl Serialize for LegacyU64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl Serialize for CurrentU64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::uint64::serialize(&self.0, serializer)
    }
}

fn timed<T: Serialize>(values: &[T], output: &mut Vec<u8>, iterations: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        output.clear();
        serde_json::to_writer(&mut *output, values).unwrap();
        std::hint::black_box(output.as_slice());
    }
    start.elapsed().as_nanos() as f64 / iterations as f64
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn compare<A: Serialize, B: Serialize>(label: &str, legacy: &[A], current: &[B]) {
    assert_eq!(
        serde_json::to_vec(legacy).unwrap(),
        serde_json::to_vec(current).unwrap(),
        "{label} serialization must be byte-identical",
    );

    let iterations = if legacy.len() < 128 { 20_000 } else { 500 };
    let mut baseline_output = Vec::with_capacity(128 * 1024);
    let mut optimized_output = Vec::with_capacity(128 * 1024);

    // Exclude cold-start effects and allocation of the output vector.
    timed(legacy, &mut baseline_output, 32);
    timed(current, &mut optimized_output, 32);

    let mut old_ns = Vec::with_capacity(11);
    let mut new_ns = Vec::with_capacity(11);
    for round in 0..11 {
        if round % 2 == 0 {
            old_ns.push(timed(legacy, &mut baseline_output, iterations));
            new_ns.push(timed(current, &mut optimized_output, iterations));
        } else {
            new_ns.push(timed(current, &mut optimized_output, iterations));
            old_ns.push(timed(legacy, &mut baseline_output, iterations));
        }
    }

    let old = median(&mut old_ns);
    let new = median(&mut new_ns);
    println!(
        "BENCH {label} count={} legacy={old:.1} ns/op optimized={new:.1} ns/op speedup={:.2}x",
        legacy.len(),
        old / new,
    );
}

fn main() {
    let signed: Vec<i64> = (0..4096)
        .map(|i| match i % 7 {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => -1,
            3 => 0,
            4 => 127,
            5 => -(i as i64),
            _ => i64::MAX - i as i64,
        })
        .collect();
    let unsigned: Vec<u64> = (0..4096)
        .map(|i| match i % 5 {
            0 => u64::MAX,
            1 => 0,
            2 => 127,
            3 => u32::MAX as u64,
            _ => u64::MAX - i as u64,
        })
        .collect();

    let old_i64: Vec<_> = signed.iter().copied().map(LegacyI64).collect();
    let new_i64: Vec<_> = signed.iter().copied().map(CurrentI64).collect();
    let old_u64: Vec<_> = unsigned.iter().copied().map(LegacyU64).collect();
    let new_u64: Vec<_> = unsigned.iter().copied().map(CurrentU64).collect();

    for count in [32, 4096] {
        compare("int64", &old_i64[..count], &new_i64[..count]);
        compare("uint64", &old_u64[..count], &new_u64[..count]);
    }
}

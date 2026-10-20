//! Run with: cargo run --release -p buffa --features json --example json_string_key_map_bench
//! Compare the same example on the base and optimized revisions.

#[cfg(feature = "json")]
mod bench {
    use std::collections::BTreeMap;
    use std::fmt::Display;
    use std::hint::black_box;
    use std::time::Instant;

    use serde::{Serialize, Serializer};

    struct JsonMap<'a, K: Ord, V>(&'a BTreeMap<K, V>);

    impl<K: Ord + Display, V: Serialize> Serialize for JsonMap<'_, K, V> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            buffa::json_helpers::string_key_map::serialize(self.0, serializer)
        }
    }

    fn measure<K: Ord + Display, V: Serialize>(label: &str, map: &BTreeMap<K, V>) {
        let iterations = match map.len() {
            0..=16 => 30_000,
            17..=256 => 5_000,
            _ => 500,
        };
        let mut output = Vec::new();
        let value = JsonMap(map);

        for _ in 0..iterations / 10 {
            output.clear();
            serde_json::to_writer(&mut output, black_box(&value)).unwrap();
        }

        let mut ns_per_op = [0f64; 9];
        for sample in &mut ns_per_op {
            let start = Instant::now();
            for _ in 0..iterations {
                output.clear();
                serde_json::to_writer(&mut output, black_box(&value)).unwrap();
                black_box(output.len());
            }
            *sample = start.elapsed().as_nanos() as f64 / iterations as f64;
        }
        ns_per_op.sort_by(f64::total_cmp);
        println!("{label}: {:.2} ns/op, {} bytes", ns_per_op[4], output.len());
    }

    pub fn run() {
        let mut bools = BTreeMap::new();
        bools.insert(false, 0u32);
        bools.insert(true, 1);
        measure("bool/2", &bools);

        let small: BTreeMap<i32, u32> = (0..16).map(|i| (i * 17 - 100, i as u32)).collect();
        measure("i32/16", &small);

        let medium: BTreeMap<i32, u32> = (0..256).map(|i| (i * 17 - 1000, i as u32)).collect();
        measure("i32/256", &medium);

        let large_signed: BTreeMap<i64, u32> = (0..4096)
            .map(|i| (i as i64 * 1_000_003 - 1_000_000_003, i as u32))
            .collect();
        measure("i64/4096", &large_signed);

        let large_unsigned: BTreeMap<u64, u32> = (0..4096)
            .map(|i| (10_000_000_000_000_u64 + i as u64 * 31_337, i as u32))
            .collect();
        measure("u64/4096", &large_unsigned);

        let strings: BTreeMap<String, u32> = (0..256)
            .map(|i| (format!("key_{i:06}"), i as u32))
            .collect();
        measure("string/256 (control)", &strings);
    }
}

#[cfg(feature = "json")]
fn main() {
    bench::run();
}

#[cfg(not(feature = "json"))]
fn main() {
    eprintln!("Run with --features json");
}

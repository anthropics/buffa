//! Generate binary BenchmarkDataset files for the custom benchmark messages.
//!
//! Each dataset contains 50 varied payloads to provide realistic branch
//! prediction and cache behavior during benchmarking.

use buffa::Message;
use rand::{Rng, SeedableRng};
use std::fs;
use std::path::Path;

#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
mod proto {
    buffa::include_proto!("bench");
}
#[allow(
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::match_single_binding,
    clippy::upper_case_acronyms,
    non_camel_case_types,
    unused_imports,
    dead_code
)]
mod dataset_proto {
    buffa::include_proto!("benchmarks");
}

use proto::analytics_event::{Nested, Property};
use proto::__buffa::oneof::analytics_event::property::Value;
use proto::log_record::Context;
use proto::*;

const NUM_PAYLOADS: usize = 50;

fn random_string(rng: &mut impl Rng, min_len: usize, max_len: usize) -> String {
    let len = rng.random_range(min_len..=max_len);
    (0..len)
        .map(|_| rng.random_range(b'a'..=b'z') as char)
        .collect()
}

fn random_hex(rng: &mut impl Rng, len: usize) -> String {
    (0..len)
        .map(|_| format!("{:x}", rng.random_range(0u8..16)))
        .collect()
}

fn choose<'a, T>(slice: &'a [T], rng: &mut impl Rng) -> &'a T {
    &slice[rng.random_range(0..slice.len())]
}

fn gen_api_response(rng: &mut impl Rng) -> ApiResponse {
    let num_tags = rng.random_range(0..=5);
    ApiResponse {
        request_id: rng.random_range(1..i64::MAX),
        status_code: *choose(&[200, 201, 400, 404, 500], rng),
        message: random_string(rng, 10, 100),
        latency_ms: rng.random_range(0.1..500.0),
        cached: rng.random_bool(0.3),
        trace_id: if rng.random_bool(0.7) {
            Some(random_hex(rng, 32))
        } else {
            None
        },
        retry_after_ms: if rng.random_bool(0.1) {
            Some(rng.random_range(1000..60000))
        } else {
            None
        },
        tags: (0..num_tags).map(|_| random_string(rng, 3, 15)).collect(),
        ..Default::default()
    }
}

fn gen_log_record(rng: &mut impl Rng) -> LogRecord {
    let num_labels = rng.random_range(2..=8);
    LogRecord {
        timestamp_nanos: rng.random_range(1_700_000_000_000_000_000i64..1_800_000_000_000_000_000),
        service_name: random_string(rng, 5, 30),
        instance_id: format!("{}-{}", random_string(rng, 5, 10), rng.random_range(1..100)),
        severity: buffa::EnumValue::from(rng.random_range(0..=4)),
        message: random_string(rng, 50, 500),
        labels: (0..num_labels)
            .map(|_| (random_string(rng, 3, 15), random_string(rng, 5, 50)))
            .collect(),
        trace_id: random_hex(rng, 32),
        span_id: random_hex(rng, 16),
        source: if rng.random_bool(0.8) {
            buffa::MessageField::some(Context {
                file: format!("src/{}.rs", random_string(rng, 5, 20)),
                line: rng.random_range(1..2000),
                function: random_string(rng, 5, 30),
                ..Default::default()
            })
        } else {
            buffa::MessageField::none()
        },
        ..Default::default()
    }
}

fn gen_property(rng: &mut impl Rng) -> Property {
    let value = match rng.random_range(0..4) {
        0 => Some(Value::StringValue(random_string(rng, 5, 50))),
        1 => Some(Value::IntValue(rng.random_range(-1000..1000))),
        2 => Some(Value::DoubleValue(rng.random_range(-100.0..100.0))),
        _ => Some(Value::BoolValue(rng.random_bool(0.5))),
    };
    Property {
        key: random_string(rng, 3, 20),
        value,
        ..Default::default()
    }
}

fn gen_nested(rng: &mut impl Rng, depth: usize) -> Nested {
    let num_attrs = rng.random_range(1..=5);
    let num_children = if depth > 0 {
        rng.random_range(0..=3)
    } else {
        0
    };
    Nested {
        name: random_string(rng, 5, 20),
        attributes: (0..num_attrs).map(|_| gen_property(rng)).collect(),
        children: (0..num_children)
            .map(|_| gen_nested(rng, depth - 1))
            .collect(),
        ..Default::default()
    }
}

fn random_bytes(rng: &mut impl Rng, min_len: usize, max_len: usize) -> Vec<u8> {
    let len = rng.random_range(min_len..=max_len);
    let mut buf = vec![0u8; len];
    rng.fill_bytes(&mut buf);
    buf
}

fn gen_media_frame(rng: &mut impl Rng) -> MediaFrame {
    let num_chunks = rng.random_range(2..=6);
    let num_attachments = rng.random_range(0..=4);
    MediaFrame {
        frame_id: random_hex(rng, 32),
        timestamp_nanos: rng.random_range(1_700_000_000_000_000_000i64..1_800_000_000_000_000_000),
        content_type: (*choose(
            &[
                "application/octet-stream",
                "image/jpeg",
                "image/png",
                "video/mp4",
                "audio/opus",
            ],
            rng,
        ))
        .to_string(),
        body: random_bytes(rng, 1024, 10_240),
        chunks: (0..num_chunks)
            .map(|_| random_bytes(rng, 200, 2000))
            .collect(),
        attachments: (0..num_attachments)
            .map(|_| (random_string(rng, 5, 20), random_bytes(rng, 50, 500)))
            .collect(),
        ..Default::default()
    }
}

fn gen_analytics_event(rng: &mut impl Rng) -> AnalyticsEvent {
    let num_props = rng.random_range(3..=10);
    let num_sections = rng.random_range(2..=5);
    AnalyticsEvent {
        event_id: random_hex(rng, 24),
        timestamp: rng.random_range(1_700_000_000..1_800_000_000),
        user_id: format!("user-{}", rng.random_range(1..100_000)),
        properties: (0..num_props).map(|_| gen_property(rng)).collect(),
        sections: (0..num_sections).map(|_| gen_nested(rng, 3)).collect(),
        ..Default::default()
    }
}

fn write_dataset<M: Message>(name: &str, message_name: &str, output_dir: &Path, payloads: Vec<M>) {
    let encoded_payloads: Vec<Vec<u8>> = payloads.iter().map(|m| m.encode_to_vec()).collect();

    let total_bytes: usize = encoded_payloads.iter().map(|p| p.len()).sum();
    let avg_bytes = total_bytes / encoded_payloads.len();
    println!(
        "{name}: {count} payloads, {total_bytes} total bytes, ~{avg_bytes} bytes/payload avg",
        count = encoded_payloads.len(),
    );

    let dataset = dataset_proto::BenchmarkDataset {
        name: name.to_string(),
        message_name: message_name.to_string(),
        payload: encoded_payloads,
        ..Default::default()
    };

    let path = output_dir.join(format!("{name}.pb"));
    fs::write(&path, dataset.encode_to_vec()).expect("failed to write dataset");
}

fn main() {
    let output_dir = Path::new("../datasets");
    fs::create_dir_all(output_dir).expect("failed to create output dir");

    let mut rng = rand::rngs::SmallRng::seed_from_u64(42);

    write_dataset(
        "api_response",
        "bench.ApiResponse",
        output_dir,
        (0..NUM_PAYLOADS)
            .map(|_| gen_api_response(&mut rng))
            .collect(),
    );

    write_dataset(
        "log_record",
        "bench.LogRecord",
        output_dir,
        (0..NUM_PAYLOADS)
            .map(|_| gen_log_record(&mut rng))
            .collect(),
    );

    write_dataset(
        "analytics_event",
        "bench.AnalyticsEvent",
        output_dir,
        (0..NUM_PAYLOADS)
            .map(|_| gen_analytics_event(&mut rng))
            .collect(),
    );

    write_dataset(
        "media_frame",
        "bench.MediaFrame",
        output_dir,
        (0..NUM_PAYLOADS)
            .map(|_| gen_media_frame(&mut rng))
            .collect(),
    );

    println!("Datasets written to {}", output_dir.display());
}

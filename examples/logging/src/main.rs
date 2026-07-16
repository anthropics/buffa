//! Structured logging CLI — demonstrates cross-package protobuf types,
//! map fields, length-delimited I/O, and `buf generate` workflow.
//!
//! Usage:
//!   logging write <file>           Write sample log entries
//!   logging read <file>            Read and display log entries
//!   logging filter <file> <level>  Show entries at or above a severity

#[allow(clippy::upper_case_acronyms)]
mod gen;

use buffa::Message;
use gen::buffa::examples::context::v1::RequestContext;
use gen::buffa::examples::log::v1::{LogBatch, LogEntry, Severity};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: logging <write|read|filter> <file> [level]");
        std::process::exit(1);
    }

    let command = &args[1];
    let file_path = &args[2];

    match command.as_str() {
        "write" => cmd_write(file_path),
        "read" => cmd_read(file_path),
        "filter" => {
            let level = args.get(3).map(|s| s.as_str()).unwrap_or("INFO");
            cmd_filter(file_path, level);
        }
        _ => {
            eprintln!("Unknown command: {command}");
            std::process::exit(1);
        }
    }
}

fn now_timestamp() -> buffa_types::google::protobuf::Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    buffa_types::google::protobuf::Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
        ..Default::default()
    }
}

fn severity_name(s: &buffa::EnumValue<Severity>) -> &'static str {
    match s {
        buffa::EnumValue::Known(Severity::DEBUG) => "DEBUG",
        buffa::EnumValue::Known(Severity::INFO) => "INFO",
        buffa::EnumValue::Known(Severity::WARN) => "WARN",
        buffa::EnumValue::Known(Severity::ERROR) => "ERROR",
        buffa::EnumValue::Known(Severity::FATAL) => "FATAL",
        _ => "UNKNOWN",
    }
}

fn parse_severity(s: &str) -> Severity {
    match s.to_uppercase().as_str() {
        "DEBUG" => Severity::DEBUG,
        "INFO" => Severity::INFO,
        "WARN" => Severity::WARN,
        "ERROR" => Severity::ERROR,
        "FATAL" => Severity::FATAL,
        _ => Severity::SEVERITY_UNSPECIFIED,
    }
}

fn cmd_write(file_path: &str) {
    // Build sample log entries demonstrating various features.
    let ctx = RequestContext {
        request_id: "req-abc-123".into(),
        user_id: "user-42".into(),
        method: "POST".into(),
        path: "/api/v1/items".into(),
        metadata: [
            ("region".into(), "us-east-1".into()),
            ("version".into(), "1.2.3".into()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };

    let entries = vec![
        LogEntry {
            timestamp: buffa::MessageField::some(now_timestamp()),
            severity: buffa::EnumValue::Known(Severity::INFO),
            message: "Request received".into(),
            logger: "http::server".into(),
            context: buffa::MessageField::some(ctx.clone()),
            fields: [
                ("content_type".into(), "application/json".into()),
                ("content_length".into(), "1024".into()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        LogEntry {
            timestamp: buffa::MessageField::some(now_timestamp()),
            severity: buffa::EnumValue::Known(Severity::DEBUG),
            message: "Parsing request body".into(),
            logger: "http::parser".into(),
            context: buffa::MessageField::some(ctx.clone()),
            ..Default::default()
        },
        LogEntry {
            timestamp: buffa::MessageField::some(now_timestamp()),
            severity: buffa::EnumValue::Known(Severity::WARN),
            message: "Slow query detected".into(),
            logger: "db::query".into(),
            context: buffa::MessageField::some(ctx.clone()),
            fields: [
                ("query_ms".into(), "1523".into()),
                ("table".into(), "items".into()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        LogEntry {
            timestamp: buffa::MessageField::some(now_timestamp()),
            severity: buffa::EnumValue::Known(Severity::ERROR),
            message: "Failed to write to cache".into(),
            logger: "cache::redis".into(),
            fields: [("error".into(), "connection refused".into())]
                .into_iter()
                .collect(),
            ..Default::default()
        },
    ];

    let batch = LogBatch {
        entries,
        ..Default::default()
    };

    // Write as a single length-delimited message.
    let mut out = Vec::new();
    batch.encode_length_delimited(&mut out);
    std::fs::write(file_path, &out).unwrap_or_else(|e| {
        eprintln!("Error writing {file_path}: {e}");
        std::process::exit(1);
    });
    println!(
        "Wrote {} entries ({} bytes, length-delimited).",
        batch.entries.len(),
        out.len()
    );
}

fn cmd_read(file_path: &str) {
    let batch = read_batch(file_path);
    for entry in &batch.entries {
        print_entry(entry);
    }
    println!("\n{} entries total.", batch.entries.len());
}

fn cmd_filter(file_path: &str, level: &str) {
    let min_severity = parse_severity(level) as i32;
    let batch = read_batch(file_path);

    let mut count = 0;
    for entry in &batch.entries {
        let entry_severity = entry.severity.to_i32();
        if entry_severity >= min_severity {
            print_entry(entry);
            count += 1;
        }
    }
    println!("\n{count} entries at {level} or above.");
}

fn read_batch(file_path: &str) -> LogBatch {
    let data = std::fs::read(file_path).unwrap_or_else(|e| {
        eprintln!("Error reading {file_path}: {e}");
        std::process::exit(1);
    });
    // Decode length-delimited message.
    let mut cursor = &data[..];
    LogBatch::decode_length_delimited(&mut cursor).unwrap_or_else(|e| {
        eprintln!("Error decoding {file_path}: {e}");
        std::process::exit(1);
    })
}

fn print_entry(entry: &LogEntry) {
    let ts = entry
        .timestamp
        .as_option()
        .map(|t| format!("{}s", t.seconds))
        .unwrap_or_else(|| "???".into());

    let severity = severity_name(&entry.severity);
    println!("[{ts}] {severity:5} {}: {}", entry.logger, entry.message);

    if let Some(ctx) = entry.context.as_option() {
        println!(
            "        ctx: {} {} {} user={}",
            ctx.request_id, ctx.method, ctx.path, ctx.user_id
        );
        if !ctx.metadata.is_empty() {
            let meta: Vec<String> = ctx
                .metadata
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            println!("        metadata: {}", meta.join(", "));
        }
    }

    if !entry.fields.is_empty() {
        let fields: Vec<String> = entry
            .fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        println!("        fields: {}", fields.join(", "));
    }
}

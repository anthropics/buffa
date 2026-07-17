// Per-message-isolated benchmark build: exactly one message feature is enabled
// (e.g. `--no-default-features --features media_frame`), and only that message's
// proto is compiled, so no other shape's decoder enters the codegen unit.
fn main() {
    let msgs = [
        ("API_RESPONSE", "../proto/iso/api_response.proto"),
        ("LOG_RECORD", "../proto/iso/log_record.proto"),
        ("ANALYTICS_EVENT", "../proto/iso/analytics_event.proto"),
        ("GOOGLE_MESSAGE1", "../proto/iso/google_message1.proto"),
        ("MEDIA_FRAME", "../proto/iso/media_frame.proto"),
        ("PACKED_TILE", "../proto/iso/packed_tile.proto"),
        ("MESH", "../proto/iso/mesh.proto"),
        ("COLUMN_BATCH", "../proto/iso/column_batch.proto"),
    ];
    let mut files = vec!["../proto/benchmarks.proto".to_string()];
    for (feat, path) in msgs {
        if std::env::var(format!("CARGO_FEATURE_{feat}")).is_ok() {
            files.push(path.to_string());
        }
    }
    buffa_build::Config::new()
        .files(&files)
        .includes(&["../proto/iso/", "../proto/"])
        .generate_json(true)
        .include_file("_include.rs")
        .compile()
        .expect("failed to compile benchmark protos");
}

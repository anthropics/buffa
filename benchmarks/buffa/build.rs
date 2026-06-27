use std::env;

// Per-message-isolated benchmark build. With `--no-default-features --features
// iso,<msg>` only that message's proto is compiled (reflect/lazy off), so no
// other shape's decoder enters the codegen unit. The default feature set emits
// all messages + reflect + lazy views for the combined `protobuf`/`reflect`
// benches.
fn main() {
    let msgs = [
        ("API_RESPONSE", "../proto/iso/api_response.proto"),
        ("LOG_RECORD", "../proto/iso/log_record.proto"),
        ("ANALYTICS_EVENT", "../proto/iso/analytics_event.proto"),
        ("MEDIA_FRAME", "../proto/iso/media_frame.proto"),
        ("PACKED_TILE", "../proto/iso/packed_tile.proto"),
        ("MESH", "../proto/iso/mesh.proto"),
        (
            "GOOGLE_MESSAGE1",
            "../proto/benchmark_message1_proto3.proto",
        ),
    ];
    let mut files = vec!["../proto/benchmarks.proto".to_string()];
    for (feat, path) in msgs {
        if env::var(format!("CARGO_FEATURE_{feat}")).is_ok() {
            files.push(path.to_string());
        }
    }
    let mode = if env::var("CARGO_FEATURE_REFLECT").is_ok() {
        buffa_build::ReflectMode::VTable
    } else {
        buffa_build::ReflectMode::Off
    };
    let lazy = env::var("CARGO_FEATURE_LAZY").is_ok();
    buffa_build::Config::new()
        .files(&files)
        .includes(&["../proto/iso/", "../proto/"])
        .generate_json(true)
        .reflect_mode(mode)
        .lazy_views(lazy)
        .compile()
        .expect("failed to compile benchmark protos");
}

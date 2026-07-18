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
        ("COLUMN_BATCH", "../proto/iso/column_batch.proto"),
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

    if env::var("CARGO_FEATURE_ANALYTICS_EVENT").is_ok() {
        let out_dir = std::path::PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set"));
        for (name, smol_strings, small_lists) in [
            ("analytics_smolstr", true, false),
            ("analytics_smallvec", false, true),
            ("analytics_smolstr_smallvec", true, true),
        ] {
            let variant_dir = out_dir.join(name);
            std::fs::create_dir_all(&variant_dir).expect("create analytics variant output");
            let mut config = buffa_build::Config::new()
                .files(&["../proto/iso/analytics_event.proto"])
                .includes(&["../proto/iso/"])
                .generate_json(true)
                .out_dir(variant_dir);
            if smol_strings {
                config = config.string_type_custom("::buffa_smolstr::SmolStr");
            }
            if small_lists {
                // ponytail: keep recursive children on Vec; add a heap-backed custom
                // list axis only if a recursive-collection benchmark is requested.
                config = config.repeated_type_custom_in(
                    "crate::SmallList<*>",
                    &[
                        ".bench.AnalyticsEvent.properties",
                        ".bench.AnalyticsEvent.sections",
                        ".bench.AnalyticsEvent.Nested.attributes",
                    ],
                );
            }
            config.compile().expect("compile AnalyticsEvent owned-type variant");
        }
    }
}

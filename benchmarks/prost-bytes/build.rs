fn main() {
    // `.bytes(["."])` tells prost-build to emit `bytes::Bytes` instead of
    // `Vec<u8>` for every `bytes` field. Combined with a `bytes::Bytes` decode
    // input, this exercises prost's zero-copy slicing path for `bytes` fields
    // (the comparison point for buffa's view-based zero-copy decode).
    prost_build::Config::new()
        .bytes(["."])
        .compile_protos(
            &[
                "../proto/bench_messages.proto",
                "../proto/benchmarks.proto",
                "../proto/benchmark_message1_proto3.proto",
            ],
            &["../proto/"],
        )
        .expect("failed to compile benchmark protos");
}

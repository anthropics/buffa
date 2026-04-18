fn main() {
    buffa_build::Config::new()
        .files(&[
            "../proto/bench_messages.proto",
            "../proto/benchmarks.proto",
            "../proto/benchmark_message1_proto3.proto",
        ])
        .includes(&["../proto/"])
        .generate_json(true)
        .compile()
        .expect("failed to compile benchmark protos");
}

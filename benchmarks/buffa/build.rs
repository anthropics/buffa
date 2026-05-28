fn main() {
    buffa_build::Config::new()
        .files(&[
            "../proto/bench_messages.proto",
            "../proto/benchmarks.proto",
            "../proto/benchmark_message1_proto3.proto",
        ])
        .includes(&["../proto/"])
        .generate_json(true)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .compile()
        .expect("failed to compile benchmark protos");
}

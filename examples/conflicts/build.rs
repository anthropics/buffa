fn main() {
    buffa_build::Config::new()
        .files(&["proto/conflicts.proto"])
        .includes(&["proto/"])
        .generate_views(true)
        .include_file("_include.rs")
        .compile()
        .expect("protobuf compilation failed");
}

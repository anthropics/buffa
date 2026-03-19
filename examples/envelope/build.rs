fn main() {
    buffa_build::Config::new()
        .files(&["proto/envelope.proto"])
        .includes(&["proto/"])
        // `generate_json(true)` is what enables the `"[pkg.ext]"` JSON keys:
        // it makes codegen emit the per-message `#[serde(flatten)]` wrapper
        // and the file-level `register_types()` function.
        .generate_json(true)
        .include_file("_include.rs")
        .compile()
        .expect("protobuf compilation failed");
}

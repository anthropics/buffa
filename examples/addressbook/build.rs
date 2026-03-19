fn main() {
    buffa_build::Config::new()
        .files(&["proto/addressbook.proto"])
        .includes(&["proto/"])
        // WKT types (Timestamp, etc.) are automatically mapped to
        // buffa-types — no extern_path needed.
        .generate_views(true)
        .generate_text(true)
        .include_file("_include.rs")
        .compile()
        .expect("protobuf compilation failed");
}

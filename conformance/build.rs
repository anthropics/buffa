use std::path::PathBuf;

fn main() {
    // Declare `no_protos` as a valid cfg name so rustc doesn't warn about it.
    println!("cargo:rustc-check-cfg=cfg(no_protos)");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let protos_dir = manifest_dir.join("protos");

    if !protos_dir.join("conformance.proto").exists() {
        // Protos haven't been fetched yet.  Emit a cfg flag so main.rs can
        // compile a stub binary that prints a helpful error on startup, rather
        // than failing to compile entirely (which would break `cargo check`).
        println!("cargo:warning=conformance/protos/ not populated.");
        println!("cargo:warning=Run `task fetch-protos` (or build inside Docker).");
        println!("cargo:warning=The binary will not function until protos are present.");
        println!("cargo:rustc-cfg=no_protos");
        return;
    }

    // WKT types come from buffa-types (with hand-written serde impls).
    // We only generate the test message types here.

    // TestAllTypesProto3 with serde + textproto enabled.
    buffa_build::Config::new()
        .files(&["protos/google/protobuf/test_messages_proto3.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .generate_text(true)
        .compile()
        .expect("buffa_build failed for test_messages_proto3.proto");

    // TestAllTypesProto2 with serde + textproto enabled for proto2 conformance.
    buffa_build::Config::new()
        .files(&["protos/google/protobuf/test_messages_proto2.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .generate_text(true)
        .allow_message_set(true)
        .compile()
        .expect("buffa_build failed for test_messages_proto2.proto");

    // Editions test messages: proto3 behavior via editions.
    let editions_proto3 = protos_dir.join("editions/golden/test_messages_proto3_editions.proto");
    if editions_proto3.exists() {
        buffa_build::Config::new()
            .files(&[&editions_proto3])
            .includes(&[&protos_dir])
            .generate_json(true)
            .generate_text(true)
            .compile()
            .expect("buffa_build failed for test_messages_proto3_editions.proto");

        // Editions test messages: proto2 behavior via editions.
        buffa_build::Config::new()
            .files(&["protos/editions/golden/test_messages_proto2_editions.proto"])
            .includes(&["protos/"])
            .generate_json(true)
            .generate_text(true)
            .allow_message_set(true)
            .compile()
            .expect("buffa_build failed for test_messages_proto2_editions.proto");

        // Pure edition 2023 test messages: file-level DELIMITED message encoding.
        // JSON enabled for the extension registry (the text `[pkg.ext]` bracket
        // syntax resolves through the same registry structs); text enabled for
        // the RunDelimitedTests suite.
        buffa_build::Config::new()
            .files(&["protos/conformance/test_protos/test_messages_edition2023.proto"])
            .includes(&["protos/"])
            .generate_json(true)
            .generate_text(true)
            .generate_views(false)
            .compile()
            .expect("buffa_build failed for test_messages_edition2023.proto");

        println!("cargo:rustc-cfg=has_editions_protos");
    }

    println!("cargo:rustc-check-cfg=cfg(has_editions_protos)");
    println!("cargo:rerun-if-changed=protos/");
}

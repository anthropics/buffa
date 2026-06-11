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

    // Vtable reflection is enabled on the view-bearing builds (proto3/proto2 and
    // their editions variants) for the BUFFA_VIA_VTABLE run. It is gated behind
    // the conformance `reflect` feature (via `gate_reflect_on_crate_feature`),
    // so the no_std binary — built `--no-default-features` — omits it.

    // TestAllTypesProto3 with serde + textproto enabled.
    buffa_build::Config::new()
        .files(&["protos/google/protobuf/test_messages_proto3.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .lazy_views(true)
        .generate_text(true)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .gate_reflect_on_crate_feature(true)
        .compile()
        .expect("buffa_build failed for test_messages_proto3.proto");

    // TestAllTypesProto2 with serde + textproto enabled for proto2 conformance.
    buffa_build::Config::new()
        .files(&["protos/google/protobuf/test_messages_proto2.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .lazy_views(true)
        .generate_text(true)
        .allow_message_set(true)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .gate_reflect_on_crate_feature(true)
        .compile()
        .expect("buffa_build failed for test_messages_proto2.proto");

    // Editions test messages: proto3 behavior via editions.
    let editions_proto3 = protos_dir.join("editions/golden/test_messages_proto3_editions.proto");
    if editions_proto3.exists() {
        buffa_build::Config::new()
            .files(&[&editions_proto3])
            .includes(&[&protos_dir])
            .generate_json(true)
            .lazy_views(true)
            .generate_text(true)
            .reflect_mode(buffa_build::ReflectMode::VTable)
            .gate_reflect_on_crate_feature(true)
            .compile()
            .expect("buffa_build failed for test_messages_proto3_editions.proto");

        // Editions test messages: proto2 behavior via editions.
        buffa_build::Config::new()
            .files(&["protos/editions/golden/test_messages_proto2_editions.proto"])
            .includes(&["protos/"])
            .generate_json(true)
            .lazy_views(true)
            .generate_text(true)
            .allow_message_set(true)
            .reflect_mode(buffa_build::ReflectMode::VTable)
            .gate_reflect_on_crate_feature(true)
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

    // Produce a FileDescriptorSet for the via-reflect mode. The reflection
    // runtime decodes this into a DescriptorPool and round-trips conformance
    // binary input through DynamicMessage.
    emit_reflect_fds(&manifest_dir);
}

/// Write `OUT_DIR/conformance_protos.fds` containing the conformance test
/// message types and their transitive imports.
fn emit_reflect_fds(manifest_dir: &std::path::Path) {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let fds_out = std::path::Path::new(&out_dir).join("conformance_protos.fds");

    let mut files = vec![
        "google/protobuf/test_messages_proto3.proto",
        "google/protobuf/test_messages_proto2.proto",
    ];
    let editions =
        manifest_dir.join("protos/conformance/test_protos/test_messages_edition2023.proto");
    if editions.exists() {
        files.push("conformance/test_protos/test_messages_edition2023.proto");
    }

    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".into());
    let status = std::process::Command::new(&protoc)
        .arg("--include_imports")
        .arg(format!("--descriptor_set_out={}", fds_out.display()))
        .arg("-I")
        .arg(manifest_dir.join("protos"))
        .args(&files)
        .status()
        .expect("protoc invocation for reflect FDS");
    assert!(status.success(), "protoc failed producing reflect FDS");
}

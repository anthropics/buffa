//! One-shot tool to generate Rust types for descriptor.proto and plugin.proto.
//!
//! This is the bootstrap step: it reads a binary FileDescriptorSet (produced
//! by `protoc --descriptor_set_out --include_imports`) and generates Rust
//! source using buffa-codegen.  The output is checked into the repo at
//! `buffa-descriptor/src/generated/`.
//!
//! Usage:
//!
//! ```text
//!   protoc --descriptor_set_out=descriptor_set.pb --include_imports \
//!       -I buffa-descriptor/protos \
//!       google/protobuf/descriptor.proto \
//!       google/protobuf/compiler/plugin.proto
//!   cargo run -p buffa-codegen --bin gen_descriptor_types -- \
//!       descriptor_set.pb buffa-descriptor/src/generated
//! ```

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: gen_descriptor_types <descriptor_set.pb> <output_dir>");
        std::process::exit(1);
    }

    let descriptor_bytes = fs::read(&args[1]).expect("failed to read descriptor set");
    let descriptor_set = FileDescriptorSet::decode_from_slice(&descriptor_bytes)
        .expect("failed to decode FileDescriptorSet");

    eprintln!("Loaded {} file descriptors", descriptor_set.file.len());
    for f in &descriptor_set.file {
        eprintln!("  - {}", f.name.as_deref().unwrap_or("<unnamed>"));
    }

    // Generate every impl kind, but gated on `buffa-descriptor`'s crate
    // features (`json` / `views` / `text` / `arbitrary`).
    //
    // - **codegen toolchain** (`buffa-codegen` / `buffa-build` /
    //   `protoc-gen-buffa`) deps on `buffa-descriptor` with
    //   `default-features = false` — only the binary codec compiles. No
    //   `serde` / `serde_json` / `base64` in the build graph.
    // - **downstream consumers** whose protos reference `descriptor.proto`
    //   types as fields (e.g. anything depending on
    //   `buf/validate/validate.proto`, which uses
    //   `google.protobuf.FieldDescriptorProto.Type`, or
    //   `buf.registry.module.v1` which embeds `FileDescriptorSet`) enable
    //   the features they need in their `Cargo.toml`. ([#113])
    let mut config = buffa_codegen::CodeGenConfig::default();
    config.generate_views = true;
    config.generate_json = true;
    config.generate_text = true;
    config.generate_arbitrary = true;
    config.gate_impls_on_crate_features = true;

    let files_to_generate = vec![
        "google/protobuf/descriptor.proto".to_string(),
        "google/protobuf/compiler/plugin.proto".to_string(),
    ];

    let generated = buffa_codegen::generate(&descriptor_set.file, &files_to_generate, &config)
        .expect("code generation failed");

    let out_dir = std::path::Path::new(&args[2]);
    fs::create_dir_all(out_dir).expect("failed to create output dir");

    for file in &generated {
        let path = out_dir.join(&file.name);
        eprintln!("Writing {}", path.display());
        fs::write(&path, &file.content).expect("failed to write file");
    }

    eprintln!("Done. Generated {} files.", generated.len());
}

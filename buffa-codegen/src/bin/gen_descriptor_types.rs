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

    // Use default config with views and serde disabled — descriptor types
    // only need binary encode/decode for codegen purposes.
    let mut config = buffa_codegen::CodeGenConfig::default();
    config.generate_views = false;
    config.generate_json = false;

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

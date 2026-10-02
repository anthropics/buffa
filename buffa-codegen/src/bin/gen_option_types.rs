//! One-shot tool to generate the Rust types for `buffa/ext/options.proto`.
//!
//! `buffa-proto-options` ships this output checked in
//! (`buffa-proto-options/src/generated/`), so that `buffa-codegen` can depend
//! on `buffa-proto-options` without first having to generate it.
//!
//! Run `task gen-option-types` (`scripts/gen-option-types.sh`) from the
//! workspace root. The script builds the descriptor set and passes it here:
//!
//! ```text
//!   gen_option_types <descriptor_set.pb> <output_dir>
//! ```

use buffa_codegen::generated::descriptor::FileDescriptorSet;
use std::fs;
use std::path::Path;

const EXT_PROTO: &str = "buffa/ext/options.proto";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: gen_option_types <descriptor_set.pb> <output_dir>");
        std::process::exit(1);
    }

    let descriptor_bytes = fs::read(&args[1]).expect("failed to read descriptor set");
    // Same tooling bound the plugins and buffa-build use: this descriptor set
    // came from a protoc run this task invoked.
    let descriptor_set = buffa_codegen::tooling_decode_options()
        .expect("BUFFA_ELEMENT_MEMORY_LIMIT is valid")
        .decode_from_slice::<FileDescriptorSet>(&descriptor_bytes)
        .expect("failed to decode FileDescriptorSet");

    // Every impl kind is generated and gated on a `buffa-proto-options` crate
    // feature (`views` / `json` / `text` / `arbitrary` / `reflect`): a
    // consumer that reads options through the binary codec compiles only
    // that, and the others enable what they use.
    //
    // Unknown fields stay preserved (the default), so an options message
    // written against a newer options.proto survives a decode and re-encode
    // by this version.
    let mut config = buffa_codegen::CodeGenConfig::default();
    config.generate_views = true;
    config.generate_json = true;
    config.generate_text = true;
    config.generate_arbitrary = true;
    config.generate_reflection = true;
    config.generate_reflection_vtable = true;
    config.gate_impls_on_crate_features = true;

    let (generated, warnings) = buffa_codegen::generate_with_diagnostics(
        &descriptor_set.file,
        &[EXT_PROTO.to_string()],
        &config,
    )
    .expect("code generation failed");
    for warning in &warnings {
        eprintln!("warning: buffa: {warning}");
    }

    let out_dir = Path::new(&args[2]);
    fs::create_dir_all(out_dir).expect("failed to create output dir");

    for file in &generated {
        let path = out_dir.join(&file.name);
        eprintln!("Writing {}", path.display());
        fs::write(&path, &file.content).expect("failed to write file");
    }

    eprintln!("Done. Generated {} files.", generated.len());
}

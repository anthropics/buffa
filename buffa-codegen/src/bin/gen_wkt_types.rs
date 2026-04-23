//! One-shot tool to generate the well-known-type Rust structs for buffa-types.
//!
//! buffa-types ships with this output **checked in** (`buffa-types/src/generated/`)
//! so that downstream consumers don't need `protoc` or the buffa-build/buffa-codegen
//! toolchain just to use `Timestamp`, `Duration`, etc.
//!
//! The WKT protos are Google-owned and frozen — they change approximately never.
//! Regeneration is only needed when buffa-codegen's output format changes.
//!
//! Usage:
//!
//! ```text
//!   protoc --descriptor_set_out=/tmp/wkt.pb --include_imports \
//!       -I buffa-types/protos \
//!       google/protobuf/any.proto \
//!       google/protobuf/duration.proto \
//!       google/protobuf/empty.proto \
//!       google/protobuf/field_mask.proto \
//!       google/protobuf/struct.proto \
//!       google/protobuf/timestamp.proto \
//!       google/protobuf/wrappers.proto
//!   cargo run --bin gen_wkt_types -- /tmp/wkt.pb buffa-types/src/generated
//! ```
//!
//! Or just `task gen-wkt-types` from the workspace root.

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;
use std::fs;
use std::path::Path;

/// WKT proto paths. Fixed — Google owns these and they don't move.
const WKT_PROTOS: &[&str] = &[
    "google/protobuf/any.proto",
    "google/protobuf/duration.proto",
    "google/protobuf/empty.proto",
    "google/protobuf/field_mask.proto",
    "google/protobuf/struct.proto",
    "google/protobuf/timestamp.proto",
    "google/protobuf/wrappers.proto",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: gen_wkt_types <descriptor_set.pb> <output_dir>");
        eprintln!();
        eprintln!("Typical invocation from workspace root:");
        eprintln!("  cargo run -p buffa-codegen --bin gen_wkt_types -- \\");
        eprintln!("      /tmp/wkt.pb buffa-types/src/generated");
        std::process::exit(1);
    }

    let descriptor_bytes = fs::read(&args[1]).expect("failed to read descriptor set");
    let descriptor_set = FileDescriptorSet::decode_from_slice(&descriptor_bytes)
        .expect("failed to decode FileDescriptorSet");

    eprintln!("Loaded {} file descriptors", descriptor_set.file.len());
    for f in &descriptor_set.file {
        eprintln!("  - {}", f.name.as_deref().unwrap_or("<unnamed>"));
    }

    // Config matches what buffa-types/build.rs used to do, with two
    // differences:
    //
    //   generate_arbitrary = true   Always on. Codegen emits
    //        #[cfg_attr(feature = "arbitrary", derive(...))] so one
    //        checked-in file covers both feature states. The old
    //        build.rs branched on cfg!(feature = "arbitrary") which
    //        produced two different outputs — unnecessary given the
    //        attribute is already cfg-gated.
    //
    //   generate_json = false       Unchanged. All WKT serde impls are
    //        hand-written in the *_ext.rs modules (Timestamp → RFC3339,
    //        Duration → "3.000001s", Any → type-URL dispatch, etc.).
    //        None of the WKTs use derive-serde.
    //
    //   generate_text = true        Textproto has no special WKT treatment
    //        (unlike JSON), so the generated field-by-field impls are
    //        correct. `buffa/text` is zero-dep — enabled unconditionally
    //        in buffa-types so no feature-gate wrapping is needed.
    //
    //   emit_register_fn = false    Per-package output means one fn would
    //        be emitted (all seven WKTs share `google.protobuf`), so the
    //        old multi-file collision is gone — but WKTs register via the
    //        hand-written `register_wkt_types` in `any_ext.rs` (which knows
    //        the JSON-Any `is_wkt` special-casing the generic fn doesn't),
    //        so the generated fn would be redundant.
    let mut config = buffa_codegen::CodeGenConfig::default();
    config.generate_views = true;
    config.preserve_unknown_fields = true;
    config.generate_arbitrary = true;
    config.generate_json = false;
    config.generate_text = true;
    config.emit_register_fn = false;
    // `Any.value` carries arbitrary encoded payloads that callers commonly
    // cache and clone into `repeated google.protobuf.Any` response fields.
    // `Bytes::clone()` is a refcount bump rather than a payload memcpy.
    config.bytes_fields = vec![".google.protobuf.Any.value".into()];

    let files_to_generate: Vec<String> = WKT_PROTOS.iter().map(|s| s.to_string()).collect();

    let generated = buffa_codegen::generate(&descriptor_set.file, &files_to_generate, &config)
        .expect("code generation failed");

    let out_dir = Path::new(&args[2]);
    fs::create_dir_all(out_dir).expect("failed to create output dir");

    for file in &generated {
        let path = out_dir.join(&file.name);
        eprintln!("Writing {}", path.display());
        fs::write(&path, &file.content).expect("failed to write file");
    }

    eprintln!("Done. Generated {} files.", generated.len());
}

fn main() {
    // Basic proto — the original test file.
    buffa_build::Config::new()
        .files(&["protos/basic.proto"])
        .includes(&["protos/"])
        .generate_text(true)
        .compile()
        .expect("buffa_build failed for basic.proto");

    // Comprehensive proto3 semantics: implicit vs explicit presence for all
    // scalar types, open-enum contexts, default packing, synthetic oneofs.
    buffa_build::Config::new()
        .files(&["protos/proto3_semantics.proto"])
        .includes(&["protos/"])
        .compile()
        .expect("buffa_build failed for proto3_semantics.proto");

    // Keyword handling — Rust keywords in package/message/field names.
    buffa_build::Config::new()
        .files(&["protos/keywords.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for keywords.proto");

    // Deep nesting — 3+ levels, oneof with same-package message variants,
    // direct and mutual recursion through a oneof. Views enabled to test
    // boxed view-enum variants.
    buffa_build::Config::new()
        .files(&["protos/nested_deep.proto"])
        .includes(&["protos/"])
        .compile()
        .expect("buffa_build failed for nested_deep.proto");

    // WKT usage — well-known types are auto-mapped to buffa-types.
    buffa_build::Config::new()
        .files(&["protos/wkt_usage.proto"])
        .includes(&["protos/"])
        .compile()
        .expect("buffa_build failed for wkt_usage.proto");

    // Name collisions — messages named after Rust types, fields named
    // after generated methods, oneof name matching parent message.
    buffa_build::Config::new()
        .files(&["protos/name_collisions.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for name_collisions.proto");

    // Prelude shadowing (gh#36) — nested `message Option` with optional/oneof
    // fields, built with views + JSON so all `Option<...>` emission paths are
    // exercised. Compilation is the assertion.
    buffa_build::Config::new()
        .files(&["protos/prelude_shadow.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for prelude_shadow.proto");

    // Proto2 with custom defaults, required fields, closed enums.
    buffa_build::Config::new()
        .files(&["protos/proto2_defaults.proto"])
        .includes(&["protos/"])
        .generate_text(true)
        .compile()
        .expect("buffa_build failed for proto2_defaults.proto");

    // JSON code generation — proto3 JSON serialization for all field types.
    buffa_build::Config::new()
        .files(&["protos/json_types.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for json_types.proto");

    // Proto2 + JSON — closed-enum JSON helpers (map_closed_enum,
    // repeated_closed_enum, closed_enum). Proto2 enums are always closed.
    buffa_build::Config::new()
        .files(&["protos/proto2_json.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for proto2_json.proto");

    // Cross-package references — types from basic + nested_deep.
    // Uses extern_path to map sibling packages to crate-level modules.
    buffa_build::Config::new()
        .files(&["protos/cross_package.proto"])
        .includes(&["protos/"])
        .extern_path(".basic", "crate::basic")
        .extern_path(".test.nested", "crate::nested")
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for cross_package.proto");

    // Cross-syntax import: proto2 file using a proto3-declared enum.
    // Spec: enum closedness follows the DECLARING file's syntax, so the
    // proto3 enum stays open even when referenced from proto2.
    buffa_build::Config::new()
        .files(&["protos/cross_syntax.proto"])
        .includes(&["protos/"])
        .extern_path(".basic", "crate::basic")
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for cross_syntax.proto");

    // utf8_validation = NONE → string fields become Vec<u8> / &[u8]
    // (opt-in via strict_utf8_mapping; default would keep them as String).
    buffa_build::Config::new()
        .files(&["protos/utf8_validation.proto"])
        .includes(&["protos/"])
        .strict_utf8_mapping(true)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for utf8_validation.proto");

    // Edge cases — reserved, large field numbers, packed override,
    // json_name override, non-string map keys, sub-message merge semantics.
    buffa_build::Config::new()
        .files(&["protos/edge_cases.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .generate_views(true)
        .compile()
        .expect("buffa_build failed for edge_cases.proto");

    // Regression: use_bytes_type() previously produced uncompilable decode
    // code (merge_bytes expects &mut Vec<u8>, struct field was bytes::Bytes).
    // basic.proto has bytes fields (Person.avatar singular; BytesContexts
    // repeated + optional + oneof + map). Views + JSON both enabled here
    // so every bytes_fields codegen path is compiled:
    //   - binary decode: scalar/repeated/oneof merge_arm use_bytes branches
    //   - JSON ser/deser: json_helpers::{bytes, opt_bytes} (generic over
    //     From<Vec<u8>> / AsRef<[u8]>) + ProtoElemJson for bytes::Bytes
    //   - view to_owned: bytes_to_owned for singular/repeated/oneof/optional
    let bytes_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bytes_variant");
    std::fs::create_dir_all(&bytes_out).expect("create bytes_variant dir");
    buffa_build::Config::new()
        .files(&["protos/basic.proto"])
        .includes(&["protos/"])
        .use_bytes_type()
        .generate_json(true)
        .out_dir(bytes_out)
        .compile()
        .expect("buffa_build failed for basic.proto with use_bytes_type");

    // Views + preserve_unknown_fields=false: the else-branches in view
    // codegen that omit the unknown-fields view field and before_tag tracking.
    // Compiled into a sub-directory; no runtime tests needed — the coverage
    // goal is to verify these branches produce compilable code.
    let no_uf_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("no_unknown_views");
    std::fs::create_dir_all(&no_uf_out).expect("create no_unknown_views dir");
    buffa_build::Config::new()
        .files(&["protos/basic.proto"])
        .includes(&["protos/"])
        .preserve_unknown_fields(false)
        .out_dir(no_uf_out)
        .compile()
        .expect("buffa_build failed for basic.proto with preserve_unknown_fields=false");

    // Per-enum enum_type overrides in JSON contexts (edition 2023, so
    // compilable with protoc v27+ unlike editions_2024.proto). Verifies
    // map_serde_module respects the value enum's closedness, not the map
    // field's (TYPE_MESSAGE) file-level default.
    buffa_build::Config::new()
        .files(&["protos/editions_enum_json.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for editions_enum_json.proto");

    // Custom options — `extend google.protobuf.FieldOptions` emits extension
    // descriptor consts. The extendee is never named in generated Rust (only
    // the value types and field numbers), so descriptor.proto is only needed
    // for protoc's resolution pass, not as an include path for codegen.
    buffa_build::Config::new()
        .files(&["protos/custom_options.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for custom_options.proto");

    // Extension JSON registry — message/enum/repeated extensions with a local
    // extendee. `generate_json(true)` so the `#[serde(flatten)]` wrapper and
    // `register_extensions` are emitted alongside the `Extension<_>` consts.
    buffa_build::Config::new()
        .files(&["protos/ext_json.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for ext_json.proto");

    // Group-encoded extensions — editions `features.message_encoding = DELIMITED`
    // makes message-typed extensions emit `GroupCodec<M>` instead of
    // `MessageCodec<M>` (wire types 3/4 instead of 2).
    buffa_build::Config::new()
        .files(&["protos/group_ext.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for group_ext.proto");

    // MessageSet wire format — legacy group-wrapped extension encoding.
    // Gated behind `allow_message_set(true)`; default is a codegen error.
    buffa_build::Config::new()
        .files(&["protos/messageset.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .allow_message_set(true)
        .compile()
        .expect("buffa_build failed for messageset.proto");

    // Edition 2024 — requires protoc v30+ (stabilized edition 2024).
    // Older protoc rejects it with "later than the maximum supported edition".
    // Skip gracefully on older protoc so the crate still builds; tests are
    // cfg-gated on has_edition_2024.
    println!("cargo:rustc-check-cfg=cfg(has_edition_2024)");
    match buffa_build::Config::new()
        .files(&["protos/editions_2024.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .compile()
    {
        Ok(()) => println!("cargo:rustc-cfg=has_edition_2024"),
        Err(e) => {
            println!("cargo:warning=editions_2024.proto skipped (protoc too old?): {e}");
        }
    }
}

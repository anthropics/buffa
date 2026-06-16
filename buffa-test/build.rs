fn main() {
    // Basic proto — the original test file. Also the codegen target for
    // bridge-mode reflection (`generate_reflection(true)` emits
    // `impl Reflectable` per message + a per-package descriptor pool).
    buffa_build::Config::new()
        .files(&["protos/basic.proto"])
        .includes(&["protos/"])
        .generate_text(true)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .compile()
        .expect("buffa_build failed for basic.proto");

    // views(false) + vtable: owned-message vtable reflection is self-contained,
    // so it must compile without view generation (only owned impls emitted).
    buffa_build::Config::new()
        .files(&["protos/vtable_no_views.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .compile()
        .expect("buffa_build failed for vtable_no_views.proto");

    // string_type(SmolStr) + vtable: exercises `ReflectElement for SmolStr` on
    // the repeated-string element path (`Vec<SmolStr>`). Singular string fields
    // reflect via deref; map string keys/values stay `String`.
    buffa_build::Config::new()
        .files(&["protos/vtable_string_repr.proto"])
        .includes(&["protos/"])
        .string_type(buffa_build::StringRepr::SmolStr)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .compile()
        .expect("buffa_build failed for vtable_string_repr.proto");

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

    // unbox_oneof — a non-recursive message oneof variant stored inline rather
    // than behind a Box. `Envelope.body.small` is opted out; `large` stays
    // boxed. Views + JSON + text + vtable reflection all enabled so every
    // boxing site is compiled for both shapes (enum decl, From impl, binary
    // merge, JSON deser, text encode, owned ReflectMessage oneof arms).
    // Runtime round-trips live in `tests/unbox_oneof.rs`.
    buffa_build::Config::new()
        .files(&["protos/unbox_oneof.proto"])
        .includes(&["protos/"])
        .unbox_oneof_in(&[".unboxoneof.Envelope.body.small"])
        .generate_json(true)
        .generate_text(true)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .compile()
        .expect("buffa_build failed for unbox_oneof.proto");

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

    // Prelude shadowing (gh#36, gh#64) — nested + cross-file `message Option`
    // with optional/oneof fields, built with views + JSON so all `Option<...>`
    // emission paths are exercised. The sibling file shares the package, so
    // its `Wrapper.kind: Option<...>` would resolve to the proto-defined
    // `Option` struct unless the codegen path is fully qualified.
    // Compilation is the assertion.
    buffa_build::Config::new()
        .files(&[
            "protos/prelude_shadow.proto",
            "protos/prelude_shadow_sibling.proto",
        ])
        .includes(&["protos/"])
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for prelude_shadow.proto");

    // Nested-package pair (gh#80) — `test.nestpkg` + `test.nestpkg.inner`.
    // `lib.rs` wraps these with the same `pub mod a { use super::*; pub mod
    // a_b { use super::*; … } }` chain that `buffa-build`'s `_include.rs`
    // emits, exercising the natural-path `pub use self::__buffa::…;`
    // re-exports under the only consumer layout where a bare `__buffa`
    // import path is E0659-ambiguous. Compilation is the assertion.
    buffa_build::Config::new()
        .files(&["protos/nestpkg_outer.proto", "protos/nestpkg_inner.proto"])
        .includes(&["protos/"])
        .compile()
        .expect("buffa_build failed for nestpkg_*.proto");

    // Issue #135: a message whose snake_case module name collides with a
    // sibling sub-package. `message Oof` (nested types) in `modcollide` vs
    // `package modcollide.oof`. Both files compiled together so codegen sees the
    // sub-package and deconflicts the nested module to `oof_`. JSON is enabled so
    // the Any-registry paths bubbled from the nested message resolve through the
    // deconflicted module (`super::oof_::__INNER_JSON_ANY`, not `super::oof::…`).
    buffa_build::Config::new()
        .files(&["protos/modcollide.proto", "protos/modcollide_oof.proto"])
        .includes(&["protos/"])
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for modcollide.proto (module collision)");

    // Issue #135, multi-message race: `Oof` + `Oof_` in `modrace`, with
    // sub-packages `modrace.oof` + `modrace.oof_`. The two nested-types modules
    // must get distinct deconflicted names (`oof__`, `oof___`). Compiling the
    // nested layout in lib.rs is the end-to-end guard.
    buffa_build::Config::new()
        .files(&[
            "protos/modrace.proto",
            "protos/modrace_oof.proto",
            "protos/modrace_oof_us.proto",
        ])
        .includes(&["protos/"])
        .compile()
        .expect("buffa_build failed for modrace.proto (multi-message race)");

    // Proto2 with custom defaults, required fields, closed enums. Vtable
    // reflection is enabled here specifically to compile the closed-enum and
    // required-field reflect paths (basic.proto is proto3 / open enums only):
    // closed enums are stored as bare enum types, whose `to_i32` is the
    // `Enumeration` trait method.
    buffa_build::Config::new()
        .files(&["protos/proto2_defaults.proto"])
        .includes(&["protos/"])
        .generate_text(true)
        .lazy_views(true)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .compile()
        .expect("buffa_build failed for proto2_defaults.proto");

    // Mixed-mode reflection: a bridge-mode dependency embedded by a
    // vtable-mode parent (via extern_path). Every message-typed position in
    // Outer (singular, repeated, map value, oneof variant) holds the
    // bridge-grade Inner, so the vtable accessors must degrade through
    // Inner's own Reflectable::reflect() / ReflectElement impls. Runtime
    // assertions live in `tests/reflect_mixed_mode.rs`.
    buffa_build::Config::new()
        .files(&["protos/mixed_reflect_dep.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .reflect_mode(buffa_build::ReflectMode::Bridge)
        .compile()
        .expect("buffa_build failed for mixed_reflect_dep.proto");
    buffa_build::Config::new()
        .files(&["protos/mixed_reflect.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .reflect_mode(buffa_build::ReflectMode::VTable)
        .extern_path(".mixedref.dep", "crate::mixed_reflect_dep")
        .compile()
        .expect("buffa_build failed for mixed_reflect.proto");

    // JSON code generation — proto3 JSON serialization for all field types.
    buffa_build::Config::new()
        .files(&["protos/json_types.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for json_types.proto");

    // View + JSON round-trip tests (issue #83): views and JSON both enabled.
    // The proto3 file imports WKTs (Timestamp, Duration, wrappers) so the
    // hand-written WKT view Serialize impls in buffa-types are exercised; the
    // proto2 file covers required fields, proto2 optional, and closed enums.
    buffa_build::Config::new()
        .files(&["protos/view_json.proto", "protos/view_json_proto2.proto"])
        .includes(&["protos/"])
        .generate_views(true)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for view_json protos");

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

    // Per-type extern_path references (issue #111) — maps individual type
    // FQNs (.basic.Person, .basic.Status) to crate::basic, rather than the
    // whole `.basic` package. Exercises exact-FQN resolution end-to-end.
    buffa_build::Config::new()
        .files(&["protos/cross_package_pertype.proto"])
        .includes(&["protos/"])
        .extern_path(".basic.Person", "crate::basic::Person")
        .extern_path(".basic.Status", "crate::basic::Status")
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for cross_package_pertype.proto");

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

    // `[debug_redact = true]` — annotated fields must print a placeholder
    // (never the value) in generated Debug output: owned message, oneof enum,
    // view, and view-oneof. Views enabled so the view Debug paths compile.
    buffa_build::Config::new()
        .files(&["protos/debug_redact.proto"])
        .includes(&["protos/"])
        .generate_views(true)
        .compile()
        .expect("buffa_build failed for debug_redact.proto");

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

    // Carve-out (#76): a `map<string, bytes>` whose key carries
    // `[features.utf8_validation = NONE]`, compiled with BOTH
    // strict_utf8_mapping() and use_bytes_type(). strict_utf8_mapping normalizes
    // the NONE-validated string key to an effective `bytes` key, so the entry is
    // an effective `map<bytes, bytes>`, whose JSON helper
    // (`bytes_key_bytes_val_map`) is the concrete `HashMap<Vec<u8>, Vec<u8>>`.
    // The value must therefore stay `Vec<u8>` despite use_bytes_type(), NOT
    // promote to `Bytes`. This module pins that the predicate honors the carve-out.
    let utf8_bytes_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("utf8_bytes_variant");
    std::fs::create_dir_all(&utf8_bytes_out).expect("create utf8_bytes_variant dir");
    buffa_build::Config::new()
        .files(&["protos/utf8_validation.proto"])
        .includes(&["protos/"])
        .strict_utf8_mapping(true)
        .use_bytes_type()
        .generate_json(true)
        .out_dir(utf8_bytes_out)
        .compile()
        .expect("buffa_build failed for utf8_validation.proto with strict_utf8_mapping + use_bytes_type");

    // Configurable string_type: a broad SmolStr default plus per-field
    // CompactString / EcoString overrides, with generate_json + arbitrary so
    // every string code path (decode/clear/view/json/arbitrary, including the
    // EcoString arbitrary shim) is compiled. Map keys/values stay String.
    let string_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("string_variant");
    std::fs::create_dir_all(&string_out).expect("create string_variant dir");
    buffa_build::Config::new()
        .files(&["protos/string_types.proto"])
        .includes(&["protos/"])
        .string_type(buffa_build::StringRepr::SmolStr)
        .string_type_in(
            buffa_build::StringRepr::CompactString,
            &[".stringtypes.StringContexts.compact"],
        )
        .string_type_in(
            buffa_build::StringRepr::EcoString,
            &[".stringtypes.StringContexts.eco"],
        )
        .generate_json(true)
        .generate_arbitrary(true)
        .generate_text(true)
        .out_dir(string_out)
        .compile()
        .expect("buffa_build failed for string_types.proto with string_type");

    // proto2 `[default = "..."]` + string_type: a required string field is a
    // bare type, so its Default impl and clear() must build the literal via the
    // configured repr's From<String>, not String::from.
    let string_p2_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("string_proto2_variant");
    std::fs::create_dir_all(&string_p2_out).expect("create string_proto2_variant dir");
    buffa_build::Config::new()
        .files(&["protos/string_proto2.proto"])
        .includes(&["protos/"])
        .string_type(buffa_build::StringRepr::SmolStr)
        .out_dir(string_p2_out)
        .compile()
        .expect("buffa_build failed for string_proto2.proto with string_type");

    // Regression #88: bytes_fields + generate_arbitrary(true).
    // BytesContexts in basic.proto has singular, optional, repeated, and oneof
    // bytes fields — this compilation exercises all four shim paths.
    let arb_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("arbitrary_bytes");
    std::fs::create_dir_all(&arb_out).expect("create arbitrary_bytes dir");
    buffa_build::Config::new()
        .files(&["protos/basic.proto"])
        .includes(&["protos/"])
        .use_bytes_type()
        .generate_arbitrary(true)
        .out_dir(arb_out)
        .compile()
        .expect("buffa_build failed for basic.proto with use_bytes_type + generate_arbitrary");

    // type_name_prefix (#46): every generated type name carries the "Rpc"
    // prefix; views + lazy views + JSON + text on so all cross-reference and
    // re-export emission paths compile against the prefixed names. Module
    // names, oneof enums, and the wire format are unaffected. Runtime checks
    // in tests/type_prefix.rs.
    let prefix_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("prefix_variant");
    std::fs::create_dir_all(&prefix_out).expect("create prefix_variant dir");
    buffa_build::Config::new()
        .files(&["protos/basic.proto"])
        .includes(&["protos/"])
        .type_name_prefix("Rpc")
        .generate_json(true)
        .generate_text(true)
        .lazy_views(true)
        .out_dir(prefix_out)
        .compile()
        .expect("buffa_build failed for basic.proto with type_name_prefix");

    // type_name_prefix + nested messages: nested_deep.proto's three-level
    // nesting compiles the nested view / owned-view / lazy-view re-export
    // paths against prefixed names. Compile-only coverage; no runtime tests.
    let prefix_nested_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("prefix_nested_variant");
    std::fs::create_dir_all(&prefix_nested_out).expect("create prefix_nested_variant dir");
    buffa_build::Config::new()
        .files(&["protos/nested_deep.proto"])
        .includes(&["protos/"])
        .type_name_prefix("Rpc")
        .lazy_views(true)
        .out_dir(prefix_nested_out)
        .compile()
        .expect("buffa_build failed for nested_deep.proto with type_name_prefix");

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

    // Idiomatic imports (experimental, requires file_per_package) —
    // package-root type references emitted as `use`-backed short names.
    // Compiling the output (cross-package use, extern use, parent-module
    // rung after a local collision, runtime-type claims, qualified nested
    // and oneof scopes) IS the main test; runtime tests verify wire-format
    // equivalence with default codegen. json=true hardens the serde-shadow
    // reservation at the package root.
    let idiomatic_out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("idiomatic_variant");
    std::fs::create_dir_all(&idiomatic_out).expect("create idiomatic_variant dir");
    buffa_build::Config::new()
        .files(&[
            "protos/idiomatic_imports.proto",
            "protos/idiomatic_imports_dep.proto",
        ])
        .includes(&["protos/"])
        .file_per_package(true)
        .idiomatic_imports(true)
        .generate_json(true)
        .include_file("_include.rs")
        .out_dir(idiomatic_out)
        .compile()
        .expect("buffa_build failed for idiomatic_imports.proto");

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

    // with_* setter methods (issue #30) — explicit-presence scalars/enum/bytes.
    buffa_build::Config::new()
        .files(&["protos/with_setters.proto"])
        .includes(&["protos/"])
        .generate_views(false)
        .compile()
        .expect("buffa_build failed for with_setters.proto");

    // lazy_views — additive FooLazyView family beside the unchanged eager
    // views. JSON on to exercise the lazy Serialize path.
    buffa_build::Config::new()
        .files(&["protos/lazy_views.proto"])
        .includes(&["protos/"])
        .lazy_views(true)
        .generate_json(true)
        .compile()
        .expect("buffa_build failed for lazy_views.proto");

    // lazy_views + preserve_unknown_fields(false): compiles the lazy decode
    // loop without unknown-field capture, and the PhantomData lifetime anchor
    // for an all-scalar lazy struct — branches lazy_views.proto can't reach.
    buffa_build::Config::new()
        .files(&["protos/lazy_views_lean.proto"])
        .includes(&["protos/"])
        .lazy_views(true)
        .preserve_unknown_fields(false)
        .compile()
        .expect("buffa_build failed for lazy_views_lean.proto");

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

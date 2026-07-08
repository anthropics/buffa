//! Integration tests that invoke protoc to parse real `.proto` files or
//! inline proto definitions, then run the full codegen pipeline. This
//! exercises the same code paths as `buffa-build`, but inside the test
//! binary where llvm-cov can instrument the codegen.
//!
//! Requires `protoc` on PATH. Tests panic with a clear message if protoc
//! is not available.

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;
use buffa_codegen::{CodeGenConfig, GeneratedFile};
use std::path::Path;
use std::process::Command;

/// Directory containing the shared test proto files.
const PROTOS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../buffa-test/protos");

// ── Helpers ─────────────────────────────────────────────────────────────

/// Strip common leading whitespace from a multi-line string.
fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.len() >= min_indent {
                &l[min_indent..]
            } else {
                l.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Invoke protoc to compile proto files into a `FileDescriptorSet`.
fn compile_protos(files: &[&str], includes: &[&str]) -> FileDescriptorSet {
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());

    let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
    let descriptor_path = tmp.path().to_path_buf();

    let mut cmd = Command::new(&protoc);
    cmd.arg("--include_imports");
    cmd.arg(format!(
        "--descriptor_set_out={}",
        descriptor_path.display()
    ));
    for include in includes {
        cmd.arg(format!("--proto_path={include}"));
    }
    for file in files {
        cmd.arg(file);
    }

    let output = cmd.output().unwrap_or_else(|e| {
        panic!("protoc not found ({e}). Install protoc to run codegen integration tests.")
    });

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("protoc failed: {stderr}");
    }

    let bytes = std::fs::read(&descriptor_path).expect("failed to read descriptor set");
    FileDescriptorSet::decode_from_slice(&bytes).expect("failed to decode descriptors")
}

/// Compile an inline proto definition and run codegen, returning the
/// generated Rust source. The proto string is dedented automatically.
fn generate_proto(proto: &str, config: &CodeGenConfig) -> String {
    let proto = dedent(proto);
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let proto_path = dir.path().join("test.proto");
    std::fs::write(&proto_path, &proto).expect("failed to write proto");

    let fds = compile_protos(
        &[proto_path.to_str().unwrap()],
        &[dir.path().to_str().unwrap()],
    );

    let files = buffa_codegen::generate(&fds.file, &["test.proto".into()], config)
        .unwrap_or_else(|e| panic!("codegen failed: {e}\n\nProto:\n{proto}"));

    // 5 content files + 1 stitcher per package; concat for substring asserts.
    files
        .into_iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Helper: run codegen on a proto file from buffa-test/protos/.
fn generate_for(proto_file: &str, config: &CodeGenConfig) -> Vec<GeneratedFile> {
    let proto_path = Path::new(PROTOS_DIR).join(proto_file);
    let fds = compile_protos(&[proto_path.to_str().unwrap()], &[PROTOS_DIR]);
    let files_to_generate = vec![proto_file.to_string()];
    buffa_codegen::generate(&fds.file, &files_to_generate, config)
        .unwrap_or_else(|e| panic!("codegen failed for {proto_file}: {e}"))
}

fn no_views() -> CodeGenConfig {
    let mut c = CodeGenConfig::default();
    c.generate_views = false;
    c
}

fn json_no_views() -> CodeGenConfig {
    let mut c = CodeGenConfig::default();
    c.generate_views = false;
    c.generate_json = true;
    c
}

fn json_with_views() -> CodeGenConfig {
    let mut c = CodeGenConfig::default();
    c.generate_json = true;
    c
}

// ── Tests using shared proto files from buffa-test/protos/ ──────────────

#[test]
fn codegen_basic() {
    let files = generate_for("basic.proto", &CodeGenConfig::default());
    assert!(!files.is_empty());
    assert!(files[0].content.contains("@generated"));
}

#[test]
fn codegen_basic_views_vs_no_views() {
    let total = |files: &[GeneratedFile]| files.iter().map(|f| f.content.len()).sum::<usize>();
    let with_views = generate_for("basic.proto", &CodeGenConfig::default());
    let without_views = generate_for("basic.proto", &no_views());
    assert!(
        total(&without_views) < total(&with_views),
        "disabling views should produce shorter output"
    );
}

#[test]
fn codegen_keywords() {
    let files = generate_for("keywords.proto", &no_views());
    let content = &files[0].content;
    assert!(content.contains("pub mod r#type"));
    assert!(content.contains("self_:"));
    assert!(content.contains("r#type:"));
}

#[test]
fn codegen_proto2_defaults() {
    let files = generate_for("proto2_defaults.proto", &no_views());
    let content = &files[0].content;
    assert!(content.contains("pub struct WithDefaults"));
    assert!(content.contains("pub struct Proto2Message"));
    assert!(content.contains("pub name:"));
    assert!(content.contains("pub id:"));
}

#[test]
fn codegen_json_produces_serde_derives() {
    let files = generate_for("json_types.proto", &json_no_views());
    let content = &files[0].content;
    assert!(content.contains("::serde::Serialize"));
    assert!(content.contains("::serde::Deserialize"));
}

#[test]
fn codegen_json_disabled_has_no_serde() {
    let files = generate_for("json_types.proto", &no_views());
    assert!(!files[0].content.contains("::serde::Serialize"));
}

#[test]
fn codegen_nested_deep() {
    let files = generate_for("nested_deep.proto", &no_views());
    let content = &files[0].content;
    assert!(content.contains("pub mod outer"));
    assert!(content.contains("pub mod middle"));
    assert!(content.contains("pub struct Inner"));
}

#[test]
fn codegen_wkt_auto_mapping() {
    let files = generate_for("wkt_usage.proto", &no_views());
    let content = &files[0].content;
    assert!(content.contains("::buffa_types::google::protobuf::Timestamp"));
    assert!(content.contains("::buffa_types::google::protobuf::Duration"));
}

#[test]
fn codegen_wkt_explicit_extern_overrides_auto() {
    let mut config = no_views();
    config
        .extern_paths
        .push((".google.protobuf".into(), "::my_custom_wkts".into()));
    let files = generate_for("wkt_usage.proto", &config);
    let content = &files[0].content;
    assert!(content.contains("::my_custom_wkts::Timestamp"));
    assert!(!content.contains("::buffa_types::"));
}

#[test]
fn codegen_name_collisions() {
    let files = generate_for("name_collisions.proto", &no_views());
    let content = &files[0].content;
    assert!(content.contains("pub struct Vec"));
    assert!(content.contains("pub struct String"));
    assert!(content.contains("pub struct Option"));
    assert!(content.contains("::core::option::Option"));
}

#[test]
fn codegen_cross_package_uses_super() {
    let basic_path = Path::new(PROTOS_DIR).join("basic.proto");
    let nested_path = Path::new(PROTOS_DIR).join("nested_deep.proto");
    let cross_path = Path::new(PROTOS_DIR).join("cross_package.proto");
    let fds = compile_protos(
        &[
            basic_path.to_str().unwrap(),
            nested_path.to_str().unwrap(),
            cross_path.to_str().unwrap(),
        ],
        &[PROTOS_DIR],
    );
    let files = buffa_codegen::generate(&fds.file, &["cross_package.proto".into()], &no_views())
        .expect("codegen failed");
    assert!(files[0].content.contains("super::"));
}

// ── Module tree generation ──────────────────────────────────────────────

#[test]
fn module_tree_basic() {
    let entries = vec![
        ("foo.rs", "my.pkg"),
        ("bar.rs", "my.pkg"),
        ("baz.rs", "other"),
    ];
    let tree = buffa_codegen::generate_module_tree(
        &entries,
        buffa_codegen::IncludeMode::Relative(""),
        false,
    );
    assert!(tree.contains("pub mod my"));
    assert!(tree.contains("pub mod pkg"));
    assert!(tree.contains("pub mod other"));
    assert!(tree.contains(r#"include!("foo.rs")"#));
}

#[test]
fn module_tree_inner_allow() {
    let entries = vec![("f.rs", "pkg")];
    let with = buffa_codegen::generate_module_tree(
        &entries,
        buffa_codegen::IncludeMode::Relative(""),
        true,
    );
    let without = buffa_codegen::generate_module_tree(
        &entries,
        buffa_codegen::IncludeMode::Relative(""),
        false,
    );
    assert!(with.contains("#![allow("));
    assert!(!without.contains("#![allow("));
}

#[test]
fn module_tree_keyword_escaping() {
    let entries = vec![("t.rs", "google.type")];
    let tree = buffa_codegen::generate_module_tree(
        &entries,
        buffa_codegen::IncludeMode::Relative(""),
        false,
    );
    assert!(tree.contains("pub mod r#type"));
}

// ── Inline proto tests ──────────────────────────────────────────────────
// These replace the hand-constructed DescriptorProto tests in lib.rs,
// using inline proto definitions for clarity.

#[test]
fn inline_scalar_fields() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Scalars {
          int32 id = 1;
          string name = 2;
          bytes data = 3;
          bool active = 4;
          double score = 5;
          float ratio = 6;
          uint32 count = 7;
          uint64 big = 8;
          sint32 delta = 9;
          sint64 offset = 10;
          fixed32 hash = 11;
          fixed64 fingerprint = 12;
          sfixed32 temp = 13;
          sfixed64 precise = 14;
        }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub id: i32"));
    assert!(content.contains("pub name: ::buffa::alloc::string::String"));
    assert!(content.contains("pub data: ::buffa::alloc::vec::Vec<u8>"));
    assert!(content.contains("pub active: bool"));
    assert!(content.contains("pub score: f64"));
    assert!(content.contains("pub ratio: f32"));
}

#[test]
fn inline_nested_message() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Outer {
          message Inner { int32 value = 1; }
          Inner child = 1;
        }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub struct Outer"));
    assert!(content.contains("pub mod outer"));
    assert!(content.contains("pub struct Inner"));
    assert!(content.contains("::buffa::MessageField<outer::Inner, ::buffa::Inline<outer::Inner>>"));
}

#[test]
fn inline_enum_field() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        enum Status { UNKNOWN = 0; ACTIVE = 1; INACTIVE = 2; }
        message Item { Status status = 1; }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub enum Status"));
    assert!(content.contains("pub status: ::buffa::EnumValue<Status>"));
}

#[test]
fn inline_map_field() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Address { string street = 1; }
        message Book {
          map<string, int32> stock = 1;
          map<string, Address> locations = 2;
        }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub stock: ::buffa::__private::HashMap<"));
    assert!(content.contains("pub locations: ::buffa::__private::HashMap<"));
}

#[test]
fn inline_oneof() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Contact {
          oneof info {
            string email = 1;
            string phone = 2;
          }
        }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub info: ::core::option::Option<__buffa::oneof::contact::Info>"));
    assert!(content.contains("pub enum Info"));
    assert!(content.contains("Email("));
    assert!(content.contains("Phone("));
}

#[test]
fn inline_proto3_optional() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Item {
          optional int32 count = 1;
          optional string label = 2;
        }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub count: ::core::option::Option<i32>"));
    assert!(content.contains("pub label: ::core::option::Option<::buffa::alloc::string::String>"));
}

#[test]
fn inline_repeated_fields() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Address { string street = 1; }
        message Item {
          repeated string tags = 1;
          repeated int32 numbers = 2;
          repeated Address addresses = 3;
        }
        "#,
        &no_views(),
    );
    assert!(content.contains("pub tags: ::buffa::alloc::vec::Vec<::buffa::alloc::string::String>"));
    assert!(content.contains("pub numbers: ::buffa::alloc::vec::Vec<i32>"));
    assert!(content.contains("pub addresses: ::buffa::alloc::vec::Vec<Address>"));
}

#[test]
fn inline_type_url() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package my.pkg;
        message Foo {
          message Bar { int32 x = 1; }
          Bar bar = 1;
        }
        "#,
        &no_views(),
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/my.pkg.Foo""#),
        "top-level TYPE_URL should use package: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/my.pkg.Foo.Bar""#),
        "nested TYPE_URL should include parent: {content}"
    );
}

#[test]
fn inline_enum_alias() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        enum Priority {
          option allow_alias = true;
          LOW = 0;
          DEFAULT = 0;
          HIGH = 1;
        }
        "#,
        &no_views(),
    );
    // First value is the enum variant, alias becomes a const.
    assert!(content.contains("LOW = 0i32"));
    assert!(content.contains("pub const DEFAULT: Self = Self::LOW"));
}

#[test]
fn inline_json_message() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Item {
          int64 id = 1;
          string name = 2;
          bytes data = 3;
        }
        "#,
        &json_no_views(),
    );
    assert!(content.contains("::serde::Serialize"));
    assert!(content.contains("::serde::Deserialize"));
    // int64 should use json helper for string encoding.
    assert!(content.contains("json_helpers::int64"));
    // bytes should use base64 json helper.
    assert!(content.contains("json_helpers::bytes"));
}

#[test]
fn inline_json_oneof() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg {
          oneof value {
            string text = 1;
            int32 number = 2;
          }
        }
        "#,
        &json_no_views(),
    );
    // Oneof field should be flattened in serde.
    assert!(content.contains("serde(flatten)"));
}

#[test]
fn inline_oneof_duplicate_message_type_no_from_collision() {
    // Regression: google.api.expr.v1alpha1.Type.type_kind has TWO Empty
    // variants and TWO PrimitiveType variants. Generating From<T> for both
    // causes E0119 (conflicting impls). Types appearing multiple times must
    // not get From impls.
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Placeholder {}
        message T {
          oneof kind {
            Placeholder dyn = 1;
            Placeholder error = 2;
            string name = 3;
            T nested = 4;
          }
        }
        "#,
        &no_views(),
    );
    // Box on both message variants. Oneof body now sits at depth 3
    // (`__buffa::oneof::t::`), so 3× super.
    assert_eq!(
        content
            .matches("::buffa::alloc::boxed::Box<super::super::super::Placeholder>")
            .count(),
        2,
        "both Placeholder variants should be boxed: {content}"
    );
    // Only ONE From impl (for T, which appears once), not two for Placeholder.
    assert_eq!(
        content
            .matches("impl From<super::super::super::Placeholder> for Kind")
            .count(),
        0,
        "duplicate-type From impls must be skipped: {content}"
    );
    assert_eq!(
        content
            .matches("impl From<super::super::super::T> for Kind")
            .count(),
        1,
        "unique-type From impl must still be generated: {content}"
    );
}

#[test]
fn inline_oneof_unbox_opt_out_drops_box() {
    // unbox_oneof opts a non-recursive message variant out of Box wrapping.
    // The opted-out variant stores its message inline; sibling message
    // variants left alone stay boxed.
    let mut config = no_views();
    config
        .unboxed_oneof_fields
        .push(".test.Envelope.body.small".to_string());
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Small { int32 value = 1; }
        message Large { string label = 1; }
        message Envelope {
          oneof body {
            Small small = 1;
            Large large = 2;
          }
        }
        "#,
        &config,
    );
    // `small` is stored inline, `large` stays boxed.
    assert!(
        content.contains("Small(super::super::super::Small)"),
        "opted-out variant should be stored inline: {content}"
    );
    assert!(
        content.contains("Large(::buffa::alloc::boxed::Box<super::super::super::Large>)"),
        "unmatched variant should stay boxed: {content}"
    );
    // The From impl for the inline variant moves the value in without a Box.
    assert!(
        content.contains("Self::Small(v)"),
        "From impl for the inline variant must not wrap in Box: {content}"
    );
    // An enum with an inline message variant allows large_enum_variant: the
    // user chose inline storage and cannot edit generated code to silence it.
    assert!(
        content.contains("#[allow(clippy::large_enum_variant)]"),
        "enum with an inline variant should allow large_enum_variant: {content}"
    );
}

#[test]
fn inline_oneof_no_unbox_rules_no_large_enum_allow() {
    // Default codegen (no unbox rules) must not grow the new allow attribute —
    // output stays byte-identical for existing users.
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Small { int32 value = 1; }
        message Envelope {
          oneof body {
            Small small = 1;
          }
        }
        "#,
        &no_views(),
    );
    assert!(
        !content.contains("large_enum_variant"),
        "default output must not carry the allow: {content}"
    );
}

#[test]
fn inline_oneof_unbox_recursive_variant_is_rejected() {
    // Opting a recursive variant out of boxing would make the enum unsized,
    // so codegen must reject it rather than emit code that fails to compile.
    let proto = dedent(
        r#"
        syntax = "proto3";
        package test;
        message Node {
          oneof kind {
            Node child = 1;
            int32 leaf = 2;
          }
        }
        "#,
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let proto_path = dir.path().join("test.proto");
    std::fs::write(&proto_path, &proto).expect("write proto");
    let fds = compile_protos(
        &[proto_path.to_str().unwrap()],
        &[dir.path().to_str().unwrap()],
    );

    let mut config = no_views();
    config
        .unboxed_oneof_fields
        .push(".test.Node.kind.child".to_string());
    let result = buffa_codegen::generate(&fds.file, &["test.proto".into()], &config);
    let err = result.expect_err("unboxing a recursive variant should error");
    assert!(
        err.to_string().contains("recursive"),
        "error should explain the recursion: {err}"
    );
}

#[test]
fn inline_proto2_required_no_json_skip() {
    // Regression: proto2 required fields got skip_serializing_if, so a
    // required int32 with value 0 was omitted from JSON. The binary encoder
    // correctly always encodes required fields (is_proto2_required check);
    // JSON must do the same.
    let content = generate_proto(
        r#"
        syntax = "proto2";
        package test;
        message Item {
          required int32 id = 1;
          required string name = 2;
          optional int32 count = 3;
        }
        "#,
        &json_no_views(),
    );
    // Required fields must NOT have skip_serializing_if.
    // The id field is required int32 — it must always be emitted even at 0.
    // Check: id's serde attr should have rename but NOT skip_serializing_if.
    // The optional `count` DOES get skip_serializing_if (Option::is_none).
    // Count skip_serializing_if occurrences: should be 1 (just `count`),
    // not 3 (id + name + count).
    let skip_count = content.matches("skip_serializing_if").count();
    assert_eq!(
        skip_count, 1,
        "only the optional field should have skip_serializing_if, got {skip_count}: {content}"
    );
}

#[test]
fn inline_proto2_custom_deserialize_honours_struct_default() {
    // Regression: custom Deserialize (triggered by oneof) used
    // var.unwrap_or_default() which calls i32::default() = 0, ignoring the
    // struct's custom Default impl that sets bar: 42 per [default = 42].
    // The derive path (#[serde(default)]) correctly uses the struct Default.
    let content = generate_proto(
        r#"
        syntax = "proto2";
        package test;
        message Item {
          required int32 bar = 1 [default = 42];
          oneof choice {
            string a = 2;
            int32 b = 3;
          }
        }
        "#,
        &json_no_views(),
    );
    // Must have a custom Default impl (required field with default_value).
    assert!(
        content.contains("bar: 42i32"),
        "custom Default must set bar: 42: {content}"
    );
    // Custom Deserialize must start from Default::default() and overwrite,
    // not build a struct literal with unwrap_or_default().
    assert!(
        content.contains("::core::default::Default>::default()") && content.contains("__r.bar"),
        "custom Deserialize must init from struct Default: {content}"
    );
    // Must NOT use unwrap_or_default() which would give i32 default (0).
    assert!(
        !content.contains(".unwrap_or_default()"),
        "must not use unwrap_or_default (uses type default, not struct): {content}"
    );
}

#[test]
fn inline_proto2_optional() {
    let content = generate_proto(
        r#"
        syntax = "proto2";
        package test;
        message Item {
          optional int32 count = 1;
          required string name = 2;
          optional string label = 3 [default = "untitled"];
        }
        "#,
        &no_views(),
    );
    // Proto2 optional → ::core::option::Option<T>
    assert!(content.contains("pub count: ::core::option::Option<i32>"));
    // Proto2 required → bare type (always encoded)
    assert!(content.contains("pub name: ::buffa::alloc::string::String"));
}

#[test]
fn inline_proto2_closed_enum() {
    let content = generate_proto(
        r#"
        syntax = "proto2";
        package test;
        enum Status { UNKNOWN = 0; ACTIVE = 1; }
        message Item { optional Status status = 1; }
        "#,
        &no_views(),
    );
    // Proto2 closed enum → bare type (not EnumValue<T>)
    assert!(
        content.contains("pub status: ::core::option::Option<Status>"),
        "proto2 closed enum should be ::core::option::Option<Status>, not EnumValue: {content}"
    );
}

#[test]
fn inline_cross_package_super() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("dep.proto"),
        "syntax = \"proto3\";\npackage dep;\nmessage Shared { int32 x = 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("main.proto"),
        "syntax = \"proto3\";\npackage main;\nimport \"dep.proto\";\nmessage Msg { dep.Shared s = 1; }\n",
    )
    .unwrap();

    let dep_path = dir.path().join("dep.proto");
    let main_path = dir.path().join("main.proto");
    let fds = compile_protos(
        &[dep_path.to_str().unwrap(), main_path.to_str().unwrap()],
        &[dir.path().to_str().unwrap()],
    );
    let files = buffa_codegen::generate(&fds.file, &["main.proto".into()], &no_views())
        .expect("codegen failed");
    let content = &files[0].content;
    // Cross-package ref should use super:: to reach sibling package.
    assert!(
        content.contains("super::dep::Shared"),
        "cross-package ref should use super::dep::Shared: {content}"
    );
}

#[test]
fn inline_view_keyword_package_path() {
    // Regression: view.rs to_owned used syn::parse_str which chokes on
    // keyword path segments like `super::r#type::LatLng` (google.type pkg).
    // rust_path_to_tokens handles keywords; view.rs must use it.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("latlng.proto"),
        "syntax = \"proto3\";\npackage google.type;\nmessage LatLng { double lat = 1; double lng = 2; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("marker.proto"),
        "syntax = \"proto3\";\npackage google.maps;\nimport \"latlng.proto\";\nmessage Marker { google.type.LatLng location = 1; }\n",
    )
    .unwrap();

    let latlng_path = dir.path().join("latlng.proto");
    let marker_path = dir.path().join("marker.proto");
    let fds = compile_protos(
        &[latlng_path.to_str().unwrap(), marker_path.to_str().unwrap()],
        &[dir.path().to_str().unwrap()],
    );
    // Default config has views enabled — this exercises the to_owned path.
    let files = buffa_codegen::generate(
        &fds.file,
        &["marker.proto".into(), "latlng.proto".into()],
        &CodeGenConfig::default(),
    )
    .expect("codegen failed for keyword package with views");
    let marker = files
        .iter()
        .find(|f| f.package == "google.maps")
        .expect("google.maps.rs not generated");
    // Must emit r#type (raw ident), not plain `type`.
    assert!(
        marker.content.contains("r#type"),
        "keyword package segment must use raw ident: {}",
        marker.content
    );
}

// ── Config option tests ─────────────────────────────────────────────────

#[test]
fn inline_bytes_field_mapping() {
    let mut config = no_views();
    config
        .bytes_fields
        .push((".".into(), buffa_codegen::BytesRepr::Bytes));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg {
          bytes data = 1;
          bytes payload = 2;
        }
        "#,
        &config,
    );
    assert!(
        content.contains("::buffa::bytes::Bytes"),
        "bytes fields should use Bytes type: {content}"
    );
    assert!(
        !content.contains("Vec<u8>"),
        "bytes fields should not use Vec<u8> when bytes mapping is enabled"
    );
    // Regression: decode path must use the Bytes-producing helper (not call
    // merge_bytes which expects &mut Vec<u8>).
    assert!(
        content.contains("::buffa::types::decode_bytes_to_bytes"),
        "decode must use decode_bytes_to_bytes for Bytes field: {content}"
    );
    assert!(
        !content.contains("Bytes::from("),
        "must not wrap decode in Bytes::from (regression to alloc+copy): {content}"
    );
    assert!(
        !content.contains("merge_bytes(&mut self"),
        "must not call merge_bytes on Bytes field (no &mut): {content}"
    );
}

#[test]
fn inline_bytes_field_selective_mapping() {
    let mut config = no_views();
    config
        .bytes_fields
        .push((".test.Msg.data".into(), buffa_codegen::BytesRepr::Bytes));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg {
          bytes data = 1;
          bytes other = 2;
        }
        "#,
        &config,
    );
    // Only `data` should use Bytes; `other` should remain Vec<u8>.
    assert!(
        content.contains("::buffa::bytes::Bytes"),
        "selected field should use Bytes: {content}"
    );
    assert!(
        content.contains("Vec<u8>"),
        "non-selected field should still use Vec<u8>: {content}"
    );
}

#[test]
fn module_collision_nested_message_vs_subpackage() {
    // Issue #135: `message Oof` (with nested types) snake-cases to module `oof`,
    // colliding with sibling sub-package `foo.oof`. Both compiled together so
    // codegen sees the sub-package and deconflicts the nested module to `oof_`.
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("foo.proto"),
        "syntax = \"proto3\";\npackage foo;\nmessage Oof { message Inner { int32 x = 1; } Inner inner = 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("foo_oof.proto"),
        "syntax = \"proto3\";\npackage foo.oof;\nmessage Thing { int32 y = 1; }\n",
    )
    .unwrap();
    let fds = compile_protos(
        &[
            dir.path().join("foo.proto").to_str().unwrap(),
            dir.path().join("foo_oof.proto").to_str().unwrap(),
        ],
        &[dir.path().to_str().unwrap()],
    );
    let config = no_views();
    let content = buffa_codegen::generate(&fds.file, &["foo.proto".into()], &config)
        .expect("codegen")
        .into_iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("\n");

    // The nested-types module is deconflicted; references use the same name.
    assert!(
        content.contains("pub mod oof_"),
        "nested module should be deconflicted to `oof_`: {content}"
    );
    assert!(
        content.contains("oof_ :: Inner") || content.contains("oof_::Inner"),
        "nested-type references should use the deconflicted module: {content}"
    );
    // The struct itself (PascalCase) is untouched.
    assert!(content.contains("struct Oof"), "{content}");
}

#[test]
fn module_collision_two_messages_race_to_distinct_names() {
    // Pathological: `Oof`(oof) and `Oof_`(oof_) both collide, with sub-packages
    // `foo.oof` AND `foo.oof_`. Each deconflicted name must be distinct from the
    // other's and from both sub-packages — `oof__` and `oof___`, not both `oof__`.
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("foo.proto"),
        "syntax = \"proto3\";\npackage foo;\n\
         message Oof { message Inner { int32 x = 1; } Inner inner = 1; }\n\
         message Oof_ { message Inner { int32 x = 1; } Inner inner = 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("a.proto"),
        "syntax = \"proto3\";\npackage foo.oof;\nmessage Thing { int32 y = 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.proto"),
        "syntax = \"proto3\";\npackage foo.oof_;\nmessage Widget { int32 z = 1; }\n",
    )
    .unwrap();
    let fds = compile_protos(
        &[
            dir.path().join("foo.proto").to_str().unwrap(),
            dir.path().join("a.proto").to_str().unwrap(),
            dir.path().join("b.proto").to_str().unwrap(),
        ],
        &[dir.path().to_str().unwrap()],
    );
    let content = buffa_codegen::generate(&fds.file, &["foo.proto".into()], &no_views())
        .expect("codegen")
        .into_iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("\n");

    // Each message's `inner` field references its own deconflicted module.
    // `oof__::Inner` is not a substring of `oof___::Inner`, so finding both
    // proves the two modules are distinct (and neither is the bare `oof`/`oof_`).
    assert!(
        content.contains("oof__ :: Inner") || content.contains("oof__::Inner"),
        "Oof should reference oof__::Inner: {content}"
    );
    assert!(
        content.contains("oof___ :: Inner") || content.contains("oof___::Inner"),
        "Oof_ should reference oof___::Inner: {content}"
    );
}

#[test]
fn module_no_collision_keeps_natural_module_name() {
    // Without a colliding sub-package, the nested module stays `oof` (no churn).
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package foo;
        message Oof { message Inner { int32 x = 1; } Inner inner = 1; }
        "#,
        &no_views(),
    );
    // The nested module is present and NOT deconflicted (no `_` suffix). The
    // `!oof_` check is the meaningful guard; `oof::Inner` confirms the module
    // exists, both robust to prettyplease spacing.
    assert!(
        content.contains("oof :: Inner") || content.contains("oof::Inner"),
        "{content}"
    );
    assert!(
        !content.contains("pub mod oof_") && !content.contains("oof_ ::"),
        "no spurious dedup: {content}"
    );
}

#[test]
fn inline_string_field_mapping() {
    use buffa_codegen::StringRepr;
    let mut config = no_views();
    config
        .string_fields
        .push((".".into(), StringRepr::Custom("::smol_str::SmolStr".into())));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg {
          string name = 1;
          optional string nick = 2;
        }
        "#,
        &config,
    );
    assert!(
        content.contains("::smol_str::SmolStr"),
        "string fields should use the custom type: {content}"
    );
    // Non-optional singular decode must use the generic helper, not merge_string.
    assert!(
        content.contains("::buffa::types::decode_string_to"),
        "decode must use decode_string_to for non-default string repr: {content}"
    );
    assert!(
        !content.contains("merge_string(&mut self"),
        "must not call merge_string on a non-default string field: {content}"
    );
}

#[test]
fn inline_string_field_selective_and_override() {
    use buffa_codegen::StringRepr;
    let mut config = no_views();
    // Broad default, then a more specific override (last match wins).
    config
        .string_fields
        .push((".".into(), StringRepr::Custom("::smol_str::SmolStr".into())));
    config.string_fields.push((
        ".test.Msg.code".into(),
        StringRepr::Custom("::compact_str::CompactString".into()),
    ));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg {
          string name = 1;
          string code = 2;
        }
        "#,
        &config,
    );
    assert!(
        content.contains("::smol_str::SmolStr"),
        "broad rule should apply SmolStr to `name`: {content}"
    );
    assert!(
        content.contains("::compact_str::CompactString"),
        "specific override should apply CompactString to `code`: {content}"
    );
}

#[test]
fn inline_string_custom_emits_generic_arbitrary_builder() {
    use buffa_codegen::StringRepr;
    let mut config = no_views();
    config.generate_arbitrary = true;
    config
        .string_fields
        .push((".".into(), StringRepr::Custom("::ecow::EcoString".into())));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg { string name = 1; }
        "#,
        &config,
    );
    assert!(
        content.contains("::ecow::EcoString"),
        "string field should use the custom type: {content}"
    );
    // A non-default string repr attaches the type-agnostic generic builder,
    // regardless of whether the type has a native Arbitrary impl.
    assert!(
        content.contains("arbitrary(with = ::buffa::__private::arbitrary_proto_string"),
        "custom string field must use the generic arbitrary_proto_string builder: {content}"
    );
}

#[test]
fn inline_string_default_is_unchanged() {
    // With no string_fields rule, output must still use String + merge_string.
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg { string name = 1; }
        "#,
        &no_views(),
    );
    assert!(
        content.contains("::buffa::alloc::string::String"),
        "default string repr should remain String: {content}"
    );
    assert!(
        content.contains("merge_string(&mut self"),
        "default string field should keep the in-place merge_string fast path: {content}"
    );
    assert!(
        !content.contains("decode_string_to"),
        "default string field must not use the generic decode helper: {content}"
    );
}

#[test]
fn inline_preserve_unknown_fields_disabled() {
    let mut config = no_views();
    config.preserve_unknown_fields = false;
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg { int32 x = 1; }
        "#,
        &config,
    );
    assert!(
        !content.contains("__buffa_unknown_fields"),
        "unknown fields should not be present when preservation is disabled: {content}"
    );
}

#[test]
fn inline_empty_message() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Empty {}
        "#,
        &no_views(),
    );
    assert!(content.contains("pub struct Empty"));
    // Empty message should still implement Message.
    assert!(content.contains("impl ::buffa::Message for Empty"));
    // Should have the unknown-fields plumbing but no user fields and no
    // serialization-state field — sizes live in the external SizeCache.
    assert!(content.contains("__buffa_unknown_fields"));
    assert!(!content.contains("__buffa_cached_size"));
}

#[test]
fn inline_empty_message_no_unknown_fields() {
    let mut config = no_views();
    config.preserve_unknown_fields = false;
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Empty {}
        "#,
        &config,
    );
    assert!(content.contains("pub struct Empty"));
    assert!(!content.contains("__buffa_unknown_fields"));
}

// ── View Serialize codegen tests ────────────────────────────────────────

#[test]
fn test_view_serialize_impl_emitted_when_json_enabled() {
    let files = generate_for("json_types.proto", &json_with_views());
    let combined = files
        .iter()
        .map(|f| f.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("impl<'__a> ::serde::Serialize for"),
        "view Serialize impl must be emitted when generate_json=true: {combined}"
    );
    for file in &files {
        syn::parse_file(&file.content).unwrap_or_else(|e| {
            panic!(
                "generated file must parse ({}): {e}\n---\n{}",
                file.package, file.content
            )
        });
    }
}

#[test]
fn test_view_serialize_not_emitted_when_json_disabled() {
    // CodeGenConfig::default() has generate_views=true, generate_json=false.
    let files = generate_for("json_types.proto", &CodeGenConfig::default());
    let combined = files
        .iter()
        .map(|f| f.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.contains("impl<'__a> ::serde::Serialize for"),
        "view Serialize impl must NOT be emitted when generate_json=false"
    );
}

#[test]
fn test_view_serialize_json_helpers_used() {
    // Verify that int64 and bytes fields in a view use json_helpers paths.
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Item {
          int64 id = 1;
          bytes data = 2;
          double score = 3;
        }
        "#,
        &json_with_views(),
    );
    assert!(
        content.contains("json_helpers"),
        "view Serialize must use json_helpers for int64/bytes/double: {content}"
    );
    syn::parse_file(&content).expect("generated content must parse");
}

#[test]
fn test_view_serialize_map_keys_stringified() {
    // Protobuf JSON requires map keys to be JSON strings. The view Serialize
    // impl must emit an explicit `_WK` newtype with `collect_str` for
    // non-string scalar keys, mirroring the owned-side `DisplayKey` wrapper —
    // not rely on the serializer's `MapKeySerializer` to stringify primitives
    // (which only `serde_json` does).
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Maps {
          map<int64, string> by_id = 1;
          map<bool, string> by_flag = 2;
          map<string, string> by_name = 3;
        }
        "#,
        &json_with_views(),
    );
    syn::parse_file(&content).expect("generated content must parse");
    assert!(
        content.contains("::buffa::json_helpers::MapKeyJson"),
        "non-string map keys must be stringified via MapKeyJson: {content}"
    );
}

#[test]
fn test_view_serialize_proto2_required_multi_field_parses() {
    // Regression: proto2 required fields with helper-typed scalars previously
    // emitted `struct _W` at fn scope without a wrapping block, causing E0428
    // (duplicate definition) when two or more such fields appeared in one message.
    // This test asserts the generated output parses (syntax-level regression guard).
    let content = generate_proto(
        r#"
        syntax = "proto2";
        package test;
        message TwoRequired {
          required int64 a = 1;
          required bytes b = 2;
          required int64 c = 3;
        }
        "#,
        &json_with_views(),
    );
    syn::parse_file(&content)
        .expect("proto2 required multi-field view Serialize must produce parseable output");
    // Each required field must be serialized unconditionally (no skip_if).
    assert!(
        content.contains("impl<'__a> ::serde::Serialize for TwoRequiredView"),
        "view Serialize impl must be emitted: {content}"
    );
}

// ── with_* setter tests ───────────────────────────────────────────────────────

#[test]
fn with_setters_emitted_for_explicit_presence_fields() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        enum Color { COLOR_UNSPECIFIED = 0; RED = 1; }
        message Msg {
          optional int32   count  = 1;
          optional string  name   = 2;
          optional bytes   data   = 3;
          optional Color   color  = 4;
          // implicit-presence fields — no setter
          int32   implicit_int    = 5;
          string  implicit_str    = 6;
          // repeated — no setter
          repeated string tags    = 7;
        }
        "#,
        &no_views(),
    );

    // Explicit-presence fields get setters.
    assert!(
        content.contains("pub fn with_count"),
        "with_count missing: {content}"
    );
    assert!(
        content.contains("pub fn with_name"),
        "with_name missing: {content}"
    );
    assert!(
        content.contains("pub fn with_data"),
        "with_data missing: {content}"
    );
    assert!(
        content.contains("pub fn with_color"),
        "with_color missing: {content}"
    );

    // String setter uses impl Into<...> for &str ergonomics.
    assert!(
        content.contains("impl Into"),
        "string setter should use impl Into: {content}"
    );

    // Implicit-presence and repeated fields must NOT get setters.
    assert!(
        !content.contains("with_implicit_int"),
        "implicit int should not get setter: {content}"
    );
    assert!(
        !content.contains("with_implicit_str"),
        "implicit string should not get setter: {content}"
    );
    assert!(
        !content.contains("with_tags"),
        "repeated field should not get setter: {content}"
    );
}

#[test]
fn with_setters_disabled_by_config() {
    let mut config = no_views();
    config.generate_with_setters = false;
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg { optional int32 count = 1; }
        "#,
        &config,
    );
    assert!(
        !content.contains("pub fn with_count"),
        "setter should be absent when generate_with_setters=false: {content}"
    );
}

#[test]
fn with_setters_bytes_type_uses_into() {
    let mut config = no_views();
    config
        .bytes_fields
        .push((".test.Msg.data".into(), buffa_codegen::BytesRepr::Bytes));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg { optional bytes data = 1; }
        "#,
        &config,
    );
    // bytes::Bytes field should use impl Into for ergonomics.
    assert!(
        content.contains("pub fn with_data"),
        "with_data missing: {content}"
    );
    assert!(
        content.contains("impl Into"),
        "bytes::Bytes setter should use impl Into: {content}"
    );
}

#[test]
fn with_setters_vec_u8_uses_into() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Msg { optional bytes data = 1; }
        "#,
        &no_views(),
    );
    // Vec<u8> bytes field uses impl Into: From<&[T; N]> for Vec<T> is stable
    // since Rust 1.74, so b"hello" works directly without .to_vec().
    assert!(
        content.contains("pub fn with_data"),
        "with_data missing: {content}"
    );
    assert!(
        content.contains("impl Into"),
        "Vec<u8> setter should use impl Into: {content}"
    );
}

#[test]
fn with_setters_enum_uses_into() {
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        enum Color { COLOR_UNSPECIFIED = 0; RED = 1; }
        message Msg { optional Color color = 1; }
        "#,
        &no_views(),
    );
    // EnumValue<E>: From<E>, so impl Into<EnumValue<E>> lets callers pass the
    // enum variant directly without wrapping in EnumValue::Known(...).
    assert!(
        content.contains("pub fn with_color"),
        "with_color missing: {content}"
    );
    assert!(
        content.contains("impl Into"),
        "enum setter should use impl Into: {content}"
    );
}

#[test]
fn with_setters_proto2_repeated_no_setter() {
    // Regression: proto2 repeated fields have is_explicit_presence=true due to
    // proto2's EXPLICIT presence default, but their struct field is Vec<T>,
    // not Option<T>. They must not receive a setter.
    let content = generate_proto(
        r#"
        syntax = "proto2";
        package test;
        message Msg { repeated string items = 1; }
        "#,
        &no_views(),
    );
    assert!(
        !content.contains("with_items"),
        "proto2 repeated field should not get setter: {content}"
    );
}

#[test]
fn blanket_unbox_oneof_skips_recursive_variants() {
    // Config::unbox_oneof() ("." blanket rule) stores every NON-recursive
    // variant inline: recursive variants are silently kept boxed rather than
    // rejected. Only an exact-path rule on a recursive variant errors (see
    // inline_oneof_unbox_recursive_variant_is_rejected).
    let proto = dedent(
        r#"
        syntax = "proto3";
        package test;
        message Node {
          oneof kind {
            Node child = 1;
            int32 leaf = 2;
          }
        }
        message Small { int32 value = 1; }
        message Envelope {
          oneof body {
            Small small = 1;
          }
        }
        "#,
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let proto_path = dir.path().join("test.proto");
    std::fs::write(&proto_path, &proto).expect("write proto");
    let fds = compile_protos(
        &[proto_path.to_str().unwrap()],
        &[dir.path().to_str().unwrap()],
    );

    let mut config = no_views();
    config.unboxed_oneof_fields.push(".".to_string()); // == Config::unbox_oneof()
    let result = buffa_codegen::generate(&fds.file, &["test.proto".into()], &config);
    let files = result.unwrap_or_else(|e| {
        panic!("blanket unbox_oneof() should skip the recursive variant, but errored: {e}")
    });
    let content = files
        .into_iter()
        .map(|f| f.content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        content.contains("Child(::buffa::alloc::boxed::Box<"),
        "recursive variant must stay boxed under the blanket rule"
    );
    assert!(
        content.contains("Small(super::super::super::Small)"),
        "non-recursive variant should be inline under the blanket rule"
    );
}

#[test]
fn blanket_unbox_oneof_keeps_mutually_recursive_variants_boxed() {
    // Mutual recursion: A.body.b -> B and B.body.a -> A form a cycle only
    // when BOTH edges are inline. The blanket rule must keep both boxed
    // (the walk is conservative and order-independent), while the
    // non-recursive C variant inlines.
    let mut config = no_views();
    config.unboxed_oneof_fields.push(".".to_string());
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message A {
          oneof body {
            B b = 1;
            C c = 2;
          }
        }
        message B {
          oneof body {
            A a = 1;
          }
        }
        message C { int32 value = 1; }
        "#,
        &config,
    );
    assert!(
        content.contains("B(::buffa::alloc::boxed::Box<super::super::super::B>)"),
        "mutually recursive variant A.body.b must stay boxed: {content}"
    );
    assert!(
        content.contains("A(::buffa::alloc::boxed::Box<super::super::super::A>)"),
        "mutually recursive variant B.body.a must stay boxed: {content}"
    );
    assert!(
        content.contains("C(super::super::super::C)"),
        "non-recursive variant should be inline under the blanket rule: {content}"
    );
}

#[test]
fn prefix_unbox_rule_on_recursive_variant_skips_without_error() {
    // A non-blanket prefix rule (message scope, not exact variant path) that
    // matches a recursive variant takes the silent-skip path, not the
    // exact-rule error: the recursive variant stays boxed, siblings inline.
    let mut config = no_views();
    config.unboxed_oneof_fields.push(".test.Node".to_string());
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Leaf { int32 value = 1; }
        message Node {
          oneof kind {
            Node child = 1;
            Leaf leaf = 2;
          }
        }
        "#,
        &config,
    );
    assert!(
        content.contains("Child(::buffa::alloc::boxed::Box<super::super::super::Node>)"),
        "recursive variant must stay boxed under a prefix rule: {content}"
    );
    assert!(
        content.contains("Leaf(super::super::super::Leaf)"),
        "non-recursive sibling should be inline under the prefix rule: {content}"
    );
}

#[test]
fn default_inline_pointer_skips_recursive_fields() {
    // PointerRepr::Inline (the default) stores every NON-recursive singular
    // message field inline; recursive fields are silently kept on Box.
    // Mirrors blanket_unbox_oneof_skips_recursive_variants for #248.
    let config = no_views();
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Small { int32 value = 1; }
        message Node {
          Node child = 1;
          Small leaf = 2;
        }
        "#,
        &config,
    );
    assert!(
        content.contains("pub child: ::buffa::MessageField<Self>,"),
        "recursive singular field must stay on Box (default pointer) under the blanket rule: {content}"
    );
    assert!(
        content.contains("pub leaf: ::buffa::MessageField<Small, ::buffa::Inline<Small>>"),
        "non-recursive singular field should be inline under the blanket rule: {content}"
    );
}

#[test]
fn default_inline_pointer_keeps_mutually_recursive_boxed() {
    // Mutual recursion: A.b -> B and B.a -> A form a cycle only when BOTH
    // fields are inline. The default Inline must keep both on Box; the
    // non-recursive A.c field inlines.
    let config = no_views();
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message A { B b = 1; C c = 2; }
        message B { A a = 1; }
        message C { int32 value = 1; }
        "#,
        &config,
    );
    assert!(
        content.contains("pub b: ::buffa::MessageField<B>,"),
        "mutually recursive field A.b must stay on Box: {content}"
    );
    assert!(
        content.contains("pub a: ::buffa::MessageField<A>,"),
        "mutually recursive field B.a must stay on Box: {content}"
    );
    assert!(
        content.contains("pub c: ::buffa::MessageField<C, ::buffa::Inline<C>>"),
        "non-recursive field A.c should be inline under the blanket rule: {content}"
    );
}

#[test]
fn exact_inline_rule_on_recursive_field_is_rejected() {
    // An exact-path PointerRepr::Inline rule naming a recursive singular
    // field is a hard codegen error — the user asked for an unsized struct.
    let proto = dedent(
        r#"
        syntax = "proto3";
        package test;
        message Node { Node child = 1; }
        "#,
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let proto_path = dir.path().join("test.proto");
    std::fs::write(&proto_path, &proto).expect("write proto");
    let fds = compile_protos(
        &[proto_path.to_str().unwrap()],
        &[dir.path().to_str().unwrap()],
    );

    let mut config = no_views();
    config.pointer_fields.push((
        ".test.Node.child".to_string(),
        buffa_codegen::PointerRepr::Inline,
    ));
    let result = buffa_codegen::generate(&fds.file, &["test.proto".into()], &config);
    let err = result.expect_err("inlining a recursive singular field should error");
    let msg = err.to_string();
    assert!(
        msg.contains("recursive") && msg.contains(".test.Node.child"),
        "error should explain the recursion and name the field: {err}"
    );
}

#[test]
fn box_type_opt_out_restores_boxed_default() {
    // PointerRepr::Box is the opt-out from the Inline default: a path-scoped
    // rule keeps the matched fields on Box; a "." blanket restores the
    // pre-0.9 boxed default everywhere.
    let mut config = no_views();
    config
        .pointer_fields
        .push((".test.Node".to_string(), buffa_codegen::PointerRepr::Box));
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Leaf { int32 value = 1; }
        message Node { Leaf leaf = 1; }
        message Elsewhere { Leaf leaf = 1; }
        "#,
        &config,
    );
    assert!(
        content.contains("pub leaf: ::buffa::MessageField<Leaf>,"),
        "Node.leaf opted out via Box rule must use the boxed pointer: {content}"
    );
    assert!(
        content.contains("pub leaf: ::buffa::MessageField<Leaf, ::buffa::Inline<Leaf>>"),
        "Elsewhere.leaf (no rule) should be inline by default: {content}"
    );
}

#[test]
fn unbox_oneof_and_inline_field_detect_cross_kind_cycle() {
    // A cycle through ONE inlined oneof variant and ONE inlined singular field
    // (A.body.b -> B, B.a -> A) must be caught by both resolvers: the variant
    // stays boxed and the field stays on Box. Either alone is non-recursive,
    // so the recursion DFS must follow both edge kinds.
    let mut config = no_views();
    config.unboxed_oneof_fields.push(".".to_string());
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message A {
          oneof body { B b = 1; }
        }
        message B { A a = 1; }
        "#,
        &config,
    );
    assert!(
        content.contains("B(::buffa::alloc::boxed::Box<super::super::super::B>)"),
        "oneof variant in a cross-kind cycle must stay boxed: {content}"
    );
    assert!(
        content.contains("pub a: ::buffa::MessageField<A>,"),
        "singular field in a cross-kind cycle must stay on Box: {content}"
    );
}

#[test]
fn boxed_oneof_variant_under_inline_default_uses_box() {
    // The default PointerRepr::Inline also matches boxed oneof variant paths,
    // but the resolved-set check (singular fields only) demotes them: the
    // variant stays Box-wrapped so unbox_oneof remains the sole inlining knob
    // for oneofs and its recursion guard cannot be bypassed.
    let config = no_views();
    let content = generate_proto(
        r#"
        syntax = "proto3";
        package test;
        message Node {
          oneof kind { Node child = 1; }
        }
        "#,
        &config,
    );
    assert!(
        content.contains("Child(::buffa::alloc::boxed::Box<"),
        "boxed oneof variant must use Box even under a blanket Inline rule: {content}"
    );
    assert!(
        !content.contains("Child(::buffa::Inline<"),
        "boxed oneof variant must not use the Inline pointer: {content}"
    );
}

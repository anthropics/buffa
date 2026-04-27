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
    assert!(content.contains("::buffa::MessageField<outer::Inner>"));
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
    config.bytes_fields.push(".".into());
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
        content.contains("::bytes::Bytes"),
        "bytes fields should use Bytes type: {content}"
    );
    assert!(
        !content.contains("Vec<u8>"),
        "bytes fields should not use Vec<u8> when bytes mapping is enabled"
    );
    // Regression: decode path must convert Vec<u8> -> Bytes (not call
    // merge_bytes which expects &mut Vec<u8>).
    assert!(
        content.contains("::bytes::Bytes::from(::buffa::types::decode_bytes"),
        "decode must wrap decode_bytes in Bytes::from: {content}"
    );
    assert!(
        !content.contains("merge_bytes(&mut self"),
        "must not call merge_bytes on Bytes field (no &mut): {content}"
    );
}

#[test]
fn inline_bytes_field_selective_mapping() {
    let mut config = no_views();
    config.bytes_fields.push(".test.Msg.data".into());
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
        content.contains("::bytes::Bytes"),
        "selected field should use Bytes: {content}"
    );
    assert!(
        content.contains("Vec<u8>"),
        "non-selected field should still use Vec<u8>: {content}"
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
    // Should have internal fields but no user fields.
    assert!(content.contains("__buffa_unknown_fields"));
    assert!(content.contains("__buffa_cached_size"));
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

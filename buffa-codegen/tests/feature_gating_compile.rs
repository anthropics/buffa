//! Compile-checks gated codegen output across the feature matrix.
//!
//! This is the verification the codegen-level string assertions in
//! `tests/feature_gating.rs` cannot provide: an actual `cargo check` of
//! the gated output with each `{json, views, text}` feature subset, against
//! the full `buffa-test/protos/` fixture set (25 protos exercising
//! editions, oneofs, extensions, maps, groups, MessageSet, cross-package
//! references, …). It catches the kind of bug a string assertion cannot —
//! a `cfg`-gated item referenced from an ungated context, a `cfg_attr`
//! body that produces a dangling reference, a cross-feature dependency
//! between gated kinds.
//!
//! Marked `#[ignore]` so the default `cargo test --workspace` run stays
//! fast; CI runs it via `cargo test -p buffa-codegen --test
//! feature_gating_compile -- --ignored`. Locally: `task test` does not
//! include it; run it directly when touching the gating mechanism.
//!
//! Requires `protoc` on PATH and a populated workspace `target/` (the
//! generated crate path-deps on `buffa` and `buffa-types` and reuses the
//! workspace target directory so `buffa` itself isn't rebuilt 8 times).

use std::path::{Path, PathBuf};
use std::process::Command;

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;
use buffa_codegen::CodeGenConfig;

const PROTOS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../buffa-test/protos");
const WORKSPACE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// Fixture protos that exercise distinct codegen surfaces. Excludes
/// `editions_2024.proto` (requires protoc ≥ 30 — same skip as
/// `buffa-test/build.rs`) and the cross-package nestpkg protos which need
/// to be compiled together. Everything else compiles standalone.
const PROTOS: &[&str] = &[
    "basic.proto",
    "cross_syntax.proto",
    "custom_options.proto",
    "edge_cases.proto",
    "editions_enum_json.proto",
    "ext_json.proto",
    "group_ext.proto",
    "json_types.proto",
    "keywords.proto",
    "messageset.proto",
    "name_collisions.proto",
    "nested_deep.proto",
    "proto2_defaults.proto",
    "proto2_json.proto",
    "proto3_semantics.proto",
    "utf8_validation.proto",
    "view_json.proto",
    "view_json_proto2.proto",
    "wkt_usage.proto",
];

/// Proto sets that must be compiled together (cross-package references
/// or shared package names).
const PROTO_GROUPS: &[&[&str]] = &[
    &["nestpkg_outer.proto", "nestpkg_inner.proto"],
    &["cross_package.proto", "basic.proto", "nested_deep.proto"],
    &["prelude_shadow.proto", "prelude_shadow_sibling.proto"],
];

fn protoc() -> String {
    std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string())
}

fn compile_protos(files: &[&str], includes: &[&str]) -> FileDescriptorSet {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let mut cmd = Command::new(protoc());
    cmd.arg("--include_imports");
    cmd.arg(format!("--descriptor_set_out={}", tmp.path().display()));
    for inc in includes {
        cmd.arg(format!("--proto_path={inc}"));
    }
    for f in files {
        cmd.arg(f);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("protoc not found ({e})"));
    if !out.status.success() {
        panic!("protoc failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let bytes = std::fs::read(tmp.path()).expect("read descriptor set");
    FileDescriptorSet::decode_from_slice(&bytes).expect("decode descriptor set")
}

/// One gated-output crate, ready to `cargo check`.
struct GatedCrate {
    /// Held to keep the temp dir alive.
    _dir: tempfile::TempDir,
    manifest: PathBuf,
}

/// Generate every fixture proto with `gate_impls_on_crate_features = true`
/// and lay out a self-contained crate with `json` / `views` / `text`
/// features that path-dep on the in-tree `buffa` and `buffa-types`.
fn build_gated_crate() -> GatedCrate {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("src");
    let gen = src.join("gen");
    std::fs::create_dir_all(&gen).expect("mkdir");

    let mut cfg = CodeGenConfig::default();
    cfg.generate_json = true;
    cfg.generate_views = true;
    cfg.generate_text = true;
    cfg.generate_arbitrary = false;
    cfg.preserve_unknown_fields = true;
    cfg.gate_impls_on_crate_features = true;
    cfg.allow_message_set = true;

    // Two codegen passes: the standalone protos one-at-a-time (each its own
    // package), plus the cross-package pair as a unit.
    let mut packages = std::collections::BTreeSet::new();
    for proto in PROTOS {
        let path = Path::new(PROTOS_DIR).join(proto);
        let fds = compile_protos(&[path.to_str().unwrap()], &[PROTOS_DIR]);
        emit(&fds, &[proto], &cfg, &gen, &mut packages);
    }
    for group in PROTO_GROUPS {
        let paths: Vec<_> = group
            .iter()
            .map(|p| Path::new(PROTOS_DIR).join(p))
            .collect();
        let path_strs: Vec<&str> = paths.iter().map(|p| p.to_str().unwrap()).collect();
        let fds = compile_protos(&path_strs, &[PROTOS_DIR]);
        emit(&fds, group, &cfg, &gen, &mut packages);
    }

    // `gen/mod.rs` — nested `pub mod` blocks `include!`-ing each per-package
    // stitcher, with the same `#![allow]` block `protoc-gen-buffa-packaging`
    // would emit.
    let mut mod_rs = String::from(
        "#![allow(\n    non_camel_case_types, dead_code, unused_imports, unused_qualifications,\n    \
         clippy::derivable_impls, clippy::match_single_binding, clippy::uninlined_format_args,\n    \
         clippy::doc_lazy_continuation, clippy::module_inception\n)]\n\n",
    );
    mod_rs.push_str(&render_mod_tree(&packages));
    std::fs::write(gen.join("mod.rs"), mod_rs).expect("write mod.rs");

    std::fs::write(src.join("lib.rs"), "pub mod gen;\n").expect("write lib.rs");

    let manifest = dir.path().join("Cargo.toml");
    let workspace_root = Path::new(WORKSPACE_ROOT)
        .canonicalize()
        .expect("canonicalize workspace root");
    std::fs::write(
        &manifest,
        format!(
            r#"[package]
name = "feature-gated-fixture"
version = "0.0.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"

[features]
default = []
json = ["buffa/json", "buffa-types/json", "dep:serde", "dep:serde_json"]
views = []
text = ["buffa/text"]

[dependencies]
buffa = {{ path = "{root}/buffa" }}
buffa-types = {{ path = "{root}/buffa-types" }}
serde = {{ version = "1", features = ["derive"], optional = true }}
serde_json = {{ version = "1", optional = true }}

[workspace]
"#,
            root = workspace_root.display()
        ),
    )
    .expect("write Cargo.toml");

    GatedCrate {
        _dir: dir,
        manifest,
    }
}

fn emit(
    fds: &FileDescriptorSet,
    files_to_generate: &[&str],
    cfg: &CodeGenConfig,
    gen_dir: &Path,
    packages: &mut std::collections::BTreeSet<String>,
) {
    let to_gen: Vec<String> = files_to_generate.iter().map(|s| s.to_string()).collect();
    let outputs = buffa_codegen::generate(&fds.file, &to_gen, cfg)
        .unwrap_or_else(|e| panic!("codegen failed for {files_to_generate:?}: {e}"));
    for f in &outputs {
        // Track package paths for the mod tree. Stitchers are
        // `<dotted.pkg>.mod.rs`.
        if f.kind == buffa_codegen::GeneratedFileKind::PackageMod {
            packages.insert(f.package.clone());
        }
        std::fs::write(gen_dir.join(&f.name), &f.content)
            .unwrap_or_else(|e| panic!("write {}: {e}", f.name));
    }
}

/// Render nested `pub mod a { pub mod b { include!("a.b.mod.rs"); } }`
/// blocks for the set of dotted package names.
fn render_mod_tree(packages: &std::collections::BTreeSet<String>) -> String {
    // Build a trie of package segments.
    #[derive(Default)]
    struct Node {
        children: std::collections::BTreeMap<String, Node>,
        is_pkg: bool,
        full: String,
    }
    let mut root = Node::default();
    for pkg in packages {
        let mut node = &mut root;
        let mut full = String::new();
        for seg in pkg.split('.') {
            if !full.is_empty() {
                full.push('.');
            }
            full.push_str(seg);
            let next_full = full.clone();
            node = node.children.entry(seg.to_string()).or_default();
            node.full = next_full;
        }
        node.is_pkg = true;
    }
    fn render(node: &Node, depth: usize, out: &mut String) {
        for (seg, child) in &node.children {
            let indent = "    ".repeat(depth);
            out.push_str(&format!("{indent}pub mod {seg} {{\n"));
            if child.is_pkg {
                out.push_str(&format!(
                    "{indent}    include!(\"{}.mod.rs\");\n",
                    child.full
                ));
            }
            render(child, depth + 1, out);
            out.push_str(&format!("{indent}}}\n"));
        }
    }
    let mut out = String::new();
    render(&root, 0, &mut out);
    out
}

/// `cargo check` the gated crate with the given feature set.
fn check(manifest: &Path, features: &[&str]) {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("check")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--no-default-features")
        .arg("--quiet");
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    // Reuse the workspace target dir so `buffa` / `buffa-types` are cached
    // across runs.
    cmd.env(
        "CARGO_TARGET_DIR",
        Path::new(WORKSPACE_ROOT).join("target/feature-gating-compile"),
    );
    let out = cmd.output().expect("run cargo check");
    if !out.status.success() {
        panic!(
            "cargo check failed with features {features:?}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// The full `{json, views, text}` power set: 8 combinations. Slow (~minutes
/// from a cold target dir, seconds when warm) so `#[ignore]`d; CI runs it
/// explicitly.
#[test]
#[ignore = "slow compile-matrix test; run with --ignored"]
fn gated_output_compiles_across_feature_matrix() {
    let krate = build_gated_crate();
    let combos: &[&[&str]] = &[
        &[],                        // binary codec only
        &["json"],                  // serde, no views
        &["views"],                 // views, no serde
        &["text"],                  // textproto only
        &["json", "views"],         // BSR plugin defaults
        &["json", "text"],          // both registry types
        &["views", "text"],         //
        &["json", "views", "text"], // everything
    ];
    for combo in combos {
        check(&krate.manifest, combo);
    }
}

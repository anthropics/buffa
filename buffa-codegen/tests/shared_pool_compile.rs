//! End-to-end compile check for shared-pool reflection mode.
//!
//! Hand-builds a two-package descriptor set with a cross-package field, runs
//! codegen with `shared_descriptor_pool = true`, assembles the module tree the
//! way `buffa-build` / `protoc-gen-buffa-packaging` would (one shared
//! `__buffa_fds` root module plus per-package delegations), and `cargo check`s
//! the result against the in-tree `buffa` / `buffa-descriptor`.
//!
//! This is the verification the string-level tests cannot give: it proves the
//! `super::`-depth each package uses to reach the shared root actually
//! resolves, across packages of different nesting depth, with a real cross
//! reference. Needs no `protoc` (descriptors are built in memory), but does
//! spawn `cargo`, so it is `#[ignore]`d like `feature_gating_compile`.
//! Run: `cargo test -p buffa-codegen --test shared_pool_compile -- --ignored`.

use std::path::Path;
use std::process::Command;

use buffa_codegen::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_codegen::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use buffa_codegen::{CodeGenConfig, GeneratedFileKind, IncludeMode};

const WORKSPACE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

fn field(name: &str, number: i32, ty: Type) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(ty),
        ..Default::default()
    }
}

fn proto3(name: &str, package: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.into()),
        package: Some(package.into()),
        syntax: Some("proto3".into()),
        ..Default::default()
    }
}

#[test]
#[ignore = "spawns cargo; run explicitly with --ignored"]
fn shared_pool_tree_compiles_inline() {
    run_shared_pool_compile(false);
}

#[test]
#[ignore = "spawns cargo; run explicitly with --ignored"]
fn shared_pool_tree_compiles_include_bytes_sidecar() {
    run_shared_pool_compile(true);
}

/// Assemble a two-package shared-pool tree and `cargo test` it, proving the
/// `super::` delegation depth resolves across packages of differing nesting
/// and that the packages observe one shared pool. `sidecar` selects the
/// root-module embedding: `false` inlines the bytes (the packaging-plugin
/// path), `true` writes a binary sidecar and `include_bytes!`s it (the
/// buffa-build path).
fn run_shared_pool_compile(sidecar: bool) {
    // Package `alpha.v1` (depth 2) with a plain message.
    let mut alpha = proto3("alpha.proto", "alpha.v1");
    alpha.message_type.push(DescriptorProto {
        name: Some("Alpha".into()),
        field: vec![field("id", 1, Type::TYPE_INT32)],
        ..Default::default()
    });

    // Package `beta` (depth 1) with a cross-package field referencing Alpha —
    // exercises the shared pool resolving a type from another package.
    let mut beta = proto3("beta.proto", "beta");
    let mut cross = field("a", 2, Type::TYPE_MESSAGE);
    cross.type_name = Some(".alpha.v1.Alpha".into());
    beta.message_type.push(DescriptorProto {
        name: Some("Beta".into()),
        field: vec![field("n", 1, Type::TYPE_INT32), cross],
        ..Default::default()
    });

    // Unnamed root package (depth 0) — exercises the `shared_pool_supers("") == 2`
    // delegation path, the one arithmetic edge the other packages don't cover.
    let mut root = proto3("root.proto", "");
    root.message_type.push(DescriptorProto {
        name: Some("Root".into()),
        field: vec![field("v", 1, Type::TYPE_INT32)],
        ..Default::default()
    });

    let files = [alpha, beta, root];
    let mut cfg = CodeGenConfig::default();
    cfg.generate_reflection = true;
    cfg.generate_reflection_vtable = true;
    cfg.shared_descriptor_pool = true;
    let to_gen = vec![
        "alpha.proto".to_string(),
        "beta.proto".to_string(),
        "root.proto".to_string(),
    ];
    let generated = buffa_codegen::generate(&files, &to_gen, &cfg).expect("codegen should succeed");

    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("src");
    let gen = src.join("gen");
    std::fs::create_dir_all(&gen).expect("mkdir");

    let mut packages = std::collections::BTreeSet::new();
    for f in &generated {
        if f.kind == GeneratedFileKind::PackageMod {
            packages.insert(f.package.clone());
        }
        std::fs::write(gen.join(&f.name), &f.content).expect("write generated file");
    }

    // Assemble `gen/mod.rs`: the single shared root module (inline bytes) at
    // the tree root, then the package module tree that delegates to it.
    let fds_bytes = buffa_codegen::encode_descriptor_set(&files, &[]);
    let entries: Vec<(String, String)> = packages
        .iter()
        .map(|p| (buffa_codegen::package_to_mod_filename(p), p.clone()))
        .collect();
    let mut mod_rs = String::from(
        "#![allow(non_camel_case_types, dead_code, unused_imports, unused_qualifications, clippy::all)]\n\n",
    );
    let embedding = if sidecar {
        const SIDECAR: &str = "buffa_descriptor_set.binpb";
        std::fs::write(gen.join(SIDECAR), &fds_bytes).expect("write sidecar");
        buffa_codegen::FdsEmbedding::Sidecar {
            file_name: SIDECAR,
            mode: IncludeMode::Relative(""),
        }
    } else {
        buffa_codegen::FdsEmbedding::Inline
    };
    mod_rs.push_str(&buffa_codegen::shared_descriptor_root_module(
        &fds_bytes, embedding, None,
    ));
    mod_rs.push('\n');
    mod_rs.push_str(&buffa_codegen::generate_module_tree(
        &entries,
        IncludeMode::Relative(""),
        false,
    ));
    std::fs::write(gen.join("mod.rs"), mod_rs).expect("write mod.rs");
    std::fs::write(src.join("lib.rs"), "pub mod gen;\n").expect("write lib.rs");

    // Runtime proof: both packages' `descriptor_pool()` must return the one
    // shared instance, and `reflect()` must resolve against it.
    let tests = src.parent().unwrap().join("tests");
    std::fs::create_dir_all(&tests).expect("mkdir tests");
    // Distinct crate name per mode so the two `#[ignore]` tests don't collide
    // in the shared target dir when run together.
    let pkg_name = if sidecar {
        "shared-pool-fixture-sidecar"
    } else {
        "shared-pool-fixture-inline"
    };
    let crate_ident = pkg_name.replace('-', "_");
    std::fs::write(
        tests.join("runtime.rs"),
        format!(
            r#"use {crate_ident}::gen::{{alpha::v1 as a, beta as b}};
// Unnamed root package re-exports at the tree root (depth-0 delegation).
use {crate_ident}::gen::descriptor_pool as root_pool;

#[test]
fn packages_share_one_pool_instance() {{
    assert!(
        std::sync::Arc::ptr_eq(a::descriptor_pool(), b::descriptor_pool()),
        "every package must observe the same shared DescriptorPool"
    );
    // The unnamed root package (depth 0) delegates to the same instance.
    assert!(std::sync::Arc::ptr_eq(a::descriptor_pool(), root_pool()));
}}

#[test]
fn reflect_resolves_against_shared_pool() {{
    use buffa_descriptor::reflect::Reflectable;
    let beta = b::Beta::default();
    assert_eq!(beta.reflect().message_descriptor().full_name(), "beta.Beta");
}}
"#
        ),
    )
    .expect("write runtime test");

    let root = Path::new(WORKSPACE_ROOT)
        .canonicalize()
        .expect("canonicalize workspace root");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        format!(
            r#"[package]
name = "{pkg_name}"
version = "0.0.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"

[dependencies]
buffa = {{ path = "{root}/buffa" }}
buffa-descriptor = {{ path = "{root}/buffa-descriptor", features = ["reflect"] }}

[workspace]
"#,
            root = root.display()
        ),
    )
    .expect("write Cargo.toml");

    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("test")
        .arg("--manifest-path")
        .arg(dir.path().join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .expect("run cargo test");
    assert!(
        out.status.success(),
        "shared-pool tree failed to compile or its runtime checks failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

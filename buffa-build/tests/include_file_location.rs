//! Where [`Config::include_file`] may place the include file.
//!
//! The paths the include file names are absolute in `$OUT_DIR` mode and
//! relative to the include file in explicit-`out_dir` mode, so a
//! path-qualified include file resolves in the first mode only. With
//! `shared_descriptor_pool` the name must be bare in both modes: the sidecar
//! is named after the include file's stem alone and stays at the output root,
//! so include files in two directories would share it.

use buffa_build::Config;
use buffa_codegen::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_codegen::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use std::path::{Path, PathBuf};

/// A one-package descriptor set, so the test needs no protoc.
fn descriptor_set() -> Vec<u8> {
    let file = FileDescriptorProto {
        name: Some("foo/v1/thing.proto".into()),
        package: Some("foo.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![DescriptorProto {
            name: Some("Thing".into()),
            field: vec![FieldDescriptorProto {
                name: Some("id".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    buffa_codegen::encode_descriptor_set(&[file], &[])
}

fn config(include_file: &str, shared_pool: bool) -> Config {
    Config::new()
        .descriptor_set_bytes(descriptor_set())
        .files(&["foo/v1/thing.proto"])
        .include_file(include_file)
        .generate_reflection(shared_pool)
        .shared_descriptor_pool(shared_pool)
}

/// The string literals in `source` that name a generated file: the argument of
/// each `include!` and of the shared pool's `include_bytes!`.
fn named_files(source: &str) -> Vec<&str> {
    let mut names: Vec<&str> = source
        .split('"')
        .filter(|literal| literal.ends_with(".rs") || literal.ends_with(".binpb"))
        .collect();
    names.sort_unstable();
    names
}

/// Asserts that every file `include_file` names exists where rustc looks for
/// it: under `$OUT_DIR` when `out_dir_env` is given (the paths are
/// `concat!(env!("OUT_DIR"), "/<name>")`), next to the include file otherwise.
fn assert_includes_resolve(include_file: &Path, out_dir_env: Option<&Path>, shared_pool: bool) {
    let source = std::fs::read_to_string(include_file).unwrap();
    let names = named_files(&source);
    let expected: &[&str] = match (out_dir_env.is_some(), shared_pool) {
        (false, false) => &["foo.v1.mod.rs"],
        (false, true) => &["foo.v1.mod.rs", "gen_mod.descriptor_set.binpb"],
        (true, false) => &["/foo.v1.mod.rs"],
        (true, true) => &["/foo.v1.mod.rs", "/gen_mod.descriptor_set.binpb"],
    };
    assert_eq!(names, expected, "files named by the include file: {source}");
    for name in names {
        let resolved = match out_dir_env {
            Some(out_dir) => PathBuf::from(format!("{}{name}", out_dir.display())),
            None => include_file.parent().unwrap().join(name),
        };
        assert!(
            resolved.is_file(),
            "{name:?} resolves to {}, which does not exist (shared_pool: {shared_pool})",
            resolved.display()
        );
    }
}

/// The `$OUT_DIR`-mode rows. The only test in this file that reads or writes
/// the environment.
#[test]
fn out_dir_env_mode_takes_a_nested_include_file_only_without_the_shared_pool() {
    // Shared pool off: accepted, and `compile` creates `nested/`.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("OUT_DIR", dir.path());
    config("nested/gen_mod.rs", false)
        .compile()
        .unwrap_or_else(|e| panic!("$OUT_DIR mode, shared pool off: nested include file: {e}"));
    assert_includes_resolve(
        &dir.path().join("nested/gen_mod.rs"),
        Some(dir.path()),
        false,
    );

    // Shared pool on: rejected, because `other/gen_mod.rs` from a second
    // `compile` call would write the same `gen_mod.descriptor_set.binpb`.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("OUT_DIR", dir.path());
    let err = config("nested/gen_mod.rs", true)
        .compile()
        .expect_err("the shared pool takes a bare include file name only");
    let message = err.to_string();
    assert!(
        message.contains("shared_descriptor_pool")
            && message.contains("bare file name")
            && message.contains("sidecar")
            && message.contains("nested/gen_mod.rs"),
        "error should name the requirement, its reason and the path: {message}"
    );
    assert!(
        !dir.path().join("gen_mod.descriptor_set.binpb").exists(),
        "the sidecar must not be written"
    );

    // Shared pool on with a bare name: accepted.
    config("gen_mod.rs", true)
        .compile()
        .unwrap_or_else(|e| panic!("$OUT_DIR mode, shared pool on: bare include file: {e}"));
    assert_includes_resolve(&dir.path().join("gen_mod.rs"), Some(dir.path()), true);

    std::env::remove_var("OUT_DIR");
}

#[test]
fn explicit_out_dir_rejects_nested_include_file() {
    for shared_pool in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        // With the directory in place the write would succeed, and the include
        // file would name siblings that are one level up.
        std::fs::create_dir(dir.path().join("nested")).unwrap();

        let err = config("nested/gen_mod.rs", shared_pool)
            .out_dir(dir.path())
            .compile()
            .expect_err("explicit out_dir with a nested include file must be rejected");

        // Each rule gives its own reason; the shared pool's is checked first.
        let reason = if shared_pool {
            "sidecar"
        } else {
            "relative to the include file"
        };
        let message = err.to_string();
        assert!(
            message.contains("bare file name")
                && message.contains(reason)
                && message.contains("nested/gen_mod.rs"),
            "shared_pool {shared_pool}: error should name the requirement, its reason \
             and the path: {message}"
        );
        assert!(
            !dir.path().join("nested/gen_mod.rs").exists(),
            "shared_pool {shared_pool}: the include file must not be written"
        );
    }
}

#[test]
fn explicit_out_dir_rejects_other_spellings_of_a_file_in_out_dir() {
    // Both name a file directly in `out_dir`, where the relative paths would
    // resolve, and neither is a bare file name.
    for shared_pool in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        let absolute = dir.path().join("gen_mod.rs");

        for include_file in [absolute.to_str().unwrap(), "nested/../gen_mod.rs"] {
            let err = config(include_file, shared_pool)
                .out_dir(dir.path())
                .compile()
                .expect_err("explicit out_dir takes a bare include file name only");
            assert!(
                err.to_string().contains("bare file name"),
                "shared_pool {shared_pool}: {include_file:?}: {err}"
            );
        }
    }
}

#[test]
fn explicit_out_dir_accepts_include_file_in_out_dir() {
    for shared_pool in [false, true] {
        for include_file in ["gen_mod.rs", "./gen_mod.rs"] {
            let dir = tempfile::tempdir().unwrap();

            config(include_file, shared_pool)
                .out_dir(dir.path())
                .compile()
                .unwrap_or_else(|e| {
                    panic!("explicit out_dir, shared_pool {shared_pool}: {include_file:?}: {e}")
                });

            assert_includes_resolve(&dir.path().join("gen_mod.rs"), None, shared_pool);
        }
    }
}

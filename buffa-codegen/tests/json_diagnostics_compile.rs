//! Compile and run downstream encoding and custom-representation regressions.
//! Run explicitly with `cargo test -p buffa-codegen --test json_diagnostics_compile -- --ignored`.

use std::{fs, path::PathBuf, process::Command};

#[test]
#[ignore = "spawns Cargo; exercised separately in CI"]
fn generated_diagnostics_preserve_downstream_encodings() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let temp = tempfile::tempdir().unwrap();
    for dir in ["src", "tests", "proto"] {
        fs::create_dir(temp.path().join(dir)).unwrap();
    }
    fs::write(
        temp.path().join("src/lib.rs"),
        include_str!("fixtures/json_diagnostics/consumer.rs"),
    )
    .unwrap();
    fs::write(
        temp.path().join("tests/compat.rs"),
        include_str!("fixtures/json_diagnostics/compat_tests.rs"),
    )
    .unwrap();
    fs::write(
        temp.path().join("proto/compat.proto"),
        include_str!("fixtures/json_diagnostics/compat.proto"),
    )
    .unwrap();
    fs::write(
        temp.path().join("build.rs"),
        include_str!("fixtures/json_diagnostics/build.rs"),
    )
    .unwrap();
    fs::copy(
        root.join("buffa-descriptor/tests/protos/reflect_test.fds"),
        temp.path().join("proto/reflect.fds"),
    )
    .unwrap();
    fs::copy(
        root.join("buffa-descriptor/tests/protos/json_wrong_kind_test.fds"),
        temp.path().join("proto/wkt.fds"),
    )
    .unwrap();
    let manifest = format!(
        r#"[package]
name = "diagnostics-consumer"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[dependencies]
buffa = {{ path = {buffa:?}, features = ["json"] }}
buffa-types = {{ path = {types:?}, features = ["json"] }}
buffa-descriptor = {{ path = {descriptor:?}, features = ["json", "reflect", "std"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
thiserror = "2"
postcard = {{ version = "=1.1.3", default-features = false, features = ["alloc"] }}
[dev-dependencies]
serde_norway = "0.9"
[build-dependencies]
buffa-build = {{ path = {build:?} }}
"#,
        buffa = root.join("buffa"),
        types = root.join("buffa-types"),
        build = root.join("buffa-build"),
        descriptor = root.join("buffa-descriptor")
    );
    let manifest_path = temp.path().join("Cargo.toml");
    fs::write(&manifest_path, manifest).unwrap();
    for args in [
        &["test"][..],
        &["test", "--features", "serde_json/arbitrary_precision"][..],
        &["clippy", "--all-targets", "--", "-D", "warnings"][..],
    ] {
        let output = Command::new(env!("CARGO"))
            .args(args)
            .current_dir(temp.path())
            .env("CARGO_TARGET_DIR", temp.path().join("target"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

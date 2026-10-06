//! Exercise an actual generated consumer with optional JSON dependencies.

#[test]
fn generated_custom_strings_compile_without_std_and_roundtrip_with_json() {
    let fixture = tempfile::tempdir().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    std::fs::create_dir(fixture.path().join("src")).unwrap();
    std::fs::write(fixture.path().join("Cargo.toml"), format!(r#"
[package]
name = "remote-serde-consumer"
version = "0.0.0"
edition = "2021"
[workspace]
[features]
json = ["dep:serde", "dep:serde_json", "buffa/json"]
[dependencies]
buffa = {{ path = "{}/buffa", default-features = false }}
buffa-remote-derive = {{ path = "{}/buffa-remote-derive" }}
serde = {{ version = "1", default-features = false, features = ["derive", "alloc"], optional = true }}
serde_json = {{ version = "1", default-features = false, features = ["alloc"], optional = true }}
[build-dependencies]
buffa-build = {{ path = "{}/buffa-build" }}
"#, root.display(), root.display(), root.display())).unwrap();
    std::fs::write(
        fixture.path().join("test.proto"),
        r#"
syntax = "proto3";
package example;
message Sample {
  optional string optional = 1;
  repeated string repeated = 2;
  map<string, string> map = 3;
}
"#,
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("build.rs"),
        r#"
fn main() {
    buffa_build::Config::new()
        .files(&["test.proto"])
        .includes(&["."])
        .string_type_custom("crate::Text")
        .generate_views(false)
        .generate_json(std::env::var_os("CARGO_FEATURE_JSON").is_some())
        .compile().unwrap();
}
"#,
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("src/lib.rs"),
        r###"
#![no_std]
extern crate alloc;
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug, buffa_remote_derive::ProtoString)]
#[buffa(remote = alloc::string::String)]
#[cfg_attr(feature = "json", buffa(serde))]
pub struct Text(alloc::string::String);
pub mod example { buffa::include_proto!("example"); }
#[cfg(all(test, feature = "json"))]
mod tests {
    #[test]
    fn roundtrip() {
        let json = r##"{"optional":"present","repeated":["a","b"],"map":{"key":"value"}}"##;
        let msg: super::example::Sample = serde_json::from_str(json).unwrap();
        assert_eq!(msg.optional.as_ref().unwrap().0, "present");
        assert_eq!(msg.repeated[1].0, "b");
        let output = serde_json::to_string(&msg).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap(),
                   serde_json::from_str::<serde_json::Value>(json).unwrap());
    }
}
"###,
    )
    .unwrap();
    // Reuse workspace versions so an offline check does not resolve newer
    // releases than the parent test run has downloaded.
    std::fs::copy(root.join("Cargo.lock"), fixture.path().join("Cargo.lock")).unwrap();
    for args in [vec!["check"], vec!["test", "--features", "json"]] {
        let output = std::process::Command::new(env!("CARGO"))
            .args(args)
            .arg("--offline")
            .env(
                "CARGO_TARGET_DIR",
                root.join("target/remote-serde-consumer"),
            )
            .env("RUSTFLAGS", "-D warnings")
            .current_dir(fixture.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

//! Provider and generated consumer packages exercise foreign policy types and
//! separate generation runs. Run explicitly with `--ignored` (also in CI).
use std::path::Path;
use std::process::Command;

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;
use buffa_codegen::{BytesRepr, CodeGenConfig, StringRepr};

const SCHEMA: &str = r#"
syntax = "proto3";
package PACKAGE;
message Record {
  bytes single = 1;
  optional bytes optional_blob = 2;
  string title = 3;
  optional string optional_title = 4;
  repeated bytes blobs = 5;
  repeated string labels = 6;
  map<int32, bytes> by_id = 7;
  map<bool, bytes> by_flag = 8;
  map<string, bytes> by_key = 9;
  map<string, bytes> by_tag = 10;
  map<string, string> text_pairs = 11;
  map<int32, string> texts = 12;
  oneof choice { bytes blob = 13; string text = 14; }
}
"#;

#[test]
#[ignore = "spawns cargo; run explicitly with --ignored"]
fn foreign_policy_types_compile_across_separate_generation_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    for name in ["policy-provider", "policy-consumer"] {
        std::fs::create_dir_all(dir.path().join(name).join("src")).unwrap();
    }
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"policy-provider\", \"policy-consumer\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("policy-provider/src/lib.rs"),
        include_str!("fixtures/json_policy_provider.rs"),
    )
    .unwrap();
    let dependencies = format!(
        "buffa = {{ path = {:?}, default-features = false, features = [\"json\"] }}\n\
         serde = {{ version = \"1\", default-features = false, features = [\"alloc\", \"derive\"] }}\n\
         serde_json = {{ version = \"1\", default-features = false, features = [\"alloc\"] }}\n", root.join("buffa"));
    for name in ["policy-provider", "policy-consumer"] {
        let features = if name == "policy-provider" {
            "[features]\ndefault = []\nstd = [\"buffa/std\"]\n"
        } else {
            "[features]\ndefault = [\"json\", \"views\"]\njson = []\nviews = []\nstd = [\"buffa/std\", \"policy-provider/std\"]\nreflect = [\"std\", \"dep:buffa-descriptor\"]\n"
        };
        let provider = if name == "policy-consumer" {
            "policy-provider = { path = \"../policy-provider\", default-features = false }\n"
        } else {
            ""
        };
        let reflection = if name == "policy-consumer" {
            format!("buffa-descriptor = {{ path = {:?}, default-features = false, features = [\"json\", \"reflect\"], optional = true }}\n", root.join("buffa-descriptor"))
        } else {
            String::new()
        };
        std::fs::write(dir.path().join(name).join("Cargo.toml"), format!(
            "[package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n{features}[dependencies]\n{dependencies}{provider}{reflection}")).unwrap();
    }
    let mut lib =
        String::from("#![cfg_attr(not(feature = \"std\"), no_std)]\nextern crate alloc;\n");
    for package in ["pa", "pb"] {
        let proto = dir.path().join(format!("{package}.proto"));
        std::fs::write(&proto, SCHEMA.replace("PACKAGE", package)).unwrap();
        let descriptor = dir.path().join(format!("{package}.pb"));
        let result = Command::new(std::env::var_os("PROTOC").unwrap_or_else(|| "protoc".into()))
            .arg(format!("--descriptor_set_out={}", descriptor.display()))
            .arg("--include_imports")
            .arg("-I")
            .arg(dir.path())
            .arg(&proto)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let descriptors =
            FileDescriptorSet::decode_from_slice(&std::fs::read(descriptor).unwrap()).unwrap();
        let mut config = CodeGenConfig::default();
        config.generate_json = true;
        config.generate_reflection = true;
        config.lazy_views = true;
        config.gate_impls_on_crate_features = true;
        config.bytes_fields = vec![
            (
                ".".into(),
                BytesRepr::Custom("::policy_provider::Blob".into()),
            ),
            (
                format!(".{package}.Record.blobs"),
                BytesRepr::Custom("((::policy_provider :: r#Blob))".into()),
            ),
        ];
        config.bytes_fields.push((
            format!(".{package}.Record.by_flag"),
            BytesRepr::Custom("::policy_provider::SizedBlob<{core::mem::size_of::<u8>()}>".into()),
        ));
        config.string_fields = vec![
            (
                ".".into(),
                StringRepr::Custom("::policy_provider::Text".into()),
            ),
            (
                format!(".{package}.Record.labels"),
                StringRepr::Custom("(::policy_provider :: Text)".into()),
            ),
            (
                format!(".{package}.Record.by_tag"),
                StringRepr::Custom("::policy_provider::Tag".into()),
            ),
        ];
        config.json_policy_types = vec![
            "::policy_provider::Blob".into(),
            "::policy_provider::Text".into(),
            "::policy_provider::SizedBlob<{core::mem::size_of::<u8>()}>".into(),
        ];
        let generated =
            buffa_codegen::generate(&descriptors.file, &[format!("{package}.proto")], &config)
                .unwrap();
        let output = dir.path().join("policy-consumer/src").join(package);
        std::fs::create_dir(&output).unwrap();
        for file in generated {
            std::fs::write(output.join(file.name), file.content).unwrap();
        }
        lib.push_str(&format!(
            "pub mod {package} {{ include!(\"{package}/{package}.mod.rs\"); }}\n"
        ));
    }
    lib.push_str("#[cfg(all(test, feature = \"json\", feature = \"views\"))]\nmod tests {\n");
    lib.push_str(include_str!("fixtures/json_policy_consumer.rs"));
    lib.push_str("\n}\n");
    std::fs::write(dir.path().join("policy-consumer/src/lib.rs"), lib).unwrap();
    // A separate target avoids deadlocking against the parent Cargo process.
    let mut commands = vec![
        vec!["check", "--no-default-features"],
        vec!["test"],
        vec!["test", "--features", "std"],
        vec!["test", "--features", "reflect"],
        vec![
            "clippy",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    ];
    // Cross-target compilation is opt-in because the native CI job does not
    // install WASM. Local validation can include the actual generated tests.
    if std::env::var_os("BUFFA_TEST_WASM").as_deref() == Some(std::ffi::OsStr::new("1")) {
        commands.push(vec![
            "check",
            "--tests",
            "--target",
            "wasm32-unknown-unknown",
        ]);
    }
    for arguments in commands {
        let (command, arguments) = arguments.split_first().unwrap();
        let result = Command::new(env!("CARGO"))
            .arg(command)
            .args(["--offline", "-p", "policy-consumer", "--manifest-path"])
            .arg(dir.path().join("Cargo.toml"))
            .arg("--target-dir")
            .arg(dir.path().join("target"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

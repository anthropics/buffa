use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_protoc-gen-buffa");

#[test]
fn version_prints_name_and_version() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("protoc-gen-buffa {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag() {
    let out = Command::new(BIN).arg("-V").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .starts_with("protoc-gen-buffa "));
}

#[test]
fn help_mentions_protoc_plugin_protocol() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("protoc plugin protocol"));
    assert!(stdout.contains("--buffa_opt"));
}

#[test]
fn unrecognized_arg_exits_2() {
    let out = Command::new(BIN).arg("bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("unrecognized argument"));
    assert!(stderr.contains("bogus"));
}

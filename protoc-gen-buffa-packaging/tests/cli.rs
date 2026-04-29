use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_protoc-gen-buffa-packaging");

#[test]
fn version_prints_name_and_version() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("protoc-gen-buffa-packaging {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag() {
    let out = Command::new(BIN).arg("-V").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .starts_with("protoc-gen-buffa-packaging "));
}

#[test]
fn help_mentions_filter_option() {
    for flag in ["--help", "-h"] {
        let out = Command::new(BIN).arg(flag).output().unwrap();
        assert!(out.status.success());
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(stdout.contains("protoc plugin protocol"));
        assert!(stdout.contains("filter=services"));
    }
}

#[test]
fn unrecognized_arg_exits_2() {
    let out = Command::new(BIN).arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr)
        .unwrap()
        .contains("unrecognized argument"));
}

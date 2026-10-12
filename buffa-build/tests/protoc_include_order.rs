use buffa_build::Config;

#[test]
fn overlapping_include_roots_generate_requested_files_in_either_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proto");
    let nested = root.join("vendor");
    std::fs::create_dir_all(&nested).unwrap();
    let input = nested.join("requested.proto");
    std::fs::write(
        &input,
        r#"syntax = "proto3";
package include_order;
message Requested { int32 value = 1; }
"#,
    )
    .unwrap();

    for (name, includes) in [
        ("root_first", [&root, &nested]),
        ("nested_first", [&nested, &root]),
    ] {
        let out = dir.path().join(name);
        Config::new()
            .files(&[&input])
            .includes(&includes)
            .out_dir(&out)
            .include_file("mod.rs")
            .compile()
            .unwrap();

        let package_mod = out.join("include_order.mod.rs");
        assert!(
            package_mod.is_file(),
            "{name}: requested package is missing"
        );
        let module_tree = std::fs::read_to_string(out.join("mod.rs")).unwrap();
        assert!(
            module_tree.contains("include_order.mod.rs"),
            "{name}: requested package is missing from the module tree"
        );
        let generated = std::fs::read_dir(&out)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            generated.contains("pub struct Requested"),
            "{name}: requested message is missing"
        );
    }
}

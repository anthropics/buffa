use buffa_build::Config;
use std::path::PathBuf;

#[test]
fn current_directory_components_generate_requested_files() {
    let cwd = std::env::current_dir().unwrap();
    // The inputs must be reachable by a path relative to the working
    // directory, which rules out the system temp directory.
    let dir = tempfile::tempdir_in(&cwd).unwrap();
    let root = dir.path().strip_prefix(&cwd).unwrap().join("proto");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    let input = root.join("nested/requested.proto");
    std::fs::write(
        &input,
        r#"syntax = "proto3";
package dot_paths;
message Requested { int32 value = 1; }
"#,
    )
    .unwrap();

    for (name, input, includes) in [
        (
            "file_dot",
            PathBuf::from(".").join(&input),
            vec![root.clone()],
        ),
        (
            "include_dot",
            input.clone(),
            vec![PathBuf::from(".").join(&root)],
        ),
        (
            "both_dot",
            PathBuf::from(".").join(&input),
            vec![PathBuf::from(".").join(&root)],
        ),
        (
            "current_directory",
            PathBuf::from(".").join(&input),
            vec![PathBuf::from(".")],
        ),
        (
            "implicit_current_directory",
            PathBuf::from(".").join(&input),
            vec![],
        ),
        (
            "relative_file_after_current_directory_include",
            input.clone(),
            vec![PathBuf::from("."), root.clone()],
        ),
        (
            "absolute_file_after_current_directory_include",
            cwd.join(&input),
            vec![PathBuf::from("."), cwd.join(&root)],
        ),
        (
            "parent_components_after_current_directory_include",
            root.join("nested/../nested/requested.proto"),
            vec![PathBuf::from("."), root.join("nested/../nested")],
        ),
        (
            "parent_components_after_broader_include",
            root.join("nested/../nested/requested.proto"),
            vec![root.clone(), root.join("nested/../nested")],
        ),
        (
            "redundant_components",
            PathBuf::from(".")
                .join(&root)
                .join("nested//./requested.proto"),
            vec![root.join(".")],
        ),
    ] {
        let out = dir.path().join(name);
        Config::new()
            .files(&[input])
            .includes(&includes)
            .out_dir(&out)
            .include_file("mod.rs")
            .compile()
            .unwrap_or_else(|err| panic!("{name}: {err}"));

        let package_mod = out.join("dot_paths.mod.rs");
        assert!(
            package_mod.is_file(),
            "{name}: requested package is missing"
        );
        let module_tree = std::fs::read_to_string(out.join("mod.rs")).unwrap();
        assert!(
            module_tree.contains("dot_paths.mod.rs"),
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

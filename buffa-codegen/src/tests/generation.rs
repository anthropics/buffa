//! Proto3 codegen: WKT auto-mapping, enums, TYPE_URL, message fields, repeated.

use super::*;

#[test]
fn test_wkt_auto_mapping_not_suppressed_by_sub_package() {
    // Regression: mapping .google.protobuf.compiler (a SUB-package) used
    // to suppress the WKT auto-mapping for .google.protobuf, breaking
    // Timestamp/Duration/etc. resolve_extern_prefix's longest-prefix
    // matching handles both coexisting correctly.
    let config = CodeGenConfig {
        extern_paths: vec![(
            ".google.protobuf.compiler".into(),
            "::compiler_protos".into(),
        )],
        ..Default::default()
    };
    let effective = effective_extern_paths(&[], &[], &config);
    // The sub-package mapping is preserved...
    assert!(effective
        .iter()
        .any(|(p, _)| p == ".google.protobuf.compiler"));
    // ...AND the WKT auto-mapping is still injected.
    assert!(
        effective.iter().any(|(p, _)| p == ".google.protobuf"),
        "WKT auto-mapping must coexist with sub-package extern_path"
    );
}

#[test]
fn test_wkt_auto_mapping_suppressed_by_exact_match() {
    let config = CodeGenConfig {
        extern_paths: vec![(".google.protobuf".into(), "::my_wkts".into())],
        ..Default::default()
    };
    let effective = effective_extern_paths(&[], &[], &config);
    // Exactly one .google.protobuf mapping (user's), not two.
    let count = effective
        .iter()
        .filter(|(p, _)| p == ".google.protobuf")
        .count();
    assert_eq!(count, 1);
    // It's the user's, not the auto-injection.
    assert!(effective
        .iter()
        .any(|(p, r)| p == ".google.protobuf" && r == "::my_wkts"));
}

// ── File-level extern paths (descriptor.proto → buffa-descriptor) ────────

#[test]
fn test_file_extern_paths_default_injection() {
    let config = CodeGenConfig::default();
    let file_paths = effective_file_extern_paths(&[], &config);
    assert_eq!(
        file_paths,
        vec![
            (
                "google/protobuf/descriptor.proto".to_string(),
                "::buffa_descriptor::generated::descriptor".to_string(),
            ),
            (
                "google/protobuf/compiler/plugin.proto".to_string(),
                "::buffa_descriptor::generated::compiler".to_string(),
            ),
        ],
    );
}

#[test]
fn test_file_extern_paths_suppressed_by_user_wkt_override() {
    // A user `.google.protobuf` override has covered descriptor types
    // since the package-level mapping was introduced. Auto-injecting a
    // higher-priority file-level mapping would silently redirect them
    // away from the user's crate — preserve the old behaviour.
    let config = CodeGenConfig {
        extern_paths: vec![(".google.protobuf".into(), "::my_wkts".into())],
        ..Default::default()
    };
    let file_paths = effective_file_extern_paths(&[], &config);
    assert!(
        file_paths.is_empty(),
        "user .google.protobuf override must suppress descriptor file-level mapping"
    );
}

#[test]
fn test_file_extern_paths_sub_package_override_suppresses_only_covered_file() {
    // A `.google.protobuf.compiler` sub-package override covers
    // `plugin.proto` (package `google.protobuf.compiler`) but NOT
    // `descriptor.proto` (package `google.protobuf`). The file-level
    // mapping must yield to the user override only for the file it
    // actually covers — the same per-package precedence the WKT
    // package-level mapping uses.
    let config = CodeGenConfig {
        extern_paths: vec![(
            ".google.protobuf.compiler".into(),
            "::compiler_protos".into(),
        )],
        ..Default::default()
    };
    let file_paths = effective_file_extern_paths(&[], &config);
    assert!(
        file_paths
            .iter()
            .any(|(f, _)| f == "google/protobuf/descriptor.proto"),
        "sub-package override for compiler must not suppress descriptor.proto file-level mapping"
    );
    assert!(
        !file_paths
            .iter()
            .any(|(f, _)| f == "google/protobuf/compiler/plugin.proto"),
        "sub-package override for compiler must suppress plugin.proto file-level mapping"
    );
}

#[test]
fn test_file_extern_paths_suppressed_when_generating_descriptor_proto() {
    // When building buffa-descriptor itself (descriptor.proto is in
    // files_to_generate), its types must resolve to the local module — the
    // file-level mapping is suppressed for that file. plugin.proto is
    // suppressed independently.
    let config = CodeGenConfig::default();
    let file_paths =
        effective_file_extern_paths(&["google/protobuf/descriptor.proto".to_string()], &config);
    assert!(
        !file_paths
            .iter()
            .any(|(f, _)| f == "google/protobuf/descriptor.proto"),
        "must not externalize descriptor.proto when generating it locally"
    );
    assert!(
        file_paths
            .iter()
            .any(|(f, _)| f == "google/protobuf/compiler/plugin.proto"),
        "plugin.proto suppression is independent of descriptor.proto"
    );
}

#[test]
fn test_descriptor_enum_field_resolves_to_buffa_descriptor() {
    // Regression: a proto referencing `google.protobuf.FieldDescriptorProto.Type`
    // (which `buf/validate/validate.proto` does) must resolve to
    // `::buffa_descriptor::generated::descriptor::field_descriptor_proto::Type`,
    // not `::buffa_types::google::protobuf::field_descriptor_proto::Type` —
    // the latter doesn't exist (`buffa-types` only ships the JSON-mappable
    // WKTs, not descriptor.proto types).
    //
    // The descriptor file is an *import* (in `files`, not `files_to_generate`)
    // — exactly how protoc surfaces it for any proto that
    // `import "google/protobuf/descriptor.proto"`.
    let mut descriptor_file = proto3_file("google/protobuf/descriptor.proto");
    descriptor_file.package = Some("google.protobuf".to_string());
    descriptor_file.message_type.push(DescriptorProto {
        name: Some("FieldDescriptorProto".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("Type".to_string()),
            value: vec![enum_value("TYPE_DOUBLE", 1)],
            ..Default::default()
        }],
        ..Default::default()
    });

    let mut user_file = proto3_file("my/uses_descriptor.proto");
    user_file.package = Some("my".to_string());
    user_file.dependency = vec!["google/protobuf/descriptor.proto".to_string()];
    user_file.message_type.push(DescriptorProto {
        name: Some("Wraps".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("field_type".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".google.protobuf.FieldDescriptorProto.Type".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[descriptor_file, user_file],
        &["my/uses_descriptor.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = joined(&files);
    assert!(
        content.contains("::buffa_descriptor::generated::descriptor::field_descriptor_proto::Type"),
        "descriptor enum field must resolve to buffa-descriptor: {content}"
    );
    assert!(
        !content.contains("::buffa_types::google::protobuf::field_descriptor_proto::Type"),
        "descriptor enum field must not resolve to buffa-types (does not exist): {content}"
    );
}

#[test]
fn test_user_wkt_override_still_covers_descriptor_types() {
    // Backward-compat: an explicit user `.google.protobuf` extern_path
    // covers `descriptor.proto` types too — the file-level descriptor
    // mapping must yield to it.
    let mut descriptor_file = proto3_file("google/protobuf/descriptor.proto");
    descriptor_file.package = Some("google.protobuf".to_string());
    descriptor_file.message_type.push(DescriptorProto {
        name: Some("FieldDescriptorProto".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("Type".to_string()),
            value: vec![enum_value("TYPE_DOUBLE", 1)],
            ..Default::default()
        }],
        ..Default::default()
    });

    let mut user_file = proto3_file("my/uses_descriptor.proto");
    user_file.package = Some("my".to_string());
    user_file.dependency = vec!["google/protobuf/descriptor.proto".to_string()];
    user_file.message_type.push(DescriptorProto {
        name: Some("Wraps".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("field_type".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".google.protobuf.FieldDescriptorProto.Type".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let config = CodeGenConfig {
        extern_paths: vec![(".google.protobuf".into(), "::my_wkts".into())],
        ..Default::default()
    };
    let files = generate(
        &[descriptor_file, user_file],
        &["my/uses_descriptor.proto".to_string()],
        &config,
    )
    .expect("should generate");
    let content = joined(&files);
    assert!(
        content.contains("::my_wkts::field_descriptor_proto::Type"),
        "user .google.protobuf override must cover descriptor types: {content}"
    );
    assert!(
        !content.contains("::buffa_descriptor"),
        "auto-injected descriptor mapping must yield to user override: {content}"
    );
}

/// Build the synthetic `compiler/plugin.proto` import + a user proto that
/// references `google.protobuf.compiler.CodeGeneratorRequest` as a field.
/// Shared by the plugin.proto routing tests below.
fn plugin_proto_fixture() -> (FileDescriptorProto, FileDescriptorProto) {
    let mut plugin_file = proto3_file("google/protobuf/compiler/plugin.proto");
    plugin_file.package = Some("google.protobuf.compiler".to_string());
    plugin_file.message_type.push(DescriptorProto {
        name: Some("CodeGeneratorRequest".to_string()),
        ..Default::default()
    });

    let mut user_file = proto3_file("my/uses_plugin.proto");
    user_file.package = Some("my".to_string());
    user_file.dependency = vec!["google/protobuf/compiler/plugin.proto".to_string()];
    user_file.message_type.push(DescriptorProto {
        name: Some("Wraps".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("req".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".google.protobuf.compiler.CodeGeneratorRequest".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    (plugin_file, user_file)
}

#[test]
fn test_plugin_proto_message_field_resolves_to_buffa_descriptor() {
    // `google.protobuf.compiler.*` is in a sub-package of `google.protobuf`,
    // so the package-level WKT mapping would route it to
    // `::buffa_types::google::protobuf::compiler::*` (which doesn't exist).
    // The file-level mapping must route it to
    // `::buffa_descriptor::generated::compiler::*` instead.
    let (plugin_file, user_file) = plugin_proto_fixture();
    let files = generate(
        &[plugin_file, user_file],
        &["my/uses_plugin.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = joined(&files);
    assert!(
        content.contains("::buffa_descriptor::generated::compiler::CodeGeneratorRequest"),
        "plugin.proto types must resolve to buffa-descriptor: {content}"
    );
    assert!(
        !content.contains("::buffa_types::google::protobuf::compiler"),
        "plugin.proto types must not resolve to buffa-types (does not exist): {content}"
    );
}

#[test]
fn test_user_compiler_sub_package_override_still_covers_plugin_types() {
    // Backward-compat: a user `.google.protobuf.compiler` extern_path covers
    // `plugin.proto` types — the file-level mapping must yield to it. This
    // is the per-file analogue of the `.google.protobuf` override test
    // above: a sub-package mapping suppresses only the file it covers.
    let (plugin_file, user_file) = plugin_proto_fixture();
    let config = CodeGenConfig {
        extern_paths: vec![(".google.protobuf.compiler".into(), "::my_compiler".into())],
        ..Default::default()
    };
    let files = generate(
        &[plugin_file, user_file],
        &["my/uses_plugin.proto".to_string()],
        &config,
    )
    .expect("should generate");
    let content = joined(&files);
    assert!(
        content.contains("::my_compiler::CodeGeneratorRequest"),
        "user .google.protobuf.compiler override must cover plugin types: {content}"
    );
    assert!(
        !content.contains("::buffa_descriptor"),
        "auto-injected plugin mapping must yield to user override: {content}"
    );
}

#[test]
fn test_empty_file() {
    let file = proto3_file("empty.proto");
    let result = generate(
        &[file],
        &["empty.proto".to_string()],
        &CodeGenConfig::default(),
    );
    let files = result.expect("empty file should generate without error");
    // No content files (all empty) + 1 .mod.rs.
    assert_eq!(files.len(), 1);
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert_eq!(stitcher.name, "__buffa.mod.rs");
    assert!(
        stitcher.content.contains("@generated"),
        "missing header comment"
    );
    // No content of any kind → no `__buffa` wrapper at all.
    assert!(
        !stitcher.content.contains("pub mod __buffa"),
        "empty file should not emit a `pub mod __buffa` wrapper:\n{}",
        stitcher.content
    );
}

#[test]
fn test_empty_message_omits_empty_ancillary_files() {
    // Regression: a proto file with only an empty message should not emit
    // empty `.__oneof.rs`, `.__view_oneof.rs`, or `.__ext.rs` companion files.
    let mut file = proto3_file("example/v1/empty.proto");
    file.package = Some("example.v1".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Empty".to_string()),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["example/v1/empty.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("empty-message file should generate");

    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();

    assert!(
        files
            .iter()
            .any(|f| f.name == "example.v1.empty.rs" && f.kind == GeneratedFileKind::Owned),
        "owned file with the empty message should still be emitted; got {names:?}"
    );
    assert!(
        files
            .iter()
            .any(|f| f.kind == GeneratedFileKind::PackageMod),
        "package mod should be generated; got {names:?}"
    );

    assert!(
        !files.iter().any(|f| f.name.ends_with(".__oneof.rs")),
        "empty oneof companion file should not be emitted; got {names:?}"
    );
    assert!(
        !files.iter().any(|f| f.name.ends_with(".__view_oneof.rs")),
        "empty view-oneof companion file should not be emitted; got {names:?}"
    );
    assert!(
        !files.iter().any(|f| f.name.ends_with(".__ext.rs")),
        "empty extension companion file should not be emitted; got {names:?}"
    );

    let package_mod = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("package mod should be generated");
    assert!(
        !package_mod.content.contains("example.v1.empty.__oneof.rs"),
        "package mod should not include omitted oneof companion:\n{}",
        package_mod.content
    );
    assert!(
        !package_mod
            .content
            .contains("example.v1.empty.__view_oneof.rs"),
        "package mod should not include omitted view-oneof companion:\n{}",
        package_mod.content
    );
    assert!(
        !package_mod.content.contains("example.v1.empty.__ext.rs"),
        "package mod should not include omitted ext companion:\n{}",
        package_mod.content
    );
}

#[test]
fn stitcher_omits_empty_inner_modules_for_empty_message() {
    // A proto containing only an empty message (default config: views on,
    // no JSON, no register_fn-relevant content) should produce only
    // `pub mod view { … }` inside `__buffa` — no inner `view::oneof`,
    // no `oneof`, no `ext`.
    let mut file = proto3_file("only_msg.proto");
    file.package = Some("p".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Empty".to_string()),
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["only_msg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert!(
        stitcher.content.contains("pub mod __buffa"),
        "expected `__buffa` wrapper (view module is non-empty):\n{}",
        stitcher.content
    );
    assert!(
        stitcher.content.contains("pub mod view"),
        "expected `pub mod view`:\n{}",
        stitcher.content
    );
    assert!(
        !stitcher.content.contains("pub mod oneof"),
        "should not emit empty `pub mod oneof` (covers both top-level \
         and nested view::oneof):\n{}",
        stitcher.content
    );
    assert!(
        !stitcher.content.contains("pub mod ext"),
        "should not emit empty `pub mod ext`:\n{}",
        stitcher.content
    );
}

#[test]
fn stitcher_omits_view_module_when_views_disabled_and_only_oneof_present() {
    // Views off + a message with a oneof: only `pub mod oneof` survives;
    // `view` and `ext` are omitted.
    let mut file = proto3_file("only_oneof.proto");
    file.package = Some("p".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("M".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("x".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("y".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let config = CodeGenConfig {
        generate_views: false,
        ..Default::default()
    };
    let files =
        generate(&[file], &["only_oneof.proto".to_string()], &config).expect("should generate");
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert!(
        stitcher.content.contains("pub mod __buffa"),
        "expected `__buffa` wrapper (oneof module is non-empty):\n{}",
        stitcher.content
    );
    assert!(
        stitcher.content.contains("pub mod oneof"),
        "expected `pub mod oneof`:\n{}",
        stitcher.content
    );
    assert!(
        !stitcher.content.contains("pub mod view"),
        "should not emit `pub mod view` when views are disabled:\n{}",
        stitcher.content
    );
    assert!(
        !stitcher.content.contains("pub mod ext"),
        "should not emit empty `pub mod ext`:\n{}",
        stitcher.content
    );
}

#[test]
fn stitcher_emits_only_ext_module_for_extension_only_proto() {
    // A proto carrying only a file-level `extend` block (no own
    // messages) emits only `pub mod ext` inside `__buffa`.
    let mut file = proto3_file("ext_only.proto");
    file.package = Some("p".to_string());
    file.message_type = vec![DescriptorProto {
        name: Some("Target".to_string()),
        extension_range: vec![
            crate::generated::descriptor::descriptor_proto::ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    file.extension = vec![{
        let mut f = make_field("my_opt", 100, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
        f.extendee = Some(".p.Target".to_string());
        f
    }];
    let files = generate(
        &[file],
        &["ext_only.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert!(
        stitcher.content.contains("pub mod __buffa"),
        "expected `__buffa` wrapper (ext module is non-empty):\n{}",
        stitcher.content
    );
    assert!(
        stitcher.content.contains("pub mod ext"),
        "expected `pub mod ext`:\n{}",
        stitcher.content
    );
    // `Target` has an extension range but no oneofs and no fields, so
    // no view oneof / oneof content. View module exists for `TargetView`.
    assert!(
        stitcher.content.contains("pub mod view"),
        "expected `pub mod view` for `TargetView`:\n{}",
        stitcher.content
    );
    assert!(
        !stitcher.content.contains("pub mod oneof"),
        "should not emit empty `pub mod oneof`:\n{}",
        stitcher.content
    );
}

#[test]
fn test_package_to_mod_filename() {
    assert_eq!(
        package_to_mod_filename("google.protobuf"),
        "google.protobuf.mod.rs"
    );
    assert_eq!(package_to_mod_filename("foo"), "foo.mod.rs");
    assert_eq!(package_to_mod_filename(""), "__buffa.mod.rs");
    assert_eq!(
        proto_path_to_stem("google/protobuf/timestamp.proto"),
        "google.protobuf.timestamp"
    );
}

#[test]
fn test_multi_file_same_package_merged() {
    // Two `.proto` files in the same package → one stitcher.
    let mut a = proto3_file("a.proto");
    a.package = Some("shared.pkg".to_string());
    a.message_type.push(DescriptorProto {
        name: Some("A".to_string()),
        ..Default::default()
    });
    let mut b = proto3_file("b.proto");
    b.package = Some("shared.pkg".to_string());
    b.message_type.push(DescriptorProto {
        name: Some("B".to_string()),
        ..Default::default()
    });
    let files = generate(
        &[a, b],
        &["a.proto".to_string(), "b.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("multi-file package should merge");
    // 2 protos × 2 content kinds (owned + view; oneof / view_oneof /
    // ext omitted because the messages have no oneofs and no extensions)
    // + 1 stitcher = 5.
    assert_eq!(files.len(), 5);
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert_eq!(stitcher.name, "shared.pkg.mod.rs");
    let content = &joined(&files);
    assert!(content.contains("pub struct A"));
    assert!(content.contains("pub struct B"));
    // Exactly one `pub mod __buffa` (in the stitcher), and both content
    // files referenced from inside it.
    assert_eq!(stitcher.content.matches("pub mod __buffa {").count(), 1);
    assert!(stitcher.content.contains(r#"include!("a.__view.rs");"#));
    assert!(stitcher.content.contains(r#"include!("b.__view.rs");"#));
}

#[test]
fn test_package_to_filename() {
    assert_eq!(package_to_filename("google.protobuf"), "google.protobuf.rs");
    assert_eq!(package_to_filename("foo"), "foo.rs");
    assert_eq!(package_to_filename(""), "__buffa.rs");
}

/// Two `.proto` files in `shared.pkg`. `a.proto` has a message with an
/// explicit oneof and a nested type so the `__buffa::{oneof,view::oneof}`
/// modules and per-message child modules are non-empty — needed for the
/// module-structure parity assertions below.
fn shared_pkg_fixture() -> ([FileDescriptorProto; 2], [String; 2]) {
    let mut a = proto3_file("a.proto");
    a.package = Some("shared.pkg".to_string());
    a.message_type.push(DescriptorProto {
        name: Some("A".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("x".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("y".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let mut b = proto3_file("b.proto");
    b.package = Some("shared.pkg".to_string());
    b.message_type.push(DescriptorProto {
        name: Some("B".to_string()),
        ..Default::default()
    });
    ([a, b], ["a.proto".to_string(), "b.proto".to_string()])
}

#[test]
fn test_file_per_package_multi_file() {
    let (descs, names) = shared_pkg_fixture();
    let config = CodeGenConfig {
        file_per_package: true,
        ..Default::default()
    };
    let files = generate(&descs, &names, &config).expect("file_per_package should generate");
    // Exactly one output file for the package — no per-proto content files.
    assert_eq!(files.len(), 1, "expected single per-package file");
    let pkg = &files[0];
    assert_eq!(pkg.name, "shared.pkg.rs");
    assert_eq!(pkg.kind, GeneratedFileKind::PackageMod);
    // Both messages inlined; no `include!` calls.
    assert!(pkg.content.contains("pub struct A"));
    assert!(pkg.content.contains("pub struct B"));
    assert!(
        !pkg.content.contains("include!"),
        "per-package file must inline content, not include! per-file outputs"
    );
    // Same `__buffa` module wrappers as the per-file stitcher.
    // The fixture has views and a oneof but no extensions, so `view`
    // and `oneof` are emitted but `ext` is omitted as empty.
    assert_eq!(pkg.content.matches("pub mod __buffa {").count(), 1);
    assert!(pkg.content.contains("pub mod view {"));
    assert!(pkg.content.contains("pub mod oneof {"));
    assert!(
        !pkg.content.contains("pub mod ext {"),
        "no extensions in fixture → empty `pub mod ext` should be omitted"
    );
}

#[test]
fn test_file_per_package_module_structure_matches_stitcher() {
    // The single-file output's module structure must be identical to what
    // the per-file stitcher produces after `include!` resolution, so
    // consumers see the same API regardless of mode.
    let (descs, names) = shared_pkg_fixture();
    let per_file = generate(&descs, &names, &CodeGenConfig::default()).unwrap();
    let stitcher = per_file
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .unwrap();
    let per_package = generate(
        &descs,
        &names,
        &CodeGenConfig {
            file_per_package: true,
            ..Default::default()
        },
    )
    .unwrap();
    let pkg = &per_package[0];

    // Splice each `include!("X.rs");` in the stitcher with the matching
    // content file's body — modeling what rustc sees after expansion.
    // Drop the `// @generated …` / `// source: …` header so spliced
    // content doesn't introduce comment lines mid-module.
    fn strip_header(s: &str) -> &str {
        s.find("\n\n").map_or(s, |i| &s[i + 2..])
    }
    let mut spliced = stitcher.content.clone();
    for f in per_file
        .iter()
        .filter(|f| f.kind != GeneratedFileKind::PackageMod)
    {
        let needle = format!(r#"include!("{}");"#, f.name);
        spliced = spliced.replace(&needle, strip_header(&f.content));
    }
    assert!(
        !spliced.contains("include!"),
        "splice missed an include: {spliced}"
    );

    // Compare the depth-aware sequence of `pub mod` declarations. Depth
    // is brace-tracked (not indent-tracked) so the spliced content —
    // which is not re-indented — is measured correctly.
    let mod_decls = |s: &str| -> Vec<(usize, String)> {
        let mut depth = 0usize;
        let mut out = Vec::new();
        for l in s.lines() {
            let trimmed = l.trim_start();
            if let Some(rest) = trimmed.strip_prefix("pub mod ") {
                out.push((depth, rest.trim_end_matches(" {").to_string()));
            }
            depth += l.matches('{').count();
            depth = depth.saturating_sub(l.matches('}').count());
        }
        out
    };
    let spliced_mods = mod_decls(&spliced);
    let pkg_mods = mod_decls(&pkg.content);
    // Non-vacuous: fixture has a oneof and a nested type, so the
    // `__buffa::oneof::a`, `__buffa::view::oneof::a`, and per-message
    // `a` child modules are present in addition to the wrapper modules.
    assert!(
        spliced_mods.len() > 5,
        "fixture should produce >5 pub mod decls, got {}: {spliced_mods:?}",
        spliced_mods.len()
    );
    assert_eq!(spliced_mods, pkg_mods);
}

#[test]
fn test_file_per_package_register_types_with_text() {
    // `register_types` paths are package-root-relative (`super::…`) and
    // must resolve identically when content is inlined vs `include!`d.
    let (descs, names) = shared_pkg_fixture();
    let config = CodeGenConfig {
        file_per_package: true,
        generate_text: true,
        ..Default::default()
    };
    let files = generate(&descs, &names, &config).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    assert!(
        content.contains("pub fn register_types("),
        "register_types fn missing: {content}"
    );
    assert!(
        content.contains("reg.register_text_any(super::__A_TEXT_ANY)"),
        "A text-any path: {content}"
    );
    assert!(
        content.contains("reg.register_text_any(super::a::__INNER_TEXT_ANY)"),
        "nested Inner text-any path: {content}"
    );
    assert!(
        content.contains("reg.register_text_any(super::__B_TEXT_ANY)"),
        "B text-any path: {content}"
    );
}

#[test]
fn test_file_per_package_unnamed_package() {
    let file = proto3_file("noname.proto");
    let config = CodeGenConfig {
        file_per_package: true,
        ..Default::default()
    };
    let files = generate(&[file], &["noname.proto".to_string()], &config)
        .expect("unnamed package should generate");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "__buffa.rs");
    // No messages → no view/oneof/ext content → no `__buffa` wrapper.
    assert!(
        !files[0].content.contains("pub mod __buffa"),
        "empty package should not emit `__buffa` wrapper:\n{}",
        files[0].content
    );
}

#[test]
fn test_file_per_package_multiple_packages() {
    // Each package gets exactly one file.
    let mut a = proto3_file("a.proto");
    a.package = Some("alpha".to_string());
    let mut b = proto3_file("b.proto");
    b.package = Some("beta".to_string());
    let config = CodeGenConfig {
        file_per_package: true,
        ..Default::default()
    };
    let files = generate(
        &[a, b],
        &["a.proto".to_string(), "b.proto".to_string()],
        &config,
    )
    .expect("multi-package should generate");
    assert_eq!(files.len(), 2);
    let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, &["alpha.rs", "beta.rs"]);
}

#[test]
fn test_child_package_named_view_no_collision() {
    // Regression: under the pre-sentinel design, `package foo.view`
    // emitted `pub mod view { ... }` (child package) as a sibling of the
    // kind-tree `pub mod view { ... }` inside `foo` — E0428. With the
    // sentinel wrapper, the kind tree is `pub mod __buffa { pub mod view
    // { ... } }`, so a child package literally named `view` is fine.
    let mut a = proto3_file("a.proto");
    a.package = Some("foo".to_string());
    a.message_type.push(DescriptorProto {
        name: Some("A".to_string()),
        ..Default::default()
    });
    let mut b = proto3_file("b.proto");
    b.package = Some("foo.view".to_string());
    b.message_type.push(DescriptorProto {
        name: Some("B".to_string()),
        ..Default::default()
    });
    let files = generate(
        &[a, b],
        &["a.proto".to_string(), "b.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("`package foo.view` alongside `package foo` must compile");
    // One stitcher per package.
    let stitchers: Vec<_> = files
        .iter()
        .filter(|f| f.kind == GeneratedFileKind::PackageMod)
        .collect();
    assert_eq!(stitchers.len(), 2);
    // Neither stitcher emits a bare `pub mod view {` at top level — the
    // view kind tree is always nested under `pub mod __buffa {`.
    for s in &stitchers {
        let top_level_view = s
            .content
            .lines()
            .any(|l| l.starts_with("pub mod view {") || l == "pub mod view {");
        assert!(
            !top_level_view,
            "stitcher {} must not emit top-level `pub mod view`: {}",
            s.name, s.content
        );
        assert!(s.content.contains("pub mod __buffa {"));
    }
    // The package tree assembled by `generate_module_tree` puts `view` as
    // a child of `foo`, separate from `foo`'s `__buffa` wrapper.
    let entries: Vec<_> = stitchers
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.name.trim_end_matches(".mod.rs").to_string(),
            )
        })
        .collect();
    let tree = crate::generate_module_tree(&entries, crate::IncludeMode::Relative(""), false);
    assert!(
        tree.contains("pub mod foo {") && tree.contains("pub mod view {"),
        "tree must nest `view` under `foo`: {tree}"
    );
}

#[test]
fn test_simple_enum() {
    let mut file = proto3_file("status.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![
            enum_value("UNKNOWN", 0),
            enum_value("ACTIVE", 1),
            enum_value("INACTIVE", 2),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["status.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("simple enum should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub enum Status"),
        "missing enum: {content}"
    );
    assert!(
        content.contains("UNKNOWN = 0"),
        "missing UNKNOWN: {content}"
    );
    assert!(content.contains("ACTIVE = 1"), "missing ACTIVE: {content}");
    assert!(
        content.contains("INACTIVE = 2"),
        "missing INACTIVE: {content}"
    );
    assert!(
        content.contains("impl ::buffa::Enumeration for Status"),
        "missing Enumeration impl: {content}"
    );
    assert!(
        content.contains("impl ::core::default::Default for Status"),
        "missing Default impl: {content}"
    );
}

#[test]
fn test_enum_with_alias() {
    let mut file = proto3_file("code.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Code".to_string()),
        value: vec![
            enum_value("OK", 0),
            enum_value("SUCCESS", 0), // alias for OK
            enum_value("ERROR", 1),
        ],
        options: (crate::generated::descriptor::EnumOptions {
            allow_alias: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["code.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("aliased enum should generate");
    let content = &joined(&files);
    // OK is a primary variant; SUCCESS is a const alias.
    assert!(content.contains("OK = 0"), "missing primary: {content}");
    assert!(
        content.contains("pub const SUCCESS"),
        "alias not emitted as const: {content}"
    );
    assert!(
        !content.contains("SUCCESS = 0"),
        "alias must not be a variant: {content}"
    );
}

#[test]
fn test_enum_values_emits_static_slice_in_declaration_order() {
    let mut file = proto3_file("status.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![
            enum_value("UNKNOWN", 0),
            enum_value("ACTIVE", 1),
            enum_value("INACTIVE", 2),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["status.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("enum should generate");
    let content = &joined(&files);
    // Generated impl includes a `values()` returning a static slice in
    // proto declaration order. The generated body collapses to a single
    // line so we match against that exact form.
    assert!(
        content.contains("&[Self::UNKNOWN, Self::ACTIVE, Self::INACTIVE]"),
        "missing declaration-order values() slice: {content}"
    );
}

#[test]
fn test_enum_values_skips_aliases() {
    let mut file = proto3_file("code.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Code".to_string()),
        value: vec![
            enum_value("OK", 0),
            enum_value("SUCCESS", 0), // alias for OK — not its own variant
            enum_value("ERROR", 1),
        ],
        options: (crate::generated::descriptor::EnumOptions {
            allow_alias: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["code.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("aliased enum should generate");
    let content = &joined(&files);
    // values() lists only primary variants — aliases are `pub const` items,
    // not enum variants, so they don't belong in the slice.
    assert!(
        content.contains("&[Self::OK, Self::ERROR]"),
        "values() should mirror primary variants only: {content}"
    );
    assert!(
        !content.contains("Self::SUCCESS"),
        "alias `SUCCESS` must not appear in values(): {content}"
    );
}

#[test]
fn test_file_not_found_error() {
    let file = proto3_file("other.proto");
    let result = generate(
        &[file],
        &["missing.proto".to_string()],
        &CodeGenConfig::default(),
    );
    assert!(
        matches!(result, Err(CodeGenError::FileNotFound(_))),
        "expected FileNotFound error"
    );
}

#[test]
fn test_type_url_top_level_with_package() {
    let mut file = proto3_file("person.proto");
    file.package = Some("my.company".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Person".to_string()),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["person.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/my.company.Person""#),
        "wrong or missing TYPE_URL: {content}"
    );
}

#[test]
fn test_type_url_top_level_no_package() {
    let mut file = proto3_file("root.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Root".to_string()),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["root.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/Root""#),
        "wrong or missing TYPE_URL for no-package message: {content}"
    );
}

#[test]
fn test_type_url_nested_message() {
    let mut file = proto3_file("nested_type_url.proto");
    file.package = Some("acme".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["nested_type_url.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/acme.Outer""#),
        "wrong Outer TYPE_URL: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/acme.Outer.Inner""#),
        "wrong Inner TYPE_URL: {content}"
    );
}

#[test]
fn test_type_url_nested_no_package() {
    // Empty package + nested message: FQN should be "Outer.Inner", no leading dot.
    let mut file = proto3_file("nested_nopackage.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["nested_nopackage.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/Outer""#),
        "wrong Outer TYPE_URL: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/Outer.Inner""#),
        "wrong Inner TYPE_URL (no package): {content}"
    );
}

#[test]
fn test_type_url_doubly_nested() {
    // Three levels: pkg.Outer.Middle.Inner — verifies recursive FQN propagation.
    let mut file = proto3_file("doubly_nested.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Middle".to_string()),
            nested_type: vec![DescriptorProto {
                name: Some("Inner".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["doubly_nested.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/pkg.Outer""#),
        "wrong Outer TYPE_URL: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/pkg.Outer.Middle""#),
        "wrong Middle TYPE_URL: {content}"
    );
    assert!(
        content
            .contains(r#"TYPE_URL: &'static str = "type.googleapis.com/pkg.Outer.Middle.Inner""#),
        "wrong Inner TYPE_URL: {content}"
    );
}

#[test]
fn test_message_name_consts() {
    // The four `MessageName` consts must hold the documented invariant
    // `PACKAGE + "." + NAME == FULL_NAME` (joining dot omitted when
    // `PACKAGE` is empty), and `TYPE_URL == "type.googleapis.com/" +
    // FULL_NAME`. The atomic-prefix-strip in `message_name_impl` makes
    // a partial-match (`package = "foo"` against `proto_fqn =
    // "food.Bar"`) impossible to slip through silently — pin the shape
    // here so a refactor that re-introduces a two-step strip fails this
    // test instead of shipping a broken `NAME`.
    let mut file = proto3_file("named.proto");
    file.package = Some("my.pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[file],
        &["named.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = joined(&files);
    // Top-level: PACKAGE + "." + NAME == FULL_NAME.
    for snippet in [
        r#"const PACKAGE: &'static str = "my.pkg""#,
        r#"const NAME: &'static str = "Outer""#,
        r#"const FULL_NAME: &'static str = "my.pkg.Outer""#,
        r#"const TYPE_URL: &'static str = "type.googleapis.com/my.pkg.Outer""#,
        // Nested: NAME carries the dotted nesting path; PACKAGE stays
        // at the proto package — NOT `DescriptorProto.name` (which is
        // just `"Inner"`).
        r#"const NAME: &'static str = "Outer.Inner""#,
        r#"const FULL_NAME: &'static str = "my.pkg.Outer.Inner""#,
    ] {
        assert!(content.contains(snippet), "missing `{snippet}`: {content}");
    }

    // Empty package: PACKAGE is "", NAME == FULL_NAME, no joining dot.
    let mut root = proto3_file("root.proto");
    root.message_type.push(DescriptorProto {
        name: Some("Root".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Leaf".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let files = generate(
        &[root],
        &["root.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = joined(&files);
    for snippet in [
        r#"const PACKAGE: &'static str = """#,
        r#"const NAME: &'static str = "Root""#,
        r#"const FULL_NAME: &'static str = "Root""#,
        r#"const NAME: &'static str = "Root.Leaf""#,
        r#"const FULL_NAME: &'static str = "Root.Leaf""#,
        r#"const TYPE_URL: &'static str = "type.googleapis.com/Root.Leaf""#,
    ] {
        assert!(content.contains(snippet), "missing `{snippet}`: {content}");
    }
}

#[test]
fn test_message_scalar_fields() {
    let mut file = proto3_file("scalars.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Scalars".to_string()),
        field: vec![
            make_field("count", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
            make_field("active", 2, Label::LABEL_OPTIONAL, Type::TYPE_BOOL),
            make_field("score", 3, Label::LABEL_OPTIONAL, Type::TYPE_DOUBLE),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["scalars.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("scalar fields message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Scalars"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub count: i32"),
        "missing count field: {content}"
    );
    assert!(
        content.contains("pub active: bool"),
        "missing active field: {content}"
    );
    assert!(
        content.contains("pub score: f64"),
        "missing score field: {content}"
    );
    assert!(
        content.contains("::buffa::impl_default_instance!(Scalars);"),
        "missing DefaultInstance impl: {content}"
    );
    assert!(
        content.contains("impl ::buffa::Message for Scalars"),
        "missing Message impl: {content}"
    );
    assert!(
        content.contains("fn compute_size"),
        "missing compute_size: {content}"
    );
    assert!(
        content.contains("fn merge_field"),
        "missing merge_field: {content}"
    );
}

#[test]
fn test_message_nested_message_field() {
    let mut file = proto3_file("nested.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("inner".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["nested.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("nested message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Outer"),
        "missing Outer: {content}"
    );
    assert!(
        content.contains("pub inner: ::buffa::MessageField<Inner, ::buffa::Inline<Inner>>"),
        "missing MessageField: {content}"
    );
    // impl Message should use the two-pass size computation for sub-messages.
    assert!(
        content.contains("compute_size"),
        "missing compute_size call for sub-message: {content}"
    );
    assert!(
        content.contains("merge_length_delimited"),
        "missing merge_length_delimited for sub-message: {content}"
    );
    assert!(
        content.contains("get_or_insert_default"),
        "missing get_or_insert_default in merge: {content}"
    );
}

#[test]
fn test_message_map_field() {
    let mut file = proto3_file("withmap.proto");
    // Synthetic map entry: key=string, value=int32
    let map_entry = DescriptorProto {
        name: Some("AttrsEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        ],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    file.message_type.push(DescriptorProto {
        name: Some("WithMap".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("attrs".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".WithMap.AttrsEntry".to_string()),
            ..Default::default()
        }],
        nested_type: vec![map_entry],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["withmap.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("map field message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct WithMap"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub attrs:"),
        "missing attrs field: {content}"
    );
    assert!(
        content.contains("::buffa::__private::HashMap"),
        "map field must use ::buffa::__private::HashMap, got: {content}"
    );
}

#[test]
fn test_message_oneof() {
    let mut file = proto3_file("oneof.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOneof".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("count".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("name".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["oneof.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("oneof message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct WithOneof"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub kind:"),
        "missing oneof field: {content}"
    );
    assert!(
        content.contains("pub mod with_oneof"),
        "missing message module: {content}"
    );
    assert!(
        content.contains("pub enum Kind"),
        "missing oneof enum: {content}"
    );
    assert!(
        content.contains("Count(i32)"),
        "missing Count variant: {content}"
    );
    assert!(
        content.contains("impl ::buffa::Oneof for Kind"),
        "missing Oneof impl: {content}"
    );
}

#[test]
fn test_message_proto3_optional() {
    let mut file = proto3_file("proto3opt.proto");
    // Proto3 optional fields are assigned to a synthetic oneof.
    file.message_type.push(DescriptorProto {
        name: Some("WithOptional".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("count".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_INT32),
            oneof_index: Some(0),
            proto3_optional: Some(true),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_count".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["proto3opt.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct WithOptional"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub count: ::core::option::Option<i32>"),
        "missing optional field: {content}"
    );
    // impl Message should use if-let pattern for optional
    assert!(
        content.contains("if let Some"),
        "missing if-let in impl: {content}"
    );
    assert!(
        content.contains("Option::Some"),
        "missing Some assignment in merge: {content}"
    );
}

#[test]
fn test_message_proto3_optional_string() {
    let mut file = proto3_file("optstr.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOptStr".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("label".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_STRING),
            oneof_index: Some(0),
            proto3_optional: Some(true),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_label".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["optstr.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional string should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub label: ::core::option::Option<::buffa::alloc::string::String>"),
        "missing optional string field: {content}"
    );
    assert!(
        content.contains("put_string_field"),
        "missing put_string_field in write_to: {content}"
    );
    assert!(
        content.contains("merge_string"),
        "missing merge_string in merge: {content}"
    );
}

#[test]
fn test_message_proto3_optional_enum() {
    let mut file = proto3_file("optenu.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("WithOptEnum".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("color".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Color".to_string()),
            oneof_index: Some(0),
            proto3_optional: Some(true),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_color".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["optenu.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional enum should generate");
    let content = &joined(&files);
    // Enum optional must resolve to EnumValue<Color>, not ()
    assert!(
        content.contains("Option<::buffa::EnumValue<Color>>"),
        "wrong type for optional enum: {content}"
    );
    assert!(
        content.contains("EnumValue::from"),
        "missing EnumValue::from in merge: {content}"
    );
}

#[test]
fn test_message_proto3_optional_bytes_and_bool() {
    let mut file = proto3_file("optmisc.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOptMisc".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("data".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_BYTES),
                oneof_index: Some(0),
                proto3_optional: Some(true),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("flag".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_BOOL),
                oneof_index: Some(1),
                proto3_optional: Some(true),
                ..Default::default()
            },
        ],
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("_data".to_string()),
                ..Default::default()
            },
            OneofDescriptorProto {
                name: Some("_flag".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["optmisc.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional bytes/bool should generate");
    let content = &joined(&files);
    assert!(
        content.contains("Option<::buffa::alloc::vec::Vec<u8>>"),
        "missing optional bytes field: {content}"
    );
    assert!(
        content.contains("Option<bool>"),
        "missing optional bool field: {content}"
    );
    // Bool is fixed-size: compute_size should use is_some(), not if-let
    assert!(
        content.contains("is_some()"),
        "fixed-size optional should use is_some(): {content}"
    );
    // Bytes uses encode_bytes
    assert!(
        content.contains("put_bytes_field"),
        "missing put_bytes_field for optional bytes: {content}"
    );
}

#[test]
fn test_message_string_and_bytes_fields() {
    let mut file = proto3_file("strings.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithStrings".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("data", 2, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["strings.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("string/bytes message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub name: ::buffa::alloc::string::String"),
        "missing string field: {content}"
    );
    assert!(
        content.contains("pub data: ::buffa::alloc::vec::Vec<u8>"),
        "missing bytes field: {content}"
    );
    // impl Message should encode/decode these fields
    assert!(
        content.contains("put_string_field"),
        "missing put_string_field: {content}"
    );
    assert!(
        content.contains("merge_string"),
        "missing merge_string: {content}"
    );
    assert!(
        content.contains("string_encoded_len"),
        "missing string_encoded_len: {content}"
    );
    assert!(
        content.contains("put_bytes_field"),
        "missing put_bytes_field: {content}"
    );
    assert!(
        content.contains("merge_bytes"),
        "missing merge_bytes: {content}"
    );
    assert!(
        content.contains("bytes_encoded_len"),
        "missing bytes_encoded_len: {content}"
    );
}

#[test]
fn test_message_enum_field() {
    let mut file = proto3_file("enumfield.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![enum_value("UNKNOWN", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("WithEnum".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("status".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Status".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["enumfield.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("enum field message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub status: ::buffa::EnumValue<Status>"),
        "missing enum field: {content}"
    );
    // impl Message should encode via to_i32() and decode via EnumValue::from
    assert!(
        content.contains("to_i32()"),
        "missing to_i32 in generated code: {content}"
    );
    assert!(
        content.contains("int32_encoded_len"),
        "missing int32_encoded_len in compute_size: {content}"
    );
    assert!(
        content.contains("EnumValue::from"),
        "missing EnumValue::from in generated code: {content}"
    );
}

#[test]
fn test_repeated_packed_scalar() {
    let mut file = proto3_file("repeatedscalar.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithRepeated".to_string()),
        field: vec![
            make_field("ids", 1, Label::LABEL_REPEATED, Type::TYPE_INT32),
            make_field("scores", 2, Label::LABEL_REPEATED, Type::TYPE_DOUBLE),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedscalar.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated scalar message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub ids: ::buffa::alloc::vec::Vec<i32>"),
        "missing ids field: {content}"
    );
    assert!(
        content.contains("pub scores: ::buffa::alloc::vec::Vec<f64>"),
        "missing scores field: {content}"
    );
    // Packed encoding: payload written as a single LengthDelimited blob.
    assert!(
        content.contains("is_empty()"),
        "packed repeated should check is_empty: {content}"
    );
    assert!(
        content.contains("int32_encoded_len"),
        "missing int32_encoded_len in payload size: {content}"
    );
    assert!(
        content.contains("encode_int32"),
        "missing encode_int32 in write_to: {content}"
    );
    assert!(
        content.contains("decode_int32"),
        "missing decode_int32 in merge: {content}"
    );
    // Merge must accept both packed and unpacked.
    assert!(
        content.contains("WireType::LengthDelimited"),
        "missing packed merge branch: {content}"
    );
}

#[test]
fn test_repeated_unpacked_string() {
    let mut file = proto3_file("repeatedstr.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithRepeatedStr".to_string()),
        field: vec![make_field(
            "tags",
            1,
            Label::LABEL_REPEATED,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedstr.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated string message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub tags: ::buffa::alloc::vec::Vec<::buffa::alloc::string::String>"),
        "missing tags field: {content}"
    );
    // Unpacked: each element has its own tag (for loop, no payload length).
    assert!(
        content.contains("string_encoded_len"),
        "missing string_encoded_len: {content}"
    );
    assert!(
        content.contains("put_string_field"),
        "missing put_string_field: {content}"
    );
    assert!(
        content.contains("decode_string"),
        "missing decode_string: {content}"
    );
}

#[test]
fn test_repeated_message_field() {
    let mut file = proto3_file("repeatedmsg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Item".to_string()),
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Container".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("items".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".Item".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedmsg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub items: ::buffa::alloc::vec::Vec<Item>"),
        "missing items field: {content}"
    );
    // Uses two-pass size model for each element.
    assert!(
        content.contains("merge_length_delimited"),
        "missing merge_length_delimited for repeated msg: {content}"
    );
    assert!(
        content.contains("__cache.consume_next()"),
        "missing SizeCache consume in write_to: {content}"
    );
}

#[test]
fn test_repeated_enum_field() {
    let mut file = proto3_file("repeatedenu.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![enum_value("UNKNOWN", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("WithRepeatedEnum".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("statuses".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Status".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedenu.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated enum should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub statuses: ::buffa::alloc::vec::Vec<::buffa::EnumValue<Status>>"),
        "missing statuses field: {content}"
    );
    // Packed enum encoding uses to_i32() and EnumValue::from.
    assert!(
        content.contains("to_i32()"),
        "missing to_i32 for packed enum write: {content}"
    );
    assert!(
        content.contains("EnumValue::from"),
        "missing EnumValue::from in packed decode: {content}"
    );
}

#[test]
fn extension_set_impl_on_generated_options() {
    // Smoke test: the bootstrap-generated FieldOptions implements
    // ExtensionSet, and extension get/set roundtrips through its
    // __buffa_unknown_fields storage.
    use crate::generated::descriptor::FieldOptions;
    use buffa::extension::codecs::{Int32, StringCodec};
    use buffa::{Extension, ExtensionSet};

    // Use the bootstrap-generated PROTO_FQN so the extendee check passes.
    const WEIGHT: Extension<Int32> = Extension::new(50001, FieldOptions::PROTO_FQN);
    const TAG: Extension<StringCodec> = Extension::new(50002, FieldOptions::PROTO_FQN);

    let mut opts = FieldOptions::default();
    assert!(!opts.has_extension(&WEIGHT));

    opts.set_extension(&WEIGHT, -7);
    opts.set_extension(&TAG, "hello".to_string());

    assert_eq!(opts.extension(&WEIGHT), Some(-7));
    assert_eq!(opts.extension(&TAG), Some("hello".to_string()));
    assert!(opts.has_extension(&WEIGHT));

    // Roundtrip through wire encoding: the extension bytes live in
    // __buffa_unknown_fields and are re-encoded by Message::write_to.
    use buffa::Message;
    let bytes = opts.encode_to_vec();
    let decoded = FieldOptions::decode_from_slice(&bytes).expect("decode");
    assert_eq!(decoded.extension(&WEIGHT), Some(-7));
    assert_eq!(decoded.extension(&TAG), Some("hello".to_string()));

    // And an ExtensionSet impl was emitted in the generated output.
    let file = proto3_file("ext.proto");
    let files = generate(
        &[FileDescriptorProto {
            message_type: vec![DescriptorProto {
                name: Some("M".to_string()),
                ..Default::default()
            }],
            ..file
        }],
        &["ext.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate");
    assert!(
        files[0]
            .content
            .contains("impl ::buffa::ExtensionSet for M"),
        "missing ExtensionSet impl: {}",
        files[0].content
    );
}

#[test]
fn editions_delimited_message_encoding() {
    // Editions 2023 `features.message_encoding = DELIMITED`: the descriptor
    // field type stays TYPE_MESSAGE, but the codegen must route it through
    // the group (wire types 3/4) encode/decode paths via effective_type.
    //
    // Also verifies the map-entry exemption: map values are always
    // length-prefixed (protobuf spec hard rule), even under a file-level
    // DELIMITED default.
    use crate::generated::descriptor::{
        feature_set::MessageEncoding as FsMessageEncoding, Edition, FeatureSet, FieldOptions,
        FileOptions,
    };

    let inner_msg = DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("x", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    };

    // Field with per-field LENGTH_PREFIXED override (common pattern in
    // test_messages_edition2023.proto to opt specific fields back out).
    let mut lp_field = make_field("lp_child", 2, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    lp_field.type_name = Some(".Inner".to_string());
    lp_field.options = FieldOptions {
        features: FeatureSet {
            message_encoding: Some(FsMessageEncoding::LENGTH_PREFIXED),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();

    // Field that inherits the file-level DELIMITED default.
    let mut delim_field = make_field("delim_child", 3, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    delim_field.type_name = Some(".Inner".to_string());

    // Map field with message value — must stay length-prefixed.
    let mut map_field = make_field("inners", 4, Label::LABEL_REPEATED, Type::TYPE_MESSAGE);
    map_field.type_name = Some(".Outer.InnersEntry".to_string());
    let mut map_val = make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    map_val.type_name = Some(".Inner".to_string());
    let map_entry = DescriptorProto {
        name: Some("InnersEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            map_val,
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };

    let outer_msg = DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![lp_field, delim_field, map_field],
        nested_type: vec![map_entry],
        ..Default::default()
    };

    let file = FileDescriptorProto {
        name: Some("delim.proto".to_string()),
        edition: Some(Edition::EDITION_2023),
        options: FileOptions {
            features: FeatureSet {
                message_encoding: Some(FsMessageEncoding::DELIMITED),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
        .into(),
        message_type: vec![inner_msg, outer_msg],
        ..Default::default()
    };

    let files = generate(
        &[file],
        &["delim.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate");
    let content = &joined(&files);

    // Field 3 (delim_child): inherits DELIMITED → StartGroup/EndGroup.
    assert!(
        content.contains("::buffa::types::put_group_start(3u32, buf)"),
        "delim_child should encode as group: {content}"
    );
    assert!(
        content.contains("merge_group"),
        "delim_child should decode via merge_group: {content}"
    );

    // Field 2 (lp_child): explicit LENGTH_PREFIXED → regular message encoding.
    // prettyplease wraps Tag::new across lines for this field number, so
    // check the decode arm instead (wire-type check on the next lines).
    assert!(
        content.contains(
            "2u32 => {\n                ::buffa::encoding::check_wire_type(\n                    \
             tag,\n                    ::buffa::encoding::WireType::LengthDelimited,"
        ),
        "lp_child should decode as length-delimited: {content}"
    );
    // And it should NOT have a StartGroup encode for field 2.
    assert!(
        !content.contains("put_group_start(2u32, buf)"),
        "lp_child should not encode as group"
    );

    // Map entry value: must NOT be group-encoded despite file-level DELIMITED.
    // If the map-entry exemption fails, codegen panics in type_encoded_size_expr
    // (TYPE_GROUP is unreachable there), so reaching this line is the key
    // evidence. Spot-check group-decode call counts: only delim_child should
    // use them (merge_group in owned impl, borrow_group in view).
    assert_eq!(
        content.matches("merge_group").count(),
        1,
        "owned: {content}"
    );
    assert_eq!(
        content.matches("borrow_group").count(),
        1,
        "view: {content}"
    );
}

#[test]
fn nested_message_cross_package_reference_uses_correct_nesting() {
    // Proto layout:
    //   package a.admin.v1: message Svc { message Filter { a.v1.Biz.Status status = 1; } }
    //   package a.v1: message Biz { enum Status { UNSPECIFIED=0; ACTIVE=1; } }
    //
    // In the generated code, Filter lives inside `pub mod svc { ... }` (nesting=1).
    // Its field must reference `super::super::super::v1::biz::Status` (3 supers),
    // not `super::super::v1::biz::Status` (2 supers, which is the nesting=0 path).
    let status_enum = EnumDescriptorProto {
        name: Some("Status".into()),
        value: vec![enum_value("UNSPECIFIED", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    };
    let biz = DescriptorProto {
        name: Some("Biz".into()),
        enum_type: vec![status_enum],
        ..Default::default()
    };
    let filter = DescriptorProto {
        name: Some("Filter".into()),
        field: vec![FieldDescriptorProto {
            name: Some("status".into()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".a.v1.Biz.Status".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let svc = DescriptorProto {
        name: Some("Svc".into()),
        nested_type: vec![filter],
        ..Default::default()
    };

    let file_admin = FileDescriptorProto {
        name: Some("admin.proto".into()),
        package: Some("a.admin.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![svc],
        ..Default::default()
    };
    let file_biz = FileDescriptorProto {
        name: Some("biz.proto".into()),
        package: Some("a.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![biz],
        ..Default::default()
    };

    let files = generate(
        &[file_admin, file_biz],
        &["admin.proto".into()],
        &CodeGenConfig::default(),
    )
    .expect("codegen should succeed");

    let content = &joined(&files);

    // Filter (nesting=1) must emit 3 supers to reach the common ancestor:
    //   super (svc) → super (v1) → super (admin) → v1::biz::Status
    assert!(
        content.contains("super::super::super::v1::biz::Status"),
        "nested message should use 3 supers for cross-package reference at nesting=1.\n\
         Generated code:\n{content}"
    );
    // Every occurrence of the path must have exactly 3 supers (nesting=1),
    // not 2 (which would be the nesting=0 bug). Since "super::super::super::"
    // contains "super::super::" as a substring, count occurrences directly.
    let path_3 = content
        .matches("super::super::super::v1::biz::Status")
        .count();
    let path_2 = content.matches("super::super::v1::biz::Status").count();
    assert_eq!(
        path_3, path_2,
        "every v1::biz::Status reference must use 3 supers (nesting=1), \
         but found {} with 3 supers and {} total with >=2 supers.\n\
         Generated code:\n{content}",
        path_3, path_2
    );
}

#[test]
fn doubly_nested_message_paths_use_correct_nesting() {
    // Stress the nesting plumbing at nesting=2, across each site that has
    // historically hardcoded a too-shallow depth:
    //   - message field (handled by the base fix)
    //   - oneof variant type (oneof.rs collect_variant_info)
    //   - textproto enum lookup (impl_text.rs enum_type_path)
    //
    // Proto layout:
    //   package a.admin.v1:
    //     message Svc { message Outer { message Inner {
    //       a.v1.Biz.Status direct = 1;
    //       oneof choice { a.v1.Biz.Status via_oneof = 2; }
    //     } } }
    //   package a.v1: message Biz { enum Status { UNSPECIFIED=0; ACTIVE=1; } }
    //
    // Inner sits at nesting=3 (`svc::outer::inner`), so every cross-package
    // reference it emits must use 4 supers.
    let status_enum = EnumDescriptorProto {
        name: Some("Status".into()),
        value: vec![enum_value("UNSPECIFIED", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    };
    let biz = DescriptorProto {
        name: Some("Biz".into()),
        enum_type: vec![status_enum],
        ..Default::default()
    };

    let inner = DescriptorProto {
        name: Some("Inner".into()),
        field: vec![
            FieldDescriptorProto {
                name: Some("direct".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".a.v1.Biz.Status".into()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("via_oneof".into()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".a.v1.Biz.Status".into()),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer = DescriptorProto {
        name: Some("Outer".into()),
        nested_type: vec![inner],
        ..Default::default()
    };
    let svc = DescriptorProto {
        name: Some("Svc".into()),
        nested_type: vec![outer],
        ..Default::default()
    };

    let file_admin = FileDescriptorProto {
        name: Some("admin.proto".into()),
        package: Some("a.admin.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![svc],
        ..Default::default()
    };
    let file_biz = FileDescriptorProto {
        name: Some("biz.proto".into()),
        package: Some("a.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![biz],
        ..Default::default()
    };

    // Exercise the impl_text path too — textproto enum references used to
    // hardcode nesting=0 regardless of the struct's actual depth.
    let config = CodeGenConfig {
        generate_text: true,
        ..Default::default()
    };
    let files = generate(&[file_admin, file_biz], &["admin.proto".into()], &config)
        .expect("codegen should succeed");
    let content = &joined(&files);

    // Inner is at nesting=3 (inside svc::outer::inner), so every cross-
    // package reference must emit 4 supers.
    let path_4 = content
        .matches("super::super::super::super::v1::biz::Status")
        .count();
    // path_3 should match only the Svc::Filter-style depth-1 references —
    // since we don't emit any here, we use path_3 purely as a sanity baseline.
    let path_3 = content
        .matches("super::super::super::v1::biz::Status")
        .count();
    assert_eq!(
        path_4, path_3,
        "every Inner (nesting=3) reference must use 4 supers, but \
         found {path_4} with 4 supers and {path_3} with >=3 supers.\n\
         Generated code:\n{content}"
    );
    assert!(
        path_4 > 0,
        "expected the cross-package enum reference to appear at least once \
         with 4 supers in Inner's module.\nGenerated code:\n{content}"
    );
}

#[test]
fn apply_companions_patches_stitcher() {
    let mut file = proto3_file("svc/echo.proto");
    file.package = Some("svc".to_string());
    let mut files = generate(
        &[file],
        &["svc/echo.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate ok");

    let stem = proto_path_to_stem("svc/echo.proto");
    let companion = GeneratedFile {
        name: format!("{stem}.__service.rs"),
        package: "svc".to_string(),
        kind: GeneratedFileKind::Companion,
        content: "pub struct EchoService;".to_string(),
    };
    apply_companions(&mut files, vec![companion]);

    // Companion file was appended to the list.
    assert!(
        files
            .iter()
            .any(|f| f.kind == GeneratedFileKind::Companion && f.name == "svc.echo.__service.rs"),
        "companion file missing from output"
    );

    // Stitcher now includes the companion file.
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert!(
        stitcher
            .content
            .contains(r#"include!("svc.echo.__service.rs");"#),
        "stitcher missing companion include: {}",
        stitcher.content
    );

    // Stitcher is still valid Rust after the include! was appended.
    syn::parse_file(&stitcher.content).expect("stitcher still parses after apply_companions");
}

#[test]
fn apply_companions_multiple_same_package() {
    let mut file = proto3_file("acme/v1/msg.proto");
    file.package = Some("acme.v1".to_string());
    let mut files = generate(
        &[file],
        &["acme/v1/msg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate ok");

    let stem = proto_path_to_stem("acme/v1/msg.proto");
    let companions = vec![
        GeneratedFile {
            name: format!("{stem}.__service_a.rs"),
            package: "acme.v1".to_string(),
            kind: GeneratedFileKind::Companion,
            content: "pub struct ServiceA;".to_string(),
        },
        GeneratedFile {
            name: format!("{stem}.__service_b.rs"),
            package: "acme.v1".to_string(),
            kind: GeneratedFileKind::Companion,
            content: "pub struct ServiceB;".to_string(),
        },
    ];
    apply_companions(&mut files, companions);

    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert!(
        stitcher
            .content
            .contains(r#"include!("acme.v1.msg.__service_a.rs");"#),
        "missing service_a include"
    );
    assert!(
        stitcher
            .content
            .contains(r#"include!("acme.v1.msg.__service_b.rs");"#),
        "missing service_b include"
    );
    assert_eq!(
        files
            .iter()
            .filter(|f| f.kind == GeneratedFileKind::Companion)
            .count(),
        2
    );
    // Two appended include! lines must still parse as a valid module.
    syn::parse_file(&stitcher.content).expect("stitcher still parses with two companion includes");
}

#[test]
fn apply_companions_no_matching_package_mod() {
    // Companion file for a package with no PackageMod in `files` —
    // still appended to the list, no panic, no include emitted.
    let mut files: Vec<GeneratedFile> = vec![];
    let companion = GeneratedFile {
        name: "orphan.__service.rs".to_string(),
        package: "orphan".to_string(),
        kind: GeneratedFileKind::Companion,
        content: "pub struct Orphan;".to_string(),
    };
    apply_companions(&mut files, vec![companion]);

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].kind, GeneratedFileKind::Companion);
}

#[test]
fn apply_companions_empty_is_noop() {
    let mut file = proto3_file("svc/echo.proto");
    file.package = Some("svc".to_string());
    let mut files = generate(
        &[file],
        &["svc/echo.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate ok");
    let before: Vec<String> = files.iter().map(|f| f.content.clone()).collect();
    apply_companions(&mut files, vec![]);
    let after: Vec<String> = files.iter().map(|f| f.content.clone()).collect();
    assert_eq!(before, after, "empty companions list must not mutate files");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "contains a character that would break")]
fn apply_companions_rejects_unsafe_name() {
    let mut files: Vec<GeneratedFile> = vec![];
    apply_companions(
        &mut files,
        vec![GeneratedFile {
            name: r#"bad".rs"#.to_string(),
            package: "svc".to_string(),
            kind: GeneratedFileKind::Companion,
            content: String::new(),
        }],
    );
}

#[test]
fn apply_companions_file_per_package() {
    let mut file = proto3_file("svc/msg.proto");
    file.package = Some("svc".to_string());
    let config = CodeGenConfig {
        file_per_package: true,
        ..Default::default()
    };
    let mut files =
        generate(&[file], &["svc/msg.proto".to_string()], &config).expect("generate ok");

    let companion = GeneratedFile {
        name: "svc.msg.__service.rs".to_string(),
        package: "svc".to_string(),
        kind: GeneratedFileKind::Companion,
        content: "pub struct MsgService;".to_string(),
    };
    apply_companions(&mut files, vec![companion]);

    let pkg_file = &files[0];
    assert!(
        pkg_file
            .content
            .contains(r#"include!("svc.msg.__service.rs");"#),
        "file_per_package stitcher missing companion include"
    );
    syn::parse_file(&pkg_file.content).expect("file_per_package output still parses");
}

//! Shared-pool reflection mode: `compile()` emits per-package delegating
//! reflect modules and embeds no per-package descriptor bytes. The single
//! shared `__buffa_fds` root module is emitted by the module-tree builder
//! (buffa-build / the packaging plugin), not by `generate`, so `generate`'s
//! output should contain delegations but no byte-literal array.

use super::*;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};

fn shared_config() -> CodeGenConfig {
    CodeGenConfig {
        generate_reflection: true,
        generate_reflection_vtable: true,
        shared_descriptor_pool: true,
        ..Default::default()
    }
}

fn msg_in_package(file_name: &str, package: &str, msg: &str) -> FileDescriptorProto {
    let mut file = proto3_file(file_name);
    file.package = Some(package.to_string());
    file.message_type.push(DescriptorProto {
        name: Some(msg.to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    file
}

#[test]
fn shared_mode_delegates_per_package_and_embeds_no_bytes() {
    let files = generate(
        &[
            msg_in_package("alpha.proto", "alpha.v1", "Alpha"),
            msg_in_package("beta.proto", "beta", "Beta"),
        ],
        &["alpha.proto".to_string(), "beta.proto".to_string()],
        &shared_config(),
    )
    .expect("should generate");

    let package_mods: Vec<&GeneratedFile> = files
        .iter()
        .filter(|f| f.kind == GeneratedFileKind::PackageMod)
        .collect();
    assert_eq!(package_mods.len(), 2, "one .mod.rs per package");

    for pm in &package_mods {
        assert!(
            pm.content.contains("__buffa_fds"),
            "package {} must delegate to the shared root: {}",
            pm.package,
            pm.content
        );
    }

    // The whole point: no package embeds its own copy of the bytes.
    let all = joined(&files);
    assert!(
        !all.contains("FILE_DESCRIPTOR_SET_BYTES: &[u8] = &["),
        "shared mode must not embed any per-package byte array: {all}"
    );
    // And `generate` does not emit the root module itself — that's the tree
    // builder's job — so there is no byte-array definition anywhere here.
    assert!(
        !all.contains("pub mod __buffa_fds"),
        "generate() must not define the root module (the tree builder does): {all}"
    );
}

#[test]
fn shared_mode_rejects_reserved_root_package() {
    // A package whose first segment is the shared-root name would collide with
    // the `pub mod __buffa_fds` the tree builder emits at the root.
    let err = generate(
        &[msg_in_package("x.proto", "__buffa_fds.v1", "X")],
        &["x.proto".to_string()],
        &shared_config(),
    )
    .expect_err("reserved root package must be rejected in shared mode");
    assert!(
        matches!(err, CodeGenError::ReservedModuleName { .. }),
        "expected ReservedModuleName, got {err:?}"
    );
}

#[test]
fn shared_mode_rejects_reserved_root_message_in_unnamed_package() {
    let mut file = proto3_file("root.proto"); // unnamed package
    file.message_type.push(DescriptorProto {
        name: Some("__buffa_fds".to_string()),
        ..Default::default()
    });
    let err = generate(&[file], &["root.proto".to_string()], &shared_config())
        .expect_err("reserved root-package message name must be rejected in shared mode");
    assert!(
        matches!(err, CodeGenError::ReservedModuleName { .. }),
        "{err:?}"
    );
}

#[test]
fn shared_mode_rejects_reserved_name_in_any_package_segment() {
    // Not just the first segment: any segment becomes a `pub mod`.
    let err = generate(
        &[msg_in_package("x.proto", "foo.__buffa_fds", "X")],
        &["x.proto".to_string()],
        &shared_config(),
    )
    .expect_err("reserved name in a nested package segment must be rejected");
    assert!(
        matches!(err, CodeGenError::ReservedModuleName { .. }),
        "{err:?}"
    );
}

#[test]
fn shared_mode_allows_reserved_name_in_ungenerated_import() {
    // An import-only package named `__buffa_fds` emits no module, so the
    // reservation must not reject it — only generated files are checked.
    let imported = msg_in_package("dep.proto", "__buffa_fds", "Dep");
    let user = msg_in_package("user.proto", "user.v1", "User");
    let files = generate(
        &[imported, user],
        &["user.proto".to_string()],
        &shared_config(),
    )
    .expect("import-only reserved-name package must not be rejected");
    assert!(files.iter().any(|f| f.package == "user.v1"));
}

#[test]
fn root_module_inline_embeds_bytes_and_defines_module() {
    let bytes = crate::encode_descriptor_set(&[msg_in_package("a.proto", "a", "A")], &[]);
    assert!(!bytes.is_empty());
    let src = crate::shared_descriptor_root_module(&bytes, crate::FdsEmbedding::Inline, None);
    assert!(src.contains("pub mod __buffa_fds"), "{src}");
    assert!(src.contains("FILE_DESCRIPTOR_SET_BYTES"));
    assert!(src.contains("descriptor_pool"));
    assert!(
        !src.contains("include_bytes"),
        "inline mode has no sidecar: {src}"
    );
    assert!(!src.contains("#[cfg("), "ungated by default: {src}");
}

#[test]
fn root_module_relative_sidecar_uses_include_bytes() {
    let bytes = crate::encode_descriptor_set(&[msg_in_package("a.proto", "a", "A")], &[]);
    let src = crate::shared_descriptor_root_module(
        &bytes,
        crate::FdsEmbedding::Sidecar {
            file_name: "descriptor_set.binpb",
            mode: IncludeMode::Relative("gen/"),
        },
        None,
    );
    assert!(src.contains("include_bytes!"), "{src}");
    assert!(
        src.contains(r#""gen/descriptor_set.binpb""#),
        "relative sidecar prepends the prefix: {src}"
    );
}

#[test]
fn root_module_out_dir_sidecar_uses_concat_env() {
    let bytes = crate::encode_descriptor_set(&[msg_in_package("a.proto", "a", "A")], &[]);
    let src = crate::shared_descriptor_root_module(
        &bytes,
        crate::FdsEmbedding::Sidecar {
            file_name: "descriptor_set.binpb",
            mode: IncludeMode::OutDir,
        },
        None,
    );
    assert!(src.contains("include_bytes!"), "{src}");
    assert!(src.contains("env!(\"OUT_DIR\")"), "{src}");
    assert!(src.contains("/descriptor_set.binpb"), "{src}");
}

#[test]
fn root_module_gate_wraps_in_cfg() {
    let bytes = crate::encode_descriptor_set(&[msg_in_package("a.proto", "a", "A")], &[]);
    let src =
        crate::shared_descriptor_root_module(&bytes, crate::FdsEmbedding::Inline, Some("reflect"));
    assert!(
        src.contains(r#"#[cfg(feature = "reflect")]"#),
        "gated root must carry the cfg: {src}"
    );
}

#[test]
fn encode_descriptor_set_applies_feature_overrides() {
    // A proto2 (closed) enum: an open-enum override changes the embedded
    // descriptor set. A front-end computing the shared copy must apply the
    // same overrides `generate` does, or the shared pool would report the enum
    // as closed while the generated code treats it as open.
    let file = FileDescriptorProto {
        name: Some("e.proto".to_string()),
        package: Some("pkg".to_string()),
        syntax: Some("proto2".to_string()),
        enum_type: vec![EnumDescriptorProto {
            name: Some("Color".to_string()),
            value: vec![enum_value("RED", 0)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = [file];
    let plain = crate::encode_descriptor_set(&files, &[]);
    let overridden = crate::encode_descriptor_set(&files, &open_enum_overrides(&[".pkg.Color"]));
    assert_ne!(
        plain, overridden,
        "open-enum override must change the embedded descriptor set"
    );
}

#[test]
fn default_reflection_still_embeds_per_package() {
    // Contrast: without the flag, today's per-package embedding is unchanged.
    let config = CodeGenConfig {
        generate_reflection: true,
        generate_reflection_vtable: true,
        ..Default::default()
    };
    let files = generate(
        &[msg_in_package("alpha.proto", "alpha.v1", "Alpha")],
        &["alpha.proto".to_string()],
        &config,
    )
    .expect("should generate");
    let all = joined(&files);
    assert!(
        all.contains("FILE_DESCRIPTOR_SET_BYTES: &[u8] = &["),
        "default mode still embeds the byte array per package: {all}"
    );
}

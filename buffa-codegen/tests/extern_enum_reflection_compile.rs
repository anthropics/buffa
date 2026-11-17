//! Exercise the enum owner's Rust crate boundary and optional reflection
//! dependency, which separate codegen passes inside one crate cannot check.
//! Run with `cargo test -p buffa-codegen --test extern_enum_reflection_compile -- --ignored`.

use std::path::Path;
use std::process::Command;

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;
use buffa_codegen::{
    CodeGenConfig, EnumTypeOverride, FeatureOverride, GeneratedFileKind, IncludeMode,
};

const OPEN_OWNER: &str = "extern_open_enum_owner.proto";
const BRIDGE_OWNER: &str = "extern_bridge_enum_owner.proto";
const CONSUMER: &str = "extern_open_enum_user.proto";

fn emit(
    root: &Path,
    descriptors: &FileDescriptorSet,
    file: &str,
    config: &CodeGenConfig,
) -> (String, String) {
    let generated = buffa_codegen::generate(&descriptors.file, &[file.into()], config).unwrap();
    let mut package = None;
    for generated in generated {
        if generated.kind == GeneratedFileKind::PackageMod {
            package = Some((generated.name.clone(), generated.package));
        }
        std::fs::write(root.join(generated.name), generated.content).unwrap();
    }
    package.expect("one package per fixture")
}

fn cargo(workspace: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO"))
        .current_dir(workspace)
        .args(args)
        // Keep nested Cargo invocations away from the parent test's locks.
        .arg("--target-dir")
        .arg(workspace.join("target"))
        .output()
        .unwrap()
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "spawns cargo; run explicitly with --ignored"]
fn extern_enum_owners_supply_feature_gated_reflection_to_a_separate_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let descriptor_path = dir.path().join("fixtures.pb");
    let output = Command::new(std::env::var_os("PROTOC").unwrap_or_else(|| "protoc".into()))
        .arg("--include_imports")
        .arg(format!(
            "--descriptor_set_out={}",
            descriptor_path.display()
        ))
        .arg("-I")
        .arg(root.join("buffa-test/protos"))
        .args([OPEN_OWNER, BRIDGE_OWNER, CONSUMER])
        .output()
        .unwrap();
    assert_success(&output);
    let descriptors =
        FileDescriptorSet::decode_from_slice(&std::fs::read(descriptor_path).unwrap()).unwrap();

    for package in ["owner", "consumer"] {
        std::fs::create_dir_all(dir.path().join(package).join("src/gen")).unwrap();
    }
    let mut owner_config = CodeGenConfig::default();
    owner_config.generate_reflection = true;
    owner_config.generate_reflection_vtable = true;
    owner_config.gate_reflect_on_crate_feature = true;
    owner_config.generate_views = false;
    owner_config.feature_overrides.push((
        ".test.extopenenum.owner.Color".into(),
        FeatureOverride::EnumType(EnumTypeOverride::Open),
    ));
    let owner_src = dir.path().join("owner/src");
    let opened = emit(
        &owner_src.join("gen"),
        &descriptors,
        OPEN_OWNER,
        &owner_config,
    );
    owner_config.generate_reflection_vtable = false;
    owner_config.feature_overrides.clear();
    let bridged = emit(
        &owner_src.join("gen"),
        &descriptors,
        BRIDGE_OWNER,
        &owner_config,
    );
    let owner_tree = buffa_codegen::generate_module_tree(
        &[opened, bridged],
        IncludeMode::Relative("gen/"),
        false,
    );
    std::fs::write(
        owner_src.join("lib.rs"),
        format!("#![cfg_attr(not(feature = \"reflect\"), no_std)]\n{owner_tree}"),
    )
    .unwrap();

    let mut consumer_config = CodeGenConfig::default();
    consumer_config.generate_reflection = true;
    consumer_config.generate_reflection_vtable = true;
    consumer_config.gate_reflect_on_crate_feature = true;
    consumer_config.extern_paths = vec![
        (
            ".test.extopenenum.owner".into(),
            "::enum_owner::test::extopenenum::owner".into(),
        ),
        (
            ".test.extopenenum.bridge".into(),
            "::enum_owner::test::extopenenum::bridge".into(),
        ),
    ];
    let consumer_src = dir.path().join("consumer/src");
    let consumer = emit(
        &consumer_src.join("gen"),
        &descriptors,
        CONSUMER,
        &consumer_config,
    );
    let consumer_tree =
        buffa_codegen::generate_module_tree(&[consumer], IncludeMode::Relative("gen/"), false);
    std::fs::write(
        consumer_src.join("lib.rs"),
        format!(
            "#![cfg_attr(not(feature = \"reflect\"), no_std)]\n{consumer_tree}\n{}",
            r#"
#[cfg(all(test, feature = "reflect"))]
mod checks {
    use buffa::{Message, MessageView};
    use buffa_descriptor::reflect::{MapKey, ReflectMessage, ValueRef};
    use enum_owner::test::extopenenum::{bridge::Shade, owner::{Color, OwnerColors}};
    use crate::test::extopenenum::user::{Palette, PaletteView};

    #[test]
    fn owned_and_view_reflection_preserve_external_enum_numbers() {
        // The owner opens Color, while the consumer resolves it as closed.
        let mut opened = OwnerColors::default();
        opened.colors.push(buffa::EnumValue::from(Color::COLOR_GREEN));
        let mut message = Palette::default();
        message.colors.push(Color::COLOR_GREEN);
        message.by_id.insert(7, Color::COLOR_GREEN);
        message.shades.push(Shade::SHADE_DARK);
        message.shade_by_id.insert(7, Shade::SHADE_DARK);
        let wire = message.encode_to_vec();
        let view = PaletteView::decode_view(&wire).unwrap();
        for reflected in [&message as &dyn ReflectMessage, &view as &dyn ReflectMessage] {
            let descriptor = reflected.message_descriptor();
            for (list_field, map_field) in [(1, 2), (3, 4)] {
                let ValueRef::List(list) = reflected.get(descriptor.field(list_field).unwrap()) else {
                    panic!("expected enum list");
                };
                assert!(matches!(list.get(0), Some(ValueRef::EnumNumber(1))));
                let ValueRef::Map(map) = reflected.get(descriptor.field(map_field).unwrap()) else {
                    panic!("expected enum map");
                };
                assert!(matches!(map.get(&MapKey::I32(7)), Some(ValueRef::EnumNumber(1))));
            }
        }
    }
}
"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"owner\", \"consumer\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    for (package, name, extra, reflect) in [
        ("owner", "enum-owner", String::new(), ""),
        (
            "consumer",
            "enum-consumer",
            "enum-owner = { path = \"../owner\", default-features = false }\n".into(),
            ", \"enum-owner/reflect\"",
        ),
    ] {
        std::fs::write(
            dir.path().join(package).join("Cargo.toml"),
            format!(
                "[package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\
                 [features]\ndefault = []\nreflect = [\"dep:buffa-descriptor\", \"buffa/std\"{reflect}]\n\
                 [dependencies]\n\
                 buffa = {{ path = {runtime:?}, default-features = false }}\n\
                 buffa-descriptor = {{ path = {descriptor:?}, default-features = false, optional = true, features = [\"reflect\", \"std\"] }}\n{extra}",
                runtime = root.join("buffa"),
                descriptor = root.join("buffa-descriptor"),
            ),
        )
        .unwrap();
    }
    assert_success(&cargo(
        dir.path(),
        &["check", "--offline", "-p", "enum-consumer"],
    ));
    // No optional descriptor dependency may leak into reflection-disabled,
    // no_std owner/consumer crates just because an enum impl was generated.
    let graph = Command::new(env!("CARGO"))
        .current_dir(dir.path())
        .args(["tree", "--offline", "-p", "enum-consumer"])
        .output()
        .unwrap();
    assert_success(&graph);
    assert!(!String::from_utf8(graph.stdout)
        .unwrap()
        .contains("buffa-descriptor"));
    assert_success(&cargo(
        dir.path(),
        &[
            "test",
            "--offline",
            "-p",
            "enum-consumer",
            "--features",
            "reflect",
        ],
    ));
}

//! Compile and round-trip custom representations named by associated types or
//! parenthesized paths. Parsing alone cannot prove these names work everywhere
//! codegen uses a representation, including trait calls and constructors.
//! Run with `cargo test -p buffa-codegen --test custom_type_paths_compile -- --ignored`.

use std::path::Path;
use std::process::Command;

use buffa_codegen::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_codegen::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use buffa_codegen::{BytesRepr, CodeGenConfig, RepeatedRepr, StringRepr};

#[test]
#[ignore = "spawns cargo; run explicitly with --ignored"]
fn qualified_and_parenthesized_representations_compile_and_round_trip() {
    let mut fields = Vec::new();
    for (name, number, ty, label) in [
        ("blob", 1, Type::TYPE_BYTES, Label::LABEL_OPTIONAL),
        ("text", 2, Type::TYPE_STRING, Label::LABEL_OPTIONAL),
        ("ids", 3, Type::TYPE_UINT32, Label::LABEL_REPEATED),
    ] {
        fields.push(FieldDescriptorProto {
            name: Some(name.into()),
            number: Some(number),
            r#type: Some(ty),
            label: Some(label),
            ..Default::default()
        });
    }
    let file = FileDescriptorProto {
        name: Some("paths.proto".into()),
        package: Some("paths".into()),
        syntax: Some("proto3".into()),
        message_type: vec![DescriptorProto {
            name: Some("Sample".into()),
            field: fields,
            ..Default::default()
        }],
        ..Default::default()
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let mut lib = String::from(
        "pub struct Carrier;\n\
         pub trait Representations { type Blob; type Text; type List<T>; }\n\
         impl Representations for Carrier {\n\
             type Blob = Vec<u8>; type Text = String; type List<T> = Vec<T>;\n\
         }\n",
    );
    // Both syntactic forms must preserve the concrete field types and wire
    // behavior. Use separate modules so each pass exercises all three knobs.
    for (module, parens) in [("qualified", false), ("parenthesized", true)] {
        let wrap = |path: &str| {
            if parens {
                format!("(({path}))")
            } else {
                path.to_owned()
            }
        };
        let mut config = CodeGenConfig::default();
        config.generate_views = false;
        config.bytes_fields.push((
            ".".into(),
            BytesRepr::Custom(wrap("<crate::Carrier as crate::Representations>::Blob")),
        ));
        config.string_fields.push((
            ".".into(),
            StringRepr::Custom(wrap("<crate::Carrier as crate::Representations>::Text")),
        ));
        config.repeated_fields.push((
            ".".into(),
            RepeatedRepr::Custom(wrap("<crate::Carrier as crate::Representations>::List<*>")),
        ));
        let generated = buffa_codegen::generate(
            std::slice::from_ref(&file),
            &["paths.proto".into()],
            &config,
        )
        .unwrap();
        let output = src.join(module);
        std::fs::create_dir(&output).unwrap();
        for generated in generated {
            std::fs::write(output.join(generated.name), generated.content).unwrap();
        }
        lib.push_str(&format!(
            "pub mod {module} {{ include!(\"{module}/paths.mod.rs\");\n\
             #[test] fn round_trip() {{\n\
                 use buffa::Message;\n\
                 let msg = Sample {{ blob: vec![0, 255], text: String::from(\"hello\"),\n\
                     ids: vec![0, 127, 300], ..Default::default() }};\n\
                 let wire = msg.encode_to_vec();\n\
                 assert_eq!(Sample::decode_from_slice(&wire).unwrap(), msg);\n\
             }} }}\n"
        ));
    }
    std::fs::write(src.join("lib.rs"), lib).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"custom-type-path-fixture\"\nversion = \"0.0.0\"\n\
             edition = \"2021\"\npublish = false\n[workspace]\n\
             [dependencies]\nbuffa = {{ path = {:?} }}\n",
            root.join("buffa")
        ),
    )
    .unwrap();
    // A separate target avoids holding the parent test process's Cargo lock.
    let result = Command::new(env!("CARGO"))
        .args(["test", "--offline", "--manifest-path"])
        .arg(dir.path().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(dir.path().join("target"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

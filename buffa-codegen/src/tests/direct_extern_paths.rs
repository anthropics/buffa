//! Direct callers must get the same extern-path validation as the front ends.

use super::*;

fn descriptors(import: &str, package: &str) -> Vec<FileDescriptorProto> {
    let mut imported = proto3_file(import);
    imported.package = Some(package.into());
    imported.message_type = ["Shared", "Other"]
        .iter()
        .map(|name| DescriptorProto {
            name: Some((*name).into()),
            ..Default::default()
        })
        .collect();
    let mut consumer = proto3_file("consumer.proto");
    consumer.package = Some("consumer".into());
    consumer.dependency = vec![import.into()];
    consumer.message_type.push(DescriptorProto {
        name: Some("Consumer".into()),
        field: ["Shared", "Other"]
            .iter()
            .enumerate()
            .map(|(i, name)| FieldDescriptorProto {
                type_name: Some(format!(".{package}.{name}")),
                ..make_field(
                    &name.to_lowercase(),
                    i as i32 + 1,
                    Label::LABEL_OPTIONAL,
                    Type::TYPE_MESSAGE,
                )
            })
            .collect(),
        ..Default::default()
    });
    vec![imported, consumer]
}

fn run(
    diagnostics: bool,
    descriptors: &[FileDescriptorProto],
    names: &[String],
    config: &CodeGenConfig,
) -> Result<Vec<GeneratedFile>, CodeGenError> {
    if diagnostics {
        generate_with_diagnostics(descriptors, names, config).map(|(files, _)| files)
    } else {
        generate(descriptors, names, config)
    }
}

#[test]
fn direct_extern_paths_normalize_for_resolution_without_changing_config() {
    let files = descriptors("common.proto", "my.common");
    for diagnostics in [false, true] {
        for proto in [".my.common", "my.common", " .my.common. ", " my.common "] {
            let config = CodeGenConfig {
                extern_paths: vec![(proto.into(), "::external".into())],
                ..Default::default()
            };
            let output = run(diagnostics, &files, &["consumer.proto".into()], &config).unwrap();
            let content = joined(&output);
            assert!(
                content.contains("::external::Shared"),
                "{proto:?}: {content}"
            );
            assert!(content.contains("::external::Other"));
            assert_eq!(config.extern_paths, [(proto.into(), "::external".into())]);
            let canonical = CodeGenConfig {
                extern_paths: vec![(".my.common".into(), "::external".into())],
                ..config.clone()
            };
            let expected =
                run(diagnostics, &files, &["consumer.proto".into()], &canonical).unwrap();
            assert_eq!(
                output
                    .iter()
                    .map(|f| (&f.name, &f.content))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|f| (&f.name, &f.content))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn direct_extern_paths_preserve_exact_and_longest_prefix_precedence() {
    let files = descriptors("common.proto", "my.common");
    let config = CodeGenConfig {
        extern_paths: vec![
            ("my".into(), "::broad".into()),
            ("my.common.Shared".into(), "::exact::Renamed".into()),
            (" my.common. ".into(), "::specific".into()),
        ],
        ..Default::default()
    };
    for diagnostics in [false, true] {
        let content =
            joined(&run(diagnostics, &files, &["consumer.proto".into()], &config).unwrap());
        assert!(content.contains("::exact::Renamed"));
        assert!(content.contains("::specific::Other"));
        assert!(!content.contains("::broad::"));
        let unrelated = descriptors("common.proto", "my.commonality");
        let content =
            joined(&run(diagnostics, &unrelated, &["consumer.proto".into()], &config).unwrap());
        assert!(content.contains("::broad::commonality::Shared"));
        assert!(!content.contains("::specific::"));
    }
}

#[test]
fn direct_extern_paths_keep_the_catch_all() {
    let files = descriptors("common.proto", "my.common");
    let config = CodeGenConfig {
        extern_paths: vec![(" . ".into(), "::external".into())],
        ..Default::default()
    };
    for diagnostics in [false, true] {
        let content =
            joined(&run(diagnostics, &files, &["consumer.proto".into()], &config).unwrap());
        assert!(content.contains("::external::my::common::Shared"));
    }
}

#[test]
fn direct_extern_paths_override_automatic_wkt_and_file_mappings() {
    let config = CodeGenConfig {
        extern_paths: vec![(" google.protobuf. ".into(), "::custom".into())],
        ..Default::default()
    };
    for diagnostics in [false, true] {
        for import in [
            "google/protobuf/timestamp.proto",
            "google/protobuf/descriptor.proto",
        ] {
            let files = descriptors(import, "google.protobuf");
            let content =
                joined(&run(diagnostics, &files, &["consumer.proto".into()], &config).unwrap());
            assert!(content.contains("::custom::Shared"), "{import}: {content}");
            assert!(!content.contains("::buffa_types::"));
            assert!(!content.contains("::buffa_descriptor::"));
        }
    }
}

#[test]
fn direct_extern_paths_reject_invalid_names_even_with_empty_input() {
    let files = descriptors("common.proto", "my.common");
    for diagnostics in [false, true] {
        for proto in [
            "",
            "  ",
            "..",
            "my..common",
            "my.common..",
            "my.123",
            " my.bad-name ",
        ] {
            let config = CodeGenConfig {
                extern_paths: vec![(proto.into(), "::external".into())],
                ..Default::default()
            };
            for (descriptors, names) in [
                (&files[..], vec!["consumer.proto".into()]),
                (&[][..], vec![]),
            ] {
                let err = run(diagnostics, descriptors, &names, &config).unwrap_err();
                assert!(matches!(err, CodeGenError::Other(_)));
                assert!(err.to_string().contains(&format!("{proto:?}")), "{err}");
                assert_eq!(config.extern_paths, [(proto.into(), "::external".into())]);
            }
        }
    }
}

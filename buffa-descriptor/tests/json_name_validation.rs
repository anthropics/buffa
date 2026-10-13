#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    MessageOptions,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn file_with_json_name(json_name: Option<&str>) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("json-name.proto".into()),
        package: Some("json.test".into()),
        syntax: Some("proto3".into()),
        message_type: vec![DescriptorProto {
            name: Some("Item".into()),
            field: vec![FieldDescriptorProto {
                name: Some("field_name".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                json_name: json_name.map(str::to_owned),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn descriptor_set(file: FileDescriptorProto) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    }
}

/// Asserts that `err` is `InvalidJsonName` for `field` and `name`, and that
/// its message ends with `reason`.
fn assert_invalid_json_name_because(err: &PoolError, field: &str, name: &str, reason: &str) {
    assert!(matches!(
        err,
        PoolError::InvalidJsonName { field: actual_field, name: actual_name }
            if actual_field == field && actual_name == name
    ));
    assert_eq!(
        err.to_string(),
        format!("field {field} has JSON name {name:?}{reason}")
    );
    assert!(!err.to_string().contains('\0'));
}

fn assert_invalid_json_name(err: &PoolError, field: &str, name: &str) {
    assert_invalid_json_name_because(err, field, name, " containing NUL");
}

#[test]
// The option is deprecated in descriptor.proto; protoc still honours it.
#[allow(deprecated)]
fn json_names_with_nul_are_rejected_for_all_syntaxes() {
    for (syntax, edition) in [
        (None, None),
        (Some("proto2"), None),
        (Some("proto3"), None),
        (Some("editions"), Some(Edition::EDITION_2023)),
        (Some("editions"), Some(Edition::EDITION_2024)),
    ] {
        for legacy_conflicts in [false, true] {
            for name in [
                "\0",
                "\0prefix",
                "mid\0suffix",
                "suffix\0",
                "a\0\0b",
                "雪\0",
            ] {
                let mut file = file_with_json_name(Some(name));
                file.syntax = syntax.map(str::to_owned);
                file.edition = edition;
                file.message_type[0].options = buffa::MessageField::some(MessageOptions {
                    deprecated_legacy_json_field_conflicts: Some(legacy_conflicts),
                    ..Default::default()
                });
                let err = DescriptorPool::new(descriptor_set(file))
                    .expect_err("NUL is invalid even when JSON-name conflicts are allowed");
                assert_invalid_json_name(&err, "json.test.Item.field_name", name);
            }
        }
    }
}

#[test]
fn nested_field_json_names_with_nul_are_rejected() {
    let mut file = file_with_json_name(Some("a\0b"));
    let item = file.message_type.remove(0);
    file.message_type.push(DescriptorProto {
        name: Some("Outer".into()),
        nested_type: vec![item],
        ..Default::default()
    });
    let err = DescriptorPool::new(descriptor_set(file)).unwrap_err();
    assert_invalid_json_name(&err, "json.test.Outer.Item.field_name", "a\0b");
}

#[test]
fn extension_json_names_with_nul_are_rejected() {
    for nested in [false, true] {
        let mut file = file_with_json_name(Some("a\0b"));
        file.syntax = Some("proto2".into());
        let mut extension = file.message_type[0].field.remove(0);
        extension.number = Some(100);
        extension.extendee = Some(".json.test.Extendee".into());
        file.message_type = vec![DescriptorProto {
            name: Some("Extendee".into()),
            extension_range: vec![ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let field = if nested {
            file.message_type.push(DescriptorProto {
                name: Some("Scope".into()),
                extension: vec![extension],
                ..Default::default()
            });
            "json.test.Scope.field_name"
        } else {
            file.extension.push(extension);
            "json.test.field_name"
        };
        let err = DescriptorPool::new(descriptor_set(file)).unwrap_err();
        assert_invalid_json_name(&err, field, "a\0b");
    }
}

#[test]
fn encoded_descriptor_json_names_with_nul_are_rejected() {
    let bytes = descriptor_set(file_with_json_name(Some("a\0b"))).encode_to_vec();
    let err = DescriptorPool::decode(&bytes).unwrap_err();
    assert_invalid_json_name(&err, "json.test.Item.field_name", "a\0b");
}

#[test]
fn invalid_json_names_do_not_mutate_an_existing_pool() {
    let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
    let baseline_files = pool.files().to_vec();
    let baseline_counts = (
        pool.messages().len(),
        pool.enums().len(),
        pool.services().len(),
        pool.extensions().len(),
    );
    let baseline_index = pool.message_index("reflect.test.Scalars");
    let mut valid_file = file_with_json_name(None);
    valid_file.name = Some("valid-json-name.proto".into());
    valid_file.package = Some("valid.json".into());
    let err = pool
        .add_file_descriptor_set(FileDescriptorSet {
            file: vec![valid_file, file_with_json_name(Some("a\0b"))],
            ..Default::default()
        })
        .unwrap_err();
    assert_invalid_json_name(&err, "json.test.Item.field_name", "a\0b");
    assert_eq!(pool.files(), baseline_files);
    assert_eq!(
        (
            pool.messages().len(),
            pool.enums().len(),
            pool.services().len(),
            pool.extensions().len(),
        ),
        baseline_counts
    );
    assert_eq!(pool.message_index("reflect.test.Scalars"), baseline_index);
    assert_eq!(
        pool.message(baseline_index.unwrap())
            .field(3)
            .unwrap()
            .json_name(),
        "fInt32"
    );
    for name in ["valid-json-name.proto", "json-name.proto"] {
        assert!(pool.file_by_name(name).is_none());
    }
    for name in ["valid.json.Item", "json.test.Item"] {
        assert!(pool.message_index(name).is_none());
        assert!(pool.file_containing_symbol(name).is_none());
    }
}

#[test]
fn bracketed_custom_json_names_are_rejected() {
    for name in ["[x]", "[]", "[json.test.ext]", "[a][b]"] {
        let err = DescriptorPool::new(descriptor_set(file_with_json_name(Some(name))))
            .expect_err("a bracketed JSON name reads back as an extension key");
        assert_invalid_json_name_because(
            &err,
            "json.test.Item.field_name",
            name,
            ", which has the form of an extension key",
        );
    }
}

#[test]
// The option is deprecated in descriptor.proto; protoc still honours it.
#[allow(deprecated)]
fn bracketed_json_names_link_under_legacy_json_field_conflicts() {
    let mut file = file_with_json_name(Some("[x]"));
    file.message_type[0].options = buffa::MessageField::some(MessageOptions {
        deprecated_legacy_json_field_conflicts: Some(true),
        ..Default::default()
    });
    let pool = DescriptorPool::new(descriptor_set(file)).unwrap();
    let message = pool.message_by_name("json.test.Item").unwrap();
    assert_eq!(message.field(1).unwrap().json_name(), "[x]");
}

#[test]
fn bracketed_extension_json_names_link() {
    let mut file = file_with_json_name(None);
    file.syntax = Some("proto2".into());
    file.message_type[0].extension_range.push(ExtensionRange {
        start: Some(100),
        end: Some(200),
        ..Default::default()
    });
    file.extension.push(FieldDescriptorProto {
        name: Some("ext".into()),
        number: Some(100),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_INT32),
        extendee: Some(".json.test.Item".into()),
        json_name: Some("[json.test.ext]".into()),
        ..Default::default()
    });
    let pool = DescriptorPool::new(descriptor_set(file)).unwrap();
    assert!(pool.extension_by_name("json.test.ext").is_some());
}

#[test]
fn valid_custom_json_names_are_preserved() {
    for name in [
        "",
        "customName",
        "[",
        "]",
        "[open",
        "close]",
        "a[x]",
        "雪",
        "with spaces",
        "with-dash",
        "line\nfeed",
        "\u{1}",
        r"literal\0text",
    ] {
        let pool = DescriptorPool::new(descriptor_set(file_with_json_name(Some(name))))
            .expect("the name has no NUL and is not bracketed");
        let message = pool.message_by_name("json.test.Item").unwrap();
        let field = message.field(1).unwrap();
        assert_eq!(field.json_name(), name);
        assert_eq!(message.field_by_name(name).unwrap().number(), 1);
        assert_eq!(message.field_by_name("field_name").unwrap().number(), 1);
    }
}

#[test]
fn absent_json_names_are_derived() {
    let pool = DescriptorPool::new(descriptor_set(file_with_json_name(None))).unwrap();
    let message = pool.message_by_name("json.test.Item").unwrap();
    assert_eq!(message.field(1).unwrap().json_name(), "fieldName");
    assert_eq!(message.field_by_name("fieldName").unwrap().number(), 1);
}

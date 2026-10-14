#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet, OneofDescriptorProto,
};
use buffa_descriptor::{DescriptorPool, PoolError, MAX_SYMBOL_LEN};

fn field(name: &str, number: i32) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_INT32),
        ..Default::default()
    }
}

fn message(oneof: bool) -> DescriptorProto {
    let mut message = DescriptorProto {
        name: Some("Host".into()),
        field: vec![field(if oneof { "value" } else { "choice" }, 1)],
        ..Default::default()
    };
    if oneof {
        message.field[0].oneof_index = Some(0);
        message.oneof_decl.push(OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        });
    }
    message
}

fn add_declaration(message: &mut DescriptorProto, kind: &str, name: &str) {
    match kind {
        "message" => message.nested_type.push(DescriptorProto {
            name: Some(name.into()),
            ..Default::default()
        }),
        "enum" | "enum value" => message.enum_type.push(EnumDescriptorProto {
            name: Some(if kind == "enum" { name } else { "Mode" }.into()),
            value: vec![EnumValueDescriptorProto {
                name: Some(if kind == "enum value" { name } else { "ZERO" }.into()),
                number: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        }),
        _ => unreachable!(),
    }
}

fn file(
    syntax: &str,
    edition: Option<Edition>,
    nested: bool,
    host: DescriptorProto,
) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("symbols.proto".into()),
        package: Some("collision.test".into()),
        syntax: Some(syntax.into()),
        edition,
        message_type: vec![if nested {
            DescriptorProto {
                name: Some("Outer".into()),
                nested_type: vec![host],
                ..Default::default()
            }
        } else {
            host
        }],
        ..Default::default()
    }
}

fn host_name(nested: bool) -> &'static str {
    if nested {
        "collision.test.Outer.Host"
    } else {
        "collision.test.Host"
    }
}

fn syntaxes() -> [(&'static str, Option<Edition>); 4] {
    [
        ("proto2", None),
        ("proto3", None),
        ("editions", Some(Edition::EDITION_2023)),
        ("editions", Some(Edition::EDITION_2024)),
    ]
}

fn baseline() -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("baseline.proto".into()),
            package: Some("baseline.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![message(false)],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn assert_duplicate(file: FileDescriptorProto, host: &str) {
    let set = FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    };
    let symbol = format!("{host}.choice");
    let assert_error = |error: PoolError| {
        assert!(
            matches!(&error, PoolError::DuplicateName(name) if name == &symbol),
            "unexpected error: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!("duplicate symbol name {symbol:?}")
        );
    };

    assert_error(DescriptorPool::new(set.clone()).expect_err("symbol names must be unique"));
    assert_error(
        DescriptorPool::decode(&set.encode_to_vec()).expect_err("symbol names must be unique"),
    );

    let existing = baseline();
    let mut pool = DescriptorPool::new(existing.clone()).unwrap();
    assert_error(
        pool.add_file_descriptor_set(set)
            .expect_err("symbol names must be unique"),
    );
    assert_eq!(pool.files(), existing.file);
    assert_eq!(pool.messages().len(), 1);
    assert_eq!(pool.enums().len(), 0);
    assert_eq!(pool.services().len(), 0);
    assert_eq!(pool.extensions().len(), 0);
    assert!(pool.file_by_name("symbols.proto").is_none());
    assert!(pool.file_containing_symbol(&symbol).is_none());
    assert!(pool.message_by_name(host).is_none());
    assert_eq!(
        pool.message_by_name("baseline.test.Host")
            .unwrap()
            .field(1)
            .unwrap()
            .name(),
        "choice"
    );
    assert_eq!(
        pool.file_containing_symbol("baseline.test.Host.choice")
            .unwrap()
            .name
            .as_deref(),
        Some("baseline.proto")
    );
}

#[test]
fn fields_and_oneofs_cannot_reuse_nested_type_or_enum_value_names() {
    for (syntax, edition) in syntaxes() {
        for nested in [false, true] {
            for oneof in [false, true] {
                for kind in ["message", "enum", "enum value"] {
                    let mut host = message(oneof);
                    add_declaration(&mut host, kind, "choice");
                    assert_duplicate(file(syntax, edition, nested, host), host_name(nested));
                }
            }
        }
    }
}

#[test]
fn fields_and_oneofs_cannot_reuse_message_scoped_extension_names() {
    for (syntax, edition) in syntaxes()
        .into_iter()
        .filter(|(syntax, _)| *syntax != "proto3")
    {
        for nested in [false, true] {
            for oneof in [false, true] {
                let mut host = message(oneof);
                host.extension_range.push(ExtensionRange {
                    start: Some(100),
                    end: Some(200),
                    ..Default::default()
                });
                let mut extension = field("choice", 100);
                extension.extendee = Some(format!(".{}", host_name(nested)));
                host.extension.push(extension);
                assert_duplicate(file(syntax, edition, nested, host), host_name(nested));
            }
        }
    }
}

#[test]
fn synthetic_oneofs_cannot_reuse_nested_type_or_enum_value_names() {
    for nested in [false, true] {
        for kind in ["message", "enum", "enum value"] {
            let mut host = message(true);
            host.field[0].proto3_optional = Some(true);
            add_declaration(&mut host, kind, "choice");
            assert_duplicate(file("proto3", None, nested, host), host_name(nested));
        }
    }
}

#[test]
fn symbol_names_are_case_sensitive() {
    for (syntax, edition) in syntaxes() {
        for nested in [false, true] {
            for oneof in [false, true] {
                for kind in ["message", "enum", "enum value"] {
                    let mut host = message(oneof);
                    add_declaration(&mut host, kind, "Choice");
                    let pool = DescriptorPool::new(FileDescriptorSet {
                        file: vec![file(syntax, edition, nested, host)],
                        ..Default::default()
                    })
                    .unwrap();
                    for name in ["choice", "Choice"] {
                        assert_eq!(
                            pool.file_containing_symbol(&format!("{}.{name}", host_name(nested)))
                                .unwrap()
                                .name
                                .as_deref(),
                            Some("symbols.proto")
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn json_names_do_not_register_as_proto_symbols() {
    for (syntax, edition) in syntaxes() {
        for nested in [false, true] {
            for kind in ["message", "enum", "enum value"] {
                let mut host = message(false);
                host.field[0].name = Some("payload".into());
                host.field[0].json_name = Some("choice".into());
                add_declaration(&mut host, kind, "choice");
                let pool = DescriptorPool::new(FileDescriptorSet {
                    file: vec![file(syntax, edition, nested, host)],
                    ..Default::default()
                })
                .unwrap();
                let descriptor = pool.message_by_name(host_name(nested)).unwrap();
                assert_eq!(
                    descriptor.field_by_name("choice").unwrap().name(),
                    "payload"
                );
                assert!(pool
                    .file_containing_symbol(&format!("{}.payload", host_name(nested)))
                    .is_some());
            }
        }
    }
}

#[test]
fn symbol_names_can_repeat_in_different_message_scopes() {
    for (syntax, edition) in syntaxes() {
        for oneof in [false, true] {
            let mut left = message(oneof);
            left.name = Some("Left".into());
            let mut right = DescriptorProto {
                name: Some("Right".into()),
                ..Default::default()
            };
            add_declaration(&mut right, "message", "choice");
            let mut file = file(syntax, edition, false, left);
            file.message_type.push(right);
            let mut pool = DescriptorPool::new(baseline()).unwrap();
            pool.add_file_descriptor_set(FileDescriptorSet {
                file: vec![file],
                ..Default::default()
            })
            .unwrap();
            for name in ["collision.test.Left.choice", "collision.test.Right.choice"] {
                assert_eq!(
                    pool.file_containing_symbol(name).unwrap().name.as_deref(),
                    Some("symbols.proto")
                );
            }
        }
    }
}

#[test]
fn fields_and_oneofs_without_a_package_use_the_message_scope() {
    for nested in [false, true] {
        for oneof in [false, true] {
            for kind in ["message", "enum", "enum value"] {
                let mut host = message(oneof);
                add_declaration(&mut host, kind, "choice");
                let mut invalid = file("proto2", None, nested, host.clone());
                invalid.package = None;
                let name = if nested { "Outer.Host" } else { "Host" };
                assert_duplicate(invalid, name);

                if oneof {
                    host.oneof_decl[0].name = Some("selection".into());
                } else {
                    host.field[0].name = Some("selection".into());
                }
                let mut valid = file("proto2", None, nested, host);
                valid.package = None;
                let pool = DescriptorPool::new(FileDescriptorSet {
                    file: vec![valid],
                    ..Default::default()
                })
                .unwrap();
                for member in ["choice", "selection"] {
                    assert!(pool
                        .file_containing_symbol(&format!("{name}.{member}"))
                        .is_some());
                }
            }
        }
    }
}

#[test]
fn field_and_oneof_names_do_not_gain_a_length_limit() {
    for (syntax, edition) in syntaxes() {
        for nested in [false, true] {
            for oneof in [false, true] {
                for len in [
                    MAX_SYMBOL_LEN - 1,
                    MAX_SYMBOL_LEN,
                    MAX_SYMBOL_LEN + 1,
                    MAX_SYMBOL_LEN * 2,
                ] {
                    let mut host = message(oneof);
                    let name = "c".repeat(len - host_name(nested).len() - 1);
                    if oneof {
                        host.oneof_decl[0].name = Some(name.clone());
                    } else {
                        host.field[0].name = Some(name.clone());
                    }
                    let set = FileDescriptorSet {
                        file: vec![file(syntax, edition, nested, host)],
                        ..Default::default()
                    };
                    let mut added = DescriptorPool::new(baseline()).unwrap();
                    added.add_file_descriptor_set(set.clone()).unwrap();
                    for pool in [
                        DescriptorPool::new(set.clone()).unwrap(),
                        DescriptorPool::decode(&set.encode_to_vec()).unwrap(),
                        added,
                    ] {
                        let symbol = format!("{}.{name}", host_name(nested));
                        assert_eq!(symbol.len(), len);
                        assert!(pool.file_containing_symbol(&symbol).is_some());
                    }
                }
            }
        }
    }
}

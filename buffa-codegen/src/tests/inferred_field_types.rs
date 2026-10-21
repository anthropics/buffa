//! A field or extension with `type_name` set and `type` unset (#625).
//!
//! `FieldDescriptorProto.type` "need not be set" when `type_name` is, so a
//! descriptor set built by another tool can leave it out. Codegen infers it
//! from the message or enum that `type_name` names, as protoc and
//! `DescriptorPool` do.

use super::*;

fn field(name: &str, number: i32, label: Label, ty: Type, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        type_name: Some(type_name.to_string()),
        ..make_field(name, number, label, ty)
    }
}

fn extension(name: &str, number: i32, ty: Type, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        extendee: Some(".demo.Base".to_string()),
        ..field(name, number, Label::LABEL_OPTIONAL, ty, type_name)
    }
}

/// A proto2 file whose message and enum fields and extensions all carry
/// `type`, as protoc writes them.
fn typed_file() -> FileDescriptorProto {
    let message = Type::TYPE_MESSAGE;
    let enumeration = Type::TYPE_ENUM;
    let optional = Label::LABEL_OPTIONAL;
    FileDescriptorProto {
        name: Some("demo.proto".to_string()),
        package: Some("demo".to_string()),
        syntax: Some("proto2".to_string()),
        message_type: vec![
            DescriptorProto {
                name: Some("Base".to_string()),
                extension_range: vec![
                    crate::generated::descriptor::descriptor_proto::ExtensionRange {
                        start: Some(100),
                        end: Some(200),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Inner".to_string()),
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Outer".to_string()),
                field: vec![
                    field("inner", 1, optional, message, ".demo.Inner"),
                    field("color", 2, optional, enumeration, ".demo.Color"),
                    field("nested", 3, optional, message, ".demo.Outer.Nested"),
                    field("kind", 4, optional, enumeration, ".demo.Outer.Kind"),
                    field("inners", 5, Label::LABEL_REPEATED, message, ".demo.Inner"),
                    field(
                        "colors",
                        6,
                        Label::LABEL_REPEATED,
                        message,
                        ".demo.Outer.ColorsEntry",
                    ),
                ],
                nested_type: vec![
                    DescriptorProto {
                        name: Some("Nested".to_string()),
                        ..Default::default()
                    },
                    DescriptorProto {
                        name: Some("ColorsEntry".to_string()),
                        field: vec![
                            make_field("key", 1, optional, Type::TYPE_STRING),
                            field("value", 2, optional, enumeration, ".demo.Color"),
                        ],
                        options: MessageOptions {
                            map_entry: Some(true),
                            ..Default::default()
                        }
                        .into(),
                        ..Default::default()
                    },
                ],
                enum_type: vec![EnumDescriptorProto {
                    name: Some("Kind".to_string()),
                    value: vec![enum_value("K0", 0)],
                    ..Default::default()
                }],
                extension: vec![extension("nested_ext", 102, enumeration, ".demo.Color")],
                ..Default::default()
            },
        ],
        enum_type: vec![EnumDescriptorProto {
            name: Some("Color".to_string()),
            value: vec![enum_value("RED", 0)],
            ..Default::default()
        }],
        extension: vec![
            extension("ext", 100, message, ".demo.Inner"),
            extension("ext_color", 101, enumeration, ".demo.Color"),
        ],
        ..Default::default()
    }
}

/// `file` with `type` cleared on every field and extension, nested ones
/// included, that has a `type_name`.
fn untyped(mut file: FileDescriptorProto) -> FileDescriptorProto {
    fn clear(fields: &mut [FieldDescriptorProto]) {
        for f in fields {
            if f.type_name.is_some() {
                f.r#type = None;
            }
        }
    }
    fn clear_message(message: &mut DescriptorProto) {
        clear(&mut message.field);
        clear(&mut message.extension);
        message.nested_type.iter_mut().for_each(clear_message);
    }
    clear(&mut file.extension);
    file.message_type.iter_mut().for_each(clear_message);
    file
}

fn untyped_file() -> FileDescriptorProto {
    untyped(typed_file())
}

fn generate_one(file: FileDescriptorProto, config: &CodeGenConfig) -> Result<String, CodeGenError> {
    generate(&[file], &["demo.proto".to_string()], config).map(|files| joined(&files))
}

#[test]
fn an_untyped_field_generates_what_its_typed_form_does() {
    for config in [
        CodeGenConfig::default(),
        CodeGenConfig {
            generate_json: true,
            generate_text: true,
            generate_reflection: true,
            generate_reflection_vtable: true,
            ..Default::default()
        },
    ] {
        let typed = generate_one(typed_file(), &config).expect("typed set generates");
        let untyped = generate_one(untyped_file(), &config).expect("untyped set generates");
        assert_eq!(untyped, typed);
        assert!(
            !untyped.contains("f64"),
            "no field became a double:\n{untyped}"
        );
    }
}

#[test]
fn an_untyped_field_naming_no_type_is_an_error() {
    let mut file = untyped_file();
    file.message_type[2].field.push(FieldDescriptorProto {
        type_name: Some(".demo.Missing".to_string()),
        r#type: None,
        ..make_field("missing", 9, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE)
    });
    let err = generate_one(file, &CodeGenConfig::default()).expect_err("nothing to infer from");
    let message = err.to_string();
    assert!(
        message.contains("demo.Outer.missing") && message.contains(".demo.Missing"),
        "the error must name the field and its type_name: {message}"
    );
}

#[test]
fn an_untyped_extension_naming_no_type_is_an_error() {
    let mut file = untyped_file();
    file.extension.push(FieldDescriptorProto {
        r#type: None,
        ..extension("lost", 103, Type::TYPE_MESSAGE, ".demo.Missing")
    });
    let err = generate_one(file, &CodeGenConfig::default()).expect_err("nothing to infer from");
    let message = err.to_string();
    assert!(
        message.contains("demo.lost") && message.contains(".demo.Missing"),
        "the error must name the extension and its type_name: {message}"
    );
}

/// A shared corpus context precomputes which message fields are stored
/// inline from the descriptors `generate` reads, so an untyped set generates
/// the same code with a shared context as without one.
#[test]
fn a_shared_corpus_context_sees_the_inferred_types() {
    let typed = generate_one(typed_file(), &CodeGenConfig::default()).expect("typed set generates");
    let mut config = CodeGenConfig::default();
    config.shared_corpus_context = Some(SharedCorpusContext::new(&[untyped_file()], &config));
    let untyped = generate_one(untyped_file(), &config).expect("untyped set generates");
    assert_eq!(untyped, typed);
}

/// A field-path feature override reads the field's type, so it must see the
/// inferred one.
#[test]
fn a_field_override_applies_to_an_inferred_enum_field() {
    let config = CodeGenConfig {
        feature_overrides: vec![(
            ".demo.Outer.color".to_string(),
            FeatureOverride::EnumType(EnumTypeOverride::Open),
        )],
        ..Default::default()
    };
    let typed = generate_one(typed_file(), &config).expect("typed set generates");
    let untyped = generate_one(untyped_file(), &config).expect("untyped set generates");
    assert_eq!(untyped, typed);
    assert!(
        typed.contains("EnumValue<"),
        "the override opens the enum field:\n{typed}"
    );
}

/// Shared-pool mode embeds the bytes `encode_descriptor_set` returns, which
/// match what the default mode embeds: the inferred set.
#[test]
fn the_shared_descriptor_set_has_the_inferred_types() {
    assert_eq!(
        encode_descriptor_set(&[untyped_file()], &[]),
        encode_descriptor_set(&[typed_file()], &[])
    );
}

#[test]
fn a_file_without_a_package_infers_its_fields() {
    let file = FileDescriptorProto {
        name: Some("nopkg.proto".to_string()),
        syntax: Some("proto3".to_string()),
        message_type: vec![
            DescriptorProto {
                name: Some("Holder".to_string()),
                field: vec![field(
                    "item",
                    1,
                    Label::LABEL_OPTIONAL,
                    Type::TYPE_MESSAGE,
                    ".Item",
                )],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Item".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let generate_nopkg = |file: FileDescriptorProto| {
        generate(
            &[file],
            &["nopkg.proto".to_string()],
            &CodeGenConfig::default(),
        )
        .map(|files| joined(&files))
        .expect("generates")
    };
    assert_eq!(generate_nopkg(untyped(file.clone())), generate_nopkg(file));
}

#[test]
fn an_untyped_field_with_a_relative_type_name_is_an_error() {
    let mut file = untyped_file();
    file.message_type[2].field.push(FieldDescriptorProto {
        type_name: Some("Inner".to_string()),
        r#type: None,
        ..make_field("relative", 7, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE)
    });
    let err = generate_one(file, &CodeGenConfig::default()).expect_err("not fully qualified");
    let message = err.to_string();
    assert!(
        message.contains("demo.Outer.relative") && message.contains("does not start with '.'"),
        "the error must say the name lacks the leading dot: {message}"
    );
}

/// `DescriptorPool` treats an empty `type_name` as unset, so codegen does
/// not try to infer from it.
#[test]
fn an_empty_type_name_is_not_inferred() {
    let mut file = typed_file();
    file.message_type[2].field.push(FieldDescriptorProto {
        type_name: Some(String::new()),
        r#type: None,
        ..make_field("plain", 7, Label::LABEL_OPTIONAL, Type::TYPE_DOUBLE)
    });
    generate_one(file, &CodeGenConfig::default()).expect("an empty type_name is not an error");
}

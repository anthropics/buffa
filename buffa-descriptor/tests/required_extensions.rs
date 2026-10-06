#![cfg(feature = "reflect")]

use buffa::editions::FieldPresence;
use buffa::Message;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    descriptor_proto::ExtensionRange, DescriptorProto, EnumDescriptorProto,
    EnumValueDescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn extension_file(label: Option<Label>, ty: Type, depth: usize) -> FileDescriptorProto {
    let mut extension = FieldDescriptorProto {
        name: Some("flag".into()),
        number: Some(100),
        label,
        r#type: Some(ty),
        extendee: Some(".required.extension.Carrier".into()),
        ..Default::default()
    };
    extension.type_name = match ty {
        Type::TYPE_ENUM => Some(".required.extension.Status".into()),
        Type::TYPE_MESSAGE => Some(".required.extension.Payload".into()),
        Type::TYPE_GROUP => Some(
            match depth {
                0 => ".required.extension.Flag",
                1 => ".required.extension.Scope.Flag",
                _ => ".required.extension.Scope.Inner.Flag",
            }
            .into(),
        ),
        _ => None,
    };
    let mut file = FileDescriptorProto {
        name: Some("required-extension.proto".into()),
        package: Some("required.extension".into()),
        syntax: Some("proto2".into()),
        message_type: vec![
            DescriptorProto {
                name: Some("Carrier".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("value".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_REQUIRED),
                    r#type: Some(Type::TYPE_INT32),
                    ..Default::default()
                }],
                extension_range: vec![ExtensionRange {
                    start: Some(100),
                    end: Some(200),
                    ..Default::default()
                }],
                ..Default::default()
            },
            DescriptorProto {
                name: Some("Payload".into()),
                ..Default::default()
            },
        ],
        enum_type: vec![EnumDescriptorProto {
            name: Some("Status".into()),
            value: vec![EnumValueDescriptorProto {
                name: Some("UNSPECIFIED".into()),
                number: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    if depth == 0 {
        file.extension.push(extension);
        if ty == Type::TYPE_GROUP {
            file.message_type.push(DescriptorProto {
                name: Some("Flag".into()),
                ..Default::default()
            });
        }
    } else {
        let mut scope = DescriptorProto {
            name: Some(if depth == 1 { "Scope" } else { "Inner" }.into()),
            extension: vec![extension],
            ..Default::default()
        };
        if ty == Type::TYPE_GROUP {
            scope.nested_type.push(DescriptorProto {
                name: Some("Flag".into()),
                ..Default::default()
            });
        }
        if depth == 2 {
            scope = DescriptorProto {
                name: Some("Scope".into()),
                nested_type: vec![scope],
                ..Default::default()
            };
        }
        file.message_type.push(scope);
    }
    file
}

fn descriptor_set(file: FileDescriptorProto) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    }
}

fn assert_required_extension_error(err: PoolError, name: &str) {
    assert!(matches!(&err, PoolError::RequiredExtension { field } if field == name));
    assert_eq!(
        err.to_string(),
        format!("extension {name} cannot be required")
    );
}

const SCOPES: [(usize, &str); 3] = [
    (0, "required.extension.flag"),
    (1, "required.extension.Scope.flag"),
    (2, "required.extension.Scope.Inner.flag"),
];

const TYPES: [Type; 5] = [
    Type::TYPE_INT32,
    Type::TYPE_STRING,
    Type::TYPE_ENUM,
    Type::TYPE_MESSAGE,
    Type::TYPE_GROUP,
];

#[test]
fn required_extensions_are_rejected_in_every_declaration_scope() {
    for syntax in [None, Some(""), Some("proto2")] {
        for (depth, name) in SCOPES {
            for ty in TYPES {
                let mut file = extension_file(Some(Label::LABEL_REQUIRED), ty, depth);
                file.syntax = syntax.map(Into::into);
                let result = DescriptorPool::new(descriptor_set(file));
                let err = match result {
                    Err(err) => err,
                    Ok(_) => {
                        panic!("required extension {name} must be rejected ({syntax:?}, {ty:?})")
                    }
                };
                assert_required_extension_error(err, name);
            }
        }
    }
}

#[test]
fn decoding_required_extension_descriptors_returns_an_error() {
    for (depth, name) in SCOPES {
        let bytes = descriptor_set(extension_file(
            Some(Label::LABEL_REQUIRED),
            Type::TYPE_INT32,
            depth,
        ))
        .encode_to_vec();
        let result = DescriptorPool::decode(&bytes);
        let err = match result {
            Err(err) => err,
            Ok(_) => panic!("decoded required extension {name} must be rejected"),
        };
        assert_required_extension_error(err, name);
    }
}

#[test]
fn rejected_required_extensions_leave_an_existing_pool_unchanged() {
    for (depth, name) in SCOPES {
        let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
        let counts = (
            pool.files().len(),
            pool.messages().len(),
            pool.enums().len(),
            pool.services().len(),
            pool.extensions().len(),
        );
        let mut file = extension_file(Some(Label::LABEL_REQUIRED), Type::TYPE_INT32, depth);
        file.extension.insert(
            0,
            FieldDescriptorProto {
                name: Some("earlier".into()),
                number: Some(101),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                extendee: Some(".required.extension.Carrier".into()),
                ..Default::default()
            },
        );
        let err = pool
            .add_file_descriptor_set(descriptor_set(file))
            .unwrap_err();
        assert_required_extension_error(err, name);
        assert_eq!(
            (
                pool.files().len(),
                pool.messages().len(),
                pool.enums().len(),
                pool.services().len(),
                pool.extensions().len(),
            ),
            counts
        );
        assert!(pool.file_by_name("required-extension.proto").is_none());
        assert!(pool.message_by_name("required.extension.Carrier").is_none());
        assert!(pool.enum_by_name("required.extension.Status").is_none());
        assert!(pool.extension_by_name(name).is_none());
        assert!(pool
            .extension_by_name("required.extension.earlier")
            .is_none());
        assert!(pool.file_containing_symbol(name).is_none());
        assert!(pool.message_by_name("reflect.test.Scalars").is_some());

        pool.add_file_descriptor_set(descriptor_set(extension_file(
            Some(Label::LABEL_OPTIONAL),
            Type::TYPE_INT32,
            depth,
        )))
        .expect("the same extension can be added after correcting its label");
        assert!(pool.extension_by_name(name).is_some());
    }
}

#[test]
fn optional_repeated_and_unset_extension_labels_are_accepted() {
    for syntax in [None, Some(""), Some("proto2")] {
        for label in [
            None,
            Some(Label::LABEL_OPTIONAL),
            Some(Label::LABEL_REPEATED),
        ] {
            for (depth, name) in SCOPES {
                for ty in TYPES {
                    let mut file = extension_file(label, ty, depth);
                    file.syntax = syntax.map(Into::into);
                    let pool = DescriptorPool::new(descriptor_set(file))
                        .expect("extensions may be optional or repeated");
                    assert!(pool.extension_by_name(name).is_some());
                    assert_eq!(
                        pool.message_by_name("required.extension.Carrier")
                            .unwrap()
                            .field(1)
                            .unwrap()
                            .presence(),
                        FieldPresence::LegacyRequired
                    );
                }
            }
        }
    }
}

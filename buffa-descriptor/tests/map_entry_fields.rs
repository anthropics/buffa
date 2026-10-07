#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    MessageOptions, OneofDescriptorProto,
};
use buffa_descriptor::reflect::{DynamicMessage, Value};
use buffa_descriptor::{DescriptorPool, FieldKind, PoolError, ScalarType, SingularKind};

fn entry() -> DescriptorProto {
    DescriptorProto {
        name: Some("EntriesEntry".into()),
        field: vec![
            FieldDescriptorProto {
                name: Some("key".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("value".into()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                ..Default::default()
            },
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

fn descriptor_set(entry: DescriptorProto) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("map-entry-fields.proto".into()),
            package: Some("map.entry.test".into()),
            syntax: Some("proto2".into()),
            message_type: vec![DescriptorProto {
                name: Some("Holder".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("entries".into()),
                    number: Some(1),
                    label: Some(Label::LABEL_REPEATED),
                    r#type: Some(Type::TYPE_MESSAGE),
                    type_name: Some(".map.entry.test.Holder.EntriesEntry".into()),
                    ..Default::default()
                }],
                nested_type: vec![entry],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn assert_malformed(entry: DescriptorProto) {
    let set = descriptor_set(entry);
    let assert_error = |error: PoolError| {
        assert!(
            matches!(error, PoolError::MalformedMapEntry { ref message }
                if message == "map.entry.test.Holder.entries"),
            "unexpected error: {error}"
        );
    };
    assert_error(DescriptorPool::new(set.clone()).unwrap_err());

    let mut pool = DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap();
    let counts = (
        pool.messages().len(),
        pool.enums().len(),
        pool.services().len(),
        pool.extensions().len(),
        pool.files().len(),
    );
    assert_error(pool.add_file_descriptor_set(set).unwrap_err());
    assert_eq!(
        counts,
        (
            pool.messages().len(),
            pool.enums().len(),
            pool.services().len(),
            pool.extensions().len(),
            pool.files().len(),
        )
    );
    assert!(pool.file_by_name("map-entry-fields.proto").is_none());
    assert!(pool.message_by_name("map.entry.test.Holder").is_none());
    assert!(pool
        .file_containing_symbol("map.entry.test.Holder.EntriesEntry")
        .is_none());
    assert_eq!(
        pool.message_by_name("reflect.test.Scalars")
            .unwrap()
            .field(3)
            .unwrap()
            .name(),
        "f_int32"
    );
}

#[test]
fn map_entries_require_exactly_key_and_value_fields() {
    for fields in [
        vec![],
        vec![entry().field.remove(0)],
        vec![entry().field.remove(1)],
        {
            let mut fields = entry().field;
            let mut extra = fields[1].clone();
            extra.name = Some("extra".into());
            extra.number = Some(3);
            fields.push(extra);
            fields
        },
    ] {
        assert_malformed(DescriptorProto {
            field: fields,
            ..entry()
        });
    }

    for index in 0..2 {
        let mut invalid = entry();
        invalid.field[index].name = Some("other".into());
        assert_malformed(invalid);

        let mut invalid = entry();
        invalid.field[index].number = Some(3);
        assert_malformed(invalid);
    }
}

#[test]
fn map_entry_key_must_be_declared_before_value() {
    let mut reversed = entry();
    reversed.field.reverse();
    assert_malformed(reversed);
}

#[test]
fn map_entry_fields_must_be_optional() {
    for index in 0..2 {
        for label in [Label::LABEL_REPEATED, Label::LABEL_REQUIRED] {
            let mut invalid = entry();
            invalid.field[index].label = Some(label);
            assert_malformed(invalid);
        }
    }
}

#[test]
fn proto2_map_entry_fields_accept_oneofs_and_explicit_defaults() {
    for index in 0..2 {
        let mut compatible = entry();
        compatible.oneof_decl.push(OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        });
        compatible.field[index].oneof_index = Some(0);
        let pool = DescriptorPool::new(descriptor_set(compatible)).unwrap();
        assert!(matches!(
            pool.message_by_name("map.entry.test.Holder")
                .unwrap()
                .field(1)
                .unwrap()
                .kind(),
            FieldKind::Map { .. }
        ));

        let mut compatible = entry();
        compatible.field[index].default_value = Some(if index == 0 { "key" } else { "7" }.into());
        let pool = DescriptorPool::new(descriptor_set(compatible)).unwrap();
        assert!(matches!(
            pool.message_by_name("map.entry.test.Holder")
                .unwrap()
                .field(1)
                .unwrap()
                .kind(),
            FieldKind::Map { .. }
        ));
    }
}

#[test]
fn valid_map_entry_fields_link_and_round_trip() {
    for (syntax, edition) in [
        ("proto2", None),
        ("proto3", None),
        ("editions", Some(Edition::EDITION_2023)),
        ("editions", Some(Edition::EDITION_2024)),
    ] {
        for omit_labels in [false, true] {
            let mut valid = entry();
            if omit_labels {
                for field in &mut valid.field {
                    field.label = None;
                }
            }
            let mut set = descriptor_set(valid);
            set.file[0].syntax = Some(syntax.into());
            set.file[0].edition = edition;
            let pool = Arc::new(DescriptorPool::new(set).unwrap());
            let holder = pool.message_by_name("map.entry.test.Holder").unwrap();
            assert_eq!(
                holder.field(1).unwrap().kind(),
                FieldKind::Map {
                    key: ScalarType::String,
                    value: SingularKind::Scalar(ScalarType::Int32),
                }
            );
            let index = pool.message_index("map.entry.test.Holder").unwrap();
            let wire = [0x0a, 5, 0x0a, 1, b'k', 0x10, 7];
            let message = DynamicMessage::decode(pool, index, &wire).unwrap();
            let Some(Value::Map(map)) = message.field_by_number(1) else {
                panic!("expected a map");
            };
            assert_eq!(map.get_str("k"), Some(&Value::I32(7)));
            assert_eq!(message.encode_to_vec(), wire);
        }
    }
}

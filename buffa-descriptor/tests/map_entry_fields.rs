#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::Message;
use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet, MessageOptions, OneofDescriptorProto,
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
    assert_malformed_set(descriptor_set(entry));
}

fn assert_malformed_set(set: FileDescriptorSet) {
    let assert_error = |error: PoolError| {
        assert!(
            matches!(error, PoolError::MalformedMapEntry { ref message }
                if message == "map.entry.test.Holder.entries"),
            "unexpected error: {error}"
        );
    };
    assert_error(DescriptorPool::new(set.clone()).unwrap_err());
    assert_error(DescriptorPool::decode(&set.encode_to_vec()).unwrap_err());

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

fn entries_with_declarations() -> [DescriptorProto; 4] {
    let mut nested_message = entry();
    nested_message.nested_type.push(DescriptorProto {
        name: Some("Nested".into()),
        ..Default::default()
    });

    let mut nested_enum = entry();
    nested_enum.enum_type.push(EnumDescriptorProto {
        name: Some("Nested".into()),
        value: vec![EnumValueDescriptorProto {
            name: Some("ZERO".into()),
            number: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    });

    let mut extension_range = entry();
    extension_range.extension_range.push(ExtensionRange {
        start: Some(100),
        end: Some(101),
        ..Default::default()
    });

    let mut extension = entry();
    extension.extension.push(FieldDescriptorProto {
        name: Some("extra".into()),
        number: Some(100),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_INT32),
        extendee: Some(".map.entry.test.Holder".into()),
        ..Default::default()
    });

    [nested_message, nested_enum, extension_range, extension]
}

fn set_with_declarations(entry: DescriptorProto) -> FileDescriptorSet {
    let declares_extension = !entry.extension.is_empty();
    let mut set = descriptor_set(entry);
    if declares_extension {
        set.file[0].message_type[0]
            .extension_range
            .push(ExtensionRange {
                start: Some(100),
                end: Some(101),
                ..Default::default()
            });
    }
    set
}

#[test]
fn map_entries_must_not_contain_declarations() {
    for (syntax, edition) in [
        ("proto2", None),
        ("proto3", None),
        ("editions", Some(Edition::EDITION_2023)),
        ("editions", Some(Edition::EDITION_2024)),
    ] {
        for entry in entries_with_declarations() {
            if syntax == "proto3"
                && (!entry.extension.is_empty() || !entry.extension_range.is_empty())
            {
                continue;
            }
            let mut set = set_with_declarations(entry);
            set.file[0].syntax = Some(syntax.into());
            set.file[0].edition = edition;
            assert_malformed_set(set);
        }
    }
}

#[test]
fn ordinary_messages_can_contain_declarations() {
    for mut entry in entries_with_declarations() {
        entry.options = MessageOptions {
            map_entry: Some(false),
            ..Default::default()
        }
        .into();
        let set = set_with_declarations(entry);
        let pool = DescriptorPool::new(set.clone()).unwrap();
        assert!(matches!(
            pool.message_by_name("map.entry.test.Holder")
                .unwrap()
                .field(1)
                .unwrap()
                .kind(),
            FieldKind::List(SingularKind::Message(_))
        ));
        assert!(DescriptorPool::decode(&set.encode_to_vec()).is_ok());
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

fn entry_field(name: &str, number: i32, label: Label) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(label),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(".map.entry.test.Holder.EntriesEntry".into()),
        ..Default::default()
    }
}

/// `field` has `Holder.EntriesEntry` as its type and is not its map field;
/// `expected_entry` is the entry a map field of that name requires.
fn assert_invalid_map_entry_use(set: FileDescriptorSet, field: &str, expected_entry: &str) {
    for error in [
        DescriptorPool::new(set.clone()).unwrap_err(),
        DescriptorPool::decode(&set.encode_to_vec()).unwrap_err(),
    ] {
        assert!(
            matches!(&error, PoolError::InvalidMapEntryUse { field: f, entry, expected_entry: e }
                if f == field
                    && entry == "map.entry.test.Holder.EntriesEntry"
                    && e == expected_entry),
            "{field}: unexpected error: {error}"
        );
    }
}

/// The pool rejects a group whose type is a map entry, even one named for the
/// entry (`PoolError::InvalidMapEntryUse` says what protobuf does with it).
#[test]
fn a_group_of_the_map_entry_type_does_not_link() {
    let mut set = descriptor_set(entry());
    set.file[0].message_type[0].field[0].r#type = Some(Type::TYPE_GROUP);
    assert_invalid_map_entry_use(
        set.clone(),
        "map.entry.test.Holder.entries",
        "map.entry.test.Holder.EntriesEntry",
    );
    // The entry has the name the field requires, so the text is about the
    // field's shape.
    assert_eq!(
        DescriptorPool::new(set).unwrap_err().to_string(),
        "field map.entry.test.Holder.entries has type map.entry.test.Holder.EntriesEntry, a \
         map entry message, but is not its map field: a map field is a repeated message field, \
         not a group, of the message that declares the entry; make the field one, or clear \
         map_entry on map.entry.test.Holder.EntriesEntry"
    );
}

#[test]
fn map_entry_rules_hold_in_a_nested_message() {
    let mut set = descriptor_set(entry());
    let holder = set.file[0].message_type.remove(0);
    set.file[0].message_type.push(DescriptorProto {
        name: Some("Outer".into()),
        nested_type: vec![holder],
        ..Default::default()
    });
    let entries = &mut set.file[0].message_type[0].nested_type[0].field[0];
    entries.type_name = Some(".map.entry.test.Outer.Holder.EntriesEntry".into());
    let pool = DescriptorPool::new(set.clone()).unwrap();
    assert!(matches!(
        pool.message_by_name("map.entry.test.Outer.Holder")
            .unwrap()
            .field(1)
            .unwrap()
            .kind(),
        FieldKind::Map { .. }
    ));
    // The linked entry keeps its options.
    assert_eq!(
        pool.message_by_name("map.entry.test.Outer.Holder.EntriesEntry")
            .unwrap()
            .options()
            .and_then(|options| options.map_entry),
        Some(true)
    );

    set.file[0].message_type[0].nested_type[0].field[0].name = Some("items".into());
    let error = DescriptorPool::new(set).unwrap_err();
    assert!(
        matches!(&error, PoolError::InvalidMapEntryUse { field, entry, expected_entry }
            if field == "map.entry.test.Outer.Holder.items"
                && entry == "map.entry.test.Outer.Holder.EntriesEntry"
                && expected_entry == "map.entry.test.Outer.Holder.ItemsEntry"),
        "unexpected error: {error}"
    );
    assert_eq!(
        error.to_string(),
        "field map.entry.test.Outer.Holder.items has type \
         map.entry.test.Outer.Holder.EntriesEntry, a map entry message, but is not its map \
         field: a map entry can be the type of one field only, the repeated message field it is \
         named for, and the entry of a map field with this name is \
         map.entry.test.Outer.Holder.ItemsEntry; give the field an entry of that name, or clear \
         map_entry on map.entry.test.Outer.Holder.EntriesEntry if it is not a map's entry"
    );
}

/// protoc lets a map entry be the type of one field only: the repeated field
/// of the entry's containing message that the entry is named for (#637).
#[test]
fn a_second_field_of_the_map_entry_type_does_not_link() {
    for label in [Label::LABEL_REPEATED, Label::LABEL_OPTIONAL] {
        let mut set = descriptor_set(entry());
        set.file[0].message_type[0]
            .field
            .push(entry_field("again", 2, label));
        assert_invalid_map_entry_use(
            set,
            "map.entry.test.Holder.again",
            "map.entry.test.Holder.AgainEntry",
        );
    }
}

#[test]
fn a_field_named_for_another_entry_does_not_link() {
    let mut set = descriptor_set(entry());
    set.file[0].message_type[0].field[0].name = Some("items".into());
    assert_invalid_map_entry_use(
        set,
        "map.entry.test.Holder.items",
        "map.entry.test.Holder.ItemsEntry",
    );
}

/// Only the first character and each one after a `_` are upper-cased, so
/// `entrIes` would own `EntrIesEntry`, not `EntriesEntry`.
#[test]
fn a_field_whose_name_differs_from_the_entry_in_case_does_not_link() {
    let mut set = descriptor_set(entry());
    set.file[0].message_type[0].field[0].name = Some("entrIes".into());
    assert_invalid_map_entry_use(
        set,
        "map.entry.test.Holder.entrIes",
        "map.entry.test.Holder.EntrIesEntry",
    );
}

#[test]
fn a_singular_field_named_for_the_entry_does_not_link() {
    let mut set = descriptor_set(entry());
    set.file[0].message_type[0].field[0].label = Some(Label::LABEL_OPTIONAL);
    assert_invalid_map_entry_use(
        set,
        "map.entry.test.Holder.entries",
        "map.entry.test.Holder.EntriesEntry",
    );
}

/// The other message is linked before and after `Holder`, so the check does
/// not depend on whether the entry itself has been linked yet.
#[test]
fn a_field_of_another_message_typed_as_the_entry_does_not_link() {
    for label in [Label::LABEL_REPEATED, Label::LABEL_OPTIONAL] {
        for other_first in [false, true] {
            let mut set = descriptor_set(entry());
            let other = DescriptorProto {
                name: Some("Other".into()),
                field: vec![entry_field("entries", 1, label)],
                ..Default::default()
            };
            if other_first {
                set.file[0].message_type.insert(0, other);
            } else {
                set.file[0].message_type.push(other);
            }
            assert_invalid_map_entry_use(
                set,
                "map.entry.test.Other.entries",
                "map.entry.test.Other.EntriesEntry",
            );
        }
    }
}

fn entry_extension(name: &str, label: Label, extendee: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        extendee: Some(extendee.into()),
        ..entry_field(name, 100, label)
    }
}

/// `Holder` with its map, a message `Other`, an extension range on both, and
/// the extension `ext`.
fn set_with_extension(ext: FieldDescriptorProto) -> FileDescriptorSet {
    let range = ExtensionRange {
        start: Some(100),
        end: Some(101),
        ..Default::default()
    };
    let mut set = descriptor_set(entry());
    set.file[0].message_type[0]
        .extension_range
        .push(range.clone());
    set.file[0].message_type.push(DescriptorProto {
        name: Some("Other".into()),
        extension_range: vec![range],
        ..Default::default()
    });
    set.file[0].extension.push(ext);
    set
}

/// An extension whose type is a map entry does not link, whatever its shape.
/// That includes the one protoc 33.5 compiles, `extend Holder { repeated
/// Holder.EntriesEntry entries = 100; }`, which passes the checks a map field
/// passes with the extendee as its message.
#[test]
fn an_extension_typed_as_a_map_entry_does_not_link() {
    let holder = ".map.entry.test.Holder";
    let compiled = || entry_extension("entries", Label::LABEL_REPEATED, holder);
    let group = FieldDescriptorProto {
        r#type: Some(Type::TYPE_GROUP),
        ..compiled()
    };
    let inferred = FieldDescriptorProto {
        r#type: None,
        ..compiled()
    };
    // Declared in `Other`, so its full name is `Other.entries`.
    let mut nested = set_with_extension(compiled());
    let ext = nested.file[0].extension.remove(0);
    nested.file[0].message_type[1].extension.push(ext);

    for (shape, set, field_name) in [
        (
            "the one protoc compiles",
            set_with_extension(compiled()),
            "map.entry.test.entries",
        ),
        (
            "as a group",
            set_with_extension(group),
            "map.entry.test.entries",
        ),
        (
            "singular",
            set_with_extension(entry_extension("entries", Label::LABEL_OPTIONAL, holder)),
            "map.entry.test.entries",
        ),
        (
            "named for another entry",
            set_with_extension(entry_extension("items", Label::LABEL_REPEATED, holder)),
            "map.entry.test.items",
        ),
        (
            "of another message",
            set_with_extension(entry_extension(
                "entries",
                Label::LABEL_REPEATED,
                ".map.entry.test.Other",
            )),
            "map.entry.test.entries",
        ),
        (
            "type inferred",
            set_with_extension(inferred),
            "map.entry.test.entries",
        ),
        (
            "declared in a message",
            nested,
            "map.entry.test.Other.entries",
        ),
    ] {
        for error in [
            DescriptorPool::new(set.clone()).unwrap_err(),
            DescriptorPool::decode(&set.encode_to_vec()).unwrap_err(),
        ] {
            assert!(
                matches!(&error, PoolError::ExtensionWithMapEntryType { field, entry }
                    if field == field_name && entry == "map.entry.test.Holder.EntriesEntry"),
                "{shape}: unexpected error: {error}"
            );
            assert_eq!(
                error.to_string(),
                format!(
                    "extension {field_name} has type map.entry.test.Holder.EntriesEntry, a map \
                     entry message; a map entry can be the type of its map field only, and an \
                     extension is never a map field (protoc accepts one such extension, the \
                     pool rejects all); give the extension a message type without map_entry"
                )
            );
        }
    }
}

/// The entry name is the field name with each `_` dropped and the character
/// after it ASCII upper-cased, as protoc's `MapEntryName` builds it. The case
/// of every other character is kept.
#[test]
fn a_map_field_links_under_the_entry_name_protoc_derives() {
    for (field, entry_name) in [
        ("entries", "EntriesEntry"),
        ("string_to_int", "StringToIntEntry"),
        ("_leading", "LeadingEntry"),
        ("double__under", "DoubleUnderEntry"),
        ("trailing_", "TrailingEntry"),
        ("v2_items", "V2ItemsEntry"),
        ("already_Upper", "AlreadyUpperEntry"),
        ("a_1b", "A1bEntry"),
        ("FOO_BAR", "FOOBAREntry"),
    ] {
        let mut set = descriptor_set(DescriptorProto {
            name: Some(entry_name.into()),
            ..entry()
        });
        let holder_field = &mut set.file[0].message_type[0].field[0];
        holder_field.name = Some(field.into());
        holder_field.type_name = Some(format!(".map.entry.test.Holder.{entry_name}"));
        let pool = DescriptorPool::new(set).unwrap();
        assert!(
            matches!(
                pool.message_by_name("map.entry.test.Holder")
                    .unwrap()
                    .field(1)
                    .unwrap()
                    .kind(),
                FieldKind::Map { .. }
            ),
            "{field}"
        );
    }
}

#![cfg(feature = "reflect")]

use buffa_descriptor::generated::descriptor::feature_set::{EnumType, FieldPresence};
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, EnumDescriptorProto, EnumOptions, EnumValueDescriptorProto,
    FeatureSet, FieldDescriptorProto, FieldOptions, FileDescriptorProto, FileDescriptorSet,
    FileOptions, MessageOptions, OneofDescriptorProto,
};
use buffa_descriptor::{DescriptorPool, PoolError};

fn field() -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some("value".into()),
        number: Some(1),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_ENUM),
        type_name: Some(".presence.test.Kind".into()),
        ..Default::default()
    }
}

fn set(field: FieldDescriptorProto, enum_type: EnumType) -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("presence.proto".into()),
            package: Some("presence.test".into()),
            syntax: Some("editions".into()),
            edition: Some(Edition::EDITION_2023),
            options: FileOptions {
                features: FeatureSet {
                    field_presence: Some(FieldPresence::IMPLICIT),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            message_type: vec![DescriptorProto {
                name: Some("Sample".into()),
                field: vec![field],
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("Kind".into()),
                value: vec![EnumValueDescriptorProto {
                    name: Some("ZERO".into()),
                    number: Some(0),
                    ..Default::default()
                }],
                options: EnumOptions {
                    features: FeatureSet {
                        enum_type: Some(enum_type),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn implicit_closed_enum_is_rejected_transactionally() {
    let mut pool = DescriptorPool::new(set(field(), EnumType::OPEN)).unwrap();
    let mut invalid = set(field(), EnumType::CLOSED);
    invalid.file[0].name = Some("invalid.proto".into());
    invalid.file[0].package = Some("invalid.test".into());
    invalid.file[0].message_type[0].field[0].type_name = Some(".invalid.test.Kind".into());
    let err = pool.add_file_descriptor_set(invalid).unwrap_err();
    assert!(
        matches!(&err, PoolError::ImplicitPresenceClosedEnum { field } if field == "invalid.test.Sample.value")
    );
    assert_eq!(
        err.to_string(),
        "field invalid.test.Sample.value has implicit presence and uses a closed enum"
    );
    assert_eq!(pool.files().len(), 1);
    assert_eq!(pool.messages().len(), 1);
    assert_eq!(pool.enums().len(), 1);
    assert!(pool.file_by_name("invalid.proto").is_none());
    assert!(pool.message_by_name("invalid.test.Sample").is_none());
    assert!(pool.enum_by_name("invalid.test.Kind").is_none());
}

#[test]
fn implicit_closed_enum_in_later_import_is_rejected() {
    let mut descriptors = set(field(), EnumType::CLOSED);
    let mut enum_type = descriptors.file[0].enum_type.remove(0);
    enum_type.options = Default::default();
    descriptors.file[0].dependency.push("enum.proto".into());
    descriptors.file.push(FileDescriptorProto {
        name: Some("enum.proto".into()),
        package: Some("presence.test".into()),
        syntax: Some("proto2".into()),
        enum_type: vec![enum_type],
        ..Default::default()
    });
    let err = DescriptorPool::new(descriptors).unwrap_err();
    assert!(
        matches!(err, PoolError::ImplicitPresenceClosedEnum { field }
        if field == "presence.test.Sample.value")
    );
}

#[test]
fn closed_enum_with_explicit_or_required_presence_is_accepted() {
    for presence in [FieldPresence::EXPLICIT, FieldPresence::LEGACY_REQUIRED] {
        let mut field = field();
        field.options = FieldOptions {
            features: FeatureSet {
                field_presence: Some(presence),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
        .into();
        DescriptorPool::new(set(field, EnumType::CLOSED)).unwrap();
    }
}

#[test]
fn repeated_closed_enum_is_accepted() {
    let mut field = field();
    field.label = Some(Label::LABEL_REPEATED);
    DescriptorPool::new(set(field, EnumType::CLOSED)).unwrap();
}

#[test]
fn oneof_closed_enum_is_accepted() {
    let mut field = field();
    field.oneof_index = Some(0);
    let mut descriptors = set(field, EnumType::CLOSED);
    descriptors.file[0].message_type[0]
        .oneof_decl
        .push(OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        });
    DescriptorPool::new(descriptors).unwrap();
}

#[test]
fn implicit_open_enum_is_accepted() {
    DescriptorPool::new(set(field(), EnumType::OPEN)).unwrap();
}

/// `features.enum_type = OPEN` on a field makes that field open whatever its
/// enum declares. protoc rejects the option on a field (it targets files and
/// enums only), so protoc output never carries it. The codegen pass that
/// applies an enum-type feature override to single fields writes it into the
/// descriptor set that generated code embeds, and the pool that links that
/// set must accept the field.
#[test]
fn field_level_open_enum_override_is_accepted() {
    let mut field = field();
    field.options = FieldOptions {
        features: FeatureSet {
            enum_type: Some(EnumType::OPEN),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();
    DescriptorPool::new(set(field, EnumType::CLOSED)).unwrap();
}

#[test]
fn closed_enum_extension_is_accepted_under_implicit_file_presence() {
    use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
    let mut field = field();
    field.extendee = Some(".presence.test.Sample".into());
    let mut descriptors = set(field.clone(), EnumType::CLOSED);
    let message = &mut descriptors.file[0].message_type[0];
    message.field.clear();
    message.extension_range.push(ExtensionRange {
        start: Some(1),
        end: Some(2),
        ..Default::default()
    });
    descriptors.file[0].extension.push(field);
    DescriptorPool::new(descriptors).unwrap();
}

#[test]
fn nested_implicit_closed_enum_is_rejected() {
    let mut descriptors = set(field(), EnumType::CLOSED);
    let mut nested = descriptors.file[0].message_type.remove(0);
    nested.name = Some("Child".into());
    descriptors.file[0].message_type.push(DescriptorProto {
        name: Some("Parent".into()),
        nested_type: vec![nested],
        ..Default::default()
    });
    let err = DescriptorPool::new(descriptors).unwrap_err();
    assert!(
        matches!(err, PoolError::ImplicitPresenceClosedEnum { field }
        if field == "presence.test.Parent.Child.value")
    );
}

/// `set` with `Sample.value` declared as `map<string, Kind>`: a repeated
/// field of the nested `ValueEntry`, whose `value` field names the enum.
fn map_set(enum_type: EnumType) -> FileDescriptorSet {
    let mut descriptors = set(field(), enum_type);
    let message = &mut descriptors.file[0].message_type[0];
    let mut value = message.field.remove(0);
    value.number = Some(2);
    message.nested_type.push(DescriptorProto {
        name: Some("ValueEntry".into()),
        field: vec![
            FieldDescriptorProto {
                name: Some("key".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                ..Default::default()
            },
            value,
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    });
    message.field.push(FieldDescriptorProto {
        name: Some("value".into()),
        number: Some(1),
        label: Some(Label::LABEL_REPEATED),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(".presence.test.Sample.ValueEntry".into()),
        ..Default::default()
    });
    descriptors
}

/// Turns the editions fixture into a proto3 file that imports its enum from
/// a proto2 file, where the enum is closed.
fn into_proto3_with_proto2_enum(mut descriptors: FileDescriptorSet) -> FileDescriptorSet {
    let mut enum_type = descriptors.file[0].enum_type.remove(0);
    enum_type.options = Default::default();
    let file = &mut descriptors.file[0];
    file.syntax = Some("proto3".into());
    file.edition = None;
    file.options = Default::default();
    file.dependency.push("enum.proto".into());
    descriptors.file.push(FileDescriptorProto {
        name: Some("enum.proto".into()),
        package: Some("presence.test".into()),
        syntax: Some("proto2".into()),
        enum_type: vec![enum_type],
        ..Default::default()
    });
    descriptors
}

#[test]
fn closed_enum_map_value_is_rejected_under_implicit_file_presence() {
    let err = DescriptorPool::new(map_set(EnumType::CLOSED)).unwrap_err();
    assert!(
        matches!(&err, PoolError::ImplicitPresenceClosedEnum { field }
        if field == "presence.test.Sample.ValueEntry.value"),
        "{err}"
    );
}

#[test]
fn closed_enum_map_value_is_accepted_when_the_entry_has_explicit_presence() {
    // Edition 2023 defaults to explicit presence, which the entry inherits.
    let mut descriptors = map_set(EnumType::CLOSED);
    descriptors.file[0].options = Default::default();
    DescriptorPool::new(descriptors).unwrap();
}

#[test]
fn open_enum_map_value_is_accepted_under_implicit_file_presence() {
    DescriptorPool::new(map_set(EnumType::OPEN)).unwrap();
}

#[test]
fn closed_enum_is_rejected_in_proto3_singular_fields_and_map_values() {
    for (descriptors, expected) in [
        (set(field(), EnumType::CLOSED), "presence.test.Sample.value"),
        (
            map_set(EnumType::CLOSED),
            "presence.test.Sample.ValueEntry.value",
        ),
    ] {
        let err = DescriptorPool::new(into_proto3_with_proto2_enum(descriptors)).unwrap_err();
        assert!(
            matches!(&err, PoolError::ImplicitPresenceClosedEnum { field } if field == expected),
            "{expected}: {err}"
        );
    }
}

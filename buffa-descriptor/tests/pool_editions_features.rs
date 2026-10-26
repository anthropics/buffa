#![cfg(feature = "reflect")]

//! protoc's rules for the features and legacy options that a field or file of
//! an editions file sets itself.

use buffa_descriptor::generated::descriptor::feature_set::{
    FieldPresence, MessageEncoding, Utf8Validation,
};
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, Edition, FeatureSet, FieldDescriptorProto, FieldOptions, FileDescriptorProto,
    FileDescriptorSet, FileOptions, MessageOptions, OneofDescriptorProto,
};
use buffa_descriptor::{DescriptorPool, PoolError};

/// An edition 2023 file with the message `rules.test.Sample`, which holds
/// `fields` and a nested message `Child`.
fn file(fields: Vec<FieldDescriptorProto>) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("rules.proto".into()),
        package: Some("rules.test".into()),
        syntax: Some("editions".into()),
        edition: Some(Edition::EDITION_2023),
        message_type: vec![DescriptorProto {
            name: Some("Sample".into()),
            field: fields,
            nested_type: vec![DescriptorProto {
                name: Some("Child".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Field number 1 of `Sample`. A message or group field has type `Child`.
fn field(name: &str, label: Label, ty: Type) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(1),
        label: Some(label),
        r#type: Some(ty),
        type_name: matches!(ty, Type::TYPE_MESSAGE | Type::TYPE_GROUP)
            .then(|| ".rules.test.Sample.Child".into()),
        ..Default::default()
    }
}

fn with_features(mut field: FieldDescriptorProto, features: FeatureSet) -> FieldDescriptorProto {
    field.options = FieldOptions {
        features: features.into(),
        ..Default::default()
    }
    .into();
    field
}

fn presence(presence: FieldPresence) -> FeatureSet {
    FeatureSet {
        field_presence: Some(presence),
        ..Default::default()
    }
}

fn utf8_none() -> FeatureSet {
    FeatureSet {
        utf8_validation: Some(Utf8Validation::NONE),
        ..Default::default()
    }
}

fn encoding(encoding: MessageEncoding) -> FeatureSet {
    FeatureSet {
        message_encoding: Some(encoding),
        ..Default::default()
    }
}

/// A file whose `Sample` holds `map<key, value> values`, as protoc writes it:
/// the map field's features are copied onto the entry's `key` and `value`.
fn map_file(key: Type, value: Type, features: FeatureSet) -> FileDescriptorProto {
    let entry_field = |name: &str, number, ty| FieldDescriptorProto {
        number: Some(number),
        ..with_features(field(name, Label::LABEL_OPTIONAL, ty), features.clone())
    };
    let mut file = file(vec![with_features(
        FieldDescriptorProto {
            type_name: Some(".rules.test.Sample.ValuesEntry".into()),
            ..field("values", Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
        },
        features.clone(),
    )]);
    file.message_type[0].nested_type.push(DescriptorProto {
        name: Some("ValuesEntry".into()),
        field: vec![entry_field("key", 1, key), entry_field("value", 2, value)],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    });
    file
}

/// A file that extends `Sample` with the optional `int32` extension
/// `rules.test.flag` (number 100), which sets `features`.
fn extension_file(features: FeatureSet) -> FileDescriptorProto {
    let mut file = file(vec![]);
    file.message_type[0]
        .extension_range
        .push(Default::default());
    file.message_type[0].extension_range[0].start = Some(100);
    file.message_type[0].extension_range[0].end = Some(200);
    file.extension.push(with_features(
        FieldDescriptorProto {
            number: Some(100),
            extendee: Some(".rules.test.Sample".into()),
            ..field("flag", Label::LABEL_OPTIONAL, Type::TYPE_INT32)
        },
        features,
    ));
    file
}

fn link(file: FileDescriptorProto) -> Result<DescriptorPool, PoolError> {
    DescriptorPool::new(FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    })
}

#[test]
fn group_field_is_rejected() {
    for edition in [Edition::EDITION_2023, Edition::EDITION_2024] {
        let mut file = file(vec![field(
            "child",
            Label::LABEL_OPTIONAL,
            Type::TYPE_GROUP,
        )]);
        file.edition = Some(edition);
        let err = link(file).unwrap_err();
        assert!(
            matches!(&err, PoolError::GroupFieldInEditions { field } if field == "rules.test.Sample.child"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "field rules.test.Sample.child uses a group type in an editions file"
        );
    }
    // An edition number this crate does not define is a later edition.
    let mut future = file(vec![field(
        "child",
        Label::LABEL_OPTIONAL,
        Type::TYPE_GROUP,
    )]);
    future.edition = None;
    future.__buffa_unknown_fields.push(buffa::UnknownField {
        number: 14,
        data: buffa::UnknownFieldData::Varint(5000),
    });
    let err = link(future).unwrap_err();
    assert!(
        matches!(&err, PoolError::GroupFieldInEditions { field } if field == "rules.test.Sample.child"),
        "{err:?}"
    );
    let mut extension = extension_file(FeatureSet::default());
    extension.extension[0].r#type = Some(Type::TYPE_GROUP);
    extension.extension[0].type_name = Some(".rules.test.Sample.Child".into());
    let err = link(extension).unwrap_err();
    assert!(
        matches!(&err, PoolError::GroupFieldInEditions { field } if field == "rules.test.flag"),
        "{err:?}"
    );
    // The editions spelling of a group links.
    let delimited = with_features(
        field("child", Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE),
        encoding(MessageEncoding::DELIMITED),
    );
    link(file(vec![delimited])).unwrap();
}

#[test]
fn packed_option_is_rejected() {
    // A string field reports this rule too, not `InvalidPackedOption`.
    for ty in [Type::TYPE_INT32, Type::TYPE_STRING] {
        for packed in [true, false] {
            let mut value = field("value", Label::LABEL_REPEATED, ty);
            value.options = FieldOptions {
                packed: Some(packed),
                ..Default::default()
            }
            .into();
            let err = link(file(vec![value])).unwrap_err();
            assert!(
                matches!(&err, PoolError::PackedOptionInEditions { field } if field == "rules.test.Sample.value"),
                "{err:?}"
            );
            assert_eq!(
                err.to_string(),
                "field rules.test.Sample.value sets the packed option in an editions file"
            );
        }
    }
}

#[test]
fn edition_before_2023_keeps_the_legacy_rules() {
    // protoc holds a file to the editions rules by its edition, whatever its
    // syntax says.
    let mut packed = field("values", Label::LABEL_REPEATED, Type::TYPE_INT32);
    packed.options = FieldOptions {
        packed: Some(true),
        ..Default::default()
    }
    .into();
    let group = FieldDescriptorProto {
        number: Some(2),
        ..field("child", Label::LABEL_OPTIONAL, Type::TYPE_GROUP)
    };
    let mut legacy = file(vec![packed.clone(), group]);
    legacy.edition = Some(Edition::EDITION_PROTO2);
    link(legacy).unwrap();
    // protoc rejects a group under the proto3 rules, so only the option here.
    let mut legacy = file(vec![packed]);
    legacy.edition = Some(Edition::EDITION_PROTO3);
    link(legacy).unwrap();
}

#[test]
fn inherited_features_link_on_every_field() {
    // protoc checks only the features that a field sets itself, so file
    // defaults apply to the fields they fit and pass over the rest.
    let mut file = map_file(Type::TYPE_INT32, Type::TYPE_MESSAGE, FeatureSet::default());
    let numbered = |name: &str, number, label, ty| FieldDescriptorProto {
        number: Some(number),
        ..field(name, label, ty)
    };
    let mut member = numbered("member", 6, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
    member.oneof_index = Some(0);
    file.message_type[0].field.extend([
        numbered("number", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        numbered("blob", 3, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
        numbered("child", 4, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE),
        numbered("numbers", 5, Label::LABEL_REPEATED, Type::TYPE_INT32),
        member,
    ]);
    file.message_type[0].oneof_decl.push(OneofDescriptorProto {
        name: Some("choice".into()),
        ..Default::default()
    });
    let extension = extension_file(FeatureSet::default());
    file.message_type[0].extension_range = extension.message_type[0].extension_range.clone();
    file.extension = extension.extension;
    file.options = FileOptions {
        features: FeatureSet {
            field_presence: Some(FieldPresence::IMPLICIT),
            utf8_validation: Some(Utf8Validation::NONE),
            message_encoding: Some(MessageEncoding::DELIMITED),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();
    link(file).unwrap();
}

#[test]
fn rejected_file_leaves_the_pool_unchanged() {
    let mut pool = link(file(vec![])).unwrap();
    let other = |mut file: FileDescriptorProto| {
        file.name = Some("other.proto".into());
        file.package = Some("other.test".into());
        file
    };
    let mut packed = field("values", Label::LABEL_REPEATED, Type::TYPE_INT32);
    packed.options = FieldOptions {
        packed: Some(true),
        ..Default::default()
    }
    .into();
    let mut required = file(vec![]);
    required.options = FileOptions {
        features: presence(FieldPresence::LEGACY_REQUIRED).into(),
        ..Default::default()
    }
    .into();
    for invalid in [other(file(vec![packed])), other(required)] {
        let err = pool
            .add_file_descriptor_set(FileDescriptorSet {
                file: vec![invalid],
                ..Default::default()
            })
            .unwrap_err();
        assert!(
            matches!(
                err,
                PoolError::PackedOptionInEditions { .. }
                    | PoolError::LegacyRequiredFileDefault { .. }
            ),
            "{err:?}"
        );
        assert_eq!(pool.files().len(), 1);
        assert!(pool.file_by_name("other.proto").is_none());
        assert!(pool.message_by_name("other.test.Sample").is_none());
    }
}

#[test]
fn field_presence_is_rejected_where_it_cannot_apply() {
    let mut member = with_features(
        field("value", Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        presence(FieldPresence::EXPLICIT),
    );
    member.oneof_index = Some(0);
    let mut oneof = file(vec![member]);
    oneof.message_type[0].oneof_decl.push(OneofDescriptorProto {
        name: Some("choice".into()),
        ..Default::default()
    });
    let repeated = file(vec![with_features(
        field("value", Label::LABEL_REPEATED, Type::TYPE_INT32),
        presence(FieldPresence::EXPLICIT),
    )]);
    let map = map_file(
        Type::TYPE_STRING,
        Type::TYPE_INT32,
        presence(FieldPresence::EXPLICIT),
    );
    let message = file(vec![with_features(
        field("value", Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE),
        presence(FieldPresence::IMPLICIT),
    )]);
    let cases = [
        (oneof, "rules.test.Sample.value"),
        (repeated, "rules.test.Sample.value"),
        (map, "rules.test.Sample.values"),
        (message, "rules.test.Sample.value"),
        (
            extension_file(presence(FieldPresence::EXPLICIT)),
            "rules.test.flag",
        ),
        (
            extension_file(presence(FieldPresence::IMPLICIT)),
            "rules.test.flag",
        ),
    ];
    for (file, name) in cases {
        let err = link(file).unwrap_err();
        assert!(
            matches!(&err, PoolError::InvalidFieldPresence { field } if field == name),
            "{name}: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!(
                "field {name} sets features.field_presence, but it is a oneof member, repeated, \
                 an extension, or a message field set to IMPLICIT"
            )
        );
    }
    // A required extension keeps its own error.
    let err = link(extension_file(presence(FieldPresence::LEGACY_REQUIRED))).unwrap_err();
    assert!(
        matches!(&err, PoolError::RequiredExtension { field } if field == "rules.test.flag"),
        "{err:?}"
    );
}

#[test]
fn field_presence_links_where_it_applies() {
    for (ty, presence_value) in [
        (Type::TYPE_INT32, FieldPresence::IMPLICIT),
        (Type::TYPE_INT32, FieldPresence::EXPLICIT),
        (Type::TYPE_INT32, FieldPresence::LEGACY_REQUIRED),
        (Type::TYPE_MESSAGE, FieldPresence::EXPLICIT),
        (Type::TYPE_MESSAGE, FieldPresence::LEGACY_REQUIRED),
    ] {
        let value = with_features(
            field("value", Label::LABEL_OPTIONAL, ty),
            presence(presence_value),
        );
        link(file(vec![value])).unwrap();
    }
}

#[test]
fn utf8_validation_is_rejected_on_non_string_fields() {
    let mut files = [Type::TYPE_INT32, Type::TYPE_BYTES, Type::TYPE_MESSAGE]
        .map(|ty| {
            file(vec![with_features(
                field("value", Label::LABEL_OPTIONAL, ty),
                utf8_none(),
            )])
        })
        .to_vec();
    files.push(file(vec![with_features(
        field("value", Label::LABEL_REPEATED, Type::TYPE_BYTES),
        utf8_none(),
    )]));
    for file in files {
        let err = link(file).unwrap_err();
        assert!(
            matches!(&err, PoolError::InvalidUtf8Validation { field } if field == "rules.test.Sample.value"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "field rules.test.Sample.value sets features.utf8_validation but is not a string field \
             or a map with a string key or value"
        );
    }
    let err = link(map_file(Type::TYPE_INT32, Type::TYPE_INT32, utf8_none())).unwrap_err();
    assert!(
        matches!(&err, PoolError::InvalidUtf8Validation { field } if field == "rules.test.Sample.values"),
        "{err:?}"
    );
}

#[test]
fn utf8_validation_links_on_string_fields_and_string_maps() {
    for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
        let value = with_features(field("value", label, Type::TYPE_STRING), utf8_none());
        link(file(vec![value])).unwrap();
    }
    // The entry's other field carries the feature too and is exempt.
    for (key, value) in [
        (Type::TYPE_STRING, Type::TYPE_INT32),
        (Type::TYPE_INT32, Type::TYPE_STRING),
        (Type::TYPE_STRING, Type::TYPE_MESSAGE),
    ] {
        link(map_file(key, value, utf8_none())).unwrap();
    }
}

#[test]
fn message_encoding_is_rejected_on_non_message_fields_and_maps() {
    let files = [
        file(vec![with_features(
            field("value", Label::LABEL_OPTIONAL, Type::TYPE_INT32),
            encoding(MessageEncoding::DELIMITED),
        )]),
        file(vec![with_features(
            field("value", Label::LABEL_REPEATED, Type::TYPE_STRING),
            encoding(MessageEncoding::LENGTH_PREFIXED),
        )]),
    ];
    for file in files {
        let err = link(file).unwrap_err();
        assert!(
            matches!(&err, PoolError::InvalidMessageEncoding { field } if field == "rules.test.Sample.value"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "field rules.test.Sample.value sets features.message_encoding but is a map or not a \
             message field"
        );
    }
    let map = map_file(
        Type::TYPE_STRING,
        Type::TYPE_MESSAGE,
        encoding(MessageEncoding::DELIMITED),
    );
    let err = link(map).unwrap_err();
    assert!(
        matches!(&err, PoolError::InvalidMessageEncoding { field } if field == "rules.test.Sample.values"),
        "{err:?}"
    );
}

#[test]
fn message_encoding_links_on_message_fields() {
    for label in [Label::LABEL_OPTIONAL, Label::LABEL_REPEATED] {
        for value in [MessageEncoding::DELIMITED, MessageEncoding::LENGTH_PREFIXED] {
            let child = with_features(field("child", label, Type::TYPE_MESSAGE), encoding(value));
            link(file(vec![child])).unwrap();
        }
    }
}

#[test]
fn legacy_required_file_default_is_rejected() {
    let with_file_presence = |value| {
        let mut file = file(vec![field(
            "value",
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )]);
        file.options = FileOptions {
            features: presence(value).into(),
            ..Default::default()
        }
        .into();
        file
    };
    let err = link(with_file_presence(FieldPresence::LEGACY_REQUIRED)).unwrap_err();
    assert!(
        matches!(&err, PoolError::LegacyRequiredFileDefault { file } if file == "rules.proto"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "file rules.proto sets features.field_presence = LEGACY_REQUIRED, which only a field may set"
    );
    for value in [FieldPresence::EXPLICIT, FieldPresence::IMPLICIT] {
        link(with_file_presence(value)).unwrap();
    }
}

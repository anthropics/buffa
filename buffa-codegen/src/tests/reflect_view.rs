//! Vtable-mode reflection codegen: config validation and emitted impls.

use super::*;
use crate::ReflectMode;

#[test]
fn reflect_mode_maps_to_config_flags() {
    let mut c = CodeGenConfig::default();

    ReflectMode::Off.apply(&mut c);
    assert!(!c.generate_reflection && !c.generate_reflection_vtable);

    ReflectMode::Bridge.apply(&mut c);
    assert!(c.generate_reflection && !c.generate_reflection_vtable);

    ReflectMode::VTable.apply(&mut c);
    assert!(c.generate_reflection && c.generate_reflection_vtable);
}

/// A config with both bridge reflection and vtable mode enabled.
fn vtable_config() -> CodeGenConfig {
    CodeGenConfig {
        generate_reflection: true,
        generate_reflection_vtable: true,
        ..Default::default()
    }
}

/// A small proto3 message with a scalar and a string field.
fn msg_file() -> FileDescriptorProto {
    let mut file = proto3_file("vt.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Msg".to_string()),
        field: vec![
            make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
            make_field("name", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    });
    file
}

#[test]
fn vtable_emits_reflect_message_and_element_impls() {
    let files = generate(&[msg_file()], &["vt.proto".to_string()], &vtable_config())
        .expect("should generate");
    let content = joined(&files);

    assert!(
        content.contains("impl<'a> ::buffa_descriptor::reflect::ReflectMessage for MsgView<'a>"),
        "vtable mode must emit ReflectMessage for the view: {content}"
    );
    assert!(
        content.contains("impl<'a> ::buffa_descriptor::reflect::ReflectElement for MsgView<'a>"),
        "vtable mode must emit ReflectElement for the view: {content}"
    );
    // The memoized index accessor is an inherent associated fn (collision-free
    // across sibling views).
    assert!(
        content.contains("fn __buffa_reflect_message_index()"),
        "vtable mode must emit the memoized MessageIndex accessor: {content}"
    );
}

#[test]
fn vtable_without_reflection_is_rejected() {
    let config = CodeGenConfig {
        generate_reflection: false,
        generate_reflection_vtable: true,
        ..Default::default()
    };
    let err = generate(&[msg_file()], &["vt.proto".to_string()], &config)
        .expect_err("vtable without reflection must error");
    assert!(
        err.to_string().contains("generate_reflection"),
        "error should name the missing prerequisite: {err}"
    );
}

#[test]
fn vtable_without_views_emits_owned_only() {
    // Owned vtable is self-contained, so views-off + vtable is allowed: it
    // emits the owned `impl ReflectMessage` but no view impls (there are no
    // views).
    let config = CodeGenConfig {
        generate_reflection: true,
        generate_reflection_vtable: true,
        generate_views: false,
        ..Default::default()
    };
    let files =
        generate(&[msg_file()], &["vt.proto".to_string()], &config).expect("should generate");
    let content = joined(&files);
    assert!(
        content.contains("impl ::buffa_descriptor::reflect::ReflectMessage for Msg"),
        "owned ReflectMessage must be emitted: {content}"
    );
    assert!(
        !content.contains("ReflectMessage for MsgView"),
        "no view impls when views are off: {content}"
    );
}

#[test]
fn bridge_only_does_not_emit_vtable_impls() {
    let config = CodeGenConfig {
        generate_reflection: true,
        generate_reflection_vtable: false,
        ..Default::default()
    };
    let files =
        generate(&[msg_file()], &["vt.proto".to_string()], &config).expect("should generate");
    let content = joined(&files);
    // Bridge mode emits Reflectable on the owned type but no ReflectMessage on
    // the view.
    assert!(
        content.contains("impl ::buffa_descriptor::reflect::Reflectable for Msg"),
        "bridge mode must still emit Reflectable: {content}"
    );
    assert!(
        !content.contains("ReflectMessage for MsgView"),
        "bridge-only must not emit the vtable ReflectMessage impl: {content}"
    );
}

#[test]
fn to_dynamic_scales_its_memory_bounds_and_floors_them_at_the_defaults() {
    // `to_dynamic` cannot report a failure, so its re-decode must not be able
    // to reject bytes the encoder on the line above just produced. The two
    // decoders measure the same message in different units, so the defaults
    // can reject; `usize::MAX` would instead leave the conversion with no
    // ceiling at all. Scaling to the input and flooring at the default is the
    // narrow path between those, and both halves are load-bearing — without
    // the floor a small message gets a *tighter* bound than a plain decode.
    let files = generate(&[msg_file()], &["vt.proto".to_string()], &vtable_config())
        .expect("should generate");
    let flat: String = joined(&files)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    assert!(
        flat.contains(".saturating_mul(128)"),
        "the element bound must scale with the encoded length"
    );
    assert!(
        flat.contains(".max(::buffa::DEFAULT_ELEMENT_MEMORY_LIMIT)"),
        "the scaled element bound must be floored at the default, not capped by it"
    );
    assert!(
        flat.contains(".max(::buffa::DEFAULT_UNKNOWN_FIELD_LIMIT)"),
        "the unknown-field bound must be floored at the default too"
    );
    assert!(
        !flat.contains("with_element_memory_limit(usize::MAX)"),
        "an unbounded re-decode would leave decode_view -> to_dynamic with no ceiling"
    );
}

#[test]
fn implicit_float_has_arms_compare_bit_patterns() {
    // `-0.0 != 0.0` is false, so a value comparison would report a field
    // holding negative zero as unset although the encoder writes it.
    let mut file = proto3_file("vt_float.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Floats".to_string()),
        field: vec![
            make_field("ratio", 1, Label::LABEL_OPTIONAL, Type::TYPE_FLOAT),
            make_field("weight", 2, Label::LABEL_OPTIONAL, Type::TYPE_DOUBLE),
        ],
        ..Default::default()
    });
    let files = generate(&[file], &["vt_float.proto".to_string()], &vtable_config())
        .expect("should generate");
    let flat: String = joined(&files)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    for arm in [
        "1u32=>self.ratio.to_bits()!=0,",
        "2u32=>self.weight.to_bits()!=0,",
    ] {
        assert_eq!(
            flat.matches(arm).count(),
            2,
            "the owned and the view `has` must each emit `{arm}`"
        );
    }
}

#[test]
fn owned_required_has_arms_are_true() {
    // An owned required field is stored bare and always encoded, so the
    // owned `has` reports it set (#627). The view keeps wire presence through
    // its `has_*` accessor, and a required message field keeps `is_set`.
    use crate::generated::descriptor::{
        feature_set::FieldPresence, Edition, FeatureSet, FieldOptions,
    };

    let mut message = make_field("child", 5, Label::LABEL_REQUIRED, Type::TYPE_MESSAGE);
    message.type_name = Some(".Req".to_string());
    let proto2 = FileDescriptorProto {
        name: Some("vt_required.proto".to_string()),
        syntax: Some("proto2".to_string()),
        message_type: vec![DescriptorProto {
            name: Some("Req".to_string()),
            field: vec![
                make_field("n", 1, Label::LABEL_REQUIRED, Type::TYPE_INT32),
                make_field("s", 2, Label::LABEL_REQUIRED, Type::TYPE_STRING),
                make_field("b", 3, Label::LABEL_REQUIRED, Type::TYPE_BYTES),
                make_field("x", 4, Label::LABEL_REQUIRED, Type::TYPE_DOUBLE),
                message,
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut legacy = make_field("n", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32);
    legacy.options = FieldOptions {
        features: FeatureSet {
            field_presence: Some(FieldPresence::LEGACY_REQUIRED),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();
    let editions = FileDescriptorProto {
        name: Some("vt_legacy_required.proto".to_string()),
        package: Some("ed".to_string()),
        syntax: Some("editions".to_string()),
        edition: Some(Edition::EDITION_2023),
        message_type: vec![DescriptorProto {
            name: Some("Legacy".to_string()),
            field: vec![legacy],
            ..Default::default()
        }],
        ..Default::default()
    };

    for (file, name, numbers) in [
        (proto2, "vt_required.proto", &[1u32, 2, 3, 4][..]),
        (editions, "vt_legacy_required.proto", &[1u32][..]),
    ] {
        let files =
            generate(&[file], &[name.to_string()], &vtable_config()).expect("should generate");
        let flat: String = joined(&files)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        for number in numbers {
            assert_eq!(
                flat.matches(&format!("{number}u32=>true,")).count(),
                1,
                "{name}: the owned `has` must report field {number} set"
            );
        }
        if name == "vt_required.proto" {
            assert_eq!(
                flat.matches("5u32=>self.child.is_set(),").count(),
                1,
                "the owned `has` of a required message field keeps `is_set`"
            );
        }
        // Any `true` arm brings the allow: an all-`true` match is
        // `clippy::match_like_matches_macro`.
        assert_eq!(
            flat.matches("#[allow(clippy::match_like_matches_macro)]fnhas(")
                .count(),
            1,
            "{name}: the owned `has` must allow the lint"
        );
    }

    // A message without required fields keeps the plain `has`.
    let files = generate(&[msg_file()], &["vt.proto".to_string()], &vtable_config())
        .expect("should generate");
    assert!(!joined(&files).contains("match_like_matches_macro"));
}

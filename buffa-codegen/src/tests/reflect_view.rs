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

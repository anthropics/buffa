//! A `type_name` without a leading dot is resolved against the enclosing
//! scope, innermost first, the way protoc and protobuf-go resolve it. Only
//! a leading dot makes a name fully qualified.

#![cfg(feature = "reflect")]

use buffa_descriptor::generated::descriptor::descriptor_proto::ExtensionRange;
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto,
};
use buffa_descriptor::{DescriptorPool, FieldKind, PoolError, SingularKind};

fn message_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.into()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(type_name.into()),
        ..Default::default()
    }
}

fn plain_message(name: &str) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.into()),
        ..Default::default()
    }
}

fn file(name: &str, package: Option<&str>, syntax: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.into()),
        package: package.map(str::to_string),
        syntax: Some(syntax.into()),
        ..Default::default()
    }
}

fn pool_with(files: Vec<FileDescriptorProto>) -> DescriptorPool {
    DescriptorPool::new(FileDescriptorSet {
        file: files,
        ..Default::default()
    })
    .expect("descriptor set links")
}

fn resolved_message_name(pool: &DescriptorPool, holder: &str, number: u32) -> String {
    let holder = pool.message_by_name(holder).expect("holder is linked");
    let field = holder.field(number).expect("field exists");
    match field.kind() {
        FieldKind::Singular(SingularKind::Message(idx)) => {
            pool.message(idx).full_name().to_string()
        }
        other => panic!("expected a message field, got {other:?}"),
    }
}

#[test]
fn dotless_name_resolves_in_its_own_package() {
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("m", 1, "M"));
    a.message_type = vec![plain_message("M"), holder];

    let pool = pool_with(vec![a]);
    assert_eq!(resolved_message_name(&pool, "a.Holder", 1), "a.M");
}

#[test]
fn dotless_name_prefers_the_innermost_scope() {
    // A root-level `M` also exists; the reference from inside package `a`
    // must still link to `a.M`, not to the fully-qualified `M`.
    let mut root = file("root.proto", None, "proto3");
    root.message_type = vec![plain_message("M")];
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("m", 1, "M"));
    a.message_type = vec![plain_message("M"), holder];
    a.dependency = vec!["root.proto".into()];

    let pool = pool_with(vec![root, a]);
    assert_eq!(resolved_message_name(&pool, "a.Holder", 1), "a.M");
}

#[test]
fn dotless_name_resolves_against_the_enclosing_message() {
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("inner", 1, "Inner"));
    let mut outer = plain_message("Outer");
    outer.nested_type = vec![plain_message("Inner"), holder];
    a.message_type = vec![outer];

    let pool = pool_with(vec![a]);
    assert_eq!(
        resolved_message_name(&pool, "a.Outer.Holder", 1),
        "a.Outer.Inner"
    );
}

#[test]
fn dotless_name_falls_back_to_wider_scopes() {
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("m", 1, "M"));
    let mut outer = plain_message("Outer");
    outer.nested_type = vec![holder];
    a.message_type = vec![plain_message("M"), outer];

    let pool = pool_with(vec![a]);
    assert_eq!(resolved_message_name(&pool, "a.Outer.Holder", 1), "a.M");
}

#[test]
fn dotless_name_with_dots_resolves_from_the_scope() {
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut sub = plain_message("Sub");
    sub.nested_type = vec![plain_message("M")];
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("m", 1, "Sub.M"));
    a.message_type = vec![sub, holder];

    let pool = pool_with(vec![a]);
    assert_eq!(resolved_message_name(&pool, "a.Holder", 1), "a.Sub.M");
}

#[test]
fn dotless_enum_name_resolves_in_its_own_package() {
    let mut a = file("a.proto", Some("a"), "proto3");
    a.enum_type = vec![EnumDescriptorProto {
        name: Some("E".into()),
        value: vec![EnumValueDescriptorProto {
            name: Some("E_UNSPECIFIED".into()),
            number: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut holder = plain_message("Holder");
    holder.field.push(FieldDescriptorProto {
        name: Some("e".into()),
        number: Some(1),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_ENUM),
        type_name: Some("E".into()),
        ..Default::default()
    });
    a.message_type = vec![holder];

    let pool = pool_with(vec![a]);
    let holder = pool.message_by_name("a.Holder").expect("holder is linked");
    match holder.field(1).expect("field exists").kind() {
        FieldKind::Singular(SingularKind::Enum(idx)) => {
            assert_eq!(pool.enumeration(idx).full_name(), "a.E");
        }
        other => panic!("expected an enum field, got {other:?}"),
    }
}

#[test]
fn dotless_method_types_resolve_against_the_service_scope() {
    let mut a = file("a.proto", Some("a"), "proto3");
    a.message_type = vec![plain_message("M")];
    a.service = vec![ServiceDescriptorProto {
        name: Some("Svc".into()),
        method: vec![MethodDescriptorProto {
            name: Some("Call".into()),
            input_type: Some("M".into()),
            output_type: Some("M".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let pool = pool_with(vec![a]);
    let svc = pool.service_by_name("a.Svc").expect("service is linked");
    let method = svc.method("Call").expect("method exists");
    assert_eq!(pool.message(method.input()).full_name(), "a.M");
    assert_eq!(pool.message(method.output()).full_name(), "a.M");
}

#[test]
fn dotless_extendee_resolves_in_its_own_package() {
    let mut a = file("a.proto", Some("a"), "proto2");
    let mut extendable = plain_message("Extendable");
    extendable.extension_range = vec![ExtensionRange {
        start: Some(100),
        end: Some(200),
        ..Default::default()
    }];
    a.message_type = vec![extendable];
    a.extension = vec![FieldDescriptorProto {
        name: Some("note".into()),
        number: Some(100),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_STRING),
        extendee: Some("Extendable".into()),
        ..Default::default()
    }];

    let pool = pool_with(vec![a]);
    let ext = pool
        .extension_by_name("a.note")
        .expect("extension is linked");
    assert_eq!(pool.message(ext.extendee()).full_name(), "a.Extendable");
}

#[test]
fn dotted_name_stays_fully_qualified() {
    let mut root = file("root.proto", None, "proto3");
    root.message_type = vec![plain_message("M")];
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("m", 1, ".M"));
    a.message_type = vec![plain_message("M"), holder];
    a.dependency = vec!["root.proto".into()];

    let pool = pool_with(vec![root, a]);
    assert_eq!(resolved_message_name(&pool, "a.Holder", 1), "M");
}

#[test]
fn unresolvable_dotless_name_still_reports_the_written_name() {
    let mut a = file("a.proto", Some("a"), "proto3");
    let mut holder = plain_message("Holder");
    holder.field.push(message_field("m", 1, "Nope"));
    a.message_type = vec![holder];

    let err = match DescriptorPool::new(FileDescriptorSet {
        file: vec![a],
        ..Default::default()
    }) {
        Ok(_) => panic!("an unknown name does not link"),
        Err(err) => err,
    };
    assert!(
        matches!(&err, PoolError::UnresolvedTypeName { type_name, .. } if type_name == "Nope"),
        "got {err:?}",
    );
}

//! End-to-end tests for custom-option access, `Any` pack/unpack, and
//! symbol→file lookup, against a protoc-compiled descriptor set that
//! includes `descriptor.proto` and `any.proto`.

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::Message;
use buffa_descriptor::reflect::{
    AnyError, DynamicMessage, ReflectMessage, ReflectMessageMut, Value,
};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test_options.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

#[cfg(feature = "json")]
fn pool_with_empty() -> Arc<DescriptorPool> {
    use buffa_descriptor::generated::descriptor::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    let mut p = DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS");
    p.add_file_descriptor_set(FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("google/protobuf/empty.proto".into()),
            package: Some("google.protobuf".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Empty".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .expect("pool accepts the hand-built Empty descriptor");
    Arc::new(p)
}

#[cfg(feature = "json")]
#[test]
fn empty_wkt_unknown_fields_follow_parse_mode() {
    let p = pool_with_empty();
    let empty_idx = p.message_index("google.protobuf.Empty").unwrap();
    let input = r#"{"futureField":{"nested":[1,2]},"anotherField":true}"#;

    let err = DynamicMessage::from_json(Arc::clone(&p), empty_idx, input).unwrap_err();
    assert!(err.to_string().contains("unexpected field on Empty"));

    let parsed = DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), empty_idx, input)
        .expect("lenient parsing must ignore unknown Empty fields");
    assert_eq!(parsed.to_json().unwrap(), "{}");
}

#[cfg(feature = "json")]
#[test]
fn empty_wkt_inside_any_unknown_fields_follow_parse_mode() {
    let p = pool_with_empty();
    let any_idx = p.message_index("google.protobuf.Any").unwrap();
    let input = r#"{
        "@type": "type.googleapis.com/google.protobuf.Empty",
        "futureField": {"nested": [1, 2]}
    }"#;

    assert!(DynamicMessage::from_json(Arc::clone(&p), any_idx, input).is_err());

    let parsed = DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), any_idx, input)
        .expect("lenient parsing must ignore unknown fields in Any<Empty>");
    assert_eq!(
        parsed.to_json().unwrap(),
        r#"{"@type":"type.googleapis.com/google.protobuf.Empty"}"#
    );
}

#[cfg(feature = "json")]
#[test]
fn any_wkt_wrapper_unknown_fields_follow_parse_mode() {
    let p = pool();
    let any_idx = p.message_index("google.protobuf.Any").unwrap();
    // Any itself has a custom JSON representation, so wrapping one inside
    // another Any exercises the `{"@type": ..., "value": ...}` WKT path.
    let input = r#"{
        "@type": "type.googleapis.com/google.protobuf.Any",
        "value": {
            "@type": "type.googleapis.com/reflect.opt.Annotated",
            "email": "a@example.com"
        },
        "futureField": 1
    }"#;

    let err = DynamicMessage::from_json(Arc::clone(&p), any_idx, input).unwrap_err();
    assert!(err.to_string().contains("unknown field \"futureField\""));

    DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), any_idx, input)
        .expect("lenient parsing must ignore unknown Any wrapper fields");
}

/// Read a custom option off a re-encoded options message: decode it as a
/// `DynamicMessage` of `options_type` and pull the extension's value. This
/// is the documented generic flow for reading a custom option by name when
/// the consumer has no compile-time knowledge of the option type.
fn read_custom_option(
    pool: &Arc<DescriptorPool>,
    options_bytes: &[u8],
    options_type: &str,
    ext_name: &str,
) -> Value {
    let idx = pool.message_index(options_type).unwrap();
    let dyn_opts = DynamicMessage::decode(Arc::clone(pool), idx, options_bytes).unwrap();
    let ext = pool
        .extension_by_name(ext_name)
        .expect("custom option registered");
    dyn_opts.get(ext.field()).to_owned()
}

#[test]
fn field_custom_option() {
    let p = pool();
    let annotated = p.message_by_name("reflect.opt.Annotated").unwrap();
    let email = annotated.field(1).unwrap();
    let opts = email.options().expect("field carries options");
    let val = read_custom_option(
        &p,
        &opts.encode_to_vec(),
        "google.protobuf.FieldOptions",
        "reflect.opt.pii_class",
    );
    assert_eq!(val, Value::String("email".into()));

    // A field with no custom option has no options at all.
    assert!(annotated.field(2).unwrap().options().is_none());
}

#[test]
fn message_custom_option() {
    let p = pool();
    let annotated = p.message_by_name("reflect.opt.Annotated").unwrap();
    let opts = annotated.options().expect("message carries options");
    let val = read_custom_option(
        &p,
        &opts.encode_to_vec(),
        "google.protobuf.MessageOptions",
        "reflect.opt.audited",
    );
    assert_eq!(val, Value::Bool(true));
}

#[test]
fn method_custom_option() {
    let p = pool();
    let svc = p.service_by_name("reflect.opt.AnnotatedService").unwrap();
    let method = svc.method("Do").unwrap();
    let opts = method.options().expect("method carries options");
    let val = read_custom_option(
        &p,
        &opts.encode_to_vec(),
        "google.protobuf.MethodOptions",
        "reflect.opt.http_path",
    );
    assert_eq!(val, Value::String("/v1/do".into()));
}

#[test]
fn any_pack_unpack_round_trip() {
    let p = pool();
    let annotated_md = p.message_by_name("reflect.opt.Annotated").unwrap();
    let ann_idx = p.message_index("reflect.opt.Annotated").unwrap();

    let mut ann = DynamicMessage::new(Arc::clone(&p), ann_idx);
    ann.set(annotated_md.field(2).unwrap(), Value::I32(7));

    let any = ann.pack_any().expect("Any is in the pool");
    assert_eq!(any.message_descriptor().full_name(), "google.protobuf.Any");
    assert_eq!(
        any.field_by_number(1),
        Some(&Value::String(
            "type.googleapis.com/reflect.opt.Annotated".into()
        ))
    );

    let back = any.unpack_any().expect("unpacks");
    assert_eq!(
        back.message_descriptor().full_name(),
        "reflect.opt.Annotated"
    );
    assert_eq!(back.field_by_number(2), Some(&Value::I32(7)));
    assert_eq!(back, ann);
}

#[test]
fn any_errors() {
    let p = pool();
    let ann_idx = p.message_index("reflect.opt.Annotated").unwrap();
    let ann = DynamicMessage::new(Arc::clone(&p), ann_idx);
    // unpack on a non-Any.
    assert!(matches!(ann.unpack_any(), Err(AnyError::NotAny { .. })));

    // An Any with no type_url at all.
    let any_idx = p.message_index("google.protobuf.Any").unwrap();
    let empty_any = DynamicMessage::new(Arc::clone(&p), any_idx);
    assert!(matches!(
        empty_any.unpack_any(),
        Err(AnyError::MissingTypeUrl)
    ));

    // An Any with an unregistered type_url.
    let any_md = p.message(any_idx);
    let mut any = DynamicMessage::new(Arc::clone(&p), any_idx);
    any.set(
        any_md.field(1).unwrap(),
        Value::String("type.googleapis.com/no.Such".into()),
    );
    assert!(matches!(
        any.unpack_any(),
        Err(AnyError::UnknownType { .. })
    ));

    // An Any with a valid type_url but malformed value bytes.
    let mut bad = DynamicMessage::new(Arc::clone(&p), any_idx);
    bad.set(
        any_md.field(1).unwrap(),
        Value::String("type.googleapis.com/reflect.opt.Annotated".into()),
    );
    // A length-delimited tag claiming more bytes than follow — a decode error.
    bad.set(any_md.field(2).unwrap(), Value::Bytes(vec![0x0a, 0xff]));
    assert!(matches!(bad.unpack_any(), Err(AnyError::Decode { .. })));
}

#[test]
fn symbol_to_file() {
    let p = pool();
    let this = "reflect_test_options.proto";
    // Message, service, method, extension all resolve to the declaring file.
    assert_eq!(
        p.file_containing_symbol("reflect.opt.Annotated")
            .and_then(|f| f.name.as_deref()),
        Some(this)
    );
    assert_eq!(
        p.file_containing_symbol("reflect.opt.AnnotatedService")
            .and_then(|f| f.name.as_deref()),
        Some(this)
    );
    assert_eq!(
        p.file_containing_symbol("reflect.opt.AnnotatedService.Do")
            .and_then(|f| f.name.as_deref()),
        Some(this),
        "method symbols resolve"
    );
    assert_eq!(
        p.file_containing_symbol("reflect.opt.pii_class")
            .and_then(|f| f.name.as_deref()),
        Some(this),
        "extension symbols resolve"
    );
    assert_eq!(
        p.file_containing_symbol("reflect.opt.Annotated.email")
            .and_then(|f| f.name.as_deref()),
        Some(this),
        "field symbols resolve"
    );
    // A WKT resolves to its own file (transitive import).
    assert_eq!(
        p.file_containing_symbol("google.protobuf.Any")
            .and_then(|f| f.name.as_deref()),
        Some("google/protobuf/any.proto")
    );
    // Leading dot accepted; unknown symbol is None.
    assert!(p.file_containing_symbol(".reflect.opt.Annotated").is_some());
    assert!(p.file_containing_symbol("reflect.opt.Nope").is_none());
}

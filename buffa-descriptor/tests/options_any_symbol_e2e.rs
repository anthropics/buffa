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

/// The reflective JSON serializer expands `Any` by decoding the payload and
/// serializing the result, so an `Any` holding an `Any` re-enters it. The
/// binary decoder cannot bound that — `Any` is flat, and the chain lives
/// inside its opaque `value` bytes — so expansion carries its own cap.
#[cfg(feature = "json")]
#[test]
fn reflective_any_json_expansion_is_depth_bounded() {
    use buffa::type_registry::MAX_ANY_EXPANSION_DEPTH;

    let p = pool();
    let any_idx = p.message_index("google.protobuf.Any").unwrap();
    let md = p.message_by_name("google.protobuf.Any").unwrap();

    // Build the chain innermost-out, each level wrapping the previous one's
    // encoded bytes.
    let mut payload: Vec<u8> = Vec::new();
    let depth = usize::try_from(MAX_ANY_EXPANSION_DEPTH).unwrap() + 5;
    for _ in 0..depth {
        let mut m = DynamicMessage::new(Arc::clone(&p), any_idx);
        m.set(
            md.field(1).unwrap(),
            Value::String("type.googleapis.com/google.protobuf.Any".into()),
        );
        m.set(md.field(2).unwrap(), Value::Bytes(payload));
        payload = m.encode_to_vec();
    }

    // Decoding is one level deep and always succeeds; the recursion is
    // entirely on the serialize side.
    let decoded = DynamicMessage::decode(Arc::clone(&p), any_idx, &payload)
        .expect("a flat two-field message decodes regardless of chain length");

    let err = decoded
        .to_json()
        .expect_err("expansion past the cap must be an error, not a deeper stack");
    assert!(
        err.to_string().contains("nested deeper"),
        "the error should say what it refused: {err}"
    );
}

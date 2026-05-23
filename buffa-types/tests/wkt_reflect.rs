//! Vtable reflection over well-known-type views.
//!
//! Verifies the generated `impl ReflectMessage for <Wkt>View` (gated behind the
//! `reflect` feature). This is the prerequisite that lets consumer protos which
//! reference WKTs reflect over them — a WKT-typed message field reflects as
//! `ValueRef::Message` borrowing the WKT view.

#![cfg(feature = "reflect")]

use buffa::{Message, MessageView};
use buffa_descriptor::reflect::{ReflectMessage, ValueRef};
use buffa_types::google::protobuf as wkt;
// `__buffa::view` / `__buffa::oneof` are the codegen-emitted module paths for
// view types and oneof enums. The double-underscore marks them as generated
// internals, but they are the canonical way to name these types until a
// friendlier re-export exists — this is the same path `wkt_roundtrip.rs` uses.
use buffa_types::google::protobuf::__buffa::oneof::value::Kind as KindOneof;
use buffa_types::google::protobuf::__buffa::view as wkt_view;

#[test]
fn timestamp_view_reflects_scalars() {
    let ts = wkt::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_456_789,
        ..Default::default()
    };
    let bytes = ts.encode_to_vec();
    let view = wkt_view::TimestampView::decode_view(&bytes).expect("decode_view");
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();
    // seconds = field 1 (int64), nanos = field 2 (int32).
    assert!(matches!(
        r.get(md.field(1).unwrap()),
        ValueRef::I64(1_700_000_000)
    ));
    assert!(matches!(
        r.get(md.field(2).unwrap()),
        ValueRef::I32(123_456_789)
    ));
    assert!(r.has(md.field(1).unwrap()));
}

#[test]
fn string_value_view_reflects_string() {
    let w = wkt::StringValue {
        value: "hello".into(),
        ..Default::default()
    };
    let bytes = w.encode_to_vec();
    let view = wkt_view::StringValueView::decode_view(&bytes).expect("decode_view");
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();
    assert!(matches!(
        r.get(md.field(1).unwrap()),
        ValueRef::String("hello")
    ));
}

#[test]
fn struct_view_reflects_map_of_nested_value_oneof() {
    // Struct.fields is map<string, Value>; Value.kind is a oneof. This
    // exercises the two trickiest WKT reflection paths together: a map whose
    // values are messages, and reflecting a nested message's oneof.
    let mut s = wkt::Struct::default();
    s.fields.insert(
        "pi".to_string(),
        wkt::Value {
            kind: Some(KindOneof::NumberValue(3.0)),
            ..Default::default()
        },
    );
    let bytes = s.encode_to_vec();
    let view = wkt_view::StructView::decode_view(&bytes).expect("decode_view");
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();

    let fields_fd = md
        .fields()
        .iter()
        .find(|f| f.name() == "fields")
        .expect("fields map");
    let ValueRef::Map(m) = r.get(fields_fd) else {
        panic!("expected Map")
    };
    assert_eq!(m.len(), 1);
    let Some(ValueRef::Message(value_cow)) = m.get_str("pi") else {
        panic!("expected nested Value message")
    };
    // Reflect the nested Value: number_value is oneof member field 2 (double).
    let value_md = value_cow.message_descriptor();
    assert!(matches!(
        value_cow.get(value_md.field(2).unwrap()),
        ValueRef::F64(3.0)
    ));
}

#[test]
fn wkt_view_to_dynamic_snapshot() {
    let d = wkt::Duration {
        seconds: 5,
        nanos: 250,
        ..Default::default()
    };
    let bytes = d.encode_to_vec();
    let view = wkt_view::DurationView::decode_view(&bytes).expect("decode_view");
    let r: &dyn ReflectMessage = &view;
    let snapshot = r.to_dynamic();
    let md = snapshot.message_descriptor();
    assert!(matches!(
        snapshot.get(md.field(1).unwrap()),
        ValueRef::I64(5)
    ));
    assert!(matches!(
        snapshot.get(md.field(2).unwrap()),
        ValueRef::I32(250)
    ));
}

// Integration tests: verify that every well-known type round-trips through
// protobuf binary encoding correctly, including zero-copy view types.

use buffa::{Message, MessageView};
use buffa_types::google::protobuf as wkt;
use buffa_types::google::protobuf::__buffa::view as wkt_view;

fn encode_decode<T: Message>(msg: &T) -> T {
    let bytes = msg.encode_to_vec();
    T::decode(&mut bytes.as_slice()).expect("decode failed")
}

// ── Timestamp ────────────────────────────────────────────────────────────────

#[test]
fn timestamp_roundtrip_positive() {
    let ts = wkt::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_456_789,
        ..Default::default()
    };
    assert_eq!(encode_decode(&ts), ts);
}

#[test]
fn timestamp_roundtrip_negative_seconds() {
    let ts = wkt::Timestamp {
        seconds: -86400,
        nanos: 500_000_000,
        ..Default::default()
    };
    assert_eq!(encode_decode(&ts), ts);
}

#[test]
fn timestamp_roundtrip_zero() {
    let ts = wkt::Timestamp::default();
    assert_eq!(encode_decode(&ts), ts);
}

// ── Duration ─────────────────────────────────────────────────────────────────

#[test]
fn duration_roundtrip() {
    let d = wkt::Duration {
        seconds: 3600,
        nanos: 999_999_999,
        ..Default::default()
    };
    assert_eq!(encode_decode(&d), d);
}

#[test]
fn duration_roundtrip_zero() {
    let d = wkt::Duration::default();
    assert_eq!(encode_decode(&d), d);
}

// ── Empty ─────────────────────────────────────────────────────────────────────

#[test]
fn empty_roundtrip() {
    let e = wkt::Empty::default();
    assert_eq!(encode_decode(&e), e);
}

#[test]
fn empty_encodes_to_zero_bytes() {
    let e = wkt::Empty::default();
    let bytes = e.encode_to_vec();
    assert!(bytes.is_empty(), "Empty must encode to zero bytes");
}

// ── FieldMask ────────────────────────────────────────────────────────────────

#[test]
fn field_mask_roundtrip() {
    let fm = wkt::FieldMask {
        paths: vec!["a.b".into(), "c.d.e".into()],
        ..Default::default()
    };
    assert_eq!(encode_decode(&fm), fm);
}

#[test]
fn field_mask_empty_paths() {
    let fm = wkt::FieldMask::default();
    assert_eq!(encode_decode(&fm), fm);
}

// ── Any ──────────────────────────────────────────────────────────────────────

#[test]
fn any_roundtrip() {
    let ts = wkt::Timestamp {
        seconds: 42,
        ..Default::default()
    };
    let any = wkt::Any::pack(&ts, "type.googleapis.com/google.protobuf.Timestamp");
    let decoded = encode_decode(&any);
    let unpacked: wkt::Timestamp = decoded.unpack_unchecked().unwrap();
    assert_eq!(unpacked, ts);
}

// ── Struct / Value / ListValue ────────────────────────────────────────────────

#[test]
fn struct_roundtrip() {
    let mut s = wkt::Struct::new();
    s.insert("number", 2.5_f64);
    s.insert("text", "hello");
    s.insert("flag", true);
    assert_eq!(encode_decode(&s), s);
}

#[test]
fn value_roundtrip_number() {
    let v = wkt::Value::from(42.0_f64);
    assert_eq!(encode_decode(&v), v);
}

#[test]
fn value_roundtrip_string() {
    let v = wkt::Value::from("hello world");
    assert_eq!(encode_decode(&v), v);
}

#[test]
fn value_roundtrip_bool() {
    let v = wkt::Value::from(false);
    assert_eq!(encode_decode(&v), v);
}

#[test]
fn value_roundtrip_null() {
    let v = wkt::Value::null();
    assert_eq!(encode_decode(&v), v);
}

#[test]
fn value_roundtrip_struct() {
    let mut inner = wkt::Struct::new();
    inner.insert("x", 1.0_f64);
    let v = wkt::Value::from(inner);
    assert_eq!(encode_decode(&v), v);
}

#[test]
fn value_roundtrip_list() {
    let l = wkt::ListValue::from_values([1.0_f64, 2.0, 3.0]);
    let v = wkt::Value::from(l);
    assert_eq!(encode_decode(&v), v);
}

#[test]
fn list_value_roundtrip() {
    let l = wkt::ListValue::from_values(["a", "b", "c"]);
    assert_eq!(encode_decode(&l), l);
}

// ── Wrapper types ─────────────────────────────────────────────────────────────

#[test]
fn bool_value_roundtrip_encoding() {
    let w = wkt::BoolValue::from(true);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn int32_value_roundtrip_encoding() {
    let w = wkt::Int32Value::from(-1000_i32);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn int64_value_roundtrip_encoding() {
    let w = wkt::Int64Value::from(i64::MIN);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn uint32_value_roundtrip_encoding() {
    let w = wkt::UInt32Value::from(u32::MAX);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn uint64_value_roundtrip_encoding() {
    let w = wkt::UInt64Value::from(u64::MAX);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn float_value_roundtrip_encoding() {
    let w = wkt::FloatValue::from(1.5_f32);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn double_value_roundtrip_encoding() {
    let w = wkt::DoubleValue::from(std::f64::consts::PI);
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn string_value_roundtrip_encoding() {
    let w = wkt::StringValue::from("buffa rocks".to_string());
    assert_eq!(encode_decode(&w), w);
}

#[test]
fn bytes_value_roundtrip_encoding() {
    let w = wkt::BytesValue::from(vec![0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(encode_decode(&w), w);
}

// ── View round-trips ──────────────────────────────────────────────────────────
//
// Verify that decode_view + to_owned_message reproduces the original message
// for a representative sample of WKT types covering different field patterns.

/// Helper: encode a message, decode as a view, convert back to owned.
fn view_roundtrip<'a, V: MessageView<'a>>(bytes: &'a [u8]) -> V::Owned {
    V::decode_view(bytes)
        .expect("decode_view")
        .to_owned_message()
}

#[test]
fn timestamp_view_roundtrip() {
    let ts = wkt::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_456_789,
        ..Default::default()
    };
    let bytes = ts.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::TimestampView>(&bytes), ts);
}

#[test]
fn duration_view_roundtrip() {
    let d = wkt::Duration {
        seconds: -3600,
        nanos: 500_000_000,
        ..Default::default()
    };
    let bytes = d.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::DurationView>(&bytes), d);
}

#[test]
fn empty_view_roundtrip() {
    let e = wkt::Empty::default();
    let bytes = e.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::EmptyView>(&bytes), e);
}

#[test]
fn any_view_roundtrip() {
    let any = wkt::Any {
        type_url: "type.googleapis.com/test.Msg".into(),
        value: vec![0x08, 0x2a].into(),
        ..Default::default()
    };
    let bytes = any.encode_to_vec();
    let owned = view_roundtrip::<wkt_view::AnyView>(&bytes);
    assert_eq!(owned.type_url, any.type_url);
    assert_eq!(owned.value, any.value);
}

#[test]
fn field_mask_view_roundtrip() {
    let fm = wkt::FieldMask {
        paths: vec!["a.b".into(), "c.d.e".into()],
        ..Default::default()
    };
    let bytes = fm.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::FieldMaskView>(&bytes), fm);
}

#[test]
fn string_value_view_roundtrip() {
    let w = wkt::StringValue {
        value: "hello view".into(),
        ..Default::default()
    };
    let bytes = w.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::StringValueView>(&bytes), w);
}

#[test]
fn bytes_value_view_roundtrip() {
    let w = wkt::BytesValue {
        value: vec![0xca, 0xfe],
        ..Default::default()
    };
    let bytes = w.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::BytesValueView>(&bytes), w);
}

#[test]
fn int64_value_view_roundtrip() {
    let w = wkt::Int64Value {
        value: i64::MIN,
        ..Default::default()
    };
    let bytes = w.encode_to_vec();
    assert_eq!(view_roundtrip::<wkt_view::Int64ValueView>(&bytes), w);
}

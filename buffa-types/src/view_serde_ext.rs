//! Hand-written `serde::Serialize` impls for well-known type **view** types.
//!
//! The owned WKT structs ([`Timestamp`](crate::Timestamp),
//! [`Duration`](crate::Duration), [`Any`](crate::Any), …) carry hand-written
//! `Serialize`/`Deserialize` impls in their respective `*_ext` modules because
//! protobuf's JSON mapping treats them specially (RFC 3339 timestamps, `"1.5s"`
//! durations, `@type`-flattened `Any`, raw JSON values, …).  The codegen
//! cannot emit these impls automatically.
//!
//! When views and JSON are both enabled in `buffa-build`, generated view types
//! reference WKT view types (`TimestampView<'_>`, …) directly, so those WKT
//! views also need `Serialize`. [`StringValueView`] and [`BytesValueView`]
//! serialize their borrowed payloads directly. The remaining impls delegate
//! to the owned type via
//! [`MessageView::to_owned_message`](buffa::MessageView::to_owned_message),
//! materializing each WKT field for parity with the owned proto3 JSON
//! encoding. The rest of the parent message stays zero-copy.
//!
//! For the flat WKTs ([`Timestamp`](crate::Timestamp),
//! [`Duration`](crate::Duration), [`FieldMask`](crate::FieldMask), the
//! scalar wrappers, [`Empty`](crate::Empty)) this copies the fields and any
//! retained unknown fields. For
//! [`Struct`](crate::Struct) / [`Value`](crate::Value) /
//! [`ListValue`](crate::ListValue) / [`Any`](crate::Any) the entire owned
//! tree is materialized before serde sees it — large nested `Struct` payloads
//! lose the zero-copy benefit on the serialize path.  Hand-rolling those four
//! impls to walk the view directly would close the gap; tracked as a
//! follow-up on the view JSON issue.
//!
//! `Deserialize` is intentionally not implemented: view types borrow from a
//! source buffer and cannot be constructed from arbitrary JSON.

use buffa::MessageView;

use crate::google::protobuf::__buffa::view::{
    AnyView, BoolValueView, BytesValueView, DoubleValueView, DurationView, EmptyView,
    FieldMaskView, FloatValueView, Int32ValueView, Int64ValueView, ListValueView, StringValueView,
    StructView, TimestampView, UInt32ValueView, UInt64ValueView, ValueView,
};

/// Implement `serde::Serialize` for a WKT view by delegating to the owned form.
macro_rules! wkt_view_serialize {
    ($($view:ident),+ $(,)?) => {
        $(
            impl serde::Serialize for $view<'_> {
                /// Serializes by converting to the owned WKT and delegating to
                /// its proto3-JSON `Serialize` impl.  Allocates the owned form
                /// for this field only; the parent message stays zero-copy.
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    self.to_owned_message()
                        .map_err(serde::ser::Error::custom)?
                        .serialize(s)
                }
            }
        )+
    };
}

wkt_view_serialize!(
    AnyView,
    BoolValueView,
    DoubleValueView,
    DurationView,
    EmptyView,
    FieldMaskView,
    FloatValueView,
    Int32ValueView,
    Int64ValueView,
    ListValueView,
    StructView,
    TimestampView,
    UInt32ValueView,
    UInt64ValueView,
    ValueView,
);

impl serde::Serialize for StringValueView<'_> {
    /// Serializes the borrowed string without constructing an owned wrapper.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.value)
    }
}

impl serde::Serialize for BytesValueView<'_> {
    /// Serializes the borrowed bytes as base64 without constructing an owned wrapper.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::bytes::serialize(self.value, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::protobuf::{
        value::Kind, BoolValue, BytesValue, DoubleValue, Duration, Empty, FieldMask, FloatValue,
        Int32Value, Int64Value, ListValue, StringValue, Struct, Timestamp, UInt32Value,
        UInt64Value, Value,
    };
    use alloc::{string::ToString, vec};
    use buffa::Message;

    /// Encode `$owned`, decode it as `$view`, serialize both to JSON, and
    /// assert the outputs match.  Returns the view's JSON string for
    /// follow-on assertions.
    macro_rules! assert_view_json_parity {
        ($view:ty, $owned:expr) => {{
            let owned = $owned;
            let bytes = owned.encode_to_vec();
            let view = <$view>::decode_view(&bytes).expect("decode_view");
            let json_owned = serde_json::to_string(&owned).expect("serialize owned");
            let json_view = serde_json::to_string(&view).expect("serialize view");
            assert_eq!(json_view, json_owned, "view JSON must match owned JSON");
            json_view
        }};
    }

    #[test]
    fn timestamp_view_serialize_matches_owned() {
        let json = assert_view_json_parity!(
            TimestampView,
            Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
                ..Default::default()
            }
        );
        // Sanity: the JSON string is actually RFC 3339, not a struct.
        assert_eq!(json, r#""2023-11-14T22:13:20.123456789Z""#);
    }

    #[test]
    fn duration_view_serialize_matches_owned() {
        let json = assert_view_json_parity!(
            DurationView,
            Duration {
                seconds: 1,
                nanos: 500_000_000,
                ..Default::default()
            }
        );
        assert_eq!(json, r#""1.500s""#);
    }

    #[test]
    fn field_mask_view_serialize_matches_owned() {
        let json = assert_view_json_parity!(
            FieldMaskView,
            FieldMask {
                paths: vec!["user.display_name".into(), "photo".into()],
                ..Default::default()
            }
        );
        assert_eq!(json, r#""user.displayName,photo""#);
    }

    #[test]
    fn wrapper_views_serialize_match_owned() {
        assert_view_json_parity!(
            BoolValueView,
            BoolValue {
                value: true,
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            BytesValueView,
            BytesValue {
                value: vec![0xDE, 0xAD],
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            DoubleValueView,
            DoubleValue {
                value: f64::INFINITY,
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            FloatValueView,
            FloatValue {
                value: 1.5,
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            Int32ValueView,
            Int32Value {
                value: -5,
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            Int64ValueView,
            Int64Value {
                value: 9_007_199_254_740_993,
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            StringValueView,
            StringValue {
                value: "hi".into(),
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            UInt32ValueView,
            UInt32Value {
                value: u32::MAX,
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            UInt64ValueView,
            UInt64Value {
                value: u64::MAX,
                ..Default::default()
            }
        );
    }

    #[test]
    fn unset_value_views_serialize_is_error() {
        let view = ValueView::decode_view(&[]).unwrap();
        assert!(serde_json::to_string(&view).is_err());

        let st = Struct::from_fields([("unset", Value::default())]);
        let bytes = st.encode_to_vec();
        let view = StructView::decode_view(&bytes).unwrap();
        assert!(serde_json::to_string(&view).is_err());

        let list = ListValue::from_values([Value::null(), Value::default()]);
        let bytes = list.encode_to_vec();
        let view = ListValueView::decode_view(&bytes).unwrap();
        assert!(serde_json::to_string(&view).is_err());
    }

    #[test]
    fn value_default_variant_views_serialize_match_owned() {
        for value in [
            Value::null(),
            Value::from(0.0_f64),
            Value::from(false),
            Value::from(""),
            Value::from(Struct::new()),
            Value::from(ListValue::default()),
        ] {
            assert_view_json_parity!(ValueView, value);
        }
    }

    #[test]
    fn struct_value_listvalue_views_serialize_match_owned() {
        assert_view_json_parity!(
            ValueView,
            Value {
                kind: Some(Kind::StringValue("x".into())),
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            ListValueView,
            ListValue {
                values: vec![
                    Value {
                        kind: Some(Kind::NumberValue(1.0)),
                        ..Default::default()
                    },
                    Value {
                        kind: Some(Kind::BoolValue(true)),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }
        );
        assert_view_json_parity!(
            StructView,
            Struct {
                fields: [(
                    "k".to_string(),
                    Value {
                        kind: Some(Kind::StringValue("v".into())),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn empty_view_serialize_matches_owned() {
        let json = assert_view_json_parity!(EmptyView, Empty::default());
        assert_eq!(json, "{}");
    }
}

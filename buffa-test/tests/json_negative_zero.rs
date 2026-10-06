use buffa::{Message, MessageView};
use buffa_test::view_json::{__buffa::view::ScalarsView, Scalars};

#[test]
fn implicit_negative_zero_survives_owned_and_view_json() {
    let message = Scalars {
        f32: -0.0,
        f64: -0.0,
        ..Default::default()
    };
    let bytes = message.encode_to_vec();
    let view = ScalarsView::decode_view(&bytes).unwrap();

    for json in [
        serde_json::to_string(&message).unwrap(),
        serde_json::to_string(&view).unwrap(),
    ] {
        assert_eq!(json, r#"{"f32":-0.0,"f64":-0.0}"#);
        let decoded: Scalars = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.f32.to_bits(), message.f32.to_bits());
        assert_eq!(decoded.f64.to_bits(), message.f64.to_bits());
    }
}

#[test]
fn implicit_positive_zero_is_omitted_from_owned_and_view_json() {
    let message = Scalars::default();
    let bytes = message.encode_to_vec();
    let view = ScalarsView::decode_view(&bytes).unwrap();

    assert_eq!(serde_json::to_string(&message).unwrap(), "{}");
    assert_eq!(serde_json::to_string(&view).unwrap(), "{}");
}

#[test]
fn implicit_float_and_double_zero_signs_are_independent() {
    for (float, double, expected) in [
        (-0.0, 0.0, r#"{"f32":-0.0}"#),
        (0.0, -0.0, r#"{"f64":-0.0}"#),
    ] {
        let message = Scalars {
            f32: float,
            f64: double,
            ..Default::default()
        };
        let bytes = message.encode_to_vec();
        let view = ScalarsView::decode_view(&bytes).unwrap();

        assert_eq!(serde_json::to_string(&message).unwrap(), expected);
        assert_eq!(serde_json::to_string(&view).unwrap(), expected);
    }
}

#[test]
fn explicit_float_and_double_zero_values_remain_present() {
    use buffa_test::json_types::OptionalScalars;

    for value in [0.0, -0.0] {
        let message = OptionalScalars {
            o_f32: Some(value as f32),
            o_f64: Some(value),
            ..Default::default()
        };
        let json = serde_json::to_string(&message).unwrap();
        let decoded: OptionalScalars = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.o_f32.unwrap().to_bits(), (value as f32).to_bits());
        assert_eq!(decoded.o_f64.unwrap().to_bits(), value.to_bits());
    }
}

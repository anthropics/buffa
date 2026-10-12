//! JSON for implicit-presence `float` and `double` fields holding `-0.0`.

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
fn explicit_zero_of_either_sign_round_trips_through_json() {
    use buffa_test::json_types::OptionalScalars;

    for value in [0.0, -0.0] {
        let message = OptionalScalars {
            o_f32: Some(value as f32),
            o_f64: Some(value),
            ..Default::default()
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(
            json.contains(r#""oF32":"#) && json.contains(r#""oF64":"#),
            "{json}"
        );
        let decoded: OptionalScalars = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.o_f32.unwrap().to_bits(), (value as f32).to_bits());
        assert_eq!(decoded.o_f64.unwrap().to_bits(), value.to_bits());
    }
}

#[test]
fn implicit_zero_sign_decides_lazy_view_json_output() {
    use buffa::view::LazyMessageView;
    use buffa_test::basic_prefixed::{RpcAllScalars, RpcAllScalarsLazyView};

    for (float, double, expected) in [
        (-0.0, -0.0, r#"{"fFloat":-0.0,"fDouble":-0.0}"#),
        (-0.0, 0.0, r#"{"fFloat":-0.0}"#),
        (0.0, -0.0, r#"{"fDouble":-0.0}"#),
        (0.0, 0.0, "{}"),
    ] {
        let message = RpcAllScalars {
            f_float: float,
            f_double: double,
            ..Default::default()
        };
        let bytes = message.encode_to_vec();
        let lazy = RpcAllScalarsLazyView::decode_lazy(&bytes).unwrap();

        assert_eq!(serde_json::to_string(&lazy).unwrap(), expected);
    }
}

// serde_json reads `-0` as a float (`visit_f64`), so the sign survives parsing.
#[test]
fn negative_zero_json_input_parses_to_negative_zero() {
    for literal in ["-0", "-0.0", r#""-0""#] {
        let json = format!(r#"{{"f32":{literal},"f64":{literal}}}"#);
        let decoded: Scalars = serde_json::from_str(&json).unwrap();

        assert!(
            decoded.f32 == 0.0 && decoded.f32.is_sign_negative(),
            "float from {literal}: {:?}",
            decoded.f32
        );
        assert!(
            decoded.f64 == 0.0 && decoded.f64.is_sign_negative(),
            "double from {literal}: {:?}",
            decoded.f64
        );
    }
}

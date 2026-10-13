#![cfg(feature = "json")]

use buffa::{Message, MessageView};
use buffa_types::google::protobuf::__buffa::view::{BytesValueView, StringValueView};
use buffa_types::google::protobuf::{BytesValue, StringValue};

#[test]
fn string_view_escapes_json_like_owned() {
    for value in [
        "",
        "plain text",
        "\"quoted\" \\ path",
        "\0\x01\x08\t\n\r\x1f",
        "café 日本語 🦀",
    ] {
        let view = StringValueView {
            value,
            ..Default::default()
        };
        let expected = serde_json::to_string(&StringValue::from(value)).unwrap();
        assert_eq!(serde_json::to_string(&view).unwrap(), expected);
        let mut output = Vec::new();
        serde_json::to_writer(&mut output, &view).unwrap();
        assert_eq!(output, expected.as_bytes());
    }
}

#[test]
fn bytes_view_emits_padded_standard_base64() {
    for (value, expected) in [
        (&b""[..], r#""""#),
        (&b"\x00"[..], r#""AA==""#),
        (&b"\x00\x01"[..], r#""AAE=""#),
        (&b"\x00\x01\x02"[..], r#""AAEC""#),
        (&b"\xfb\xff"[..], r#""+/8=""#),
        (&b"\xff\xfe\xfd\xfc"[..], r#""//79/A==""#),
    ] {
        let view = BytesValueView {
            value,
            ..Default::default()
        };
        assert_eq!(serde_json::to_string(&view).unwrap(), expected);
        let mut output = Vec::new();
        serde_json::to_writer(&mut output, &view).unwrap();
        assert_eq!(output, expected.as_bytes());
        assert_eq!(
            serde_json::to_string(&BytesValue::from(value)).unwrap(),
            expected
        );
    }
}

#[test]
fn decoded_wrapper_views_ignore_unknown_fields_in_json() {
    let string = StringValue::from("known value");
    let mut string_wire = string.encode_to_vec();
    string_wire.extend_from_slice(&[0x10, 0x01, 0x1a, 0x03, b'f', b'o', b'o']);
    let string_view = StringValueView::decode_view(&string_wire).unwrap();
    assert!(!string_view.__buffa_unknown_fields.is_empty());
    assert_eq!(
        serde_json::to_string(&string_view).unwrap(),
        serde_json::to_string(&string).unwrap()
    );

    let bytes = BytesValue::from(&b"\x00\x01\xff"[..]);
    let mut bytes_wire = bytes.encode_to_vec();
    bytes_wire.extend_from_slice(&[0x10, 0x01, 0x1a, 0x03, b'f', b'o', b'o']);
    let bytes_view = BytesValueView::decode_view(&bytes_wire).unwrap();
    assert!(!bytes_view.__buffa_unknown_fields.is_empty());
    assert_eq!(
        serde_json::to_string(&bytes_view).unwrap(),
        serde_json::to_string(&bytes).unwrap()
    );
}

#[test]
fn decoded_wrapper_views_serialize_the_last_value() {
    let mut string_wire = StringValue::from("first").encode_to_vec();
    string_wire.extend(StringValue::from("last").encode_to_vec());
    let string_view = StringValueView::decode_view(&string_wire).unwrap();
    assert_eq!(serde_json::to_string(&string_view).unwrap(), r#""last""#);
    string_wire.extend_from_slice(&[0x0a, 0x00]);
    let string_view = StringValueView::decode_view(&string_wire).unwrap();
    assert_eq!(serde_json::to_string(&string_view).unwrap(), r#""""#);

    let mut bytes_wire = BytesValue::from(&b"first"[..]).encode_to_vec();
    bytes_wire.extend(BytesValue::from(&b"last"[..]).encode_to_vec());
    let bytes_view = BytesValueView::decode_view(&bytes_wire).unwrap();
    assert_eq!(serde_json::to_string(&bytes_view).unwrap(), r#""bGFzdA==""#);
    bytes_wire.extend_from_slice(&[0x0a, 0x00]);
    let bytes_view = BytesValueView::decode_view(&bytes_wire).unwrap();
    assert_eq!(serde_json::to_string(&bytes_view).unwrap(), r#""""#);
}

#[test]
fn large_wrapper_views_match_owned_json() {
    let string = StringValue::from("hello\n日本語🦀".repeat(16 * 1024));
    let string_wire = string.encode_to_vec();
    let string_view = StringValueView::decode_view(&string_wire).unwrap();
    assert_eq!(
        serde_json::to_vec(&string_view).unwrap(),
        serde_json::to_vec(&string).unwrap()
    );

    let bytes = BytesValue::from((0..=255u8).cycle().take(256 * 1024).collect::<Vec<_>>());
    let bytes_wire = bytes.encode_to_vec();
    let bytes_view = BytesValueView::decode_view(&bytes_wire).unwrap();
    assert_eq!(
        serde_json::to_vec(&bytes_view).unwrap(),
        serde_json::to_vec(&bytes).unwrap()
    );
}

#[test]
fn wrapper_views_serialize_inside_containers() {
    let strings = [
        StringValueView::default(),
        StringValueView {
            value: "borrowed",
            ..Default::default()
        },
    ];
    assert_eq!(
        serde_json::to_string(&strings).unwrap(),
        r#"["","borrowed"]"#
    );
    let bytes = [
        BytesValueView::default(),
        BytesValueView {
            value: &[0, 1, 2],
            ..Default::default()
        },
    ];
    assert_eq!(serde_json::to_string(&bytes).unwrap(), r#"["","AAEC"]"#);
}

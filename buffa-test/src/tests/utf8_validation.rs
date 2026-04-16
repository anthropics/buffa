use crate::utf8test::*;
use buffa::{Message, MessageView};

#[test]
fn none_fields_are_vec_u8() {
    // Compile-time type check: raw fields are Vec<u8>, validated is String.
    let msg = StringNoValidation {
        raw_name: Some(b"hello".to_vec()),
        validated_name: Some("world".to_string()),
        raw_tags: vec![b"a".to_vec(), b"b".to_vec()],
        raw_comment: Some(b"comment".to_vec()),
        raw_labels: [(b"k".to_vec(), b"v".to_vec())].into_iter().collect(),
        ..Default::default()
    };
    // Round-trip preserves bytes.
    let decoded = StringNoValidation::decode_from_slice(&msg.encode_to_vec()).unwrap();
    assert_eq!(decoded.raw_name, Some(b"hello".to_vec()));
    assert_eq!(decoded.validated_name, Some("world".to_string()));
    assert_eq!(decoded.raw_tags, vec![b"a".to_vec(), b"b".to_vec()]);
    assert_eq!(decoded.raw_comment, Some(b"comment".to_vec()));
    assert_eq!(decoded.raw_labels.get(&b"k".to_vec()), Some(&b"v".to_vec()));
}

#[test]
fn none_accepts_invalid_utf8() {
    // Manually construct wire bytes with an invalid UTF-8 sequence in raw_name.
    // Field 1 (raw_name), wire type 2 (length-delimited), 2 bytes: 0xFF 0xFE.
    let wire = [0x0A, 0x02, 0xFF, 0xFE];
    let decoded = StringNoValidation::decode_from_slice(&wire).unwrap();
    assert_eq!(decoded.raw_name, Some(vec![0xFF, 0xFE]));
}

#[test]
fn verify_rejects_invalid_utf8() {
    // Same bytes, but targeting field 2 (validated_name, VERIFY).
    let wire = [0x12, 0x02, 0xFF, 0xFE];
    let result = StringNoValidation::decode_from_slice(&wire);
    assert!(matches!(result, Err(buffa::DecodeError::InvalidUtf8)));
}

#[test]
fn view_none_fields_are_byte_slices() {
    let msg = StringNoValidation {
        raw_name: Some(b"hello".to_vec()),
        validated_name: Some("world".to_string()),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = StringNoValidationView::decode_view(&bytes).unwrap();
    // raw_name is Option<&[u8]>, validated_name is Option<&str>.
    let raw: Option<&[u8]> = view.raw_name;
    let validated: Option<&str> = view.validated_name;
    assert_eq!(raw, Some(&b"hello"[..]));
    assert_eq!(validated, Some("world"));
}

#[test]
fn view_none_accepts_invalid_utf8() {
    let wire = [0x0A, 0x02, 0xFF, 0xFE];
    let view = StringNoValidationView::decode_view(&wire).unwrap();
    assert_eq!(view.raw_name, Some(&[0xFF, 0xFE][..]));
}

#[test]
fn oneof_none_variant_is_vec_u8() {
    use oneof_no_validation::ContentOneof;
    let msg = OneofNoValidation {
        content: Some(ContentOneof::RawText(b"bytes".to_vec())),
        ..Default::default()
    };
    let decoded = OneofNoValidation::decode_from_slice(&msg.encode_to_vec()).unwrap();
    assert_eq!(
        decoded.content,
        Some(ContentOneof::RawText(b"bytes".to_vec()))
    );
}

#[test]
fn user_can_validate_or_trust() {
    // Demonstrate the explicit-at-call-site pattern.
    let msg = StringNoValidation {
        raw_name: Some(b"valid utf-8".to_vec()),
        ..Default::default()
    };
    let decoded = StringNoValidation::decode_from_slice(&msg.encode_to_vec()).unwrap();
    let raw_bytes = decoded.raw_name.unwrap();
    // Option 1: checked (same cost as VERIFY would have been).
    let s = std::str::from_utf8(&raw_bytes).unwrap();
    assert_eq!(s, "valid utf-8");
    // Option 2: unchecked (caller asserts validity — the 11% win).
    // SAFETY: we just constructed this from valid UTF-8 above.
    let s = unsafe { std::str::from_utf8_unchecked(&raw_bytes) };
    assert_eq!(s, "valid utf-8");
}

#[test]
fn json_none_fields_base64_encode() {
    // With strict_utf8_mapping, NONE string fields are Vec<u8> and
    // JSON-serialize as base64 (the proto3 JSON encoding for bytes).
    let msg = StringNoValidation {
        raw_name: Some(b"hello".to_vec()),
        validated_name: Some("world".to_string()),
        raw_labels: [(b"k".to_vec(), b"v".to_vec())].into_iter().collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    // "hello" base64 = "aGVsbG8=", "world" stays literal (VERIFY field),
    // "k" = "aw==", "v" = "dg==".
    assert!(json.contains(r#""rawName":"aGVsbG8=""#), "json: {json}");
    assert!(json.contains(r#""validatedName":"world""#), "json: {json}");
    assert!(
        json.contains(r#""rawLabels":{"aw==":"dg=="}"#),
        "json: {json}"
    );
    // Round-trip.
    let decoded: StringNoValidation = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.raw_name, Some(b"hello".to_vec()));
    assert_eq!(decoded.validated_name, Some("world".to_string()));
    assert_eq!(decoded.raw_labels.get(&b"k".to_vec()), Some(&b"v".to_vec()));
}

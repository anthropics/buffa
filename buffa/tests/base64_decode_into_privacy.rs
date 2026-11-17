//! The caller-buffer API must not forward data-bearing decoder diagnostics.

#![cfg(feature = "json")]

use buffa::json_helpers::bytes::decode_into;

#[test]
fn decode_into_rejects_malformed_input_without_symbols_or_offsets() {
    for text in ["^AAA", "A^AA", "AAA^", "AAAA^===", "-_?_", "A"] {
        let mut output = [0; 16];
        let error = decode_into::<serde_json::Error>(text, &mut output).unwrap_err();
        assert_eq!(error.to_string(), "invalid base64");
    }
}

#[test]
fn decode_into_reports_insufficient_output_without_input_data() {
    let error = decode_into::<serde_json::Error>("cHJpdmF0ZQ==", &mut []).unwrap_err();
    assert_eq!(error.to_string(), "invalid base64");
}

#[test]
fn decode_into_preserves_padding_alphabet_and_trailing_bit_leniency() {
    for (text, expected) in [
        ("", &b""[..]),
        ("Zg", &b"f"[..]),
        ("Zg==", &b"f"[..]),
        ("Zh", &b"f"[..]),
        ("-_8", &[0xfb, 0xff][..]),
        ("+/8=", &[0xfb, 0xff][..]),
    ] {
        let mut output = [0; 16];
        let length = decode_into::<serde_json::Error>(text, &mut output).unwrap();
        assert_eq!(&output[..length], expected);
    }
}

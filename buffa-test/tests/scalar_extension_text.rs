//! Text format for scalar, enum and repeated scalar extensions, which had no
//! text registry entry and were dropped on output and rejected on input.

use buffa::text::{decode_from_str, encode_to_string, encode_to_string_pretty, ParseErrorKind};
use buffa::type_registry::{set_type_registry, TypeRegistry};
use buffa::{ExtensionSet, UnknownField, UnknownFieldData};
use buffa_test::extjson;
use extjson::__buffa::ext::{BIGS, COLOR, COLORS, NULL_VALUE, NUMS, WEIGHT};

fn register_types() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let mut registry = TypeRegistry::new();
        extjson::__buffa::register_types(&mut registry);
        set_type_registry(registry);
    });
}

fn carrier() -> extjson::Carrier {
    extjson::Carrier {
        x: Some(7),
        ..Default::default()
    }
}

fn parse(text: &str) -> extjson::Carrier {
    decode_from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn parse_error(text: &str) -> ParseErrorKind {
    decode_from_str::<extjson::Carrier>(text)
        .expect_err(text)
        .kind
}

#[test]
fn scalar_and_enum_extensions_round_trip() {
    register_types();
    let mut c = carrier();
    c.set_extension(&WEIGHT, -3);
    c.set_extension(&COLOR, 2);
    c.set_extension(&NUMS, vec![1, 2]);
    c.set_extension(&COLORS, vec![1, 0]);
    c.set_extension(&BIGS, vec![i64::MIN]);
    c.set_extension(&NULL_VALUE, 0);

    let text = encode_to_string(&c);
    assert_eq!(
        text,
        "x: 7 \
         [buffa.test.extjson.weight]: -3 \
         [buffa.test.extjson.color]: BLUE \
         [buffa.test.extjson.nums]: 1 [buffa.test.extjson.nums]: 2 \
         [buffa.test.extjson.colors]: GREEN [buffa.test.extjson.colors]: RED \
         [buffa.test.extjson.bigs]: -9223372036854775808 \
         [buffa.test.extjson.null_value]: NULL_VALUE"
    );
    assert_eq!(
        encode_to_string_pretty(&c),
        "x: 7\n\
         [buffa.test.extjson.weight]: -3\n\
         [buffa.test.extjson.color]: BLUE\n\
         [buffa.test.extjson.nums]: 1\n\
         [buffa.test.extjson.nums]: 2\n\
         [buffa.test.extjson.colors]: GREEN\n\
         [buffa.test.extjson.colors]: RED\n\
         [buffa.test.extjson.bigs]: -9223372036854775808\n\
         [buffa.test.extjson.null_value]: NULL_VALUE\n"
    );

    let back = parse(&text);
    assert_eq!(back.x, Some(7));
    assert_eq!(back.extension(&WEIGHT), Some(-3));
    assert_eq!(back.extension(&COLOR), Some(2));
    assert_eq!(back.extension(&NUMS), [1, 2]);
    assert_eq!(back.extension(&COLORS), [1, 0]);
    assert_eq!(back.extension(&BIGS), [i64::MIN]);
    assert_eq!(back.extension(&NULL_VALUE), Some(0));
}

#[test]
fn repeated_scalar_extensions_append_lists_and_occurrences() {
    register_types();
    let c = parse(
        "[buffa.test.extjson.nums]: [1, 2] [buffa.test.extjson.nums]: 3 \
         [buffa.test.extjson.nums]: [] \
         [buffa.test.extjson.colors]: [GREEN, 2] [buffa.test.extjson.colors]: RED",
    );
    assert_eq!(c.extension(&NUMS), [1, 2, 3]);
    assert_eq!(c.extension(&COLORS), [1, 2, 0]);
}

#[test]
fn scalar_extensions_read_the_value_forms_of_their_type() {
    register_types();
    let c = parse(
        "[buffa.test.extjson.weight]: -0x10 [buffa.test.extjson.color]: 1 \
         [buffa.test.extjson.bigs]: [0x7fffffffffffffff]",
    );
    assert_eq!(c.extension(&WEIGHT), Some(-16));
    assert_eq!(c.extension(&COLOR), Some(1));
    assert_eq!(c.extension(&BIGS), [i64::MAX]);
}

#[test]
fn invalid_scalar_extension_values_are_rejected() {
    register_types();
    // A scalar needs the colon, also before an empty list.
    assert!(matches!(
        parse_error("[buffa.test.extjson.weight] 5"),
        ParseErrorKind::UnexpectedToken { .. }
    ));
    assert!(matches!(
        parse_error("[buffa.test.extjson.nums] []"),
        ParseErrorKind::UnexpectedToken { .. }
    ));
    assert!(matches!(
        parse_error("[buffa.test.extjson.weight]: 2147483648"),
        ParseErrorKind::InvalidNumber
    ));
    // `Color` is a closed enum: a number without a variant is rejected.
    assert_eq!(
        parse_error("[buffa.test.extjson.color]: 9"),
        ParseErrorKind::UnknownEnumValue
    );
    assert_eq!(
        parse_error("[buffa.test.extjson.colors]: [GREEN, 9]"),
        ParseErrorKind::UnknownEnumValue
    );
    assert_eq!(
        parse_error("[buffa.test.extjson.color]: PURPLE"),
        ParseErrorKind::UnknownEnumValue
    );
}

fn varints(number: u32, values: &[u64]) -> impl Iterator<Item = UnknownField> + '_ {
    values.iter().map(move |&v| UnknownField {
        number,
        data: UnknownFieldData::Varint(v),
    })
}

#[test]
fn closed_enum_extensions_skip_values_without_a_variant() {
    register_types();
    // On the wire a closed-enum extension can hold any number. The text
    // parser rejects one without a variant, so the encoder leaves it out.
    let mut c = carrier();
    let unknown = c.unknown_fields_mut();
    unknown.extend(varints(102, &[2, 9]));
    unknown.extend(varints(105, &[1, 9, 2]));
    let text = encode_to_string(&c);
    assert_eq!(
        text,
        "x: 7 [buffa.test.extjson.color]: BLUE \
         [buffa.test.extjson.colors]: GREEN [buffa.test.extjson.colors]: BLUE"
    );
    let back = parse(&text);
    assert_eq!(back.extension(&COLOR), Some(2));
    assert_eq!(back.extension(&COLORS), [1, 2]);

    // With no variant at all, the singular form is the default value, as a
    // message extension with no decodable record is an empty message, and
    // the repeated form is an empty list, which reads as no elements.
    let mut c = carrier();
    let unknown = c.unknown_fields_mut();
    unknown.extend(varints(102, &[9]));
    unknown.extend(varints(105, &[9]));
    assert_eq!(
        encode_to_string(&c),
        "x: 7 [buffa.test.extjson.color]: RED [buffa.test.extjson.colors]: []"
    );
    assert!(parse("[buffa.test.extjson.colors]: []")
        .extension(&COLORS)
        .is_empty());
}

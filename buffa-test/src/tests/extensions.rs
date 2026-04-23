//! Integration tests for `extend` codegen and the `ExtensionSet` runtime API.

use crate::custopts::__buffa::ext::{
    ACTIVE, ANN, C_ACTIVE, C_ANN, C_FLAG, C_LABEL, C_MARKER, C_PRIORITY, C_TAGS, C_WEIGHT,
    IS_INTERNAL, LABEL, PRIORITY, TAGS, WEIGHT,
};
use crate::custopts::{carrier, Annotation, Carrier};
use buffa::{Extension, ExtensionSet, Message};

/// Sanity: field numbers, extendees, and codec types match the proto source.
/// Codec types are enforced by the `const` binding itself — a mismatch is a
/// compile error, not a runtime failure.
#[test]
fn generated_consts_have_correct_numbers_and_extendees() {
    // FieldOptions / MessageOptions extensions — never used on Carrier
    // (extendee check would panic), but shapes are verified here.
    assert_eq!(ANN.number(), 50001);
    assert_eq!(ANN.extendee(), "google.protobuf.FieldOptions");
    assert_eq!(WEIGHT.number(), 50002);
    assert_eq!(WEIGHT.extendee(), "google.protobuf.FieldOptions");
    assert_eq!(TAGS.number(), 50003);
    assert_eq!(IS_INTERNAL.number(), 50010);
    assert_eq!(IS_INTERNAL.extendee(), "google.protobuf.MessageOptions");
    assert_eq!(carrier::MARKER.number(), 50020);
    assert_eq!(carrier::MARKER.extendee(), "google.protobuf.FieldOptions");
    assert_eq!(PRIORITY.number(), 50030);
    assert_eq!(LABEL.number(), 50031);
    assert_eq!(ACTIVE.number(), 50032);

    // Carrier-targeting extensions — used by the roundtrip tests below.
    assert_eq!(C_ANN.number(), 100);
    assert_eq!(C_ANN.extendee(), "buffa.test.options.Carrier");
    assert_eq!(C_WEIGHT.number(), 101);
    assert_eq!(C_TAGS.number(), 102);
    assert_eq!(C_FLAG.number(), 103);
    assert_eq!(C_MARKER.number(), 104);
    assert_eq!(C_PRIORITY.number(), 110);
    assert_eq!(C_LABEL.number(), 111);
    assert_eq!(C_ACTIVE.number(), 112);

    // Codec types — a type-mismatched binding here would fail to compile.
    let _: Extension<buffa::extension::codecs::MessageCodec<Annotation>> = ANN;
    let _: Extension<buffa::extension::codecs::Sint32> = WEIGHT;
    let _: Extension<buffa::extension::codecs::Repeated<buffa::extension::codecs::StringCodec>> =
        TAGS;
    let _: Extension<buffa::extension::codecs::Bool> = IS_INTERNAL;
    let _: Extension<buffa::extension::codecs::Int32> = carrier::MARKER;
    let _: Extension<buffa::extension::codecs::MessageCodec<Annotation>> = C_ANN;
    let _: Extension<buffa::extension::codecs::Sint32> = C_WEIGHT;
}

/// Generated messages implement `ExtensionSet` (gated on unknown-field
/// preservation, which is on by default) with the correct `PROTO_FQN`.
#[test]
fn generated_messages_implement_extension_set() {
    fn takes_extension_set<T: ExtensionSet>(_: &T) {}
    takes_extension_set(&Annotation::default());
    takes_extension_set(&Carrier::default());

    assert_eq!(Carrier::PROTO_FQN, "buffa.test.options.Carrier");
    assert_eq!(Annotation::PROTO_FQN, "buffa.test.options.Annotation");
}

// ────────────────────────────────────────────────────────────────────────────
// Extendee identity check
// ────────────────────────────────────────────────────────────────────────────

/// Using a FieldOptions-targeting extension on Carrier panics. This is what
/// the check exists to catch: a bug where the caller grabbed the wrong const.
#[test]
#[should_panic(
    expected = "extension at field 50002 extends `google.protobuf.FieldOptions`, not `buffa.test.options.Carrier`"
)]
fn extension_panics_on_wrong_extendee() {
    let c = Carrier::default();
    let _ = c.extension(&WEIGHT);
}

#[test]
#[should_panic(expected = "extends `google.protobuf.FieldOptions`")]
fn set_extension_panics_on_wrong_extendee() {
    let mut c = Carrier::default();
    c.set_extension(&WEIGHT, 42);
}

#[test]
#[should_panic(expected = "extends `google.protobuf.FieldOptions`")]
fn clear_extension_panics_on_wrong_extendee() {
    let mut c = Carrier::default();
    c.clear_extension(&WEIGHT);
}

/// `has_extension` is graceful on mismatch — returns `false` instead of
/// panicking. Matches protobuf-go's `HasExtension` and protobuf-es's
/// `hasExtension`.
#[test]
fn has_extension_graceful_on_wrong_extendee() {
    let c = Carrier::default();
    assert!(!c.has_extension(&WEIGHT));
    // Not even confused by an unrelated record at the same number.
    let mut c = Carrier::default();
    // C_ANN is at field 100, WEIGHT is at 50002; use raw unknown-field push.
    // Actually simpler: just set the correct extension at 101 and verify
    // WEIGHT (50002, wrong extendee) still says false.
    c.set_extension(&C_WEIGHT, 7);
    assert!(c.has_extension(&C_WEIGHT));
    assert!(!c.has_extension(&WEIGHT));
}

// ────────────────────────────────────────────────────────────────────────────
// Roundtrip / presence tests — all use Carrier-targeting (C_*) extensions
// ────────────────────────────────────────────────────────────────────────────

/// Set a message-typed extension on a carrier, encode to wire, decode back,
/// and verify the extension value survived.
#[test]
fn message_extension_roundtrip_through_wire() {
    let ann = Annotation {
        doc: Some("hello".to_string()),
        priority: Some(5),
        ..Default::default()
    };

    let mut carrier = Carrier::default();
    carrier.set_extension(&C_ANN, ann.clone());

    let bytes = carrier.encode_to_vec();
    let decoded = Carrier::decode_from_slice(&bytes).expect("decode");

    let got = decoded.extension(&C_ANN).expect("C_ANN present");
    assert_eq!(got.doc, Some("hello".to_string()));
    assert_eq!(got.priority, Some(5));
}

/// Zigzag scalar extension: `C_WEIGHT` is `sint32`, so `-7` on the wire is
/// zigzag-encoded. Verify the codec roundtrips the signed value.
#[test]
fn sint32_extension_roundtrip() {
    let mut carrier = Carrier::default();
    carrier.set_extension(&C_WEIGHT, -7);

    let bytes = carrier.encode_to_vec();
    let decoded = Carrier::decode_from_slice(&bytes).expect("decode");
    assert_eq!(decoded.extension(&C_WEIGHT), Some(-7));
}

/// Repeated string extension: proto2 default is unpacked (one record per
/// element).
#[test]
fn repeated_string_extension_roundtrip() {
    let mut carrier = Carrier::default();
    carrier.set_extension(&C_TAGS, vec!["a".to_string(), "b".to_string()]);

    let bytes = carrier.encode_to_vec();
    let decoded = Carrier::decode_from_slice(&bytes).expect("decode");
    assert_eq!(
        decoded.extension(&C_TAGS),
        vec!["a".to_string(), "b".to_string()]
    );
}

/// Multiple extensions at different field numbers coexist and don't interfere.
#[test]
fn multiple_extensions_coexist() {
    let mut carrier = Carrier::default();
    carrier.set_extension(&C_WEIGHT, 42);
    carrier.set_extension(&C_FLAG, true);
    carrier.set_extension(&C_TAGS, vec!["x".to_string()]);
    carrier.x = Some(99); // regular field alongside extensions

    let decoded = super::round_trip(&carrier);

    assert_eq!(decoded.extension(&C_WEIGHT), Some(42));
    assert_eq!(decoded.extension(&C_FLAG), Some(true));
    assert_eq!(decoded.extension(&C_TAGS), vec!["x".to_string()]);
    assert_eq!(decoded.x, Some(99));
}

/// Extensions always have explicit presence: setting `0` makes `has` return
/// true. This is the protocolbuffers/protobuf#8234 invariant.
#[test]
fn explicit_presence_zero_value() {
    let mut carrier = Carrier::default();
    assert!(!carrier.has_extension(&C_WEIGHT));
    carrier.set_extension(&C_WEIGHT, 0);
    assert!(carrier.has_extension(&C_WEIGHT));
    assert_eq!(carrier.extension(&C_WEIGHT), Some(0));
}

/// `extend` nested inside a message produces a const in that message's module.
/// `carrier::MARKER` extends FieldOptions (not Carrier — that's the nesting
/// scope test); the const `C_MARKER` extends Carrier itself.
#[test]
fn nested_extension_const_in_module() {
    // Scope check only — MARKER can't be used on Carrier (wrong extendee).
    assert_eq!(carrier::MARKER.extendee(), "google.protobuf.FieldOptions");
    // The Carrier-targeting const works normally.
    let mut c = Carrier::default();
    c.set_extension(&C_MARKER, 77);
    assert_eq!(c.extension(&C_MARKER), Some(77));
}

// ────────────────────────────────────────────────────────────────────────────
// Proto2 [default = ...] on extensions — extension_or_default()
// ────────────────────────────────────────────────────────────────────────────

/// `[default = ...]` on absent extensions: `extension()` still returns `None`
/// (presence is distinguishable), `extension_or_default()` returns the proto
/// default. This is the additive-API contract.
#[test]
fn proto2_default_on_absent_extension() {
    let c = Carrier::default();

    // extension() returns None — presence is distinguishable.
    assert_eq!(c.extension(&C_PRIORITY), None);
    assert_eq!(c.extension(&C_LABEL), None);
    assert_eq!(c.extension(&C_ACTIVE), None);
    assert!(!c.has_extension(&C_PRIORITY));

    // extension_or_default() returns the declared [default = ...].
    assert_eq!(c.extension_or_default(&C_PRIORITY), 7);
    assert_eq!(c.extension_or_default(&C_LABEL), "none");
    assert!(c.extension_or_default(&C_ACTIVE));
}

/// Set value wins over the proto default — including an explicitly-set value
/// that happens to equal the type's zero. Extensions have explicit presence
/// (protocolbuffers/protobuf#8234), so `set(0)` is distinguishable from absent.
#[test]
fn proto2_default_ignored_when_set() {
    let mut c = Carrier::default();
    c.set_extension(&C_PRIORITY, 99);
    assert_eq!(c.extension_or_default(&C_PRIORITY), 99);
    assert_eq!(c.extension(&C_PRIORITY), Some(99));

    // Explicit zero is a set value, not absent: returns 0, not the default 7.
    c.set_extension(&C_PRIORITY, 0);
    assert_eq!(c.extension_or_default(&C_PRIORITY), 0);

    // Clear → back to the declared default.
    c.clear_extension(&C_PRIORITY);
    assert_eq!(c.extension_or_default(&C_PRIORITY), 7);
}

/// Extensions without a `[default]` fall back to the type's `Default`
/// (0 for i32, false for bool) rather than panicking.
#[test]
fn extension_or_default_no_declared_default_uses_type_default() {
    let c = Carrier::default();
    // C_WEIGHT has no [default] in the proto → i32 default is 0.
    assert_eq!(c.extension_or_default(&C_WEIGHT), 0);
    // C_FLAG has no [default] → bool default is false.
    assert!(!c.extension_or_default(&C_FLAG));
}

/// The declared default survives encode/decode: a freshly-decoded carrier
/// with no extension data still returns the proto default. (The default lives
/// in the generated `const`, not on the wire.)
#[test]
fn proto2_default_after_roundtrip() {
    let carrier = Carrier {
        x: Some(42),
        ..Default::default()
    };
    let decoded = super::round_trip(&carrier);
    assert_eq!(decoded.x, Some(42));
    assert_eq!(decoded.extension(&C_PRIORITY), None);
    assert_eq!(decoded.extension_or_default(&C_PRIORITY), 7);
    assert_eq!(decoded.extension_or_default(&C_LABEL), "none");
}

// ────────────────────────────────────────────────────────────────────────────
// Group-encoded extensions (editions DELIMITED)
// ────────────────────────────────────────────────────────────────────────────

use crate::groupext::__buffa::ext::{DELIM_INNER, DELIM_REPEATED};
use crate::groupext::{Carrier as GroupCarrier, Inner};
use buffa::extension::codecs::{GroupCodec, Repeated};

/// Codec type check: DELIMITED extension gets `GroupCodec`, not `MessageCodec`.
/// A type mismatch here is a compile error.
#[test]
fn group_extension_codec_type() {
    let _: Extension<GroupCodec<Inner>> = DELIM_INNER;
    let _: Extension<Repeated<GroupCodec<Inner>>> = DELIM_REPEATED;
    assert_eq!(DELIM_INNER.number(), 100);
    assert_eq!(DELIM_REPEATED.number(), 101);
    assert_eq!(DELIM_INNER.extendee(), "buffa.test.groupext.Carrier");
}

/// Set a group-encoded extension, encode to wire, and verify the wire bytes
/// contain group framing (StartGroup/EndGroup tags) — not a length prefix.
#[test]
fn group_extension_wire_format() {
    let mut carrier = GroupCarrier::default();
    carrier.set_extension(
        &DELIM_INNER,
        Inner {
            c: Some(42),
            ..Default::default()
        },
    );

    let bytes = carrier.encode_to_vec();

    // Field 100, wire type StartGroup (3): tag = (100 << 3) | 3 = 803
    //   varint(803) = [0xA3, 0x06]
    // Inner field c=42: tag (1 << 3)|0 = 0x08, value 0x2A
    // Field 100, wire type EndGroup (4): tag = (100 << 3) | 4 = 804
    //   varint(804) = [0xA4, 0x06]
    let start_tag = [0xA3, 0x06];
    let end_tag = [0xA4, 0x06];
    let inner = [0x08, 0x2A];

    assert!(bytes.windows(2).any(|w| w == start_tag), "{bytes:02X?}");
    assert!(bytes.windows(2).any(|w| w == end_tag), "{bytes:02X?}");
    assert!(bytes.windows(2).any(|w| w == inner), "{bytes:02X?}");

    // NO length-delimited tag for field 100 (wire type 2): (100<<3)|2 = 802
    //   varint(802) = [0xA2, 0x06]
    assert!(
        !bytes.windows(2).any(|w| w == [0xA2, 0x06]),
        "found LD tag for extension field: {bytes:02X?}"
    );
}

/// Full roundtrip through encode → decode: the group-encoded extension value
/// survives.
#[test]
fn group_extension_roundtrip() {
    let mut carrier = GroupCarrier::default();
    carrier.set_extension(
        &DELIM_INNER,
        Inner {
            c: Some(42),
            ..Default::default()
        },
    );

    let decoded = super::round_trip(&carrier);

    let got = decoded
        .extension(&DELIM_INNER)
        .expect("DELIM_INNER present");
    assert_eq!(got.c, Some(42));
}

/// Repeated group-encoded extension: each element is a separate group record.
#[test]
fn repeated_group_extension_roundtrip() {
    let mut carrier = GroupCarrier::default();
    carrier.set_extension(
        &DELIM_REPEATED,
        vec![
            Inner {
                c: Some(1),
                ..Default::default()
            },
            Inner {
                c: Some(2),
                ..Default::default()
            },
            Inner {
                c: Some(3),
                ..Default::default()
            },
        ],
    );

    let decoded = super::round_trip(&carrier);

    let got = decoded.extension(&DELIM_REPEATED);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].c, Some(1));
    assert_eq!(got[1].c, Some(2));
    assert_eq!(got[2].c, Some(3));
}

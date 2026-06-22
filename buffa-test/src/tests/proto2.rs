//! Proto2: custom defaults, required fields, groups, negative enums.

use super::round_trip;
use buffa::Message;

#[test]
fn test_proto2_defaults_and_round_trip() {
    use crate::proto2::WithDefaults;

    // Proto2 optional fields default to None (unset).
    let default = WithDefaults::default();
    assert_eq!(default.count, None);
    assert_eq!(default.label, None);
    assert_eq!(default.enabled, None);

    // Round-trip with required fields and some optionals set.
    let msg = WithDefaults {
        name: "test".into(),
        id: 1,
        count: Some(99),
        label: Some("custom".into()),
        enabled: Some(false),
        priority: Some(crate::proto2::Priority::HIGH),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.name, "test");
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.count, Some(99));
    assert_eq!(decoded.label.as_deref(), Some("custom"));
    assert_eq!(decoded.enabled, Some(false));
    assert_eq!(decoded.priority, Some(crate::proto2::Priority::HIGH));
}

#[test]
fn test_proto2_special_float_defaults() {
    use crate::proto2::WithDefaults;

    // Special float values round-trip correctly.
    let msg = WithDefaults {
        name: "floats".into(),
        id: 1,
        pos_inf: Some(f32::INFINITY),
        neg_inf: Some(f32::NEG_INFINITY),
        nan_val: Some(f32::NAN),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.pos_inf, Some(f32::INFINITY));
    assert_eq!(decoded.neg_inf, Some(f32::NEG_INFINITY));
    assert!(decoded.nan_val.unwrap().is_nan());
}

#[test]
fn test_proto2_bytes_default() {
    use crate::proto2::WithDefaults;

    // Bytes field with custom default round-trips.
    let msg = WithDefaults {
        name: "bytes".into(),
        id: 1,
        magic: Some(vec![0x00, 0xFF, 0x42]),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.magic, Some(vec![0x00, 0xFF, 0x42]));
}

#[test]
fn test_proto2_required_custom_defaults() {
    // Custom [default = ...] on REQUIRED fields produces a hand-written
    // impl Default (required fields are bare types, not Option<T>).
    // Optional fields with custom defaults stay None in Default — buffa
    // doesn't generate proto2-style getter methods.
    use crate::proto2::{Priority, RequiredDefaults};
    let d = RequiredDefaults::default();
    assert_eq!(d.count, 42);
    // Escape sequences in string defaults (\n, \t, \") are pre-unescaped
    // by protoc in the descriptor.
    assert_eq!(d.label, "line1\nline2\t\"quoted\"");
    // Hex escapes in bytes defaults.
    assert_eq!(d.magic, vec![0x00, 0xFF]);

    // One field per scalar type — covers every parse_default_value branch.
    assert!(d.on);
    assert_eq!(d.u, u32::MAX);
    assert_eq!(d.i64v, i64::MIN);
    assert_eq!(d.u64v, u64::MAX);
    assert_eq!(d.f, 1.5_f32);
    assert_eq!(d.d_inf, f64::INFINITY);
    assert_eq!(d.d_ninf, f64::NEG_INFINITY);
    assert!(d.d_nan.is_nan());
    assert_eq!(d.level, Priority::CRITICAL);
    assert_eq!(d.s32, -1);
    assert_eq!(d.s64, -1);
    assert_eq!(d.fx32, 100);
    assert_eq!(d.fx64, 200);
    assert_eq!(d.sfx32, -100);
    assert_eq!(d.sfx64, -200);

    // Required-with-default also round-trips correctly (always encoded).
    let decoded = round_trip(&d);
    assert_eq!(decoded.count, 42);
    assert_eq!(decoded.level, Priority::CRITICAL);
    assert_eq!(decoded.u64v, u64::MAX);
    assert!(decoded.d_nan.is_nan());
}

#[test]
fn test_proto2_keyword_enum_value_default() {
    // Regression: defaults.rs previously used format_ident! directly on the
    // proto enum value name, producing `MyEnum::type` instead of `MyEnum::r#type`.
    // The enum definition uses make_field_ident (keyword-escaping), so the
    // default expression must use the same escaping to match.
    use crate::proto2::{KeywordEnumDefault, KeywordValues};
    let d = KeywordEnumDefault::default();
    // `type` (lowercase) → raw ident `r#type`.
    assert_eq!(d.kind, KeywordValues::r#type);
    // `Self` → suffixed ident `Self_` (can't be raw).
    assert_eq!(d.who, KeywordValues::Self_);
}

#[test]
fn test_proto2_message_round_trip() {
    use crate::proto2::{Priority, Proto2Message, Tag};

    let msg = Proto2Message {
        text: Some("hello".into()),
        number: Some(42),
        items: vec!["a".into(), "b".into()],
        tag: buffa::MessageField::some(Tag {
            key: Some("env".into()),
            value: Some("prod".into()),
            ..Default::default()
        }),
        priority: Some(Priority::HIGH),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.text.as_deref(), Some("hello"));
    assert_eq!(decoded.number, Some(42));
    assert_eq!(decoded.items, vec!["a", "b"]);
    assert_eq!(decoded.tag.key.as_deref(), Some("env"));
    assert_eq!(decoded.priority, Some(Priority::HIGH));
}

// -----------------------------------------------------------------------
// Proto2 group tests
// -----------------------------------------------------------------------

#[test]
fn test_proto2_group_singular_round_trip() {
    use crate::proto2::with_groups::MyGroup;
    use crate::proto2::WithGroups;

    let msg = WithGroups {
        mygroup: buffa::MessageField::some(MyGroup {
            a: Some(42),
            b: Some("hello".into()),
            ..Default::default()
        }),
        label: Some("test".into()),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.mygroup.a, Some(42));
    assert_eq!(decoded.mygroup.b.as_deref(), Some("hello"));
    assert_eq!(decoded.label.as_deref(), Some("test"));
}

#[test]
fn test_proto2_group_repeated_round_trip() {
    use crate::proto2::with_groups::Item;
    use crate::proto2::WithGroups;

    let msg = WithGroups {
        item: vec![
            Item {
                id: Some(1),
                name: Some("first".into()),
                ..Default::default()
            },
            Item {
                id: Some(2),
                name: Some("second".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.item.len(), 2);
    assert_eq!(decoded.item[0].id, Some(1));
    assert_eq!(decoded.item[0].name.as_deref(), Some("first"));
    assert_eq!(decoded.item[1].id, Some(2));
    assert_eq!(decoded.item[1].name.as_deref(), Some("second"));
}

#[test]
fn test_proto2_group_empty_round_trip() {
    use crate::proto2::WithGroups;

    // All defaults — no group set.
    let msg = WithGroups::default();
    let decoded = round_trip(&msg);
    assert!(!decoded.mygroup.is_set());
    assert!(decoded.item.is_empty());
    assert!(decoded.label.is_none());
}

#[test]
fn test_proto2_group_singular_unset_fields() {
    use crate::proto2::with_groups::MyGroup;
    use crate::proto2::WithGroups;

    // Group is set but its fields are all unset.
    let msg = WithGroups {
        mygroup: buffa::MessageField::some(MyGroup::default()),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert!(decoded.mygroup.is_set());
    assert_eq!(decoded.mygroup.a, None);
    assert_eq!(decoded.mygroup.b, None);
}

#[test]
fn test_proto2_group_wire_format() {
    use crate::proto2::with_groups::MyGroup;
    use crate::proto2::WithGroups;

    // Verify the wire format uses StartGroup/EndGroup, not
    // length-delimited encoding.
    let msg = WithGroups {
        mygroup: buffa::MessageField::some(MyGroup {
            a: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();

    // First byte should be the StartGroup tag for field 1:
    // (1 << 3) | 3 = 0x0B
    assert_eq!(bytes[0], 0x0B);

    // Last two bytes before any trailing data should be the
    // EndGroup tag for field 1: (1 << 3) | 4 = 0x0C
    // Find it by scanning — it must be present.
    assert!(
        bytes.contains(&0x0C),
        "EndGroup tag (0x0C) not found in encoded bytes: {bytes:?}"
    );
}

// -----------------------------------------------------------------------
// View codegen coverage: required enum, non-string-key map, closed-enum
// map value, group-in-oneof. Exercises view.rs branches that previously
// had no test proto shape to drive them.
// -----------------------------------------------------------------------

#[test]
fn test_view_coverage_owned_round_trip() {
    use crate::proto2::__buffa::oneof::view_coverage::Choice as ChoiceOneof;
    use crate::proto2::view_coverage::Payload;
    use crate::proto2::{Priority, ViewCoverage};

    let mut by_id = buffa::Map::default();
    by_id.insert(1, "one".to_string());
    by_id.insert(2, "two".to_string());

    let mut priorities = buffa::Map::default();
    priorities.insert("low".to_string(), Priority::LOW);
    priorities.insert("high".to_string(), Priority::HIGH);

    let msg = ViewCoverage {
        level: Priority::CRITICAL,
        by_id,
        priorities,
        choice: Some(ChoiceOneof::Payload(Box::new(Payload {
            x: Some(42),
            y: Some("hello".into()),
            ..Default::default()
        }))),
        ..Default::default()
    };

    let decoded = round_trip(&msg);
    assert_eq!(decoded.level, Priority::CRITICAL);
    assert_eq!(decoded.by_id.get(&1).map(String::as_str), Some("one"));
    assert_eq!(decoded.by_id.get(&2).map(String::as_str), Some("two"));
    assert_eq!(decoded.priorities.get("low"), Some(&Priority::LOW));
    assert_eq!(decoded.priorities.get("high"), Some(&Priority::HIGH));
    match decoded.choice {
        Some(ChoiceOneof::Payload(p)) => {
            assert_eq!(p.x, Some(42));
            assert_eq!(p.y.as_deref(), Some("hello"));
        }
        other => panic!("expected Payload variant, got {other:?}"),
    }
}

#[test]
fn test_view_coverage_via_view() {
    // View-decode → to_owned_message → encode round-trip.
    // Exercises: singular closed-enum view type, MapView<i32, &str>,
    // MapView<&str, ClosedEnum>, group-in-oneof view decode + merge.
    use crate::proto2::__buffa::oneof::view_coverage::Choice as ChoiceOneof;
    use crate::proto2::__buffa::view::ViewCoverageView;
    use crate::proto2::view_coverage::Payload;
    use crate::proto2::{Priority, ViewCoverage};
    use buffa::MessageView;

    let mut by_id = buffa::Map::default();
    by_id.insert(7, "seven".to_string());

    let mut priorities = buffa::Map::default();
    priorities.insert("med".to_string(), Priority::MEDIUM);

    let original = ViewCoverage {
        level: Priority::HIGH,
        by_id,
        priorities,
        choice: Some(ChoiceOneof::Payload(Box::new(Payload {
            x: Some(99),
            y: Some("world".into()),
            ..Default::default()
        }))),
        ..Default::default()
    };

    let wire = original.encode_to_vec();
    let view = ViewCoverageView::decode_view(&wire).unwrap();

    // Direct view access.
    assert_eq!(view.level, Priority::HIGH);
    assert_eq!(view.by_id.iter().count(), 1);
    let (k, v) = view.by_id.iter().next().unwrap();
    assert_eq!((*k, *v), (7, "seven"));
    assert_eq!(view.priorities.iter().count(), 1);
    let (k, v) = view.priorities.iter().next().unwrap();
    assert_eq!((*k, *v), ("med", Priority::MEDIUM));

    // to_owned_message parity.
    let owned = view.to_owned_message().unwrap();
    assert_eq!(owned.level, Priority::HIGH);
    assert_eq!(owned.by_id.get(&7).map(String::as_str), Some("seven"));
    assert_eq!(owned.priorities.get("med"), Some(&Priority::MEDIUM));
    match &owned.choice {
        Some(ChoiceOneof::Payload(p)) => {
            assert_eq!(p.x, Some(99));
            assert_eq!(p.y.as_deref(), Some("world"));
        }
        other => panic!("expected Payload, got {other:?}"),
    }

    // Full round-trip: view → owned → encode should match original.
    assert_eq!(owned.encode_to_vec(), wire);
}

#[test]
fn test_view_coverage_required_enum_default() {
    // Required closed-enum field defaults to the first enum value (LOW=0).
    // View decode of empty buffer should also produce the default.
    use crate::proto2::__buffa::view::ViewCoverageView;
    use crate::proto2::{Priority, ViewCoverage};
    use buffa::MessageView;

    let d = ViewCoverage::default();
    assert_eq!(d.level, Priority::LOW);

    let view = ViewCoverageView::decode_view(&[]).unwrap();
    assert_eq!(view.level, Priority::LOW);
}

#[test]
fn test_view_coverage_group_in_oneof_merge() {
    // Proto spec: same oneof field on the wire twice → merge (for messages/
    // groups). Exercises the `_merge_into_view` branch for group-in-oneof.
    use crate::proto2::__buffa::oneof::view_coverage::Choice as ChoiceOneof;
    use crate::proto2::__buffa::view::ViewCoverageView;
    use crate::proto2::view_coverage::Payload;
    use crate::proto2::{Priority, ViewCoverage};
    use buffa::MessageView;

    // First occurrence: only x set.
    let first = ViewCoverage {
        level: Priority::LOW,
        choice: Some(ChoiceOneof::Payload(Box::new(Payload {
            x: Some(1),
            ..Default::default()
        }))),
        ..Default::default()
    };
    // Second occurrence: only y set.
    let second = ViewCoverage {
        level: Priority::LOW,
        choice: Some(ChoiceOneof::Payload(Box::new(Payload {
            y: Some("merged".into()),
            ..Default::default()
        }))),
        ..Default::default()
    };

    // Concatenate on the wire: emulates two occurrences of the same field.
    let mut wire = first.encode_to_vec();
    wire.extend_from_slice(&second.encode_to_vec());

    let view = ViewCoverageView::decode_view(&wire).unwrap();
    let owned = view.to_owned_message().unwrap();
    match owned.choice {
        Some(ChoiceOneof::Payload(p)) => {
            // Both x (from first) and y (from second) should be present.
            assert_eq!(p.x, Some(1), "x should survive merge");
            assert_eq!(p.y.as_deref(), Some("merged"), "y should be added by merge");
        }
        other => panic!("expected merged Payload, got {other:?}"),
    }
}

// ── Required-field presence on views (#170) ─────────────────────────────

#[test]
fn test_view_required_presence_absent_vs_explicit_default() {
    // The motivating case from gh#170: a required field explicitly encoded
    // with its default value must be distinguishable from one that was
    // absent on the wire.
    use crate::proto2::__buffa::view::WithDefaultsView;
    use crate::proto2::WithDefaults;
    use buffa::MessageView;

    // Absent: nothing on the wire.
    let view = WithDefaultsView::decode_view(&[]).unwrap();
    assert!(!view.has_name(), "absent required string must report false");
    assert!(!view.has_id(), "absent required int must report false");
    assert_eq!(view.name, "");
    assert_eq!(view.id, 0);

    // Present, explicitly encoded as type defaults ("" and 0) — required
    // fields are always written by the encoder.
    let msg = WithDefaults {
        name: String::new(),
        id: 0,
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = WithDefaultsView::decode_view(&wire).unwrap();
    assert!(view.has_name(), "explicit default string must report true");
    assert!(view.has_id(), "explicit zero int must report true");
    assert_eq!(view.name, "");
    assert_eq!(view.id, 0);
}

#[test]
fn test_view_required_presence_survives_submessage_merge() {
    // Two wire occurrences of the same message concatenated (proto merge
    // semantics): bits accumulated across both fragments.
    use crate::proto2::__buffa::view::WithDefaultsView;
    use crate::proto2::WithDefaults;
    use buffa::MessageView;

    let only_name = WithDefaults {
        name: "n".to_string(),
        ..Default::default()
    };
    let only_id = WithDefaults {
        id: 9,
        ..Default::default()
    };
    let mut wire = only_name.encode_to_vec();
    wire.extend_from_slice(&only_id.encode_to_vec());

    let view = WithDefaultsView::decode_view(&wire).unwrap();
    assert!(
        view.has_name(),
        "bit from first fragment must survive merge"
    );
    assert!(view.has_id(), "bit from second fragment must be added");
}

#[test]
fn test_view_required_closed_enum_unknown_value_not_marked_present() {
    // An unknown closed-enum value routes to unknown fields (proto2
    // semantics) — the required field was not effectively set, so it must
    // not be marked present, matching C++ initialization checks.
    use crate::proto2::__buffa::view::ViewCoverageView;
    use crate::proto2::{Priority, ViewCoverage};
    use buffa::MessageView;

    let unknown_value = super::varint_field(1, 99);
    let view = ViewCoverageView::decode_view(&unknown_value).unwrap();
    assert!(
        !view.has_level(),
        "unknown closed-enum value must not mark the required field present"
    );

    // A known value does.
    let msg = ViewCoverage {
        level: Priority::HIGH,
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = ViewCoverageView::decode_view(&wire).unwrap();
    assert!(view.has_level());
}

#[test]
fn test_view_required_presence_custom_defaults() {
    // RequiredDefaults: 18 bit-tracked required fields across every scalar
    // kind — all bits land in one word and report consistently.
    use crate::proto2::__buffa::view::RequiredDefaultsView;
    use crate::proto2::RequiredDefaults;
    use buffa::MessageView;

    let view = RequiredDefaultsView::decode_view(&[]).unwrap();
    assert!(!view.has_count());
    assert!(!view.has_label());
    assert!(!view.has_level());
    assert!(!view.has_sfx64());

    let wire = RequiredDefaults::default().encode_to_vec();
    let view = RequiredDefaultsView::decode_view(&wire).unwrap();
    assert!(view.has_count());
    assert!(view.has_label());
    assert!(view.has_level());
    assert!(view.has_sfx64());
}

#[test]
fn test_lazy_view_required_presence_parity() {
    // The lazy view family answers has_* identically to the eager view:
    // absent vs explicitly-encoded-default required fields.
    use crate::proto2::{WithDefaults, WithDefaultsLazyView};
    use buffa::view::LazyMessageView;

    let view = WithDefaultsLazyView::decode_lazy(&[]).unwrap();
    assert!(!view.has_name(), "absent required string must report false");
    assert!(!view.has_id(), "absent required int must report false");

    let msg = WithDefaults {
        name: String::new(),
        id: 0,
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = WithDefaultsLazyView::decode_lazy(&wire).unwrap();
    assert!(view.has_name(), "explicit default string must report true");
    assert!(view.has_id(), "explicit zero int must report true");
}

#[test]
fn test_view_required_reflect_has_agrees_with_accessor() {
    // ReflectMessage::has on the view consults the same wire-presence
    // tracking as the inherent has_* accessors, so the two surfaces agree
    // for required fields explicitly encoded with their default value.
    use crate::proto2::__buffa::view::WithDefaultsView;
    use crate::proto2::WithDefaults;
    use buffa::MessageView;
    use buffa_descriptor::reflect::ReflectMessage;

    let view = WithDefaultsView::decode_view(&[]).unwrap();
    let r: &dyn ReflectMessage = &view;
    let md = r.message_descriptor();
    assert!(!r.has(md.field(13).unwrap()), "absent required string");
    assert!(!r.has(md.field(14).unwrap()), "absent required int");

    let wire = WithDefaults {
        name: String::new(),
        id: 0,
        ..Default::default()
    }
    .encode_to_vec();
    let view = WithDefaultsView::decode_view(&wire).unwrap();
    let r: &dyn ReflectMessage = &view;
    assert_eq!(r.has(md.field(13).unwrap()), view.has_name());
    assert_eq!(r.has(md.field(14).unwrap()), view.has_id());
    assert!(view.has_name() && view.has_id());
}

#[test]
fn test_view_required_numeric_only_presence() {
    // All-numeric required message: the view's only borrowing state is the
    // seen-bit word (PhantomData + seen word coexist on the struct).
    use crate::proto2::__buffa::view::RequiredNumericOnlyView;
    use crate::proto2::RequiredNumericOnly;
    use buffa::MessageView;

    let view = RequiredNumericOnlyView::decode_view(&[]).unwrap();
    assert!(!view.has_a());
    assert!(!view.has_b());
    assert!(!view.has_c());

    let wire = RequiredNumericOnly::default().encode_to_vec();
    let view = RequiredNumericOnlyView::decode_view(&wire).unwrap();
    assert!(view.has_a());
    assert!(view.has_b());
    assert!(view.has_c());
}

#[test]
fn test_view_required_group_presence_uses_is_set() {
    // A required group field delegates has_* to MessageFieldView::is_set.
    use crate::proto2::__buffa::view::WithRequiredGroupView;
    use crate::proto2::with_required_group::Item;
    use crate::proto2::WithRequiredGroup;
    use buffa::MessageView;

    let view = WithRequiredGroupView::decode_view(&[]).unwrap();
    assert!(!view.has_item(), "absent required group must report false");

    let msg = WithRequiredGroup {
        item: buffa::MessageField::some(Item {
            x: Some(5),
            ..Default::default()
        }),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = WithRequiredGroupView::decode_view(&wire).unwrap();
    assert!(view.has_item(), "present required group must report true");
    assert_eq!(view.item.x, Some(5));
}

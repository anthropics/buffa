//! `open_enums_in` closed-enum representation override.

use super::varint_field;
use buffa::{EnumValue, Message, MessageView};

fn packed_field(num: u32, values: &[u64]) -> Vec<u8> {
    use buffa::encoding::{encode_varint, Tag, WireType};

    let mut payload = Vec::new();
    for value in values {
        encode_varint(*value, &mut payload);
    }

    let mut wire = Vec::new();
    Tag::new(num, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(payload.len() as u64, &mut wire);
    wire.extend_from_slice(&payload);
    wire
}

fn length_delimited_field(num: u32, payload: &[u8]) -> Vec<u8> {
    use buffa::encoding::{encode_varint, Tag, WireType};

    let mut wire = Vec::new();
    Tag::new(num, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(payload.len() as u64, &mut wire);
    wire.extend_from_slice(payload);
    wire
}

fn map_enum_entry(num: u32, key: &str, value: u64) -> Vec<u8> {
    use buffa::encoding::{encode_varint, Tag, WireType};

    let mut entry = Vec::new();
    Tag::new(1, WireType::LengthDelimited).encode(&mut entry);
    buffa::types::encode_string(key, &mut entry);
    Tag::new(2, WireType::Varint).encode(&mut entry);
    encode_varint(value, &mut entry);

    let mut wire = Vec::new();
    Tag::new(num, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(entry.len() as u64, &mut wire);
    wire.extend_from_slice(&entry);
    wire
}

fn unknown_wire() -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend(varint_field(1, 99));
    wire.extend(varint_field(2, 0));
    wire.extend(varint_field(2, 99));
    wire.extend(varint_field(2, 2));
    wire.extend(packed_field(3, &[0, 99, 2]));
    wire.extend(varint_field(4, 77));
    wire.extend(varint_field(5, 123));
    wire.extend(map_enum_entry(6, "unknown", 88));
    wire
}

fn no_unknowns_wire() -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend(varint_field(1, 99));
    wire.extend(varint_field(2, 0));
    wire.extend(varint_field(2, 99));
    wire.extend(packed_field(3, &[0, 99, 2]));
    wire.extend(varint_field(4, 123));
    wire.extend(map_enum_entry(5, "unknown", 88));
    wire
}

#[test]
fn overridden_closed_enum_unknowns_are_field_values() {
    use crate::open_enums::__buffa::oneof::open_enum_contexts::Choice;
    use crate::open_enums::{OpenEnumContexts, Priority};

    let msg = OpenEnumContexts::decode(&mut unknown_wire().as_slice()).unwrap();

    assert_eq!(msg.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(
        msg.rep,
        vec![
            EnumValue::Known(Priority::LOW),
            EnumValue::Unknown(99),
            EnumValue::Known(Priority::HIGH)
        ]
    );
    assert_eq!(
        msg.rep_packed,
        vec![
            EnumValue::Known(Priority::LOW),
            EnumValue::Unknown(99),
            EnumValue::Known(Priority::HIGH)
        ]
    );
    assert_eq!(
        msg.choice,
        Some(Choice::OneofPriority(EnumValue::Unknown(77)))
    );
    assert_eq!(msg.labels.get("unknown"), Some(&EnumValue::Unknown(88)));
    assert_eq!(msg.closed_control, None);

    let unknowns: Vec<_> = msg.__buffa_unknown_fields.iter().collect();
    assert_eq!(
        unknowns.len(),
        1,
        "overridden enum values must not double-retain"
    );
    assert_eq!(
        unknowns[0].number, 5,
        "only the closed control field is unknown"
    );
}

#[test]
fn open_enum_override_preserves_unknown_values_without_unknown_fields() {
    use crate::open_enums_no_unknowns::{OpenEnumNoUnknowns, OpenEnumNoUnknownsView, Priority};

    let wire = no_unknowns_wire();
    let msg = OpenEnumNoUnknowns::decode(&mut wire.as_slice()).unwrap();

    assert_eq!(msg.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(
        msg.rep,
        vec![EnumValue::Known(Priority::LOW), EnumValue::Unknown(99),]
    );
    assert_eq!(
        msg.rep_packed,
        vec![
            EnumValue::Known(Priority::LOW),
            EnumValue::Unknown(99),
            EnumValue::Known(Priority::HIGH),
        ]
    );
    assert_eq!(msg.labels.get("unknown"), Some(&EnumValue::Unknown(88)));
    assert_eq!(msg.closed_control, None);

    let view = OpenEnumNoUnknownsView::decode_view(&wire).unwrap();
    assert_eq!(view.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(view.labels.get(&"unknown"), Some(&EnumValue::Unknown(88)));
    let owned = view.to_owned_message().unwrap();
    assert_eq!(owned.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(owned.closed_control, None);
}

#[test]
fn view_decode_matches_owned_open_enum_override() {
    use crate::open_enums::__buffa::view::oneof::open_enum_contexts::Choice;
    use crate::open_enums::{OpenEnumContextsView, Priority};

    let wire = unknown_wire();
    let view = OpenEnumContextsView::decode_view(&wire).unwrap();

    assert_eq!(view.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(
        view.rep.iter().copied().collect::<Vec<_>>(),
        vec![
            EnumValue::Known(Priority::LOW),
            EnumValue::Unknown(99),
            EnumValue::Known(Priority::HIGH)
        ]
    );
    assert_eq!(
        view.rep_packed.iter().copied().collect::<Vec<_>>(),
        vec![
            EnumValue::Known(Priority::LOW),
            EnumValue::Unknown(99),
            EnumValue::Known(Priority::HIGH)
        ]
    );
    match view.choice {
        Some(Choice::OneofPriority(v)) => assert_eq!(v, EnumValue::Unknown(77)),
        other => panic!("unexpected view oneof value: {other:?}"),
    }
    assert_eq!(view.labels.get(&"unknown"), Some(&EnumValue::Unknown(88)));

    let owned = view.to_owned_message().unwrap();
    assert_eq!(owned.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(owned.labels.get("unknown"), Some(&EnumValue::Unknown(88)));
    assert_eq!(owned.__buffa_unknown_fields.iter().count(), 1);
}

#[test]
fn lazy_view_child_decode_uses_open_enum_override() {
    use crate::open_enums::{NonZeroFirst, Priority};
    use buffa::view::LazyMessageView;

    let child_wire = varint_field(1, 99);
    let bytes = length_delimited_field(1, &child_wire);
    let view = crate::open_enums::LazyParentLazyView::decode_lazy(&bytes).unwrap();

    let child = view.child.get().unwrap().expect("child set");
    assert_eq!(child.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(child.level, EnumValue::Known(Priority::HIGH));
    assert!(!child.has_level());

    let owned = view.to_owned_message().unwrap();
    assert_eq!(owned.child.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(owned.child.level, EnumValue::Known(Priority::HIGH));

    let implicit = crate::open_enums::RequiredImplicitDefaultLazyView::decode_lazy(&[]).unwrap();
    assert_eq!(implicit.level, EnumValue::Known(NonZeroFirst::NZ_HIGH));
}

#[test]
fn open_enum_override_json_unknowns_are_numbers() {
    use crate::open_enums::{OpenEnumContexts, Priority};

    let msg = OpenEnumContexts {
        opt: Some(EnumValue::Unknown(99)),
        rep: vec![EnumValue::Known(Priority::LOW), EnumValue::Unknown(99)],
        rep_packed: vec![EnumValue::Unknown(77)],
        labels: [("unknown".into(), EnumValue::Unknown(88))]
            .into_iter()
            .collect(),
        ..Default::default()
    };

    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["opt"], 99);
    assert_eq!(json["rep"][0], "LOW");
    assert_eq!(json["rep"][1], 99);
    assert_eq!(json["repPacked"][0], 77);
    assert_eq!(json["labels"]["unknown"], 88);

    let decoded: OpenEnumContexts = serde_json::from_value(serde_json::json!({
        "opt": 99,
        "rep": ["LOW", 99],
        "repPacked": [77],
        "labels": { "unknown": 88 },
    }))
    .unwrap();
    assert_eq!(decoded.opt, Some(EnumValue::Unknown(99)));
    assert_eq!(
        decoded.rep,
        vec![EnumValue::Known(Priority::LOW), EnumValue::Unknown(99)]
    );
    assert_eq!(decoded.rep_packed, vec![EnumValue::Unknown(77)]);
    assert_eq!(decoded.labels.get("unknown"), Some(&EnumValue::Unknown(88)));
}

#[test]
fn open_enum_override_required_defaults_are_known_enum_values() {
    use crate::open_enums::{NonZeroFirst, Priority, RequiredDefault, RequiredImplicitDefault};

    let mut msg = RequiredDefault::default();
    assert_eq!(msg.level, EnumValue::Known(Priority::HIGH));

    msg.level = EnumValue::Unknown(99);
    msg.clear();
    assert_eq!(msg.level, EnumValue::Known(Priority::HIGH));

    let mut implicit = RequiredImplicitDefault::default();
    assert_eq!(implicit.level, EnumValue::Known(NonZeroFirst::NZ_HIGH));

    implicit.level = EnumValue::Unknown(99);
    implicit.clear();
    assert_eq!(implicit.level, EnumValue::Known(NonZeroFirst::NZ_HIGH));
}

#[test]
fn open_enum_override_required_view_defaults_are_known_enum_values() {
    use crate::open_enums::{
        NonZeroFirst, Priority, RequiredDefaultView, RequiredImplicitDefaultView,
    };

    let view = RequiredDefaultView::decode_view(&[]).unwrap();
    assert_eq!(view.level, EnumValue::Known(Priority::HIGH));
    assert!(!view.has_level());
    assert_eq!(
        view.to_owned_message().unwrap().level,
        EnumValue::Known(Priority::HIGH)
    );

    let implicit = RequiredImplicitDefaultView::decode_view(&[]).unwrap();
    assert_eq!(implicit.level, EnumValue::Known(NonZeroFirst::NZ_HIGH));
    assert!(!implicit.has_level());
    assert_eq!(
        implicit.to_owned_message().unwrap().level,
        EnumValue::Known(NonZeroFirst::NZ_HIGH)
    );

    let present_wire = varint_field(1, 2);
    let present = RequiredDefaultView::decode_view(&present_wire).unwrap();
    assert_eq!(present.level, EnumValue::Known(Priority::HIGH));
    assert!(present.has_level());
}

#[test]
fn open_enum_override_vtable_reflection_reports_presence() {
    use crate::open_enums::{OpenEnumContexts, OpenEnumContextsView};
    use buffa_descriptor::reflect::{ReflectMessage, Reflectable, ValueRef};

    let wire = unknown_wire();
    let msg = OpenEnumContexts::decode(&mut wire.as_slice()).unwrap();
    let reflected = msg.reflect();
    let md = reflected.message_descriptor();
    let opt = md.field(1).unwrap();
    let closed_control = md.field(5).unwrap();

    assert!(reflected.has(opt));
    assert!(matches!(reflected.get(opt), ValueRef::EnumNumber(99)));
    assert!(!reflected.has(closed_control));

    let dynamic = reflected.to_dynamic();
    assert!(dynamic.has(opt));
    assert!(matches!(dynamic.get(opt), ValueRef::EnumNumber(99)));

    let view = OpenEnumContextsView::decode_view(&wire).unwrap();
    let reflected_view: &dyn ReflectMessage = &view;
    assert!(reflected_view.has(opt));
    assert!(matches!(reflected_view.get(opt), ValueRef::EnumNumber(99)));
}

#[test]
fn open_enum_override_required_vtable_reflection_uses_generated_default() {
    use crate::open_enums::{RequiredDefault, RequiredDefaultView, RequiredImplicitDefault};
    use buffa_descriptor::reflect::{ReflectMessage, Reflectable, ValueRef};

    let msg = RequiredDefault::default();
    let reflected = msg.reflect();
    let field = reflected.message_descriptor().field(1).unwrap();
    assert!(!reflected.has(field));
    assert!(matches!(reflected.get(field), ValueRef::EnumNumber(2)));

    let mut changed = RequiredImplicitDefault::default();
    let reflected = changed.reflect();
    let field = reflected.message_descriptor().field(1).unwrap();
    assert!(!reflected.has(field));
    assert!(matches!(reflected.get(field), ValueRef::EnumNumber(2)));

    changed.level = EnumValue::Unknown(99);
    let reflected = changed.reflect();
    let field = reflected.message_descriptor().field(1).unwrap();
    assert!(reflected.has(field));
    assert!(matches!(reflected.get(field), ValueRef::EnumNumber(99)));

    let default_view = RequiredDefaultView::decode_view(&[]).unwrap();
    let reflected_view: &dyn ReflectMessage = &default_view;
    let field = reflected_view.message_descriptor().field(1).unwrap();
    assert!(!reflected_view.has(field));
    assert!(matches!(reflected_view.get(field), ValueRef::EnumNumber(2)));

    let unknown_wire = varint_field(1, 99);
    let unknown_view = RequiredDefaultView::decode_view(&unknown_wire).unwrap();
    let reflected_view: &dyn ReflectMessage = &unknown_view;
    let field = reflected_view.message_descriptor().field(1).unwrap();
    assert!(reflected_view.has(field));
    assert!(matches!(
        reflected_view.get(field),
        ValueRef::EnumNumber(99)
    ));

    let present_wire = varint_field(1, 2);
    let present_view = RequiredDefaultView::decode_view(&present_wire).unwrap();
    let reflected_view: &dyn ReflectMessage = &present_view;
    let field = reflected_view.message_descriptor().field(1).unwrap();
    assert!(reflected_view.has(field));
    assert!(matches!(reflected_view.get(field), ValueRef::EnumNumber(2)));
}

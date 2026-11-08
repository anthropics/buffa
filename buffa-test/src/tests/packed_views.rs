//! `packed_views()` — packed varint fields of eager and lazy views as
//! borrowed `PackedView`s (`protos/packed_views.proto`), and `packed_views_in`
//! selecting some of them (`protos/packed_views_scoped.proto`).

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::view::LazyMessageView;
use buffa::{EnumValue, Message, MessageView, ViewEncode};

use crate::packedviews::{Color, Holder, HolderLazyView, Packed, PackedView};

fn sample(seed: i64) -> Packed {
    Packed {
        i32s: vec![-1, 0, seed as i32, i32::MIN],
        i64s: vec![-seed, i64::MAX],
        u32s: vec![0, 300, u32::MAX],
        u64s: vec![u64::MAX, seed as u64],
        s32s: vec![-(seed as i32), i32::MAX],
        s64s: vec![i64::MIN, seed],
        bools: vec![true, false, true],
        f32s: vec![7, u32::MAX],
        colors: vec![EnumValue::from(Color::COLOR_RED)],
        names: vec!["a".into(), "bc".into()],
        unpacked: vec![1, 2, 3],
        ..Default::default()
    }
}

fn assert_fields(view: &PackedView<'_>, owned: &Packed) {
    assert_eq!(view.i32s.to_vec(), owned.i32s);
    assert_eq!(view.i64s.to_vec(), owned.i64s);
    assert_eq!(view.u32s.iter().collect::<Vec<_>>(), owned.u32s);
    assert_eq!(view.u64s.to_vec(), owned.u64s);
    assert_eq!(view.s32s.to_vec(), owned.s32s);
    assert_eq!(view.s64s.to_vec(), owned.s64s);
    assert_eq!(view.bools.to_vec(), owned.bools);
    assert_eq!(&*view.f32s, owned.f32s.as_slice());
    assert_eq!(&*view.unpacked, owned.unpacked.as_slice());
}

#[test]
fn eager_view_round_trips() {
    let owned = sample(1000);
    let bytes = owned.encode_to_vec();
    let view = PackedView::decode_view(&bytes).unwrap();
    assert_fields(&view, &owned);
    assert_eq!(view.to_owned_message(), Ok(owned.clone()));
    assert_eq!(view.encode_to_vec(), bytes);
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        serde_json::to_value(&owned).unwrap()
    );
}

#[test]
fn split_and_unpacked_records_join() {
    // Two encodings of a message concatenate their repeated fields; then an
    // unpacked record of a packed field, which decoders must accept.
    let (a, b) = (sample(1), sample(2));
    let mut bytes = a.encode_to_vec();
    bytes.extend(b.encode_to_vec());
    Tag::new(3, WireType::Varint).encode(&mut bytes);
    encode_varint(42, &mut bytes);
    let mut expected = a;
    expected.merge_from_slice(&b.encode_to_vec()).unwrap();
    expected.u32s.push(42);

    let view = PackedView::decode_view(&bytes).unwrap();
    assert_fields(&view, &expected);
    assert_eq!(view.to_owned_message(), Ok(expected.clone()));
    assert_eq!(view.encode_to_vec(), expected.encode_to_vec());
}

#[test]
fn malformed_payloads_fail_as_owned_decoding_does() {
    for payload in [
        &[0x80][..],
        &[0xff; 11],
        &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02],
    ] {
        let mut bytes = Vec::new();
        Tag::new(3, WireType::LengthDelimited).encode(&mut bytes);
        encode_varint(payload.len() as u64, &mut bytes);
        bytes.extend_from_slice(payload);
        assert_eq!(
            PackedView::decode_view(&bytes).map(|_| ()),
            Packed::decode_from_slice(&bytes).map(|_| ()),
            "{payload:02x?}"
        );
    }
}

#[test]
fn lazy_views_hold_packed_views() {
    let owned = Holder {
        items: vec![sample(2), sample(3)],
        ..Default::default()
    };
    let bytes = owned.encode_to_vec();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    for (item, expected) in view.items.iter().zip(&owned.items) {
        assert_eq!(item.unwrap().s64s.to_vec(), expected.s64s);
    }
    assert_eq!(view.to_owned_message(), Ok(owned));
}

#[test]
fn scoped_rules_mix_packed_and_repeated_views() {
    use crate::packedviewsscoped::{mixed, Mixed, MixedLazyView, MixedView};

    let owned = Mixed {
        selected: vec![0, 300, u32::MAX],
        unlisted: vec![i64::MIN, -1, 7],
        inner: mixed::Inner {
            nested: vec![u64::MAX, 1],
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let bytes = owned.encode_to_vec();
    let view = MixedView::decode_view(&bytes).unwrap();
    // Selected by a field rule, and by a message rule on the nested message.
    let selected: &buffa::PackedView<'_, buffa::packed_view::Uint32> = &view.selected;
    assert_eq!(selected.to_vec(), owned.selected);
    let nested: &buffa::PackedView<'_, buffa::packed_view::Uint64> = &view.inner.nested;
    assert_eq!(nested.to_vec(), owned.inner.nested);
    // Matched by no rule: still a slice.
    let unlisted: &[i64] = &view.unlisted;
    assert_eq!(unlisted, owned.unlisted.as_slice());

    assert_eq!(view.to_owned_message(), Ok(owned.clone()));
    assert_eq!(view.encode_to_vec(), bytes);
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        serde_json::to_value(&owned).unwrap()
    );
    let lazy = MixedLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(lazy.selected.to_vec(), owned.selected);
    assert_eq!(&*lazy.unlisted, owned.unlisted.as_slice());
    assert_eq!(lazy.to_owned_message(), Ok(owned));
}

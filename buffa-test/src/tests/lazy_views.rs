//! `lazy_views(true)` — the additive `FooLazyView` family (`protos/lazy_views.proto`).
//!
//! Singular message fields become `LazyMessageFieldView` and repeated message
//! fields `LazyRepeatedView`; oneof message variants, map message values, and
//! scalars keep their eager representation.

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::{DecodeError, Message};

use crate::lazyviews::{
    payload, Holder, HolderLazyView, Node, NodeLazyView, Pair, Payload, PayloadLazyView,
};
use buffa::view::LazyMessageView;

fn pair(k: &str, v: &str) -> Pair {
    Pair {
        k: k.into(),
        v: v.into(),
        ..Default::default()
    }
}

fn sample_payload(i: usize) -> Payload {
    let mut p = Payload::default();
    p.name = format!("payload-{i}");
    p.data = vec![i as u8; 4];
    p.pair = pair("pk", &format!("pv-{i}")).into();
    for j in 0..3 {
        p.pairs.push(pair(&format!("k{j}"), &format!("v{j}-{i}")));
    }
    p.by_key.insert("map-key".into(), pair("mk", "mv"));
    p.kind = Some(payload::Kind::PairKind(Box::new(pair("ok", "ov"))));
    p.tags.push(format!("tag-{i}"));
    p
}

fn sample_holder() -> Holder {
    let mut h = Holder::default();
    h.payload = sample_payload(0).into();
    for i in 1..=3 {
        h.items.push(sample_payload(i));
    }
    h
}

/// Wrap `inner` as a LengthDelimited occurrence of `field_number`.
fn ld_field(field_number: u32, inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    Tag::new(field_number, WireType::LengthDelimited).encode(&mut out);
    encode_varint(inner.len() as u64, &mut out);
    out.extend_from_slice(inner);
    out
}

#[test]
fn field_parity_with_owned() {
    let owned = sample_holder();
    let bytes = owned.encode_to_vec();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();

    let p = view.payload.get().unwrap().expect("payload set");
    assert_eq!(p.name, owned.payload.name);
    assert_eq!(p.data, owned.payload.data.as_slice());
    let inner = p.pair.get().unwrap().expect("pair set");
    assert_eq!(inner.k, "pk");
    assert_eq!(inner.v, "pv-0");

    assert_eq!(view.items.len(), 3);
    for (i, item) in view.items.iter().enumerate() {
        let item = item.unwrap();
        assert_eq!(item.name, owned.items[i].name);
        assert_eq!(item.pairs.len(), 3);
        assert_eq!(item.pairs.get(1).unwrap().unwrap().k, "k1");
        assert_eq!(item.pairs.try_get(1).unwrap().unwrap().k, "k1");
        assert!(item.pairs.try_get(9).unwrap().is_none());
    }
    assert!(view.items.get(3).is_none());
}

#[test]
fn unset_and_empty_defaults() {
    let bytes = Holder::default().encode_to_vec();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert!(view.payload.is_unset());
    assert!(view.payload.get().unwrap().is_none());
    assert!(view.items.is_empty());
    assert_eq!(view.items.iter().count(), 0);
}

#[test]
fn singular_merge_across_fragments() {
    // `payload` appears twice on the wire; decoders must merge (frag 1 sets
    // `name`, frag 2 sets `data`).
    let frag1 = Payload {
        name: "from-frag-1".into(),
        ..Default::default()
    }
    .encode_to_vec();
    let frag2 = Payload {
        data: b"from-frag-2".to_vec(),
        ..Default::default()
    }
    .encode_to_vec();
    let mut bytes = ld_field(1, &frag1);
    bytes.extend_from_slice(&ld_field(1, &frag2));

    let owned = Holder::decode(&mut bytes.as_slice()).unwrap();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(view.payload.fragments().len(), 2);
    let p = view.payload.get().unwrap().expect("set");
    assert_eq!(p.name, owned.payload.name);
    assert_eq!(p.data, owned.payload.data.as_slice());

    // Owned conversion and re-encode agree with the owned decoder too.
    assert_eq!(view.to_owned_message().unwrap(), owned);
}

#[test]
fn to_owned_round_trip_preserves_unknown_fields() {
    // Unknown field 99 both at the top level and inside the deferred payload.
    let mut payload_bytes = sample_payload(7).encode_to_vec();
    payload_bytes.extend_from_slice(&crate::tests::varint_field(99, 5));
    let mut bytes = ld_field(1, &payload_bytes);
    bytes.extend_from_slice(&crate::tests::varint_field(99, 6));

    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    let owned = view.to_owned_message().unwrap();
    assert_eq!(owned, Holder::decode(&mut bytes.as_slice()).unwrap());

    // Round-tripping the owned message keeps both unknown fields on the wire.
    let reenc = owned.encode_to_vec();
    let redecoded = Holder::decode(&mut reenc.as_slice()).unwrap();
    assert_eq!(redecoded, owned);
}

#[test]
fn reencode_replays_fragments() {
    let owned = sample_holder();
    let bytes = owned.encode_to_vec();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();

    let mut cache = buffa::SizeCache::new();
    let size = view.compute_size(&mut cache) as usize;
    let mut reenc = Vec::with_capacity(size);
    view.write_to(&mut cache, &mut reenc);
    assert_eq!(reenc.len(), size);
    assert_eq!(Holder::decode(&mut reenc.as_slice()).unwrap(), owned);
}

#[test]
fn reencode_interleaves_lazy_and_cache_fields() {
    // PayloadView mixes lazy fields (pair, pairs) with SizeCache users (map
    // by_key, oneof) — locks in the reserve/consume slot-ordering invariant.
    let owned = sample_payload(2);
    let bytes = owned.encode_to_vec();
    let view = PayloadLazyView::decode_lazy(&bytes).unwrap();

    let mut cache = buffa::SizeCache::new();
    let size = view.compute_size(&mut cache) as usize;
    let mut reenc = Vec::with_capacity(size);
    view.write_to(&mut cache, &mut reenc);
    assert_eq!(reenc.len(), size);
    assert_eq!(Payload::decode(&mut reenc.as_slice()).unwrap(), owned);
}

#[test]
fn empty_fragment_merges_as_empty_message() {
    // A zero-length occurrence (empty sub-message) merged with a non-empty
    // one must behave like the owned decoder.
    let frag = Payload {
        name: "set".into(),
        ..Default::default()
    }
    .encode_to_vec();
    let mut bytes = ld_field(1, &[]);
    bytes.extend_from_slice(&ld_field(1, &frag));
    bytes.extend_from_slice(&ld_field(1, &[]));

    let owned = Holder::decode(&mut bytes.as_slice()).unwrap();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(view.payload.fragments().len(), 3);
    let p = view.payload.get().unwrap().expect("set");
    assert_eq!(p.name, "set");
    assert_eq!(view.to_owned_message().unwrap(), owned);
}

#[test]
fn json_matches_owned() {
    let owned = sample_holder();
    let bytes = owned.encode_to_vec();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(
        serde_json::to_string(&view).unwrap(),
        serde_json::to_string(&owned).unwrap()
    );
}

#[test]
fn json_surfaces_malformed_deferred_bytes() {
    let bytes = ld_field(1, &[0xFF, 0xFF, 0xFF]);
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert!(serde_json::to_string(&view).is_err());
}

#[test]
fn oneof_and_map_message_values_stay_eager() {
    let owned = sample_payload(1);
    let bytes = owned.encode_to_vec();
    let view = PayloadLazyView::decode_lazy(&bytes).unwrap();

    // Oneof message variant: eagerly decoded, boxed view.
    match view.kind {
        Some(crate::lazyviews::__buffa::view::oneof::payload::Kind::PairKind(ref p)) => {
            assert_eq!(p.k, "ok");
            assert_eq!(p.v, "ov");
        }
        ref other => panic!("expected PairKind, got {other:?}"),
    }

    // Map message value: eagerly decoded PairView.
    let (k, v) = view.by_key.iter().next().expect("one entry");
    assert_eq!(*k, "map-key");
    assert_eq!(v.k, "mk");
}

#[test]
fn deep_recursion_budget_flows_through_lazy_access() {
    // 300 nested levels: above RECURSION_LIMIT (100). The owned decoder
    // rejects the input up front; the lazy decode_view succeeds (it never
    // recurses), but navigation charges the budget recorded at decode time
    // and fails at the same boundary instead of overflowing the stack.
    let mut cur = ld_field(1, b"leaf");
    for _ in 0..300 {
        cur = ld_field(2, &cur);
    }
    assert!(matches!(
        Node::decode(&mut cur.as_slice()),
        Err(DecodeError::RecursionLimitExceeded)
    ));

    let mut node = NodeLazyView::decode_lazy(&cur).unwrap();
    let mut depth = 0;
    let err = loop {
        match node.child.get() {
            Ok(Some(child)) => {
                node = child;
                depth += 1;
            }
            Ok(None) => panic!("hit the leaf before the recursion limit"),
            Err(e) => break e,
        }
    };
    assert!(matches!(err, DecodeError::RecursionLimitExceeded));
    assert!(depth < 300, "budget must bound navigation, got {depth}");

    // A custom limit set at the outer decode flows through lazy boundaries.
    let opts = buffa::DecodeOptions::new().with_recursion_limit(400);
    let mut node: NodeLazyView<'_> = opts.decode_lazy_view(&cur).unwrap();
    let mut depth = 0;
    while let Some(child) = node.child.get().unwrap() {
        node = child;
        depth += 1;
    }
    assert_eq!(depth, 300);
    assert_eq!(node.label, "leaf");
}

#[test]
fn deep_recursion_to_owned_errors() {
    // to_owned on over-deep lazy input must fail with the recursion-limit
    // error through the fallible conversion, not a stack overflow abort.
    let mut cur = ld_field(1, b"leaf");
    for _ in 0..300 {
        cur = ld_field(2, &cur);
    }
    let view = NodeLazyView::decode_lazy(&cur).unwrap();
    assert!(matches!(
        view.to_owned_message(),
        Err(buffa::DecodeError::RecursionLimitExceeded)
    ));
}

#[test]
fn deep_recursion_json_errors() {
    let mut cur = ld_field(1, b"leaf");
    for _ in 0..300 {
        cur = ld_field(2, &cur);
    }
    let view = NodeLazyView::decode_lazy(&cur).unwrap();
    let err = serde_json::to_string(&view).unwrap_err();
    assert!(err.to_string().contains("recursion limit"), "{err}");
}

#[test]
fn recursive_repeated_children() {
    let mut root = Node::default();
    root.label = "root".into();
    for i in 0..3 {
        root.children.push(Node {
            label: format!("child-{i}"),
            ..Default::default()
        });
    }
    let bytes = root.encode_to_vec();
    let view = NodeLazyView::decode_lazy(&bytes).unwrap();
    let labels: Vec<String> = view
        .children
        .iter()
        .map(|c| c.unwrap().label.to_string())
        .collect();
    assert_eq!(labels, ["child-0", "child-1", "child-2"]);
    assert_eq!(view.to_owned_message().unwrap(), root);
}

#[test]
fn malformed_deferred_bytes_error_on_access() {
    // Truncated varint tag inside the deferred payload: the outer decode
    // succeeds (laziness), the access fails.
    let bytes = ld_field(1, &[0xFF, 0xFF, 0xFF]);
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert!(view.payload.is_set());
    assert!(view.payload.get().is_err());

    let bytes = ld_field(2, &[0xFF, 0xFF, 0xFF]);
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert!(view.items.get(0).unwrap().is_err());
}

#[test]
fn malformed_deferred_bytes_error_in_to_owned() {
    let bytes = ld_field(1, &[0xFF, 0xFF, 0xFF]);
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert!(view.to_owned_message().is_err());
}

#[test]
fn eager_family_unchanged_and_coexists() {
    let owned = sample_holder();
    let bytes = owned.encode_to_vec();
    // Eager view + OwnedView wrapper: untouched by the flag.
    use buffa::view::MessageView;
    let eager = crate::lazyviews::HolderView::decode_view(&bytes).unwrap();
    assert_eq!(eager.to_owned_message().unwrap(), owned);
    let wrapper = crate::lazyviews::HolderOwnedView::from_owned(&owned).unwrap();
    assert_eq!(wrapper.items().len(), 3);
    assert_eq!(
        wrapper.payload().as_option().expect("set").name,
        "payload-0"
    );
    // The lazy family decodes the same bytes to the same owned message.
    let lazy = HolderLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(lazy.to_owned_message().unwrap(), owned);
}

#[test]
fn extern_wkt_field_stays_eager() {
    // Extern targets (here `google.protobuf.Timestamp` from buffa-types) may
    // not ship a lazy family, so the lazy view keeps them eagerly decoded.
    let mut owned = sample_holder();
    owned.stamped_at = buffa_types::google::protobuf::Timestamp {
        seconds: 1_700_000_000,
        nanos: 42,
        ..Default::default()
    }
    .into();
    let bytes = owned.encode_to_vec();
    let view = HolderLazyView::decode_lazy(&bytes).unwrap();
    let ts = view.stamped_at.as_option().expect("set");
    assert_eq!((ts.seconds, ts.nanos), (1_700_000_000, 42));
    assert_eq!(view.to_owned_message().unwrap(), owned);
}

#[test]
fn unknown_field_allowance_flows_through_lazy_boundary() {
    // The allowance remaining at the deferred field's record site is captured
    // and replayed per access — a per-subtree approximation of the shared
    // pool, not the exact shared accounting of a fully-eager decode. View
    // decoding charges one unit per *coalesced unknown span*.
    let mut payload_bytes = Payload {
        name: "x".into(),
        ..Default::default()
    }
    .encode_to_vec();
    payload_bytes.extend_from_slice(&crate::tests::varint_field(90, 1));
    // One top-level unknown before the deferred field, so the recorded
    // allowance is the configured limit minus what the outer scan consumed.
    let mut bytes = crate::tests::varint_field(99, 7);
    bytes.extend_from_slice(&ld_field(1, &payload_bytes));

    // Limit 1: the outer unknown consumes the whole pool, the deferred
    // subtree records allowance 0 and its own unknown exceeds it on access.
    let opts = buffa::DecodeOptions::new().with_unknown_field_limit(1);
    let view: HolderLazyView<'_> = opts.decode_lazy_view(&bytes).unwrap();
    assert!(matches!(
        view.payload.get(),
        Err(DecodeError::UnknownFieldLimitExceeded)
    ));

    // Limit 2: allowance 1 reaches the deferred subtree — access succeeds.
    let opts = buffa::DecodeOptions::new().with_unknown_field_limit(2);
    let view: HolderLazyView<'_> = opts.decode_lazy_view(&bytes).unwrap();
    assert_eq!(view.payload.get().unwrap().expect("set").name, "x");
}

// --- preserve_unknown_fields(false) family (`protos/lazy_views_lean.proto`) ---

#[test]
fn lean_lazy_drops_unknown_fields() {
    use crate::lazyviewslean::{Counters, LeanHolder, LeanHolderLazyView};

    let owned = LeanHolder {
        counters: Counters {
            hits: 7,
            total: 99,
            live: true,
        }
        .into(),
        history: vec![
            Counters::default(),
            Counters {
                hits: 1,
                ..Default::default()
            },
        ],
        label: "lean".into(),
    };
    let mut bytes = crate::tests::varint_field(90, 5);
    bytes.extend_from_slice(&owned.encode_to_vec());
    bytes.extend_from_slice(&crate::tests::varint_field(91, 6));

    let view = LeanHolderLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(view.label, "lean");
    assert_eq!(view.history.len(), 2);
    assert_eq!(view.counters.get().unwrap().expect("set").hits, 7);
    // No preservation: the round-trip equals the unknown-free encoding.
    assert_eq!(view.to_owned_message().unwrap(), owned);
    assert_eq!(view.encode_to_vec(), owned.encode_to_vec());
}

#[test]
fn lean_all_scalar_lazy_struct() {
    use crate::lazyviewslean::{Counters, CountersLazyView};

    // Counters has no borrowing fields — its lazy struct is anchored by
    // PhantomData. Decode + read + convert must still work.
    let owned = Counters {
        hits: -3,
        total: u64::MAX,
        live: true,
    };
    let bytes = owned.encode_to_vec();
    let view = CountersLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!((view.hits, view.total, view.live), (-3, u64::MAX, true));
    assert_eq!(view.to_owned_message().unwrap(), owned);
}

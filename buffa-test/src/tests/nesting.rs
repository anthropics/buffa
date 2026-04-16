//! Deep nesting, recursion through oneofs + singular message fields,
//! TreeNode, Expr/BinaryOp mutual recursion, Corecursive direct recursion,
//! view merge semantics for message fields and oneofs.

use super::round_trip;
use buffa::Message;

#[test]
fn test_deep_nesting_round_trip() {
    use crate::nested;
    let msg = nested::Outer {
        label: "root".into(),
        child: buffa::MessageField::some(nested::outer::Middle {
            value: 42,
            detail: buffa::MessageField::some(nested::outer::middle::Inner {
                data: vec![0xFF],
                active: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        content: Some(nested::outer::ContentOneof::Text("hello".into())),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.label, "root");
    assert_eq!(decoded.child.value, 42);
    assert!(decoded.child.detail.active);
}

#[test]
fn test_tree_node_recursive() {
    use crate::nested::TreeNode;
    let tree = TreeNode {
        name: "root".into(),
        children: vec![
            TreeNode {
                name: "child1".into(),
                children: vec![TreeNode {
                    name: "grandchild".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            TreeNode {
                name: "child2".into(),
                ..Default::default()
            },
        ],
        attributes: [("color".into(), "blue".into())].into_iter().collect(),
        ..Default::default()
    };
    let decoded = round_trip(&tree);
    assert_eq!(decoded.name, "root");
    assert_eq!(decoded.children.len(), 2);
    assert_eq!(decoded.children[0].children[0].name, "grandchild");
    assert_eq!(decoded.attributes["color"], "blue");
}

#[test]
fn test_multi_oneof_variants() {
    use crate::nested::{self, MultiOneof};

    // Test each variant type round-trips correctly.
    let cases: Vec<MultiOneof> = vec![
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::IntVal(42)),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::LongVal(i64::MAX)),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::FloatVal(1.5)),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::DoubleVal(
                std::f64::consts::PI,
            )),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::BoolVal(true)),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::StringVal("hello".into())),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::BytesVal(vec![0xFF, 0x00])),
            ..Default::default()
        },
        MultiOneof {
            value: Some(nested::multi_oneof::ValueOneof::MessageVal(Box::new(
                nested::outer::middle::Inner {
                    data: vec![1, 2, 3],
                    active: true,
                    ..Default::default()
                },
            ))),
            ..Default::default()
        },
    ];

    for msg in &cases {
        let decoded = round_trip(msg);
        assert_eq!(&decoded, msg);
    }
}

#[test]
fn test_recursive_oneof_direct() {
    // Expr { kind { Expr negated = 3; } } is directly self-recursive
    // through the oneof. Message/group variants are always boxed to
    // break the infinite-size cycle.
    use crate::nested::{expr, Expr};
    let inner = Expr {
        kind: Some(expr::KindOneof::IntLiteral(42)),
        ..Default::default()
    };
    let outer = Expr {
        kind: Some(expr::KindOneof::Negated(Box::new(inner))),
        ..Default::default()
    };
    let decoded = round_trip(&outer);
    assert_eq!(decoded, outer);
    // Verify deref through Box works transparently in pattern matching.
    match &decoded.kind {
        Some(expr::KindOneof::Negated(e)) => match &e.kind {
            Some(expr::KindOneof::IntLiteral(n)) => assert_eq!(*n, 42),
            other => panic!("expected IntLiteral, got {other:?}"),
        },
        other => panic!("expected Negated, got {other:?}"),
    }
}

#[test]
fn test_recursive_oneof_mutual() {
    // Expr -> BinaryOp -> Expr mutual recursion. BinaryOp fields use
    // MessageField (already boxed); the Expr.kind.binary variant is
    // the boxed side of the cycle.
    use crate::nested::{expr, BinaryOp, Expr};
    let lhs = Expr {
        kind: Some(expr::KindOneof::IntLiteral(1)),
        ..Default::default()
    };
    let rhs = Expr {
        kind: Some(expr::KindOneof::IntLiteral(2)),
        ..Default::default()
    };
    let op = BinaryOp {
        op: "+".into(),
        lhs: buffa::MessageField::some(lhs),
        rhs: buffa::MessageField::some(rhs),
        ..Default::default()
    };
    // Use the generated From impl instead of manual Box::new.
    let expr = Expr {
        kind: Some(expr::KindOneof::from(op)),
        ..Default::default()
    };
    let decoded = round_trip(&expr);
    assert_eq!(decoded, expr);
}

#[test]
fn test_from_msg_for_option_oneof() {
    // `From<Msg> for Option<Oneof>` lets struct-literal construction skip both
    // the explicit `Some(...)` and `Box::new(...)` for message-typed variants.
    use crate::nested::{expr, BinaryOp, Expr};
    let op = BinaryOp {
        op: "*".into(),
        ..Default::default()
    };

    // One .into() does variant-wrap + Box + Some; field type drives inference.
    let terse = Expr {
        kind: op.clone().into(),
        ..Default::default()
    };
    let explicit = Expr {
        kind: Some(expr::KindOneof::Binary(Box::new(op))),
        ..Default::default()
    };
    assert_eq!(terse, explicit);

    // Round-trip sanity.
    let decoded = round_trip(&terse);
    match decoded.kind {
        Some(expr::KindOneof::Binary(b)) => assert_eq!(b.op, "*"),
        other => panic!("expected Binary, got {other:?}"),
    }

    // None remains spelled explicitly — no From magic for clearing.
    let cleared = Expr {
        kind: None,
        ..Default::default()
    };
    assert!(cleared.kind.is_none());
}

#[test]
fn test_recursive_oneof_merge_semantics() {
    // When the same message-typed oneof variant appears twice on the
    // wire, the second occurrence merges into the first (proto3 spec).
    use crate::nested::{expr, BinaryOp, Expr};
    let first = Expr {
        kind: Some(expr::KindOneof::from(BinaryOp {
            op: "+".into(),
            lhs: buffa::MessageField::some(Expr {
                kind: Some(expr::KindOneof::IntLiteral(1)),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let second = Expr {
        kind: Some(expr::KindOneof::from(BinaryOp {
            rhs: buffa::MessageField::some(Expr {
                kind: Some(expr::KindOneof::IntLiteral(2)),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut bytes = first.encode_to_vec();
    bytes.extend(second.encode_to_vec());
    let merged = Expr::decode(&mut bytes.as_slice()).expect("decode");
    // Both lhs (from first) and rhs (from second) should be present.
    match &merged.kind {
        Some(expr::KindOneof::Binary(b)) => {
            assert_eq!(b.op, "+");
            assert!(b.lhs.is_set(), "lhs from first merge lost");
            assert!(b.rhs.is_set(), "rhs from second merge lost");
        }
        other => panic!("expected Binary, got {other:?}"),
    }
}

#[test]
fn test_view_oneof_boxed_message_variant() {
    // View oneof enums box message/group variants for the same reason
    // as owned enums. The Box holds a lifetime-bound view struct.
    use crate::nested::{expr, Expr, ExprView};
    use buffa::MessageView;
    let inner = Expr {
        kind: Some(expr::KindOneof::IntLiteral(42)),
        ..Default::default()
    };
    let outer = Expr {
        kind: Some(expr::KindOneof::Negated(Box::new(inner))),
        ..Default::default()
    };
    let bytes = outer.encode_to_vec();
    let view = ExprView::decode_view(&bytes).expect("decode_view");
    // Pattern-matched binding auto-derefs through Box<ExprView<'_>>.
    match &view.kind {
        Some(expr::KindOneofView::Negated(v)) => match &v.kind {
            Some(expr::KindOneofView::IntLiteral(n)) => assert_eq!(*n, 42),
            other => panic!("expected IntLiteral, got {other:?}"),
        },
        other => panic!("expected Negated, got {other:?}"),
    }
    // to_owned_message round-trips through both Box levels.
    let owned = view.to_owned_message();
    assert_eq!(owned, outer);
}

#[test]
fn test_view_oneof_message_variant_to_owned() {
    // Non-recursive message variant: Middle in Outer.content.structured.
    use crate::nested::{self, Outer, OuterView};
    use buffa::MessageView;
    let msg = Outer {
        content: Some(nested::outer::ContentOneof::Structured(Box::new(
            nested::outer::Middle {
                value: 7,
                ..Default::default()
            },
        ))),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = OuterView::decode_view(&bytes).expect("decode_view");
    match &view.content {
        Some(nested::outer::ContentOneofView::Structured(m)) => assert_eq!(m.value, 7),
        other => panic!("expected Structured, got {other:?}"),
    }
    assert_eq!(view.to_owned_message(), msg);
}

#[test]
fn test_recursive_singular_message_field() {
    // Corecursive → Nested → Corecursive with NO oneof in the cycle.
    // MessageFieldView<V> boxes V internally (like MessageField<T> does)
    // to break the infinite-size cycle. Compile-time test: the types
    // exist at all (E0072 would fail compilation otherwise).
    use crate::nested::{corecursive, Corecursive};
    let msg = Corecursive {
        name: "outer".into(),
        nested: buffa::MessageField::some(corecursive::Nested {
            value: 1,
            back: buffa::MessageField::some(Corecursive {
                name: "inner".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded, msg);
    // Transparent Deref chain through two levels of MessageField.
    assert_eq!(decoded.nested.back.name, "inner");
}

#[test]
fn test_view_recursive_singular_message_field() {
    // View path through the same cycle. MessageFieldView boxes internally,
    // Deref returns &V transparently.
    use crate::nested::{corecursive, Corecursive, CorecursiveView};
    use buffa::MessageView;
    let msg = Corecursive {
        name: "root".into(),
        nested: buffa::MessageField::some(corecursive::Nested {
            value: 42,
            back: buffa::MessageField::some(Corecursive {
                name: "leaf".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = CorecursiveView::decode_view(&bytes).expect("decode_view");
    assert_eq!(view.name, "root");
    // Deref chain: MessageFieldView<NestedView> → MessageFieldView<CorecursiveView>
    assert_eq!(view.nested.value, 42);
    assert_eq!(view.nested.back.name, "leaf");
    // Round-trip through to_owned_message.
    assert_eq!(view.to_owned_message(), msg);
}

#[test]
fn test_view_message_field_merge_semantics() {
    // When a singular message field appears twice on the wire, the
    // second occurrence merges into the first field-by-field (proto
    // spec). The view decoder must do this, not replace.
    use crate::nested::{corecursive, Corecursive, CorecursiveView};
    use buffa::MessageView;
    // First: nested.value = 1, nested.back.name = "from_first"
    let first = Corecursive {
        nested: buffa::MessageField::some(corecursive::Nested {
            value: 1,
            back: buffa::MessageField::some(Corecursive {
                name: "from_first".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    // Second: nested.value = 2 (overwrites), no nested.back
    let second = Corecursive {
        nested: buffa::MessageField::some(corecursive::Nested {
            value: 2,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut wire = first.encode_to_vec();
    wire.extend(second.encode_to_vec());

    let view = CorecursiveView::decode_view(&wire).unwrap();
    // value from second (last wins for scalars).
    assert_eq!(view.nested.value, 2);
    // back.name from first (merged, second didn't set it).
    assert_eq!(view.nested.back.name, "from_first");

    // Parity: view and owned decode must produce identical output.
    let owned = Corecursive::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(
        view.to_owned_message().encode_to_vec(),
        owned.encode_to_vec()
    );
}

#[test]
fn test_view_oneof_message_variant_merge_semantics() {
    // Same merge semantics for message-typed oneof variants.
    use crate::nested::{expr, BinaryOp, Expr, ExprView};
    use buffa::MessageView;
    // First: binary with lhs set
    let first = Expr {
        kind: Some(expr::KindOneof::from(BinaryOp {
            op: "+".into(),
            lhs: buffa::MessageField::some(Expr {
                kind: Some(expr::KindOneof::IntLiteral(1)),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    // Second: binary with rhs set (no op, no lhs)
    let second = Expr {
        kind: Some(expr::KindOneof::from(BinaryOp {
            rhs: buffa::MessageField::some(Expr {
                kind: Some(expr::KindOneof::IntLiteral(2)),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut wire = first.encode_to_vec();
    wire.extend(second.encode_to_vec());

    let view = ExprView::decode_view(&wire).unwrap();
    match &view.kind {
        Some(expr::KindOneofView::Binary(b)) => {
            assert_eq!(b.op, "+", "op from first (second didn't set it)");
            assert!(b.lhs.is_set(), "lhs from first must survive merge");
            assert!(b.rhs.is_set(), "rhs from second");
        }
        other => panic!("expected Binary, got {other:?}"),
    }

    // Parity with owned decoder.
    let owned = Expr::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(
        view.to_owned_message().encode_to_vec(),
        owned.encode_to_vec()
    );
}

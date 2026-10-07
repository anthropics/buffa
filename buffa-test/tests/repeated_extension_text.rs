use buffa::text::{
    decode_from_str, decode_from_str_with_element_memory_limit, encode_to_string,
    encode_to_string_pretty, merge_from_str, ParseErrorKind,
};
use buffa::type_registry::{set_type_registry, TypeRegistry};
use buffa::{ExtensionSet, Message, UnknownField, UnknownFieldData};
use buffa_test::{extjson, groupext};

fn register_types() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let mut registry = TypeRegistry::new();
        extjson::__buffa::register_types(&mut registry);
        groupext::__buffa::register_types(&mut registry);
        set_type_registry(registry);
    });
}

fn annotation(doc: &str, priority: i32) -> extjson::Ann {
    extjson::Ann {
        doc: Some(doc.into()),
        priority: Some(priority),
        ..Default::default()
    }
}

#[test]
fn repeated_message_extension_prints_one_entry_per_element() {
    register_types();
    let expected = vec![annotation("first", 1), annotation("second", 2)];
    let mut carrier = extjson::Carrier {
        x: Some(7),
        ..Default::default()
    };
    carrier.set_extension(&extjson::__buffa::ext::ANNS, expected.clone());

    let compact = encode_to_string(&carrier);
    assert_eq!(
        compact,
        "x: 7 \
         [buffa.test.extjson.anns] {doc: \"first\" priority: 1} \
         [buffa.test.extjson.anns] {doc: \"second\" priority: 2}"
    );
    let pretty = encode_to_string_pretty(&carrier);
    assert_eq!(
        pretty,
        "x: 7\n\
         [buffa.test.extjson.anns] {\n  doc: \"first\"\n  priority: 1\n}\n\
         [buffa.test.extjson.anns] {\n  doc: \"second\"\n  priority: 2\n}\n"
    );
    for text in [compact, pretty] {
        let decoded: extjson::Carrier = decode_from_str(&text).unwrap();
        assert_eq!(decoded, carrier, "{text}");
        assert_eq!(decoded.extension(&extjson::__buffa::ext::ANNS), expected);
    }
}

#[test]
fn repeated_group_extension_prints_one_entry_per_element() {
    register_types();
    let expected = vec![
        groupext::Inner {
            c: Some(1),
            ..Default::default()
        },
        groupext::Inner {
            c: Some(2),
            ..Default::default()
        },
    ];
    let mut carrier = groupext::Carrier::default();
    carrier.set_extension(&groupext::__buffa::ext::DELIM_REPEATED, expected.clone());

    let compact = encode_to_string(&carrier);
    assert_eq!(
        compact,
        "[buffa.test.groupext.delim_repeated] {c: 1} \
         [buffa.test.groupext.delim_repeated] {c: 2}"
    );
    let pretty = encode_to_string_pretty(&carrier);
    assert_eq!(
        pretty,
        "[buffa.test.groupext.delim_repeated] {\n  c: 1\n}\n\
         [buffa.test.groupext.delim_repeated] {\n  c: 2\n}\n"
    );
    for text in [compact, pretty] {
        let decoded: groupext::Carrier = decode_from_str(&text).unwrap();
        assert_eq!(decoded, carrier, "{text}");
        assert_eq!(
            decoded.extension(&groupext::__buffa::ext::DELIM_REPEATED),
            expected
        );
    }
}

/// The entries of a repeated extension between a regular field and a singular
/// extension each carry their own name, including an empty middle element.
#[test]
fn repeated_extension_entries_keep_their_own_name_between_other_fields() {
    register_types();
    let text = "x: 7 \
                [buffa.test.extjson.anns] {priority: 1} \
                [buffa.test.extjson.anns] {} \
                [buffa.test.extjson.anns] {priority: 3} \
                [buffa.test.extjson.ann] {doc: \"single\"}";
    let carrier: extjson::Carrier = decode_from_str(text).unwrap();
    assert_eq!(carrier.extension(&extjson::__buffa::ext::ANNS).len(), 3);
    assert_eq!(encode_to_string(&carrier), text);
}

fn tree(v: i32) -> extjson::Tree {
    extjson::Tree {
        v: Some(v),
        ..Default::default()
    }
}

/// An element that writes repeated and singular extensions of its own sits
/// between two entries of the outer repeated extension.
#[test]
fn nested_repeated_extensions_keep_the_name_of_their_own_level() {
    register_types();
    let kids = &extjson::__buffa::ext::KIDS;
    let kid = &extjson::__buffa::ext::KID;
    let mut first = tree(2);
    first.set_extension(kids, vec![tree(3), tree(4)]);
    first.set_extension(kid, tree(5));
    let mut root = tree(1);
    root.set_extension(kids, vec![first, tree(6)]);

    let compact = encode_to_string(&root);
    assert_eq!(
        compact,
        "v: 1 \
         [buffa.test.extjson.kids] {\
         v: 2 \
         [buffa.test.extjson.kids] {v: 3} \
         [buffa.test.extjson.kids] {v: 4} \
         [buffa.test.extjson.kid] {v: 5}\
         } \
         [buffa.test.extjson.kids] {v: 6}"
    );
    let pretty = encode_to_string_pretty(&root);
    assert_eq!(
        pretty,
        "\
v: 1
[buffa.test.extjson.kids] {
  v: 2
  [buffa.test.extjson.kids] {
    v: 3
  }
  [buffa.test.extjson.kids] {
    v: 4
  }
  [buffa.test.extjson.kid] {
    v: 5
  }
}
[buffa.test.extjson.kids] {
  v: 6
}
"
    );
    for text in [compact, pretty] {
        let decoded: extjson::Tree = decode_from_str(&text).unwrap();
        assert_eq!(decoded, root, "{text}");
    }
}

fn record(number: u32, data: UnknownFieldData) -> UnknownField {
    UnknownField { number, data }
}

#[test]
fn repeated_message_extension_skips_records_that_do_not_decode() {
    register_types();
    let number = extjson::__buffa::ext::ANNS.number();
    let valid = |ann: &extjson::Ann| {
        record(
            number,
            UnknownFieldData::LengthDelimited(ann.encode_to_vec()),
        )
    };
    // A string field whose declared length runs past the end of the record.
    let truncated = || record(number, UnknownFieldData::LengthDelimited(vec![0x0a, 0x05]));
    let wrong_wire_type = || record(number, UnknownFieldData::Varint(5));

    let mut carrier = extjson::Carrier::default();
    let fields = carrier.unknown_fields_mut();
    fields.push(truncated());
    fields.push(valid(&annotation("first", 1)));
    fields.push(wrong_wire_type());
    fields.push(truncated());
    fields.push(valid(&annotation("second", 2)));
    let text = encode_to_string(&carrier);
    assert_eq!(
        text,
        "[buffa.test.extjson.anns] {doc: \"first\" priority: 1} \
         [buffa.test.extjson.anns] {doc: \"second\" priority: 2}"
    );
    let decoded: extjson::Carrier = decode_from_str(&text).unwrap();
    assert_eq!(
        decoded.extension(&extjson::__buffa::ext::ANNS),
        [annotation("first", 1), annotation("second", 2)]
    );
}

#[test]
fn repeated_group_extension_skips_records_of_another_wire_type() {
    register_types();
    let ext = &groupext::__buffa::ext::DELIM_REPEATED;
    let mut carrier = groupext::Carrier::default();
    carrier.set_extension(
        ext,
        vec![groupext::Inner {
            c: Some(1),
            ..Default::default()
        }],
    );
    carrier.unknown_fields_mut().push(record(
        ext.number(),
        UnknownFieldData::LengthDelimited(vec![0x08, 0x02]),
    ));
    assert_eq!(
        encode_to_string(&carrier),
        "[buffa.test.groupext.delim_repeated] {c: 1}"
    );
}

/// With records at the extension's number and zero that decode, the entry is
/// the empty list, which parses back to an extension with zero elements.
#[test]
fn repeated_extension_with_zero_decodable_records_prints_an_empty_list() {
    register_types();
    let mut message = extjson::Carrier {
        x: Some(7),
        ..Default::default()
    };
    message.unknown_fields_mut().push(record(
        extjson::__buffa::ext::ANNS.number(),
        UnknownFieldData::Varint(5),
    ));
    let mut group = groupext::Carrier::default();
    group.unknown_fields_mut().push(record(
        groupext::__buffa::ext::DELIM_REPEATED.number(),
        UnknownFieldData::Fixed32(5),
    ));

    assert_eq!(
        encode_to_string(&message),
        "x: 7 [buffa.test.extjson.anns]: []"
    );
    assert_eq!(
        encode_to_string_pretty(&message),
        "x: 7\n[buffa.test.extjson.anns]: []\n"
    );
    assert_eq!(
        encode_to_string(&group),
        "[buffa.test.groupext.delim_repeated]: []"
    );

    let decoded: extjson::Carrier = decode_from_str(&encode_to_string(&message)).unwrap();
    assert_eq!(decoded.x, Some(7));
    assert!(decoded.unknown_fields().is_empty());
    let decoded: groupext::Carrier = decode_from_str(&encode_to_string(&group)).unwrap();
    assert!(decoded.unknown_fields().is_empty());
}

#[test]
fn repeated_messages_keep_empty_duplicate_and_disjoint_elements() {
    register_types();
    let doc = extjson::Ann {
        doc: Some("λ\n\"".into()),
        ..Default::default()
    };
    let priority = extjson::Ann {
        priority: Some(3),
        ..Default::default()
    };
    for expected in [
        vec![],
        vec![extjson::Ann::default()],
        vec![extjson::Ann::default(); 3],
        vec![annotation("same", 4); 3],
        vec![doc, priority],
    ] {
        let mut carrier = extjson::Carrier {
            x: Some(7),
            ..Default::default()
        };
        carrier.set_extension(&extjson::__buffa::ext::ANNS, expected.clone());
        let carrier = extjson::Carrier::decode_from_slice(&carrier.encode_to_vec()).unwrap();
        for text in [
            encode_to_string(&carrier),
            encode_to_string_pretty(&carrier),
        ] {
            let decoded: extjson::Carrier = decode_from_str(&text).unwrap();
            assert_eq!(decoded.x, Some(7));
            assert_eq!(
                decoded.extension(&extjson::__buffa::ext::ANNS),
                expected,
                "{text}"
            );
        }
    }
}

#[test]
fn repeated_groups_keep_empty_and_duplicate_elements() {
    register_types();
    for expected in [
        vec![],
        vec![groupext::Inner::default()],
        vec![groupext::Inner::default(); 3],
        vec![
            groupext::Inner {
                c: Some(4),
                ..Default::default()
            };
            3
        ],
    ] {
        let mut carrier = groupext::Carrier::default();
        carrier.set_extension(&groupext::__buffa::ext::DELIM_REPEATED, expected.clone());
        let carrier = groupext::Carrier::decode_from_slice(&carrier.encode_to_vec()).unwrap();
        for text in [
            encode_to_string(&carrier),
            encode_to_string_pretty(&carrier),
        ] {
            let decoded: groupext::Carrier = decode_from_str(&text).unwrap();
            assert_eq!(
                decoded.extension(&groupext::__buffa::ext::DELIM_REPEATED),
                expected,
                "{text}"
            );
        }
    }
}

#[test]
fn message_extension_lists_and_occurrences_append_in_order() {
    register_types();
    let text = r#"
        [buffa.test.extjson.anns] { doc: "first" }
        [buffa.test.extjson.anns]: [{ priority: 2 }, {}]
        [buffa.test.extjson.anns] <doc: "last">
    "#;
    let mut carrier = extjson::Carrier::default();
    carrier.set_extension(
        &extjson::__buffa::ext::ANNS,
        vec![annotation("existing", 0)],
    );
    merge_from_str(&mut carrier, text).unwrap();
    let values = carrier.extension(&extjson::__buffa::ext::ANNS);
    assert_eq!(values.len(), 5);
    assert_eq!(values[0], annotation("existing", 0));
    assert_eq!(values[1].doc.as_deref(), Some("first"));
    assert_eq!(values[2].priority, Some(2));
    assert_eq!(values[3], extjson::Ann::default());
    assert_eq!(values[4].doc.as_deref(), Some("last"));
    let decoded: extjson::Carrier = decode_from_str(&encode_to_string(&carrier)).unwrap();
    assert_eq!(decoded.extension(&extjson::__buffa::ext::ANNS), values);
}

#[test]
fn group_extension_lists_and_occurrences_append_in_order() {
    register_types();
    let text = r#"
        [buffa.test.groupext.delim_repeated] { c: 1 }
        [buffa.test.groupext.delim_repeated]: [<c: 2>, {}]
        [buffa.test.groupext.delim_repeated] { c: 3 }
    "#;
    let carrier: groupext::Carrier = decode_from_str(text).unwrap();
    let values = carrier.extension(&groupext::__buffa::ext::DELIM_REPEATED);
    assert_eq!(
        values.iter().map(|v| v.c).collect::<Vec<_>>(),
        [Some(1), Some(2), None, Some(3)]
    );
    let decoded: groupext::Carrier = decode_from_str(&encode_to_string(&carrier)).unwrap();
    assert_eq!(
        decoded.extension(&groupext::__buffa::ext::DELIM_REPEATED),
        values
    );
}

#[test]
fn extension_lists_parse_without_the_colon() {
    register_types();
    let carrier: extjson::Carrier =
        decode_from_str("[buffa.test.extjson.anns] [{priority: 1}, {}] x: 9").unwrap();
    assert_eq!(carrier.x, Some(9));
    assert_eq!(
        carrier.extension(&extjson::__buffa::ext::ANNS),
        [
            extjson::Ann {
                priority: Some(1),
                ..Default::default()
            },
            extjson::Ann::default()
        ]
    );
    let carrier: groupext::Carrier =
        decode_from_str("[buffa.test.groupext.delim_repeated] [{}, {c: 2}]").unwrap();
    assert_eq!(
        carrier
            .extension(&groupext::__buffa::ext::DELIM_REPEATED)
            .iter()
            .map(|v| v.c)
            .collect::<Vec<_>>(),
        [None, Some(2)]
    );
}

#[test]
fn empty_extension_lists_add_no_records() {
    register_types();
    let carrier: extjson::Carrier = decode_from_str("[buffa.test.extjson.anns]: [] x: 9").unwrap();
    assert!(carrier.extension(&extjson::__buffa::ext::ANNS).is_empty());
    assert_eq!(encode_to_string(&carrier), "x: 9");
    let carrier: groupext::Carrier =
        decode_from_str("[buffa.test.groupext.delim_repeated]: []").unwrap();
    assert!(carrier
        .extension(&groupext::__buffa::ext::DELIM_REPEATED)
        .is_empty());
    assert!(encode_to_string(&carrier).is_empty());
}

#[test]
fn singular_and_repeated_message_extensions_coexist() {
    register_types();
    let text = r#"
        [buffa.test.extjson.ann] { doc: "single" priority: 7 }
        [buffa.test.extjson.anns]: [{doc: "first"}, {priority: 2}]
        x: 9
    "#;
    let carrier: extjson::Carrier = decode_from_str(text).unwrap();
    let expected = annotation("single", 7);
    assert_eq!(
        carrier.extension(&extjson::__buffa::ext::ANN),
        Some(expected.clone())
    );
    for text in [
        encode_to_string(&carrier),
        encode_to_string_pretty(&carrier),
    ] {
        let decoded: extjson::Carrier = decode_from_str(&text).unwrap();
        assert_eq!(decoded.x, Some(9));
        assert_eq!(
            decoded.extension(&extjson::__buffa::ext::ANN),
            Some(expected.clone())
        );
        assert_eq!(
            decoded.extension(&extjson::__buffa::ext::ANNS),
            carrier.extension(&extjson::__buffa::ext::ANNS)
        );
    }
}

#[test]
fn singular_and_repeated_group_extensions_coexist() {
    register_types();
    let text = r#"
        [buffa.test.groupext.delim_repeated]: [{c: 1}, {c: 2}]
        [buffa.test.groupext.delim_inner] { c: 7 }
    "#;
    let carrier: groupext::Carrier = decode_from_str(text).unwrap();
    for text in [
        encode_to_string(&carrier),
        encode_to_string_pretty(&carrier),
    ] {
        let decoded: groupext::Carrier = decode_from_str(&text).unwrap();
        assert_eq!(
            decoded.extension(&groupext::__buffa::ext::DELIM_INNER),
            carrier.extension(&groupext::__buffa::ext::DELIM_INNER)
        );
        assert_eq!(
            decoded.extension(&groupext::__buffa::ext::DELIM_REPEATED),
            carrier.extension(&groupext::__buffa::ext::DELIM_REPEATED)
        );
    }
}

#[test]
fn repeated_extensions_charge_each_record_against_the_element_budget() {
    register_types();
    let record_size = core::mem::size_of::<buffa::UnknownField>();
    // The list form and the single-value form charge one record per element.
    for (message, group) in [
        (
            "[buffa.test.extjson.anns]: [{}, {}]",
            "[buffa.test.groupext.delim_repeated]: [{}, {}]",
        ),
        (
            "[buffa.test.extjson.anns] {} [buffa.test.extjson.anns] {}",
            "[buffa.test.groupext.delim_repeated] {} [buffa.test.groupext.delim_repeated] {}",
        ),
    ] {
        for budget in [0, record_size, 2 * record_size - 1] {
            let error =
                decode_from_str_with_element_memory_limit::<extjson::Carrier>(message, budget)
                    .unwrap_err();
            assert_eq!(error.kind, ParseErrorKind::ElementMemoryLimitExceeded);
            let error =
                decode_from_str_with_element_memory_limit::<groupext::Carrier>(group, budget)
                    .unwrap_err();
            assert_eq!(error.kind, ParseErrorKind::ElementMemoryLimitExceeded);
        }
        let carrier =
            decode_from_str_with_element_memory_limit::<extjson::Carrier>(message, 2 * record_size)
                .unwrap();
        assert_eq!(carrier.extension(&extjson::__buffa::ext::ANNS).len(), 2);
        let carrier =
            decode_from_str_with_element_memory_limit::<groupext::Carrier>(group, 2 * record_size)
                .unwrap();
        assert_eq!(
            carrier
                .extension(&groupext::__buffa::ext::DELIM_REPEATED)
                .len(),
            2
        );
    }
}

#[test]
fn invalid_extension_lists_are_rejected() {
    register_types();
    for body in ["[{}", "[{},]", "[{} {}]", "[1]", "[{missing: 1}]"] {
        assert!(
            decode_from_str::<extjson::Carrier>(&format!("[buffa.test.extjson.anns]: {body}"))
                .is_err(),
            "{body}"
        );
        assert!(
            decode_from_str::<groupext::Carrier>(&format!(
                "[buffa.test.groupext.delim_repeated]: {body}"
            ))
            .is_err(),
            "{body}"
        );
    }
    assert!(decode_from_str::<extjson::Carrier>("[buffa.test.extjson.ann]: [{}]").is_err());
    assert!(
        decode_from_str::<groupext::Carrier>("[buffa.test.groupext.delim_inner]: [{}]").is_err()
    );
}

struct LimitedWriter(usize);

impl core::fmt::Write for LimitedWriter {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        self.0 = self.0.checked_sub(value.len()).ok_or(core::fmt::Error)?;
        Ok(())
    }
}

fn assert_writer_errors<M: buffa::text::TextFormat>(message: &M) {
    for pretty in [false, true] {
        let mut output = String::new();
        let mut encoder = if pretty {
            buffa::text::TextEncoder::new_pretty(&mut output)
        } else {
            buffa::text::TextEncoder::new(&mut output)
        };
        message.encode_text(&mut encoder).unwrap();
        for limit in 0..output.len() {
            let mut writer = LimitedWriter(limit);
            let mut encoder = if pretty {
                buffa::text::TextEncoder::new_pretty(&mut writer)
            } else {
                buffa::text::TextEncoder::new(&mut writer)
            };
            assert_eq!(message.encode_text(&mut encoder), Err(core::fmt::Error));
        }
    }
}

#[test]
fn repeated_extension_writers_propagate_errors() {
    register_types();
    let mut message = extjson::Carrier::default();
    message.set_extension(
        &extjson::__buffa::ext::ANNS,
        vec![annotation("first", 1), annotation("second", 2)],
    );
    assert_writer_errors(&message);
    let mut group = groupext::Carrier::default();
    group.set_extension(
        &groupext::__buffa::ext::DELIM_REPEATED,
        vec![groupext::Inner::default(); 2],
    );
    assert_writer_errors(&group);
}

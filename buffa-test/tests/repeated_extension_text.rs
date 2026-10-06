use buffa::text::{
    decode_from_str, decode_from_str_with_element_memory_limit, encode_to_string,
    encode_to_string_pretty, merge_from_str, ParseErrorKind,
};
use buffa::type_registry::{set_type_registry, TypeRegistry};
use buffa::{ExtensionSet, Message};
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
fn repeated_message_extensions_preserve_elements_through_text() {
    register_types();
    let expected = vec![annotation("first", 1), annotation("second", 2)];
    let mut carrier = extjson::Carrier::default();
    carrier.set_extension(&extjson::__buffa::ext::ANNS, expected.clone());

    for text in [
        encode_to_string(&carrier),
        encode_to_string_pretty(&carrier),
    ] {
        let decoded: extjson::Carrier = decode_from_str(&text).unwrap();
        assert_eq!(
            decoded.extension(&extjson::__buffa::ext::ANNS),
            expected,
            "{text}"
        );
        assert_eq!(decoded.encode_to_vec(), carrier.encode_to_vec());
    }
}

#[test]
fn repeated_group_extensions_preserve_elements_through_text() {
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
        assert_eq!(decoded.encode_to_vec(), carrier.encode_to_vec());
    }
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
        [buffa.test.extjson.ann] { doc: "merged" }
        [buffa.test.extjson.anns]: [{doc: "first"}, {priority: 2}]
        [buffa.test.extjson.ann] { priority: 7 }
        x: 9
    "#;
    let carrier: extjson::Carrier = decode_from_str(text).unwrap();
    let expected = annotation("merged", 7);
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
    let message = "[buffa.test.extjson.anns]: [{}, {}]";
    let group = "[buffa.test.groupext.delim_repeated]: [{}, {}]";
    let record_size = core::mem::size_of::<buffa::UnknownField>();
    for budget in [0, record_size, 2 * record_size - 1] {
        let error = decode_from_str_with_element_memory_limit::<extjson::Carrier>(message, budget)
            .unwrap_err();
        assert_eq!(error.kind, ParseErrorKind::ElementMemoryLimitExceeded);
        let error = decode_from_str_with_element_memory_limit::<groupext::Carrier>(group, budget)
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

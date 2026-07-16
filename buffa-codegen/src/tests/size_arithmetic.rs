//! Locks the u64 size-arithmetic discipline in generated encode code.
//!
//! Generated `compute_size` bodies must accumulate in `u64` and saturate to
//! `u32` once at return (`::buffa::saturate_size`), and the transient
//! lengths recomputed in `write_to` (packed payloads, map entries) must be
//! `u64` as well. Any plain `u32` arithmetic in a size path can wrap for
//! over-limit messages — three 1.5 GiB sub-trees wrap `u32` to an
//! *under-limit* value that the encode entry points' 2 GiB check cannot
//! distinguish from a legitimate size, silently producing corrupt output.
//! These tests fail if a future codegen change reintroduces `u32` size
//! arithmetic anywhere in the emitted encode code.

use super::*;

/// A proto3 file exercising every size-emitting codegen shape: scalar
/// string/bytes/enum/varint, explicit presence, message fields, repeated
/// (message / packed fixed / packed varint / unpacked fixed / string),
/// maps (message-valued, const-folded fixed/fixed), and a oneof with
/// message/string/fixed arms. Views are generated too, so the view
/// builder's output is covered by the same assertions.
fn size_corpus_file() -> FileDescriptorProto {
    let mut file = proto3_file("size_corpus.proto");
    file.package = Some("corpus".to_string());

    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("COLOR_UNSPECIFIED", 0), enum_value("RED", 1)],
        ..Default::default()
    });

    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("n", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });

    let msg_field = |name: &str, number: i32, label: Label| FieldDescriptorProto {
        type_name: Some(".corpus.Inner".to_string()),
        ..make_field(name, number, label, Type::TYPE_MESSAGE)
    };
    let unpacked = |f: FieldDescriptorProto| FieldDescriptorProto {
        options: Some(crate::generated::descriptor::FieldOptions {
            packed: Some(false),
            ..Default::default()
        })
        .into(),
        ..f
    };
    let oneof_member = |f: FieldDescriptorProto| FieldDescriptorProto {
        oneof_index: Some(0),
        ..f
    };

    // map<string, Inner> and map<int32, fixed64> synthetic entry messages.
    let map_entry =
        |name: &str, key: FieldDescriptorProto, val: FieldDescriptorProto| DescriptorProto {
            name: Some(name.to_string()),
            field: vec![key, val],
            options: Some(MessageOptions {
                map_entry: Some(true),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };

    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("data", 2, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
            make_field("count", 3, Label::LABEL_OPTIONAL, Type::TYPE_INT64),
            FieldDescriptorProto {
                type_name: Some(".corpus.Color".to_string()),
                ..make_field("color", 4, Label::LABEL_OPTIONAL, Type::TYPE_ENUM)
            },
            FieldDescriptorProto {
                proto3_optional: Some(true),
                oneof_index: Some(1),
                ..make_field("maybe", 5, Label::LABEL_OPTIONAL, Type::TYPE_UINT32)
            },
            msg_field("child", 6, Label::LABEL_OPTIONAL),
            msg_field("children", 7, Label::LABEL_REPEATED),
            make_field("weights", 8, Label::LABEL_REPEATED, Type::TYPE_DOUBLE),
            make_field("ids", 9, Label::LABEL_REPEATED, Type::TYPE_SINT64),
            unpacked(make_field(
                "loose",
                10,
                Label::LABEL_REPEATED,
                Type::TYPE_FIXED32,
            )),
            make_field("tags", 11, Label::LABEL_REPEATED, Type::TYPE_STRING),
            FieldDescriptorProto {
                type_name: Some(".corpus.Outer.ByNameEntry".to_string()),
                ..make_field("by_name", 12, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
            FieldDescriptorProto {
                type_name: Some(".corpus.Outer.FlagsEntry".to_string()),
                ..make_field("flags", 13, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
            oneof_member(make_field(
                "as_text",
                14,
                Label::LABEL_OPTIONAL,
                Type::TYPE_STRING,
            )),
            oneof_member(msg_field("as_msg", 15, Label::LABEL_OPTIONAL)),
            oneof_member(make_field(
                "as_num",
                16,
                Label::LABEL_OPTIONAL,
                Type::TYPE_DOUBLE,
            )),
        ],
        nested_type: vec![
            map_entry(
                "ByNameEntry",
                make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
                FieldDescriptorProto {
                    type_name: Some(".corpus.Inner".to_string()),
                    ..make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE)
                },
            ),
            map_entry(
                "FlagsEntry",
                make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
                make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_FIXED64),
            ),
        ],
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("payload".to_string()),
                ..Default::default()
            },
            // Synthetic oneof for the proto3-optional `maybe` field.
            OneofDescriptorProto {
                name: Some("_maybe".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    });

    file
}

fn corpus_output() -> &'static str {
    static OUTPUT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OUTPUT.get_or_init(|| {
        let files = generate(
            &[size_corpus_file()],
            &["size_corpus.proto".to_string()],
            &CodeGenConfig::default(),
        )
        .expect("corpus should generate");
        joined(&files)
    })
}

/// Extract the bodies of every `fn` whose name starts with `prefix`
/// (matching `fn compute_size` and `fn write_to` in owned, view, and lazy
/// impls) by brace counting from the signature.
fn fn_bodies<'a>(content: &'a str, prefix: &str) -> Vec<&'a str> {
    let needle = format!("fn {prefix}");
    let mut bodies = Vec::new();
    let mut from = 0;
    while let Some(pos) = content[from..].find(&needle) {
        let start = from + pos;
        let open = start + content[start..].find('{').expect("fn body opens");
        let mut depth = 0usize;
        let mut end = open;
        for (i, ch) in content[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        bodies.push(&content[start..end]);
        from = end;
    }
    bodies
}

#[test]
fn compute_size_accumulates_in_u64_and_saturates() {
    let content = corpus_output();
    assert!(
        content.contains("let mut size = 0u64;"),
        "compute_size must accumulate in u64: {content}"
    );
    assert!(
        content.contains("::buffa::saturate_size(size)"),
        "compute_size must saturate its u64 total at return: {content}"
    );
    assert!(
        !content.contains("let mut size = 0u32;"),
        "u32 size accumulator reintroduced: {content}"
    );
}

#[test]
fn write_to_transient_lengths_are_u64() {
    let content = corpus_output();
    assert!(
        content.contains("let payload: u64 ="),
        "packed payload length must be u64: {content}"
    );
    assert!(
        content.contains("let entry_size: u64 ="),
        "map entry length must be u64: {content}"
    );
}

#[test]
fn no_u32_arithmetic_in_size_paths() {
    // Scoped lock: every `compute_size` and `write_to` body in the corpus
    // is free of u32 casts and u32 sums. Size helpers return usize and are
    // widened `as u64`; the only u32 values in encode paths are
    // `compute_size` return values, widened at their use sites. Any u32
    // arithmetic inside these bodies is a wrap bug (see module docs);
    // u32 uses elsewhere in generated code (decode paths, accessors) are
    // out of scope by construction.
    let content = corpus_output();
    let bodies: Vec<&str> = fn_bodies(content, "compute_size")
        .into_iter()
        .chain(fn_bodies(content, "write_to"))
        .collect();
    assert!(
        bodies.len() >= 6,
        "corpus should produce compute_size/write_to bodies for owned and \
         view impls, got {}",
        bodies.len()
    );
    for body in bodies {
        assert!(
            !body.contains("as u32"),
            "u32 cast reintroduced in a size path: {body}"
        );
        assert!(
            !body.contains("sum::<u32>()"),
            "u32 sum reintroduced in a size path: {body}"
        );
    }
}

//! `generate_text` with custom containers: the text impl must reach a custom
//! map, collection or oneof pointer only through `MapStorage`, `ProtoList` and
//! `ProtoBox`, and keep the default containers' output as it was.

use super::*;

/// `message Holder { repeated int32 nums = 1; map<string, int32> scores = 2;
/// oneof pick { Holder child = 3; } }`
fn holder_file() -> FileDescriptorProto {
    let mut file = proto3_file("holder.proto");
    file.package = Some("pkg".to_string());
    let entry = DescriptorProto {
        name: Some("ScoresEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
        ],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    file.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![
            make_field("nums", 1, Label::LABEL_REPEATED, Type::TYPE_INT32),
            FieldDescriptorProto {
                type_name: Some(".pkg.Holder.ScoresEntry".to_string()),
                ..make_field("scores", 2, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
            FieldDescriptorProto {
                type_name: Some(".pkg.Holder".to_string()),
                oneof_index: Some(0),
                ..make_field("child", 3, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE)
            },
        ],
        nested_type: vec![entry],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("pick".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    file
}

/// The generated text impl for `Holder`, with whitespace removed so the
/// assertions do not depend on how `prettyplease` breaks lines.
fn text_impl(config: CodeGenConfig) -> String {
    let config = CodeGenConfig {
        generate_text: true,
        generate_views: false,
        ..config
    };
    let files = generate(&[holder_file()], &["holder.proto".to_string()], &config)
        .expect("codegen should succeed");
    let content = joined(&files);
    let start = content
        .find("impl ::buffa::text::TextFormat for Holder")
        .expect("text impl for Holder");
    let len = content[start..]
        .find("__HOLDER_TEXT_ANY")
        .expect("text `Any` entry after the impl");
    content[start..start + len].split_whitespace().collect()
}

#[test]
fn default_containers_keep_their_text_output() {
    let got = text_impl(CodeGenConfig::default());
    for expected in [
        "for__vin&self.nums{",
        "dec.read_repeated_scalar_into(&mutself.nums,|__d|__d.read_i32())?",
        "for(__k,__v)in&self.scores{",
        "self.scores.insert(__k,__v);",
        "::buffa::alloc::boxed::Box::new(__m)",
    ] {
        assert!(got.contains(expected), "missing `{expected}` in {got}");
    }
}

#[test]
fn custom_containers_go_through_their_traits() {
    let got = text_impl(CodeGenConfig {
        repeated_fields: vec![(".".into(), RepeatedRepr::Custom("::my::List<*>".into()))],
        map_fields: vec![(".".into(), MapRepr::Custom("::my::Map".into()))],
        pointer_fields: vec![(".".into(), PointerRepr::Custom("::my::SBox<*>".into()))],
        ..CodeGenConfig::default()
    });
    for expected in [
        "for__vinself.nums.iter(){",
        "dec.read_repeated_scalar_into(&mut__items,|__d|__d.read_i32())?;",
        "::buffa::ProtoList::push(&mutself.nums,__v);",
        "for(__k,__v)in::buffa::map_codec::MapStorage::storage_iter(&self.scores){",
        "::buffa::map_codec::MapStorage::storage_insert(&mutself.scores,__k,__v,);",
        "<::my::SBox<Holder>as::buffa::ProtoBox<Holder>>::new(__m)",
    ] {
        assert!(got.contains(expected), "missing `{expected}` in {got}");
    }
    assert!(!got.contains("Box::new(__m)"), "{got}");
}

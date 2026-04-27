//! Proto3 codegen: WKT auto-mapping, enums, TYPE_URL, message fields, repeated.

use super::*;

#[test]
fn test_wkt_auto_mapping_not_suppressed_by_sub_package() {
    // Regression: mapping .google.protobuf.compiler (a SUB-package) used
    // to suppress the WKT auto-mapping for .google.protobuf, breaking
    // Timestamp/Duration/etc. resolve_extern_prefix's longest-prefix
    // matching handles both coexisting correctly.
    let config = CodeGenConfig {
        extern_paths: vec![(
            ".google.protobuf.compiler".into(),
            "::compiler_protos".into(),
        )],
        ..Default::default()
    };
    let effective = effective_extern_paths(&[], &[], &config);
    // The sub-package mapping is preserved...
    assert!(effective
        .iter()
        .any(|(p, _)| p == ".google.protobuf.compiler"));
    // ...AND the WKT auto-mapping is still injected.
    assert!(
        effective.iter().any(|(p, _)| p == ".google.protobuf"),
        "WKT auto-mapping must coexist with sub-package extern_path"
    );
}

#[test]
fn test_wkt_auto_mapping_suppressed_by_exact_match() {
    let config = CodeGenConfig {
        extern_paths: vec![(".google.protobuf".into(), "::my_wkts".into())],
        ..Default::default()
    };
    let effective = effective_extern_paths(&[], &[], &config);
    // Exactly one .google.protobuf mapping (user's), not two.
    let count = effective
        .iter()
        .filter(|(p, _)| p == ".google.protobuf")
        .count();
    assert_eq!(count, 1);
    // It's the user's, not the auto-injection.
    assert!(effective
        .iter()
        .any(|(p, r)| p == ".google.protobuf" && r == "::my_wkts"));
}

#[test]
fn test_empty_file() {
    let file = proto3_file("empty.proto");
    let result = generate(
        &[file],
        &["empty.proto".to_string()],
        &CodeGenConfig::default(),
    );
    let files = result.expect("empty file should generate without error");
    // 5 content files + 1 .mod.rs.
    assert_eq!(files.len(), 6);
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert_eq!(stitcher.name, "__buffa.mod.rs");
    assert!(
        stitcher.content.contains("@generated"),
        "missing header comment"
    );
}

#[test]
fn test_package_to_mod_filename() {
    assert_eq!(
        package_to_mod_filename("google.protobuf"),
        "google.protobuf.mod.rs"
    );
    assert_eq!(package_to_mod_filename("foo"), "foo.mod.rs");
    assert_eq!(package_to_mod_filename(""), "__buffa.mod.rs");
    assert_eq!(
        proto_path_to_stem("google/protobuf/timestamp.proto"),
        "google.protobuf.timestamp"
    );
}

#[test]
fn test_multi_file_same_package_merged() {
    // Two `.proto` files in the same package → one stitcher.
    let mut a = proto3_file("a.proto");
    a.package = Some("shared.pkg".to_string());
    a.message_type.push(DescriptorProto {
        name: Some("A".to_string()),
        ..Default::default()
    });
    let mut b = proto3_file("b.proto");
    b.package = Some("shared.pkg".to_string());
    b.message_type.push(DescriptorProto {
        name: Some("B".to_string()),
        ..Default::default()
    });
    let files = generate(
        &[a, b],
        &["a.proto".to_string(), "b.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("multi-file package should merge");
    // 2 protos × 5 content files + 1 stitcher = 11.
    assert_eq!(files.len(), 11);
    let stitcher = files
        .iter()
        .find(|f| f.kind == GeneratedFileKind::PackageMod)
        .expect("stitcher present");
    assert_eq!(stitcher.name, "shared.pkg.mod.rs");
    let content = &joined(&files);
    assert!(content.contains("pub struct A"));
    assert!(content.contains("pub struct B"));
    // Exactly one `pub mod __buffa` (in the stitcher), and both content
    // files referenced from inside it.
    assert_eq!(stitcher.content.matches("pub mod __buffa {").count(), 1);
    assert!(stitcher.content.contains(r#"include!("a.__view.rs");"#));
    assert!(stitcher.content.contains(r#"include!("b.__view.rs");"#));
}

#[test]
fn test_child_package_named_view_no_collision() {
    // Regression: under the pre-sentinel design, `package foo.view`
    // emitted `pub mod view { ... }` (child package) as a sibling of the
    // kind-tree `pub mod view { ... }` inside `foo` — E0428. With the
    // sentinel wrapper, the kind tree is `pub mod __buffa { pub mod view
    // { ... } }`, so a child package literally named `view` is fine.
    let mut a = proto3_file("a.proto");
    a.package = Some("foo".to_string());
    a.message_type.push(DescriptorProto {
        name: Some("A".to_string()),
        ..Default::default()
    });
    let mut b = proto3_file("b.proto");
    b.package = Some("foo.view".to_string());
    b.message_type.push(DescriptorProto {
        name: Some("B".to_string()),
        ..Default::default()
    });
    let files = generate(
        &[a, b],
        &["a.proto".to_string(), "b.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("`package foo.view` alongside `package foo` must compile");
    // One stitcher per package.
    let stitchers: Vec<_> = files
        .iter()
        .filter(|f| f.kind == GeneratedFileKind::PackageMod)
        .collect();
    assert_eq!(stitchers.len(), 2);
    // Neither stitcher emits a bare `pub mod view {` at top level — the
    // view kind tree is always nested under `pub mod __buffa {`.
    for s in &stitchers {
        let top_level_view = s
            .content
            .lines()
            .any(|l| l.starts_with("pub mod view {") || l == "pub mod view {");
        assert!(
            !top_level_view,
            "stitcher {} must not emit top-level `pub mod view`: {}",
            s.name, s.content
        );
        assert!(s.content.contains("pub mod __buffa {"));
    }
    // The package tree assembled by `generate_module_tree` puts `view` as
    // a child of `foo`, separate from `foo`'s `__buffa` wrapper.
    let entries: Vec<_> = stitchers
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.name.trim_end_matches(".mod.rs").to_string(),
            )
        })
        .collect();
    let tree = crate::generate_module_tree(&entries, crate::IncludeMode::Relative(""), false);
    assert!(
        tree.contains("pub mod foo {") && tree.contains("pub mod view {"),
        "tree must nest `view` under `foo`: {tree}"
    );
}

#[test]
fn test_simple_enum() {
    let mut file = proto3_file("status.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![
            enum_value("UNKNOWN", 0),
            enum_value("ACTIVE", 1),
            enum_value("INACTIVE", 2),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["status.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("simple enum should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub enum Status"),
        "missing enum: {content}"
    );
    assert!(
        content.contains("UNKNOWN = 0"),
        "missing UNKNOWN: {content}"
    );
    assert!(content.contains("ACTIVE = 1"), "missing ACTIVE: {content}");
    assert!(
        content.contains("INACTIVE = 2"),
        "missing INACTIVE: {content}"
    );
    assert!(
        content.contains("impl ::buffa::Enumeration for Status"),
        "missing Enumeration impl: {content}"
    );
    assert!(
        content.contains("impl ::core::default::Default for Status"),
        "missing Default impl: {content}"
    );
}

#[test]
fn test_enum_with_alias() {
    let mut file = proto3_file("code.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Code".to_string()),
        value: vec![
            enum_value("OK", 0),
            enum_value("SUCCESS", 0), // alias for OK
            enum_value("ERROR", 1),
        ],
        options: (crate::generated::descriptor::EnumOptions {
            allow_alias: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["code.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("aliased enum should generate");
    let content = &joined(&files);
    // OK is a primary variant; SUCCESS is a const alias.
    assert!(content.contains("OK = 0"), "missing primary: {content}");
    assert!(
        content.contains("pub const SUCCESS"),
        "alias not emitted as const: {content}"
    );
    assert!(
        !content.contains("SUCCESS = 0"),
        "alias must not be a variant: {content}"
    );
}

#[test]
fn test_enum_values_emits_static_slice_in_declaration_order() {
    let mut file = proto3_file("status.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![
            enum_value("UNKNOWN", 0),
            enum_value("ACTIVE", 1),
            enum_value("INACTIVE", 2),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["status.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("enum should generate");
    let content = &joined(&files);
    // Generated impl includes a `values()` returning a static slice in
    // proto declaration order. The generated body collapses to a single
    // line so we match against that exact form.
    assert!(
        content.contains("&[Self::UNKNOWN, Self::ACTIVE, Self::INACTIVE]"),
        "missing declaration-order values() slice: {content}"
    );
}

#[test]
fn test_enum_values_skips_aliases() {
    let mut file = proto3_file("code.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Code".to_string()),
        value: vec![
            enum_value("OK", 0),
            enum_value("SUCCESS", 0), // alias for OK — not its own variant
            enum_value("ERROR", 1),
        ],
        options: (crate::generated::descriptor::EnumOptions {
            allow_alias: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["code.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("aliased enum should generate");
    let content = &joined(&files);
    // values() lists only primary variants — aliases are `pub const` items,
    // not enum variants, so they don't belong in the slice.
    assert!(
        content.contains("&[Self::OK, Self::ERROR]"),
        "values() should mirror primary variants only: {content}"
    );
    assert!(
        !content.contains("Self::SUCCESS"),
        "alias `SUCCESS` must not appear in values(): {content}"
    );
}

#[test]
fn test_file_not_found_error() {
    let file = proto3_file("other.proto");
    let result = generate(
        &[file],
        &["missing.proto".to_string()],
        &CodeGenConfig::default(),
    );
    assert!(
        matches!(result, Err(CodeGenError::FileNotFound(_))),
        "expected FileNotFound error"
    );
}

#[test]
fn test_type_url_top_level_with_package() {
    let mut file = proto3_file("person.proto");
    file.package = Some("my.company".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Person".to_string()),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["person.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/my.company.Person""#),
        "wrong or missing TYPE_URL: {content}"
    );
}

#[test]
fn test_type_url_top_level_no_package() {
    let mut file = proto3_file("root.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Root".to_string()),
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["root.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/Root""#),
        "wrong or missing TYPE_URL for no-package message: {content}"
    );
}

#[test]
fn test_type_url_nested_message() {
    let mut file = proto3_file("nested_type_url.proto");
    file.package = Some("acme".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["nested_type_url.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/acme.Outer""#),
        "wrong Outer TYPE_URL: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/acme.Outer.Inner""#),
        "wrong Inner TYPE_URL: {content}"
    );
}

#[test]
fn test_type_url_nested_no_package() {
    // Empty package + nested message: FQN should be "Outer.Inner", no leading dot.
    let mut file = proto3_file("nested_nopackage.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["nested_nopackage.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/Outer""#),
        "wrong Outer TYPE_URL: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/Outer.Inner""#),
        "wrong Inner TYPE_URL (no package): {content}"
    );
}

#[test]
fn test_type_url_doubly_nested() {
    // Three levels: pkg.Outer.Middle.Inner — verifies recursive FQN propagation.
    let mut file = proto3_file("doubly_nested.proto");
    file.package = Some("pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Middle".to_string()),
            nested_type: vec![DescriptorProto {
                name: Some("Inner".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["doubly_nested.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("should generate");
    let content = &joined(&files);
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/pkg.Outer""#),
        "wrong Outer TYPE_URL: {content}"
    );
    assert!(
        content.contains(r#"TYPE_URL: &'static str = "type.googleapis.com/pkg.Outer.Middle""#),
        "wrong Middle TYPE_URL: {content}"
    );
    assert!(
        content
            .contains(r#"TYPE_URL: &'static str = "type.googleapis.com/pkg.Outer.Middle.Inner""#),
        "wrong Inner TYPE_URL: {content}"
    );
}

#[test]
fn test_message_scalar_fields() {
    let mut file = proto3_file("scalars.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Scalars".to_string()),
        field: vec![
            make_field("count", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32),
            make_field("active", 2, Label::LABEL_OPTIONAL, Type::TYPE_BOOL),
            make_field("score", 3, Label::LABEL_OPTIONAL, Type::TYPE_DOUBLE),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["scalars.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("scalar fields message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Scalars"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub count: i32"),
        "missing count field: {content}"
    );
    assert!(
        content.contains("pub active: bool"),
        "missing active field: {content}"
    );
    assert!(
        content.contains("pub score: f64"),
        "missing score field: {content}"
    );
    assert!(
        content.contains("impl ::buffa::DefaultInstance for Scalars"),
        "missing DefaultInstance impl: {content}"
    );
    assert!(
        content.contains("impl ::buffa::Message for Scalars"),
        "missing Message impl: {content}"
    );
    assert!(
        content.contains("fn compute_size"),
        "missing compute_size: {content}"
    );
    assert!(
        content.contains("fn merge_field"),
        "missing merge_field: {content}"
    );
}

#[test]
fn test_message_nested_message_field() {
    let mut file = proto3_file("nested.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("inner".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["nested.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("nested message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct Outer"),
        "missing Outer: {content}"
    );
    assert!(
        content.contains("pub inner: ::buffa::MessageField<Inner>"),
        "missing MessageField: {content}"
    );
    // impl Message should use the two-pass size computation for sub-messages.
    assert!(
        content.contains("compute_size"),
        "missing compute_size call for sub-message: {content}"
    );
    assert!(
        content.contains("merge_length_delimited"),
        "missing merge_length_delimited for sub-message: {content}"
    );
    assert!(
        content.contains("get_or_insert_default"),
        "missing get_or_insert_default in merge: {content}"
    );
}

#[test]
fn test_message_map_field() {
    let mut file = proto3_file("withmap.proto");
    // Synthetic map entry: key=string, value=int32
    let map_entry = DescriptorProto {
        name: Some("AttrsEntry".to_string()),
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
        name: Some("WithMap".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("attrs".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".WithMap.AttrsEntry".to_string()),
            ..Default::default()
        }],
        nested_type: vec![map_entry],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["withmap.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("map field message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct WithMap"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub attrs:"),
        "missing attrs field: {content}"
    );
    assert!(
        content.contains("::buffa::__private::HashMap"),
        "map field must use ::buffa::__private::HashMap, got: {content}"
    );
}

#[test]
fn test_message_oneof() {
    let mut file = proto3_file("oneof.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOneof".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("count".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("name".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["oneof.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("oneof message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct WithOneof"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub kind:"),
        "missing oneof field: {content}"
    );
    assert!(
        content.contains("pub mod with_oneof"),
        "missing message module: {content}"
    );
    assert!(
        content.contains("pub enum Kind"),
        "missing oneof enum: {content}"
    );
    assert!(
        content.contains("Count(i32)"),
        "missing Count variant: {content}"
    );
    assert!(
        content.contains("impl ::buffa::Oneof for Kind"),
        "missing Oneof impl: {content}"
    );
}

#[test]
fn test_message_proto3_optional() {
    let mut file = proto3_file("proto3opt.proto");
    // Proto3 optional fields are assigned to a synthetic oneof.
    file.message_type.push(DescriptorProto {
        name: Some("WithOptional".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("count".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_INT32),
            oneof_index: Some(0),
            proto3_optional: Some(true),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_count".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["proto3opt.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub struct WithOptional"),
        "missing struct: {content}"
    );
    assert!(
        content.contains("pub count: ::core::option::Option<i32>"),
        "missing optional field: {content}"
    );
    // impl Message should use if-let pattern for optional
    assert!(
        content.contains("if let Some"),
        "missing if-let in impl: {content}"
    );
    assert!(
        content.contains("Option::Some"),
        "missing Some assignment in merge: {content}"
    );
}

#[test]
fn test_message_proto3_optional_string() {
    let mut file = proto3_file("optstr.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOptStr".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("label".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_STRING),
            oneof_index: Some(0),
            proto3_optional: Some(true),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_label".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["optstr.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional string should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub label: ::core::option::Option<::buffa::alloc::string::String>"),
        "missing optional string field: {content}"
    );
    assert!(
        content.contains("encode_string"),
        "missing encode_string in write_to: {content}"
    );
    assert!(
        content.contains("merge_string"),
        "missing merge_string in merge: {content}"
    );
}

#[test]
fn test_message_proto3_optional_enum() {
    let mut file = proto3_file("optenu.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("WithOptEnum".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("color".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Color".to_string()),
            oneof_index: Some(0),
            proto3_optional: Some(true),
            ..Default::default()
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_color".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["optenu.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional enum should generate");
    let content = &joined(&files);
    // Enum optional must resolve to EnumValue<Color>, not ()
    assert!(
        content.contains("Option<::buffa::EnumValue<Color>>"),
        "wrong type for optional enum: {content}"
    );
    assert!(
        content.contains("EnumValue::from"),
        "missing EnumValue::from in merge: {content}"
    );
}

#[test]
fn test_message_proto3_optional_bytes_and_bool() {
    let mut file = proto3_file("optmisc.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithOptMisc".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("data".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_BYTES),
                oneof_index: Some(0),
                proto3_optional: Some(true),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("flag".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_BOOL),
                oneof_index: Some(1),
                proto3_optional: Some(true),
                ..Default::default()
            },
        ],
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("_data".to_string()),
                ..Default::default()
            },
            OneofDescriptorProto {
                name: Some("_flag".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["optmisc.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("proto3 optional bytes/bool should generate");
    let content = &joined(&files);
    assert!(
        content.contains("Option<::buffa::alloc::vec::Vec<u8>>"),
        "missing optional bytes field: {content}"
    );
    assert!(
        content.contains("Option<bool>"),
        "missing optional bool field: {content}"
    );
    // Bool is fixed-size: compute_size should use is_some(), not if-let
    assert!(
        content.contains("is_some()"),
        "fixed-size optional should use is_some(): {content}"
    );
    // Bytes uses encode_bytes
    assert!(
        content.contains("encode_bytes"),
        "missing encode_bytes for optional bytes: {content}"
    );
}

#[test]
fn test_message_string_and_bytes_fields() {
    let mut file = proto3_file("strings.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithStrings".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("data", 2, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["strings.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("string/bytes message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub name: ::buffa::alloc::string::String"),
        "missing string field: {content}"
    );
    assert!(
        content.contains("pub data: ::buffa::alloc::vec::Vec<u8>"),
        "missing bytes field: {content}"
    );
    // impl Message should encode/decode these fields
    assert!(
        content.contains("encode_string"),
        "missing encode_string: {content}"
    );
    assert!(
        content.contains("merge_string"),
        "missing merge_string: {content}"
    );
    assert!(
        content.contains("string_encoded_len"),
        "missing string_encoded_len: {content}"
    );
    assert!(
        content.contains("encode_bytes"),
        "missing encode_bytes: {content}"
    );
    assert!(
        content.contains("merge_bytes"),
        "missing merge_bytes: {content}"
    );
    assert!(
        content.contains("bytes_encoded_len"),
        "missing bytes_encoded_len: {content}"
    );
}

#[test]
fn test_message_enum_field() {
    let mut file = proto3_file("enumfield.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![enum_value("UNKNOWN", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("WithEnum".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("status".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Status".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["enumfield.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("enum field message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub status: ::buffa::EnumValue<Status>"),
        "missing enum field: {content}"
    );
    // impl Message should encode via to_i32() and decode via EnumValue::from
    assert!(
        content.contains("to_i32()"),
        "missing to_i32 in generated code: {content}"
    );
    assert!(
        content.contains("int32_encoded_len"),
        "missing int32_encoded_len in compute_size: {content}"
    );
    assert!(
        content.contains("EnumValue::from"),
        "missing EnumValue::from in generated code: {content}"
    );
}

#[test]
fn test_repeated_packed_scalar() {
    let mut file = proto3_file("repeatedscalar.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithRepeated".to_string()),
        field: vec![
            make_field("ids", 1, Label::LABEL_REPEATED, Type::TYPE_INT32),
            make_field("scores", 2, Label::LABEL_REPEATED, Type::TYPE_DOUBLE),
        ],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedscalar.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated scalar message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub ids: ::buffa::alloc::vec::Vec<i32>"),
        "missing ids field: {content}"
    );
    assert!(
        content.contains("pub scores: ::buffa::alloc::vec::Vec<f64>"),
        "missing scores field: {content}"
    );
    // Packed encoding: payload written as a single LengthDelimited blob.
    assert!(
        content.contains("is_empty()"),
        "packed repeated should check is_empty: {content}"
    );
    assert!(
        content.contains("int32_encoded_len"),
        "missing int32_encoded_len in payload size: {content}"
    );
    assert!(
        content.contains("encode_int32"),
        "missing encode_int32 in write_to: {content}"
    );
    assert!(
        content.contains("decode_int32"),
        "missing decode_int32 in merge: {content}"
    );
    // Merge must accept both packed and unpacked.
    assert!(
        content.contains("WireType::LengthDelimited"),
        "missing packed merge branch: {content}"
    );
}

#[test]
fn test_repeated_unpacked_string() {
    let mut file = proto3_file("repeatedstr.proto");
    file.message_type.push(DescriptorProto {
        name: Some("WithRepeatedStr".to_string()),
        field: vec![make_field(
            "tags",
            1,
            Label::LABEL_REPEATED,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedstr.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated string message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub tags: ::buffa::alloc::vec::Vec<::buffa::alloc::string::String>"),
        "missing tags field: {content}"
    );
    // Unpacked: each element has its own tag (for loop, no payload length).
    assert!(
        content.contains("string_encoded_len"),
        "missing string_encoded_len: {content}"
    );
    assert!(
        content.contains("encode_string"),
        "missing encode_string: {content}"
    );
    assert!(
        content.contains("decode_string"),
        "missing decode_string: {content}"
    );
}

#[test]
fn test_repeated_message_field() {
    let mut file = proto3_file("repeatedmsg.proto");
    file.message_type.push(DescriptorProto {
        name: Some("Item".to_string()),
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Container".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("items".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".Item".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedmsg.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated message should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub items: ::buffa::alloc::vec::Vec<Item>"),
        "missing items field: {content}"
    );
    // Uses two-pass size model for each element.
    assert!(
        content.contains("merge_length_delimited"),
        "missing merge_length_delimited for repeated msg: {content}"
    );
    assert!(
        content.contains("__cache.consume_next()"),
        "missing SizeCache consume in write_to: {content}"
    );
}

#[test]
fn test_repeated_enum_field() {
    let mut file = proto3_file("repeatedenu.proto");
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Status".to_string()),
        value: vec![enum_value("UNKNOWN", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("WithRepeatedEnum".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("statuses".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".Status".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[file],
        &["repeatedenu.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("repeated enum should generate");
    let content = &joined(&files);
    assert!(
        content.contains("pub statuses: ::buffa::alloc::vec::Vec<::buffa::EnumValue<Status>>"),
        "missing statuses field: {content}"
    );
    // Packed enum encoding uses to_i32() and EnumValue::from.
    assert!(
        content.contains("to_i32()"),
        "missing to_i32 for packed enum write: {content}"
    );
    assert!(
        content.contains("EnumValue::from"),
        "missing EnumValue::from in packed decode: {content}"
    );
}

#[test]
fn extension_set_impl_on_generated_options() {
    // Smoke test: the bootstrap-generated FieldOptions implements
    // ExtensionSet, and extension get/set roundtrips through its
    // __buffa_unknown_fields storage.
    use crate::generated::descriptor::FieldOptions;
    use buffa::extension::codecs::{Int32, StringCodec};
    use buffa::{Extension, ExtensionSet};

    // Use the bootstrap-generated PROTO_FQN so the extendee check passes.
    const WEIGHT: Extension<Int32> = Extension::new(50001, FieldOptions::PROTO_FQN);
    const TAG: Extension<StringCodec> = Extension::new(50002, FieldOptions::PROTO_FQN);

    let mut opts = FieldOptions::default();
    assert!(!opts.has_extension(&WEIGHT));

    opts.set_extension(&WEIGHT, -7);
    opts.set_extension(&TAG, "hello".to_string());

    assert_eq!(opts.extension(&WEIGHT), Some(-7));
    assert_eq!(opts.extension(&TAG), Some("hello".to_string()));
    assert!(opts.has_extension(&WEIGHT));

    // Roundtrip through wire encoding: the extension bytes live in
    // __buffa_unknown_fields and are re-encoded by Message::write_to.
    use buffa::Message;
    let bytes = opts.encode_to_vec();
    let decoded = FieldOptions::decode_from_slice(&bytes).expect("decode");
    assert_eq!(decoded.extension(&WEIGHT), Some(-7));
    assert_eq!(decoded.extension(&TAG), Some("hello".to_string()));

    // And an ExtensionSet impl was emitted in the generated output.
    let file = proto3_file("ext.proto");
    let files = generate(
        &[FileDescriptorProto {
            message_type: vec![DescriptorProto {
                name: Some("M".to_string()),
                ..Default::default()
            }],
            ..file
        }],
        &["ext.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate");
    assert!(
        files[0]
            .content
            .contains("impl ::buffa::ExtensionSet for M"),
        "missing ExtensionSet impl: {}",
        files[0].content
    );
}

#[test]
fn editions_delimited_message_encoding() {
    // Editions 2023 `features.message_encoding = DELIMITED`: the descriptor
    // field type stays TYPE_MESSAGE, but the codegen must route it through
    // the group (wire types 3/4) encode/decode paths via effective_type.
    //
    // Also verifies the map-entry exemption: map values are always
    // length-prefixed (protobuf spec hard rule), even under a file-level
    // DELIMITED default.
    use crate::generated::descriptor::{
        feature_set::MessageEncoding as FsMessageEncoding, Edition, FeatureSet, FieldOptions,
        FileOptions,
    };

    let inner_msg = DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("x", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    };

    // Field with per-field LENGTH_PREFIXED override (common pattern in
    // test_messages_edition2023.proto to opt specific fields back out).
    let mut lp_field = make_field("lp_child", 2, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    lp_field.type_name = Some(".Inner".to_string());
    lp_field.options = FieldOptions {
        features: FeatureSet {
            message_encoding: Some(FsMessageEncoding::LENGTH_PREFIXED),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
    .into();

    // Field that inherits the file-level DELIMITED default.
    let mut delim_field = make_field("delim_child", 3, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    delim_field.type_name = Some(".Inner".to_string());

    // Map field with message value — must stay length-prefixed.
    let mut map_field = make_field("inners", 4, Label::LABEL_REPEATED, Type::TYPE_MESSAGE);
    map_field.type_name = Some(".Outer.InnersEntry".to_string());
    let mut map_val = make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    map_val.type_name = Some(".Inner".to_string());
    let map_entry = DescriptorProto {
        name: Some("InnersEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            map_val,
        ],
        options: MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };

    let outer_msg = DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![lp_field, delim_field, map_field],
        nested_type: vec![map_entry],
        ..Default::default()
    };

    let file = FileDescriptorProto {
        name: Some("delim.proto".to_string()),
        edition: Some(Edition::EDITION_2023),
        options: FileOptions {
            features: FeatureSet {
                message_encoding: Some(FsMessageEncoding::DELIMITED),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
        .into(),
        message_type: vec![inner_msg, outer_msg],
        ..Default::default()
    };

    let files = generate(
        &[file],
        &["delim.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .expect("generate");
    let content = &joined(&files);

    // Field 3 (delim_child): inherits DELIMITED → StartGroup/EndGroup.
    assert!(
        content
            .contains("::buffa::encoding::Tag::new(3u32, ::buffa::encoding::WireType::StartGroup)"),
        "delim_child should encode as group: {content}"
    );
    assert!(
        content.contains("merge_group"),
        "delim_child should decode via merge_group: {content}"
    );

    // Field 2 (lp_child): explicit LENGTH_PREFIXED → regular message encoding.
    // prettyplease wraps Tag::new across lines for this field number, so
    // check the decode arm instead (single-line wire-type check).
    assert!(
        content.contains("2u32 => {\n                if tag.wire_type() != ::buffa::encoding::WireType::LengthDelimited"),
        "lp_child should decode as length-delimited: {content}"
    );
    // And it should NOT have a StartGroup encode for field 2.
    assert!(
        !content.contains("Tag::new(2u32, ::buffa::encoding::WireType::StartGroup)"),
        "lp_child should not encode as group"
    );

    // Map entry value: must NOT be group-encoded despite file-level DELIMITED.
    // If the map-entry exemption fails, codegen panics in type_encoded_size_expr
    // (TYPE_GROUP is unreachable there), so reaching this line is the key
    // evidence. Spot-check group-decode call counts: only delim_child should
    // use them (merge_group in owned impl, borrow_group in view).
    assert_eq!(
        content.matches("merge_group").count(),
        1,
        "owned: {content}"
    );
    assert_eq!(
        content.matches("borrow_group").count(),
        1,
        "view: {content}"
    );
}

#[test]
fn nested_message_cross_package_reference_uses_correct_nesting() {
    // Proto layout:
    //   package a.admin.v1: message Svc { message Filter { a.v1.Biz.Status status = 1; } }
    //   package a.v1: message Biz { enum Status { UNSPECIFIED=0; ACTIVE=1; } }
    //
    // In the generated code, Filter lives inside `pub mod svc { ... }` (nesting=1).
    // Its field must reference `super::super::super::v1::biz::Status` (3 supers),
    // not `super::super::v1::biz::Status` (2 supers, which is the nesting=0 path).
    let status_enum = EnumDescriptorProto {
        name: Some("Status".into()),
        value: vec![enum_value("UNSPECIFIED", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    };
    let biz = DescriptorProto {
        name: Some("Biz".into()),
        enum_type: vec![status_enum],
        ..Default::default()
    };
    let filter = DescriptorProto {
        name: Some("Filter".into()),
        field: vec![FieldDescriptorProto {
            name: Some("status".into()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".a.v1.Biz.Status".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let svc = DescriptorProto {
        name: Some("Svc".into()),
        nested_type: vec![filter],
        ..Default::default()
    };

    let file_admin = FileDescriptorProto {
        name: Some("admin.proto".into()),
        package: Some("a.admin.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![svc],
        ..Default::default()
    };
    let file_biz = FileDescriptorProto {
        name: Some("biz.proto".into()),
        package: Some("a.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![biz],
        ..Default::default()
    };

    let files = generate(
        &[file_admin, file_biz],
        &["admin.proto".into()],
        &CodeGenConfig::default(),
    )
    .expect("codegen should succeed");

    let content = &joined(&files);

    // Filter (nesting=1) must emit 3 supers to reach the common ancestor:
    //   super (svc) → super (v1) → super (admin) → v1::biz::Status
    assert!(
        content.contains("super::super::super::v1::biz::Status"),
        "nested message should use 3 supers for cross-package reference at nesting=1.\n\
         Generated code:\n{content}"
    );
    // Every occurrence of the path must have exactly 3 supers (nesting=1),
    // not 2 (which would be the nesting=0 bug). Since "super::super::super::"
    // contains "super::super::" as a substring, count occurrences directly.
    let path_3 = content
        .matches("super::super::super::v1::biz::Status")
        .count();
    let path_2 = content.matches("super::super::v1::biz::Status").count();
    assert_eq!(
        path_3, path_2,
        "every v1::biz::Status reference must use 3 supers (nesting=1), \
         but found {} with 3 supers and {} total with >=2 supers.\n\
         Generated code:\n{content}",
        path_3, path_2
    );
}

#[test]
fn doubly_nested_message_paths_use_correct_nesting() {
    // Stress the nesting plumbing at nesting=2, across each site that has
    // historically hardcoded a too-shallow depth:
    //   - message field (handled by the base fix)
    //   - oneof variant type (oneof.rs collect_variant_info)
    //   - textproto enum lookup (impl_text.rs enum_type_path)
    //
    // Proto layout:
    //   package a.admin.v1:
    //     message Svc { message Outer { message Inner {
    //       a.v1.Biz.Status direct = 1;
    //       oneof choice { a.v1.Biz.Status via_oneof = 2; }
    //     } } }
    //   package a.v1: message Biz { enum Status { UNSPECIFIED=0; ACTIVE=1; } }
    //
    // Inner sits at nesting=3 (`svc::outer::inner`), so every cross-package
    // reference it emits must use 4 supers.
    let status_enum = EnumDescriptorProto {
        name: Some("Status".into()),
        value: vec![enum_value("UNSPECIFIED", 0), enum_value("ACTIVE", 1)],
        ..Default::default()
    };
    let biz = DescriptorProto {
        name: Some("Biz".into()),
        enum_type: vec![status_enum],
        ..Default::default()
    };

    let inner = DescriptorProto {
        name: Some("Inner".into()),
        field: vec![
            FieldDescriptorProto {
                name: Some("direct".into()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".a.v1.Biz.Status".into()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("via_oneof".into()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".a.v1.Biz.Status".into()),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("choice".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer = DescriptorProto {
        name: Some("Outer".into()),
        nested_type: vec![inner],
        ..Default::default()
    };
    let svc = DescriptorProto {
        name: Some("Svc".into()),
        nested_type: vec![outer],
        ..Default::default()
    };

    let file_admin = FileDescriptorProto {
        name: Some("admin.proto".into()),
        package: Some("a.admin.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![svc],
        ..Default::default()
    };
    let file_biz = FileDescriptorProto {
        name: Some("biz.proto".into()),
        package: Some("a.v1".into()),
        syntax: Some("proto3".into()),
        message_type: vec![biz],
        ..Default::default()
    };

    // Exercise the impl_text path too — textproto enum references used to
    // hardcode nesting=0 regardless of the struct's actual depth.
    let config = CodeGenConfig {
        generate_text: true,
        ..Default::default()
    };
    let files = generate(&[file_admin, file_biz], &["admin.proto".into()], &config)
        .expect("codegen should succeed");
    let content = &joined(&files);

    // Inner is at nesting=3 (inside svc::outer::inner), so every cross-
    // package reference must emit 4 supers.
    let path_4 = content
        .matches("super::super::super::super::v1::biz::Status")
        .count();
    // path_3 should match only the Svc::Filter-style depth-1 references —
    // since we don't emit any here, we use path_3 purely as a sanity baseline.
    let path_3 = content
        .matches("super::super::super::v1::biz::Status")
        .count();
    assert_eq!(
        path_4, path_3,
        "every Inner (nesting=3) reference must use 4 supers, but \
         found {path_4} with 4 supers and {path_3} with >=3 supers.\n\
         Generated code:\n{content}"
    );
    assert!(
        path_4 > 0,
        "expected the cross-package enum reference to appear at least once \
         with 4 supers in Inner's module.\nGenerated code:\n{content}"
    );
}

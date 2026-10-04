//! The `(buffa.ext.message).name` and `(buffa.ext.enum).name` options: a
//! schema-chosen Rust name for a struct or an enum.
//!
//! `buffa-test`'s `ext_type_name` fixture compiles the generated code and
//! checks the formats that keep the proto name; these tests cover which
//! names are generated and which values are rejected.

use super::*;
use crate::generated::descriptor::EnumOptions;
use buffa::ExtensionSet;

const FILE: &str = "type_name.proto";
const MESSAGE_OPTION: &str = "(buffa.ext.message).name";
const ENUM_OPTION: &str = "(buffa.ext.enum).name";

fn message(name: &str) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.to_string()),
        ..Default::default()
    }
}

/// A message with `(buffa.ext.message).name = rust_name`.
fn named_message(name: &str, rust_name: &str) -> DescriptorProto {
    let mut options = MessageOptions::default();
    options.set_extension(
        &buffa_proto_options::MESSAGE,
        buffa_proto_options::MessageOptions {
            name: Some(rust_name.to_string()),
            ..Default::default()
        },
    );
    DescriptorProto {
        options: options.into(),
        ..message(name)
    }
}

fn enumeration(name: &str) -> EnumDescriptorProto {
    EnumDescriptorProto {
        name: Some(name.to_string()),
        value: vec![enum_value("UNSPECIFIED", 0)],
        ..Default::default()
    }
}

/// An enum with `(buffa.ext.enum).name = rust_name`.
fn named_enum(name: &str, rust_name: &str) -> EnumDescriptorProto {
    let mut options = EnumOptions::default();
    options.set_extension(
        &buffa_proto_options::ENUM,
        buffa_proto_options::EnumOptions {
            name: Some(rust_name.to_string()),
            ..Default::default()
        },
    );
    EnumDescriptorProto {
        options: options.into(),
        ..enumeration(name)
    }
}

/// A proto3 file in package `pkg`.
fn file_of(messages: Vec<DescriptorProto>, enums: Vec<EnumDescriptorProto>) -> FileDescriptorProto {
    FileDescriptorProto {
        package: Some("pkg".to_string()),
        message_type: messages,
        enum_type: enums,
        ..proto3_file(FILE)
    }
}

/// A singular field of the message or enum type `type_name`.
fn typed_field(name: &str, number: i32, ty: Type, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        type_name: Some(type_name.to_string()),
        ..make_field(name, number, Label::LABEL_OPTIONAL, ty)
    }
}

fn generate_one(file: FileDescriptorProto, config: &CodeGenConfig) -> Result<String, CodeGenError> {
    generate(&[file], &[FILE.to_string()], config).map(|files| joined(&files))
}

fn with_views() -> CodeGenConfig {
    CodeGenConfig {
        generate_views: true,
        ..Default::default()
    }
}

#[track_caller]
fn assert_invalid(
    file: FileDescriptorProto,
    option: &str,
    element: &str,
    name: &str,
    want: &NameOptionProblem,
) {
    let err = generate_one(file, &CodeGenConfig::default())
        .expect_err("an unusable name option must be rejected");
    let CodeGenError::InvalidNameOption {
        option: got_option,
        element: got_element,
        name: got_name,
        problem,
    } = &err
    else {
        panic!("expected InvalidNameOption, got: {err}");
    };
    assert_eq!(
        (
            *got_option,
            got_element.as_str(),
            got_name.as_str(),
            problem
        ),
        (option, element, name, want)
    );
}

#[track_caller]
fn assert_conflict(
    result: Result<String, CodeGenError>,
    option: &str,
    element: &str,
    other: &str,
    rust_name: &str,
) {
    let err = result.expect_err("a colliding name option must be rejected");
    let CodeGenError::NameOptionConflict {
        option: got_option,
        element: got_element,
        other: got_other,
        rust_name: got_rust_name,
    } = &err
    else {
        panic!("expected NameOptionConflict, got: {err}");
    };
    assert_eq!(
        (
            *got_option,
            got_element.as_str(),
            got_other.as_str(),
            got_rust_name.as_str()
        ),
        (option, element, other, rust_name)
    );
}

#[test]
fn option_names_the_struct_and_its_view() {
    let file = file_of(vec![named_message("Type", "Kind")], vec![]);
    let code = generate_one(file, &with_views()).unwrap();
    assert!(code.contains("pub struct Kind {"), "{code}");
    assert!(code.contains("pub struct KindView<"), "{code}");
    assert!(!code.contains("pub struct Type"), "{code}");
    // The proto name is still the message's full name.
    assert!(code.contains("\"pkg.Type\""), "{code}");
    assert!(!code.contains("pkg.Kind"), "{code}");
}

#[test]
fn option_names_the_enum() {
    let file = file_of(vec![], vec![named_enum("Type", "Kind")]);
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub enum Kind {"), "{code}");
    assert!(!code.contains("pub enum Type"), "{code}");
}

#[test]
fn value_is_used_as_written() {
    // A lower-case value is not converted, and a keyword-like proto name
    // needs no raw identifier once the option names the type.
    let file = file_of(
        vec![named_message("type", "kind_of_thing")],
        vec![named_enum("Level", "LEVEL")],
    );
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub struct kind_of_thing {"), "{code}");
    assert!(code.contains("pub enum LEVEL {"), "{code}");
    assert!(!code.contains("r#type"), "{code}");
}

#[test]
fn fields_refer_to_the_renamed_types() {
    let mut holder = message("Holder");
    holder.field = vec![
        typed_field("m", 1, Type::TYPE_MESSAGE, ".pkg.Type"),
        typed_field("e", 2, Type::TYPE_ENUM, ".pkg.Level"),
    ];
    let file = file_of(
        vec![named_message("Type", "Kind"), holder],
        vec![named_enum("Level", "Severity")],
    );
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("MessageField<Kind"), "{code}");
    assert!(code.contains("EnumValue<Severity>"), "{code}");
}

#[test]
fn another_file_refers_to_the_renamed_type() {
    let mut dependency = file_of(vec![named_message("Type", "Kind")], vec![]);
    dependency.name = Some("dep.proto".to_string());
    dependency.package = Some("dep".to_string());

    let mut holder = message("Holder");
    holder.field = vec![typed_field("m", 1, Type::TYPE_MESSAGE, ".dep.Type")];
    let mut file = file_of(vec![holder], vec![]);
    file.dependency.push("dep.proto".to_string());

    let files = generate(
        &[dependency, file],
        &[FILE.to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();
    let code = joined(&files);
    assert!(code.contains("dep::Kind"), "{code}");
    assert!(!code.contains("dep::Type"), "{code}");
}

#[test]
fn option_names_nested_types_and_leaves_their_module() {
    let mut inner = named_message("Inner", "Leaf");
    inner.nested_type.push(message("Deep"));
    let mut outer = message("Outer");
    outer.nested_type.push(inner);
    outer.enum_type.push(named_enum("Mode", "Setting"));
    outer.field = vec![
        typed_field("leaf", 1, Type::TYPE_MESSAGE, ".pkg.Outer.Inner"),
        typed_field("deep", 2, Type::TYPE_MESSAGE, ".pkg.Outer.Inner.Deep"),
        typed_field("mode", 3, Type::TYPE_ENUM, ".pkg.Outer.Mode"),
    ];
    let code = generate_one(file_of(vec![outer], vec![]), &with_views()).unwrap();
    assert!(code.contains("pub struct Leaf {"), "{code}");
    assert!(code.contains("pub struct LeafView<"), "{code}");
    assert!(code.contains("pub enum Setting {"), "{code}");
    assert!(code.contains("MessageField<outer::Leaf"), "{code}");
    assert!(code.contains("EnumValue<outer::Setting>"), "{code}");
    // The module of `Inner`'s nested types keeps the proto-derived name.
    assert!(code.contains("pub mod inner {"), "{code}");
    assert!(code.contains("outer::inner::Deep"), "{code}");
    assert!(!code.contains("pub mod leaf"), "{code}");
}

#[test]
fn type_name_prefix_is_not_added_to_the_value() {
    let config = CodeGenConfig {
        type_name_prefix: "Pb".to_string(),
        ..Default::default()
    };
    let file = file_of(
        vec![named_message("Type", "Kind"), message("Plain")],
        vec![named_enum("Level", "Severity")],
    );
    let code = generate_one(file, &config).unwrap();
    assert!(code.contains("pub struct Kind {"), "{code}");
    assert!(code.contains("pub enum Severity {"), "{code}");
    assert!(code.contains("pub struct PbPlain {"), "{code}");
    assert!(!code.contains("PbKind"), "{code}");
    assert!(!code.contains("PbSeverity"), "{code}");
}

#[test]
fn extern_prefix_mapping_follows_the_option() {
    let mut dependency = file_of(vec![named_message("Thing", "Kind")], vec![]);
    dependency.name = Some("dep.proto".to_string());
    dependency.package = Some("dep".to_string());
    dependency.message_type[0]
        .nested_type
        .push(named_message("Inner", "Leaf"));
    dependency.enum_type.push(named_enum("Level", "Severity"));
    let files = [dependency];
    let config = CodeGenConfig::default();

    // The crate that owns `dep` read the same descriptor, so its types have
    // the names the options give.
    let prefix = [(".dep".to_string(), "::dep_crate".to_string())];
    let ctx = context::CodeGenContext::new(&files, &config, &prefix);
    assert_eq!(ctx.rust_type(".dep.Thing"), Some("::dep_crate::Kind"));
    assert_eq!(ctx.rust_type(".dep.Level"), Some("::dep_crate::Severity"));
    assert_eq!(
        ctx.rust_type(".dep.Thing.Inner"),
        Some("::dep_crate::thing::Leaf")
    );

    // An exact entry is the caller's own path, used as written.
    let exact = [(".dep.Thing".to_string(), "::other::Item".to_string())];
    let ctx = context::CodeGenContext::new(&files, &config, &exact);
    assert_eq!(ctx.rust_type(".dep.Thing"), Some("::other::Item"));
}

#[test]
fn file_level_extern_mapping_follows_the_option() {
    let mut dependency = file_of(
        vec![named_message("Thing", "Kind"), message("Plain")],
        vec![],
    );
    dependency.name = Some("dep.proto".to_string());
    dependency.package = Some("dep".to_string());
    let files = [dependency];
    let config = CodeGenConfig::default();
    let file_roots = [("dep.proto".to_string(), "::other::dep".to_string())];
    let ctx = context::CodeGenContext::with_extern_resolution(&files, &config, &[], &file_roots);
    assert_eq!(ctx.rust_type(".dep.Thing"), Some("::other::dep::Kind"));
    assert_eq!(ctx.rust_type(".dep.Plain"), Some("::other::dep::Plain"));
}

#[test]
fn view_reexports_use_the_option_names() {
    // The root re-export of `Event`'s view is dropped when a sibling's
    // option takes the name `EventView`, as for a sibling with that proto
    // name.
    let file = file_of(
        vec![message("Event"), named_message("Other", "EventView")],
        vec![],
    );
    let code = generate_one(file, &with_views()).unwrap();
    assert!(
        !code.contains("pub use self::__buffa::view::EventView;"),
        "{code}"
    );
    assert!(
        code.contains("pub use self::__buffa::view::EventViewView;"),
        "{code}"
    );

    // A nested message's view is re-exported under the option's name.
    let mut outer = message("Outer");
    outer.nested_type.push(named_message("Inner", "Leaf"));
    let code = generate_one(file_of(vec![outer], vec![]), &with_views()).unwrap();
    assert!(code.contains("LeafView;"), "{code}");
    assert!(code.contains("LeafOwnedView;"), "{code}");
    assert!(!code.contains("InnerView"), "{code}");
}

#[test]
fn table_codec_names_the_tables_after_the_option() {
    let mut parent = message("HasLeaf");
    parent.field = vec![typed_field("leaf", 1, Type::TYPE_MESSAGE, ".pkg.Leaf")];
    let mut leaf = named_message("Leaf", "Twig");
    leaf.field = vec![make_field("n", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)];
    let config = CodeGenConfig {
        codec_strategy: CodecStrategy::Table,
        ..Default::default()
    };
    let code = generate_one(file_of(vec![parent, leaf], vec![]), &config).unwrap();
    let squashed: String = code.split_whitespace().collect();
    assert!(squashed.contains("static__BUFFA_TABLE_Twig"), "{code}");
    assert!(squashed.contains("(&__BUFFA_TABLE_Twig)"), "{code}");
    assert!(!squashed.contains("__BUFFA_TABLE_Leaf"), "{code}");
}

#[test]
fn unusable_value_in_a_file_that_is_not_generated_is_ignored() {
    let mut dependency = file_of(vec![named_message("Dep", "type")], vec![]);
    dependency.name = Some("dep.proto".to_string());
    dependency.package = Some("dep".to_string());

    let mut holder = message("Holder");
    holder.field = vec![typed_field("m", 1, Type::TYPE_MESSAGE, ".dep.Dep")];
    let mut file = file_of(vec![holder], vec![]);
    file.dependency.push("dep.proto".to_string());

    let files = generate(
        &[dependency, file],
        &[FILE.to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();
    assert!(joined(&files).contains("dep::Dep"));
}

#[test]
fn value_that_cannot_name_a_type_is_rejected() {
    let cases = [
        ("", NameOptionProblem::NotAnIdentifier),
        ("1st", NameOptionProblem::NotAnIdentifier),
        ("a::B", NameOptionProblem::NotAnIdentifier),
        ("r#type", NameOptionProblem::NotAnIdentifier),
        ("_", NameOptionProblem::NotAnIdentifier),
        ("Caf\u{e9}", NameOptionProblem::NotAnIdentifier),
        ("type", NameOptionProblem::Keyword),
        ("Self", NameOptionProblem::Keyword),
        ("__buffa", NameOptionProblem::ReservedPrefix),
        ("__buffa_x", NameOptionProblem::ReservedPrefix),
        ("bool", NameOptionProblem::PrimitiveType),
        ("str", NameOptionProblem::PrimitiveType),
        ("u8", NameOptionProblem::PrimitiveType),
        ("usize", NameOptionProblem::PrimitiveType),
        ("i32", NameOptionProblem::PrimitiveType),
        ("i64", NameOptionProblem::PrimitiveType),
        ("u32", NameOptionProblem::PrimitiveType),
        ("u64", NameOptionProblem::PrimitiveType),
        ("f32", NameOptionProblem::PrimitiveType),
        ("f64", NameOptionProblem::PrimitiveType),
    ];
    for (value, problem) in &cases {
        let file = file_of(vec![named_message("Msg", value)], vec![]);
        assert_invalid(file, MESSAGE_OPTION, "pkg.Msg", value, problem);

        let file = file_of(vec![], vec![named_enum("Level", value)]);
        assert_invalid(file, ENUM_OPTION, "pkg.Level", value, problem);
    }
}

#[test]
fn primitive_that_generated_code_does_not_use_is_a_usable_value() {
    let file = file_of(vec![named_message("Msg", "char")], vec![]);
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub struct char {"), "{code}");
}

#[test]
fn invalid_value_is_reported_before_a_conflict() {
    // `A`'s value is unusable, so `A` would fall back to its derived name,
    // which `B`'s value also claims. The error is the invalid value.
    let file = file_of(
        vec![named_message("A", "type"), named_message("B", "A")],
        vec![],
    );
    assert_invalid(
        file,
        MESSAGE_OPTION,
        "pkg.A",
        "type",
        &NameOptionProblem::Keyword,
    );

    let mut outer = message("Outer");
    outer.nested_type = vec![named_message("bool", "u32"), message("bool_")];
    assert_invalid(
        file_of(vec![outer], vec![]),
        MESSAGE_OPTION,
        "pkg.Outer.bool",
        "u32",
        &NameOptionProblem::PrimitiveType,
    );
}

#[test]
fn file_level_enum_is_checked_against_the_reserved_module_by_its_rust_name() {
    let file = file_of(vec![], vec![named_enum("__buffa", "Fine")]);
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert!(code.contains("pub enum Fine {"), "{code}");

    let config = CodeGenConfig {
        type_name_prefix: "Pb".to_string(),
        ..Default::default()
    };
    let file = file_of(vec![], vec![enumeration("__buffa")]);
    let code = generate_one(file, &config).unwrap();
    assert!(code.contains("pub enum Pb__buffa {"), "{code}");

    let file = file_of(vec![], vec![enumeration("__buffa")]);
    let err = generate_one(file, &CodeGenConfig::default()).unwrap_err();
    assert!(
        matches!(err, CodeGenError::ReservedModuleName { .. }),
        "{err}"
    );
}

#[test]
fn invalid_value_on_a_nested_type_names_the_type() {
    let mut outer = message("Outer");
    outer.nested_type.push(named_message("Inner", "u8"));
    assert_invalid(
        file_of(vec![outer], vec![]),
        MESSAGE_OPTION,
        "pkg.Outer.Inner",
        "u8",
        &NameOptionProblem::PrimitiveType,
    );

    let mut outer = message("Outer");
    outer.enum_type.push(named_enum("Mode", "fn"));
    assert_invalid(
        file_of(vec![outer], vec![]),
        ENUM_OPTION,
        "pkg.Outer.Mode",
        "fn",
        &NameOptionProblem::Keyword,
    );
}

#[test]
fn two_types_with_one_name_conflict() {
    let default = CodeGenConfig::default();

    // The value is a sibling's proto name, in either declaration order.
    let file = file_of(vec![message("A"), named_message("B", "A")], vec![]);
    assert_conflict(
        generate_one(file, &default),
        MESSAGE_OPTION,
        "pkg.B",
        "pkg.A",
        "A",
    );
    let file = file_of(vec![named_message("B", "A"), message("A")], vec![]);
    assert_conflict(
        generate_one(file, &default),
        MESSAGE_OPTION,
        "pkg.B",
        "pkg.A",
        "A",
    );

    // Two options with one value: the error names the second.
    let file = file_of(
        vec![named_message("A", "Same"), named_message("B", "Same")],
        vec![],
    );
    assert_conflict(
        generate_one(file, &default),
        MESSAGE_OPTION,
        "pkg.B",
        "pkg.A",
        "Same",
    );

    // A message and an enum share the type namespace.
    let file = file_of(vec![message("A")], vec![named_enum("E", "A")]);
    assert_conflict(
        generate_one(file, &default),
        ENUM_OPTION,
        "pkg.E",
        "pkg.A",
        "A",
    );

    // The value is the escaped name of a sibling: `bool` is declared `bool_`.
    let file = file_of(vec![message("bool"), named_message("B", "bool_")], vec![]);
    assert_conflict(
        generate_one(file, &default),
        MESSAGE_OPTION,
        "pkg.B",
        "pkg.bool",
        "bool_",
    );
}

#[test]
fn value_conflicts_with_a_prefixed_sibling() {
    let config = CodeGenConfig {
        type_name_prefix: "Pb".to_string(),
        ..Default::default()
    };
    let file = file_of(vec![message("A"), named_message("B", "PbA")], vec![]);
    assert_conflict(
        generate_one(file, &config),
        MESSAGE_OPTION,
        "pkg.B",
        "pkg.A",
        "PbA",
    );
    // The value `A` is free: the prefix moved the sibling to `PbA`.
    let file = file_of(vec![message("A"), named_message("B", "A")], vec![]);
    let code = generate_one(file, &config).unwrap();
    assert!(code.contains("pub struct A {"), "{code}");
    assert!(code.contains("pub struct PbA {"), "{code}");
}

#[test]
fn nested_types_conflict_inside_their_message_only() {
    let mut outer = message("Outer");
    outer.nested_type.push(message("A"));
    outer.enum_type.push(named_enum("E", "A"));
    assert_conflict(
        generate_one(file_of(vec![outer], vec![]), &CodeGenConfig::default()),
        ENUM_OPTION,
        "pkg.Outer.E",
        "pkg.Outer.A",
        "A",
    );

    // The same name in two scopes is two types.
    let mut outer = message("Outer");
    outer.nested_type.push(named_message("Inner", "A"));
    let file = file_of(vec![outer, message("A")], vec![]);
    let code = generate_one(file, &CodeGenConfig::default()).unwrap();
    assert_eq!(code.matches("pub struct A {").count(), 2, "{code}");
}

#[test]
fn types_of_one_package_conflict_across_files() {
    let first = file_of(vec![message("A")], vec![]);
    let mut second = file_of(vec![named_message("B", "A")], vec![]);
    second.name = Some("second.proto".to_string());
    let result = generate(
        &[first, second],
        &[FILE.to_string(), "second.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .map(|files| joined(&files));
    assert_conflict(result, MESSAGE_OPTION, "pkg.B", "pkg.A", "A");
}

#[test]
fn two_derived_names_still_report_a_type_name_conflict() {
    // An option elsewhere in the scope does not change the error for two
    // names that buffa derived.
    let file = file_of(
        vec![
            message("bool"),
            message("bool_"),
            named_message("C", "Other"),
        ],
        vec![],
    );
    let err = generate_one(file, &CodeGenConfig::default()).unwrap_err();
    assert!(
        matches!(
            &err,
            CodeGenError::TypeNameConflict { scope, first_type, second_type, rust_name }
                if scope == "pkg" && first_type == "bool" && second_type == "bool_"
                    && rust_name == "bool_"
        ),
        "{err}"
    );
}

#[test]
fn error_messages_name_the_option_and_the_type() {
    let file = file_of(vec![named_message("Msg", "bool")], vec![]);
    let err = generate_one(file, &CodeGenConfig::default()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid `(buffa.ext.message).name` = \"bool\" on 'pkg.Msg': the value is a primitive \
         type name that generated code uses (`bool`, `str`, `u8`, `usize`, `i32`, `i64`, `u32`, \
         `u64`, `f32`, `f64`), and a type with that name would shadow it; choose another name, \
         such as `Bool`"
    );

    let file = file_of(vec![message("A")], vec![named_enum("E", "A")]);
    let err = generate_one(file, &CodeGenConfig::default()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "name conflict: `(buffa.ext.enum).name` gives 'pkg.E' the Rust name `A`, \
         which 'pkg.A' also has; change the option"
    );
}

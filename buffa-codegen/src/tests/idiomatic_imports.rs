//! End-to-end tests for `CodeGenConfig::idiomatic_imports` (file_per_package
//! package-root import shortening).
//!
//! With the flag off, output is byte-for-byte identical to the default. With
//! the flag on (requires `file_per_package`), message/enum types and runtime
//! types referenced at the package root are shortened to `use`-backed names,
//! falling back to parent-module qualification and fully-qualified paths on
//! collisions. Deeper scopes (nested-message modules, `__buffa` wrappers)
//! keep qualified paths.

use super::*;

fn idiomatic_config() -> CodeGenConfig {
    CodeGenConfig {
        file_per_package: true,
        idiomatic_imports: true,
        ..Default::default()
    }
}

fn opt_message_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(type_name.to_string()),
        ..Default::default()
    }
}

/// `test.other.Dep` + `test.pkg.Holder` referencing it cross-package.
fn cross_package_fixture() -> [FileDescriptorProto; 2] {
    let mut dep = proto3_file("dep.proto");
    dep.package = Some("test.other".to_string());
    dep.message_type.push(DescriptorProto {
        name: Some("Dep".to_string()),
        ..Default::default()
    });

    let mut holder = proto3_file("holder.proto");
    holder.package = Some("test.pkg".to_string());
    holder.dependency = vec!["dep.proto".to_string()];
    holder.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![opt_message_field("d", 1, ".test.other.Dep")],
        ..Default::default()
    });
    [dep, holder]
}

fn pkg_file<'a>(files: &'a [GeneratedFile], name: &str) -> &'a GeneratedFile {
    files
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no output file named {name}"))
}

#[test]
fn flag_requires_file_per_package() {
    let config = CodeGenConfig {
        idiomatic_imports: true,
        ..Default::default()
    };
    let err = generate(&[proto3_file("a.proto")], &["a.proto".to_string()], &config)
        .expect_err("idiomatic_imports without file_per_package must be rejected");
    assert!(
        err.to_string().contains("file_per_package"),
        "error should name the missing prerequisite: {err}"
    );
}

#[test]
fn cross_package_field_gets_use_and_short_name() {
    let descs = cross_package_fixture();
    let files = generate(
        &descs,
        &["dep.proto".to_string(), "holder.proto".to_string()],
        &idiomatic_config(),
    )
    .expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    assert!(
        pkg.content.contains("use super::other::Dep;"),
        "cross-package reference should produce a use directive: {}",
        pkg.content
    );
    assert!(
        pkg.content.contains("use ::buffa::MessageField;"),
        "MessageField should be imported once used: {}",
        pkg.content
    );
    assert!(
        pkg.content
            .contains("pub d: MessageField<Dep, ::buffa::Inline<Dep>>,"),
        "field should use the short names: {}",
        pkg.content
    );
    // No string/bytes fields in the fixture: the alloc types must not be
    // imported (an unreferenced `use` would trip `unused_imports` in the
    // consumer crate). Guards against eager type computation in
    // `classify_field` recording phantom collection-pass requests.
    for phantom in [
        "use ::buffa::alloc::string::String;",
        "use ::buffa::alloc::vec::Vec;",
    ] {
        assert!(
            !pkg.content.contains(phantom),
            "unreferenced import `{phantom}` must not be emitted: {}",
            pkg.content
        );
    }
}

#[test]
fn flag_off_keeps_qualified_paths_in_fpp_mode() {
    let descs = cross_package_fixture();
    let config = CodeGenConfig {
        file_per_package: true,
        ..Default::default()
    };
    let files = generate(
        &descs,
        &["dep.proto".to_string(), "holder.proto".to_string()],
        &config,
    )
    .expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    assert!(
        pkg.content.contains(
            "pub d: ::buffa::MessageField<super::other::Dep, ::buffa::Inline<super::other::Dep>>,"
        ),
        "flag-off output must keep qualified paths: {}",
        pkg.content
    );
    assert!(
        !pkg.content.contains("\nuse "),
        "flag-off output must not emit root use directives: {}",
        pkg.content
    );
}

#[test]
fn runtime_types_are_shortened_at_root() {
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
    let mut file = proto3_file("kitchen.proto");
    file.package = Some("test.pkg".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Kitchen".to_string()),
        field: vec![
            make_field("name", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("data", 2, Label::LABEL_OPTIONAL, Type::TYPE_BYTES),
            FieldDescriptorProto {
                proto3_optional: Some(true),
                oneof_index: Some(0),
                ..make_field("maybe", 3, Label::LABEL_OPTIONAL, Type::TYPE_INT32)
            },
            FieldDescriptorProto {
                type_name: Some(".test.pkg.Kitchen.AttrsEntry".to_string()),
                ..make_field("attrs", 4, Label::LABEL_REPEATED, Type::TYPE_MESSAGE)
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_maybe".to_string()),
            ..Default::default()
        }],
        nested_type: vec![map_entry],
        ..Default::default()
    });

    let files = generate(&[file], &["kitchen.proto".to_string()], &idiomatic_config())
        .expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    for (use_line, field_line) in [
        ("use ::buffa::alloc::string::String;", "pub name: String,"),
        ("use ::buffa::alloc::vec::Vec;", "pub data: Vec<u8>,"),
        ("use ::core::option::Option;", "pub maybe: Option<i32>,"),
        (
            "use ::buffa::__private::HashMap;",
            "pub attrs: HashMap<String, i32>,",
        ),
    ] {
        assert!(
            pkg.content.contains(use_line),
            "missing `{use_line}`: {}",
            pkg.content
        );
        assert!(
            pkg.content.contains(field_line),
            "missing `{field_line}`: {}",
            pkg.content
        );
    }
}

#[test]
fn extern_reference_gets_use_directive() {
    let mut ts = proto3_file("google/protobuf/timestamp.proto");
    ts.package = Some("google.protobuf".to_string());
    ts.message_type.push(DescriptorProto {
        name: Some("Timestamp".to_string()),
        ..Default::default()
    });
    let mut holder = proto3_file("holder.proto");
    holder.package = Some("test.pkg".to_string());
    holder.dependency = vec!["google/protobuf/timestamp.proto".to_string()];
    holder.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![opt_message_field("at", 1, ".google.protobuf.Timestamp")],
        ..Default::default()
    });

    let config = CodeGenConfig {
        extern_paths: vec![(
            ".google.protobuf".to_string(),
            "::buffa_types::google::protobuf".to_string(),
        )],
        ..idiomatic_config()
    };
    let files =
        generate(&[ts, holder], &["holder.proto".to_string()], &config).expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    assert!(
        pkg.content
            .contains("use ::buffa_types::google::protobuf::Timestamp;"),
        "extern reference should produce a use directive: {}",
        pkg.content
    );
    assert!(
        pkg.content
            .contains("pub at: MessageField<Timestamp, ::buffa::Inline<Timestamp>>,"),
        "field should use the short names: {}",
        pkg.content
    );
}

#[test]
fn package_item_collision_falls_down_ladder() {
    // The package defines its own `Dep`, so the cross-package `Dep` falls
    // to parent-module qualification; it also defines `String`, so the
    // alloc type stays fully qualified (runtime types are
    // rung-1-or-nothing).
    let mut dep = proto3_file("dep.proto");
    dep.package = Some("test.other".to_string());
    dep.message_type.push(DescriptorProto {
        name: Some("Dep".to_string()),
        ..Default::default()
    });
    let mut holder = proto3_file("holder.proto");
    holder.package = Some("test.pkg".to_string());
    holder.dependency = vec!["dep.proto".to_string()];
    holder.message_type.push(DescriptorProto {
        name: Some("Dep".to_string()),
        ..Default::default()
    });
    holder.message_type.push(DescriptorProto {
        name: Some("String".to_string()),
        ..Default::default()
    });
    holder.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![
            opt_message_field("d", 1, ".test.other.Dep"),
            make_field("name", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    });

    let files = generate(
        &[dep, holder],
        &["dep.proto".to_string(), "holder.proto".to_string()],
        &idiomatic_config(),
    )
    .expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    assert!(
        pkg.content.contains("use super::other;"),
        "parent-module rung should import the module: {}",
        pkg.content
    );
    assert!(
        pkg.content
            .contains("pub d: MessageField<other::Dep, ::buffa::Inline<other::Dep>>,"),
        "collided leaf should be parent-qualified: {}",
        pkg.content
    );
    assert!(
        pkg.content
            .contains("pub name: ::buffa::alloc::string::String,"),
        "alloc String must stay qualified when the package defines `String`: {}",
        pkg.content
    );
}

#[test]
fn reflection_pool_reexport_name_is_reserved() {
    // `descriptor_pool` is re-exported at the package root by the
    // reflection stitcher (not via a ReexportCandidate the dry run would
    // capture), so a cross-package type with that leaf name must not claim
    // it — the import falls to parent-module qualification instead.
    let mut dep = proto3_file("dep.proto");
    dep.package = Some("test.other".to_string());
    dep.message_type.push(DescriptorProto {
        name: Some("descriptor_pool".to_string()),
        ..Default::default()
    });
    let mut holder = proto3_file("holder.proto");
    holder.package = Some("test.pkg".to_string());
    holder.dependency = vec!["dep.proto".to_string()];
    holder.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![opt_message_field("p", 1, ".test.other.descriptor_pool")],
        ..Default::default()
    });

    let config = CodeGenConfig {
        generate_reflection: true,
        ..idiomatic_config()
    };
    let files = generate(
        &[dep, holder],
        &["dep.proto".to_string(), "holder.proto".to_string()],
        &config,
    )
    .expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    assert!(
        !pkg.content.contains("use super::other::descriptor_pool;"),
        "`descriptor_pool` must not be claimed while reflection re-exports it: {}",
        pkg.content
    );
    assert!(
        pkg.content.contains(
            "pub p: MessageField<other::descriptor_pool, ::buffa::Inline<other::descriptor_pool>>,"
        ),
        "reference should fall to parent-module qualification: {}",
        pkg.content
    );
}

#[test]
fn nested_and_oneof_scopes_stay_qualified() {
    let mut dep = proto3_file("dep.proto");
    dep.package = Some("test.other".to_string());
    dep.message_type.push(DescriptorProto {
        name: Some("Dep".to_string()),
        ..Default::default()
    });
    let mut holder = proto3_file("holder.proto");
    holder.package = Some("test.pkg".to_string());
    holder.dependency = vec!["dep.proto".to_string()];
    holder.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![FieldDescriptorProto {
            oneof_index: Some(0),
            ..opt_message_field("v", 1, ".test.other.Dep")
        }],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            field: vec![opt_message_field("d", 1, ".test.other.Dep")],
            ..Default::default()
        }],
        ..Default::default()
    });

    let files = generate(
        &[dep, holder],
        &["dep.proto".to_string(), "holder.proto".to_string()],
        &idiomatic_config(),
    )
    .expect("should generate");
    let pkg = pkg_file(&files, "test.pkg.rs");
    // Nested-message struct (one module below root): no `use` visible
    // there, so the field keeps its qualified path. The inline pointer's
    // type argument carries the same qualified path.
    assert!(
        pkg.content
            .contains("::buffa::Inline<super::super::other::Dep>"),
        "nested-message field must stay qualified: {}",
        pkg.content
    );
    // Oneof enum body (three modules below root): same.
    assert!(
        pkg.content
            .contains("super::super::super::super::other::Dep"),
        "oneof variant type must stay qualified: {}",
        pkg.content
    );
}

#[test]
fn output_is_stable_under_file_reordering() {
    // Two files in one package, each referencing a different cross-package
    // type whose leaves collide — alias assignment must not depend on
    // which file is generated first.
    let mut deps = proto3_file("deps.proto");
    deps.package = Some("test.other".to_string());
    deps.message_type.push(DescriptorProto {
        name: Some("Thing".to_string()),
        ..Default::default()
    });
    let mut deps2 = proto3_file("deps2.proto");
    deps2.package = Some("test.more".to_string());
    deps2.message_type.push(DescriptorProto {
        name: Some("Thing".to_string()),
        ..Default::default()
    });

    let mk = |fname: &str, msg: &str, target: &str| {
        let mut f = proto3_file(fname);
        f.package = Some("test.pkg".to_string());
        f.dependency = vec!["deps.proto".to_string(), "deps2.proto".to_string()];
        f.message_type.push(DescriptorProto {
            name: Some(msg.to_string()),
            field: vec![opt_message_field("t", 1, target)],
            ..Default::default()
        });
        f
    };
    let a = mk("a.proto", "UsesOther", ".test.other.Thing");
    let b = mk("b.proto", "UsesMore", ".test.more.Thing");
    let deps_clone = [deps.clone(), deps2.clone()];

    let forward = generate(
        &[deps.clone(), deps2.clone(), a.clone(), b.clone()],
        &[
            "deps.proto".to_string(),
            "deps2.proto".to_string(),
            "a.proto".to_string(),
            "b.proto".to_string(),
        ],
        &idiomatic_config(),
    )
    .expect("forward order should generate");
    let reverse = generate(
        &[deps_clone[0].clone(), deps_clone[1].clone(), b, a],
        &[
            "deps.proto".to_string(),
            "deps2.proto".to_string(),
            "b.proto".to_string(),
            "a.proto".to_string(),
        ],
        &idiomatic_config(),
    )
    .expect("reverse order should generate");

    let fwd = pkg_file(&forward, "test.pkg.rs");
    let rev = pkg_file(&reverse, "test.pkg.rs");
    // Item order in the package file follows the input file order (same as
    // flag-off fpp output), so whole-file equality is not expected — but
    // the alias assignment must be identical: same use block, same short
    // forms for the same paths.
    let use_lines = |content: &str| -> Vec<String> {
        content
            .lines()
            .filter(|l| l.starts_with("use "))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        use_lines(&fwd.content),
        use_lines(&rev.content),
        "use block must not depend on file generation order"
    );
    for field in [
        "pub t: MessageField<Thing, ::buffa::Inline<Thing>>,",
        "pub t: MessageField<other::Thing, ::buffa::Inline<other::Thing>>,",
    ] {
        assert!(
            fwd.content.contains(field) && rev.content.contains(field),
            "both orders must emit `{field}`:\n--- forward:\n{}\n--- reverse:\n{}",
            fwd.content,
            rev.content
        );
    }
    // Sorted assignment: `super::more::Thing` < `super::other::Thing`, so
    // `test.more.Thing` wins the bare leaf in both orders.
    assert!(
        fwd.content.contains("use super::more::Thing;"),
        "sorted-order winner should hold the bare leaf: {}",
        fwd.content
    );
}

#[test]
fn reordered_outputs_are_byte_identical() {
    let descs = cross_package_fixture();
    let [dep, holder] = descs.clone();
    let forward = generate(
        &descs,
        &["dep.proto".to_string(), "holder.proto".to_string()],
        &idiomatic_config(),
    )
    .expect("should generate");
    let reverse = generate(
        &[holder, dep],
        &["holder.proto".to_string(), "dep.proto".to_string()],
        &idiomatic_config(),
    )
    .expect("should generate");
    assert_eq!(
        pkg_file(&forward, "test.pkg.rs").content,
        pkg_file(&reverse, "test.pkg.rs").content,
        "output must be stable under file reordering"
    );
}

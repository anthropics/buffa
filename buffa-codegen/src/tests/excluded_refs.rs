//! Tests for the ExcludedPackageFieldRef warning emitted when a kept file
//! references a type from a package that is not being generated and has no
//! extern_path mapping.

use super::*;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};

fn dep_file_with_message(file_name: &str, package: &str, msg_name: &str) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(file_name.to_string()),
        package: Some(package.to_string()),
        syntax: Some("proto3".to_string()),
        message_type: vec![DescriptorProto {
            name: Some(msg_name.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn field_referencing(name: &str, number: i32, type_fqn: &str) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(type_fqn.to_string()),
        ..Default::default()
    }
}

/// Build a minimal two-file setup: `dep.proto` (not kept) and `kept.proto`
/// (in files_to_generate). `kept.proto` has one message with one field
/// typed `.dep.Constraint`.
fn fixture() -> (FileDescriptorProto, FileDescriptorProto) {
    let dep = dep_file_with_message("dep.proto", "dep", "Constraint");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        field: vec![field_referencing("constraint", 1, ".dep.Constraint")],
        ..Default::default()
    });
    (dep, kept)
}

// ── Basic detection ──────────────────────────────────────────────────────────

#[test]
fn test_excluded_package_ref_warns() {
    // dep.proto is a dependency providing `dep.Constraint`, but is NOT in
    // files_to_generate (simulating an exclude_package scenario).
    let (dep, kept) = fixture();
    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::ExcludedPackageFieldRef { ref_package, .. }
                if ref_package == "dep"
        )),
        "must warn when a field references a type from an excluded package: {warnings:?}"
    );
}

#[test]
fn test_excluded_package_ref_structured_fields() {
    let (dep, kept) = fixture();
    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    match warnings
        .iter()
        .find(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. }))
        .expect("must have ExcludedPackageFieldRef warning")
    {
        CodeGenWarning::ExcludedPackageFieldRef {
            file_name,
            message_name,
            field_name,
            ref_package,
            type_fqn,
            ..
        } => {
            assert_eq!(file_name, "kept.proto");
            assert_eq!(message_name, "Request");
            assert_eq!(field_name, "constraint");
            assert_eq!(ref_package, "dep");
            assert_eq!(type_fqn, ".dep.Constraint");
        }
        other => panic!("unexpected warning variant: {other:?}"),
    }
}

#[test]
fn test_excluded_package_ref_display() {
    let (dep, kept) = fixture();
    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    let w = warnings
        .iter()
        .find(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. }))
        .expect("must have ExcludedPackageFieldRef warning");
    let s = w.to_string();
    assert!(s.contains("kept.proto"), "display must name the file: {s}");
    assert!(
        s.contains(".dep.Constraint"),
        "display must name the type: {s}"
    );
    assert!(s.contains("dep"), "display must name the package: {s}");
    assert!(
        s.contains("extern_path"),
        "display must hint at extern_path fix: {s}"
    );
    assert!(
        s.contains("buffa-build"),
        "display must give build-tool-specific hint: {s}"
    );
}

// ── Suppression cases ─────────────────────────────────────────────────────────

#[test]
fn test_no_warn_when_type_is_generated() {
    // Both files are in files_to_generate — the type is being generated locally.
    let (dep, kept) = fixture();
    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["dep.proto".to_string(), "kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. })),
        "must not warn when the referenced type is being generated: {warnings:?}"
    );
}

#[test]
fn test_no_warn_with_package_extern_path() {
    // dep package is excluded but covered by a package-level extern_path.
    let (dep, kept) = fixture();
    let config = CodeGenConfig {
        extern_paths: vec![(".dep".to_string(), "::dep_crate".to_string())],
        ..Default::default()
    };
    let (_, warnings) =
        generate_with_diagnostics(&[dep, kept], &["kept.proto".to_string()], &config).unwrap();

    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. })),
        "must not warn when the referenced package has an extern_path: {warnings:?}"
    );
}

#[test]
fn test_no_warn_with_per_type_extern_path() {
    // A per-type extern_path (exact FQN) also suppresses the warning, not
    // just package-level ones.
    let (dep, kept) = fixture();
    let config = CodeGenConfig {
        extern_paths: vec![(
            ".dep.Constraint".to_string(),
            "::dep_crate::Constraint".to_string(),
        )],
        ..Default::default()
    };
    let (_, warnings) =
        generate_with_diagnostics(&[dep, kept], &["kept.proto".to_string()], &config).unwrap();

    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. })),
        "must not warn when the type has a per-type extern_path: {warnings:?}"
    );
}

#[test]
fn test_no_warn_for_wkt_auto_extern_path() {
    // Fields referencing WKTs (auto-injected extern_path) must not warn.
    let wkt_dep = dep_file_with_message(
        "google/protobuf/timestamp.proto",
        "google.protobuf",
        "Timestamp",
    );
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Event".to_string()),
        field: vec![field_referencing("ts", 1, ".google.protobuf.Timestamp")],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[wkt_dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. })),
        "must not warn for WKT references (auto extern_path): {warnings:?}"
    );
}

// ── Structural coverage ───────────────────────────────────────────────────────

#[test]
fn test_excluded_package_ref_in_nested_message() {
    // Warning fires for a field inside a nested message.
    let dep = dep_file_with_message("dep.proto", "dep", "Constraint");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            field: vec![field_referencing("constraint", 1, ".dep.Constraint")],
            ..Default::default()
        }],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    match warnings
        .iter()
        .find(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. }))
    {
        Some(CodeGenWarning::ExcludedPackageFieldRef {
            message_name,
            field_name,
            ..
        }) => {
            assert_eq!(
                message_name, "Outer.Inner",
                "message_name must include nested path"
            );
            assert_eq!(field_name, "constraint");
        }
        other => panic!("expected ExcludedPackageFieldRef, got: {other:?}"),
    }
}

#[test]
fn test_excluded_package_ref_enum_type() {
    // The check is type-agnostic: an enum-typed field also warns.
    let mut dep = proto3_file("dep.proto");
    dep.package = Some("dep".to_string());
    dep.enum_type
        .push(crate::generated::descriptor::EnumDescriptorProto {
            name: Some("Status".to_string()),
            value: vec![enum_value("UNKNOWN", 0)],
            ..Default::default()
        });

    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("status".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(".dep.Status".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::ExcludedPackageFieldRef { type_fqn, .. }
                if type_fqn == ".dep.Status"
        )),
        "must warn for an enum-typed field: {warnings:?}"
    );
}

#[test]
fn test_excluded_package_ref_file_level_extension() {
    // A file-level extension whose value type is from an excluded package warns.
    let dep = dep_file_with_message("dep.proto", "dep", "Constraint");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    // File-level extension: no message container, parent_path is empty.
    kept.extension
        .push(field_referencing("constraint_ext", 1001, ".dep.Constraint"));

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    match warnings
        .iter()
        .find(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. }))
    {
        Some(CodeGenWarning::ExcludedPackageFieldRef {
            message_name,
            field_name,
            ..
        }) => {
            assert!(
                message_name.is_empty(),
                "file-level extension must have empty message_name, got: {message_name:?}"
            );
            assert_eq!(field_name, "constraint_ext");
        }
        other => panic!("expected ExcludedPackageFieldRef, got: {other:?}"),
    }
}

// ── Deduplication and type-granularity ───────────────────────────────────────

#[test]
fn test_dedup_same_type_multiple_fields() {
    // Same excluded type referenced by two fields → only one warning per file.
    let dep = dep_file_with_message("dep.proto", "dep", "Constraint");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        field: vec![
            field_referencing("c1", 1, ".dep.Constraint"),
            field_referencing("c2", 2, ".dep.Constraint"),
        ],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    let count = warnings
        .iter()
        .filter(|w| {
            matches!(w, CodeGenWarning::ExcludedPackageFieldRef { type_fqn, .. }
                if type_fqn == ".dep.Constraint")
        })
        .count();
    assert_eq!(
        count, 1,
        "must emit only one warning per unique (file, type_fqn) pair: {warnings:?}"
    );
}

#[test]
fn test_cross_file_dedup_two_warnings() {
    // Two kept files both referencing the same excluded type → two warnings
    // (one per file), not one.
    let dep = dep_file_with_message("dep.proto", "dep", "Constraint");

    let mut kept1 = proto3_file("kept1.proto");
    kept1.package = Some("svc".to_string());
    kept1.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        field: vec![field_referencing("constraint", 1, ".dep.Constraint")],
        ..Default::default()
    });

    let mut kept2 = proto3_file("kept2.proto");
    kept2.package = Some("svc2".to_string());
    kept2.message_type.push(DescriptorProto {
        name: Some("Response".to_string()),
        field: vec![field_referencing("constraint", 1, ".dep.Constraint")],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept1, kept2],
        &["kept1.proto".to_string(), "kept2.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    let count = warnings
        .iter()
        .filter(|w| {
            matches!(w, CodeGenWarning::ExcludedPackageFieldRef { type_fqn, .. }
                if type_fqn == ".dep.Constraint")
        })
        .count();
    assert_eq!(
        count, 2,
        "must emit one warning per (file, type_fqn) pair across files: {warnings:?}"
    );
}

#[test]
fn test_type_granular_partial_package() {
    // Package `dep` has TWO files: dep1.proto (kept) and dep2.proto (not kept).
    // A reference to `.dep.TypeFromDep2` must still warn even though `dep` is
    // in the generated set — the specific type is not being generated.
    let mut dep1 = proto3_file("dep1.proto");
    dep1.package = Some("dep".to_string());
    dep1.message_type.push(DescriptorProto {
        name: Some("TypeFromDep1".to_string()),
        ..Default::default()
    });

    let mut dep2 = proto3_file("dep2.proto");
    dep2.package = Some("dep".to_string());
    dep2.message_type.push(DescriptorProto {
        name: Some("TypeFromDep2".to_string()),
        ..Default::default()
    });

    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        field: vec![
            field_referencing("from_dep1", 1, ".dep.TypeFromDep1"),
            field_referencing("from_dep2", 2, ".dep.TypeFromDep2"),
        ],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep1, dep2, kept],
        // dep1.proto IS being generated; dep2.proto is NOT.
        &["dep1.proto".to_string(), "kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    // Reference to the generated type must NOT warn.
    assert!(
        !warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::ExcludedPackageFieldRef { type_fqn, .. }
                if type_fqn == ".dep.TypeFromDep1"
        )),
        "must not warn for a type that IS being generated: {warnings:?}"
    );

    // Reference to the excluded type from the same package MUST warn.
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::ExcludedPackageFieldRef { type_fqn, .. }
                if type_fqn == ".dep.TypeFromDep2"
        )),
        "must warn for a type that is NOT being generated (partial package): {warnings:?}"
    );
}

// ── Map and oneof coverage ────────────────────────────────────────────────────

#[test]
fn test_excluded_package_ref_in_map_value() {
    // map<string, .dep.Money> prices = 1;
    // The map field's own type_name points at the synthetic PricesEntry (same
    // package, in declared_in_kept), so the plain field check would pass silently.
    // The fix looks through the entry to the value field.
    let dep = dep_file_with_message("dep.proto", "dep", "Money");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());

    let map_entry = DescriptorProto {
        name: Some("PricesEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            field_referencing("value", 2, ".dep.Money"),
        ],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    kept.message_type.push(DescriptorProto {
        name: Some("Order".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("prices".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".svc.Order.PricesEntry".to_string()),
            ..Default::default()
        }],
        nested_type: vec![map_entry],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    match warnings
        .iter()
        .find(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. }))
        .expect("must warn for excluded type in map value field")
    {
        CodeGenWarning::ExcludedPackageFieldRef {
            file_name,
            message_name,
            field_name,
            ref_package,
            type_fqn,
            ..
        } => {
            assert_eq!(file_name, "kept.proto");
            assert_eq!(message_name, "Order");
            // Warning is attributed to the outer map field, not "value" inside the entry.
            assert_eq!(field_name, "prices");
            assert_eq!(ref_package, "dep");
            assert_eq!(type_fqn, ".dep.Money");
        }
        other => panic!("unexpected warning variant: {other:?}"),
    }
}

#[test]
fn test_no_warn_map_value_when_generated() {
    // map<string, .dep.Money> where dep.proto IS in files_to_generate — no warning.
    let dep = dep_file_with_message("dep.proto", "dep", "Money");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());

    let map_entry = DescriptorProto {
        name: Some("PricesEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            field_referencing("value", 2, ".dep.Money"),
        ],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    kept.message_type.push(DescriptorProto {
        name: Some("Order".to_string()),
        field: vec![FieldDescriptorProto {
            name: Some("prices".to_string()),
            number: Some(1),
            label: Some(Label::LABEL_REPEATED),
            r#type: Some(Type::TYPE_MESSAGE),
            type_name: Some(".svc.Order.PricesEntry".to_string()),
            ..Default::default()
        }],
        nested_type: vec![map_entry],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["dep.proto".to_string(), "kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, CodeGenWarning::ExcludedPackageFieldRef { .. })),
        "must not warn when the map value type is being generated: {warnings:?}"
    );
}

#[test]
fn test_map_entry_guard_prevents_suffix_collision() {
    // Regression for the find_map_entry suffix-collision false negative:
    // `dep.proto` exports a message named `PricesEntry` (same simple name as the
    // synthetic map-entry `Order.PricesEntry`). A field `dep.PricesEntry legacy = 2`
    // is NOT a map field (label=OPTIONAL, type=MESSAGE), so `find_map_entry` must
    // not match it against the synthetic entry. Without the label/type guard, the
    // imported-type field would be silently skipped instead of warned about.
    let dep = dep_file_with_message("dep.proto", "dep", "PricesEntry");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());

    let map_entry = DescriptorProto {
        name: Some("PricesEntry".to_string()),
        field: vec![
            make_field("key", 1, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
            make_field("value", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        options: (MessageOptions {
            map_entry: Some(true),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    kept.message_type.push(DescriptorProto {
        name: Some("Order".to_string()),
        field: vec![
            // Map field — synthetic entry, same package, must NOT trigger warning.
            FieldDescriptorProto {
                name: Some("prices".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_REPEATED),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".svc.Order.PricesEntry".to_string()),
                ..Default::default()
            },
            // Non-map field whose type_name ends with ".PricesEntry" — same simple
            // name as the synthetic entry. Must warn because `dep.PricesEntry` is
            // from an excluded package.
            field_referencing("legacy", 2, ".dep.PricesEntry"),
        ],
        nested_type: vec![map_entry],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::ExcludedPackageFieldRef { type_fqn, field_name, .. }
                if type_fqn == ".dep.PricesEntry" && field_name == "legacy"
        )),
        "must warn for excluded imported type even when its simple name \
         collides with a synthetic map-entry: {warnings:?}"
    );
}

#[test]
fn test_excluded_package_ref_in_oneof() {
    // oneof body { .dep.Constraint constraint = 1; string note = 2; }
    // Oneof members are regular fields in the descriptor and should be caught
    // by the existing per-field check.
    let dep = dep_file_with_message("dep.proto", "dep", "Constraint");
    let mut kept = proto3_file("kept.proto");
    kept.package = Some("svc".to_string());
    kept.message_type.push(DescriptorProto {
        name: Some("Request".to_string()),
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("body".to_string()),
            ..Default::default()
        }],
        field: vec![
            FieldDescriptorProto {
                name: Some("constraint".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_MESSAGE),
                type_name: Some(".dep.Constraint".to_string()),
                oneof_index: Some(0),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("note".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                oneof_index: Some(0),
                ..Default::default()
            },
        ],
        ..Default::default()
    });

    let (_, warnings) = generate_with_diagnostics(
        &[dep, kept],
        &["kept.proto".to_string()],
        &CodeGenConfig::default(),
    )
    .unwrap();

    assert!(
        warnings.iter().any(|w| matches!(
            w,
            CodeGenWarning::ExcludedPackageFieldRef { type_fqn, field_name, .. }
                if type_fqn == ".dep.Constraint" && field_name == "constraint"
        )),
        "must warn for an excluded type referenced from a oneof variant: {warnings:?}"
    );
}

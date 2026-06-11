//! `gate_impls_on_crate_features` end-to-end output tests.
//!
//! These verify that the *shape* of the generated code changes when gating
//! is on — `#[cfg(feature = "...")]` / `#[cfg_attr(...)]` wrappers appear
//! around the right items. The opt-out path (gating off, the default) is
//! covered by every other test in this suite, which would fail if the
//! gating refactor changed the unconditional output.

use super::*;

/// Collapse all whitespace to single spaces so assertions are robust to
/// prettyplease line-wrapping decisions, which can change across rustfmt /
/// prettyplease version bumps without changing the token sequence.
fn squash(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A proto3 file with a message exercising every gated surface: scalar
/// fields, an enum, a oneof, a nested message, and an `extend` block.
fn fixture() -> FileDescriptorProto {
    let mut file = proto3_file("gated.proto");
    file.package = Some("pkg".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Color".to_string()),
        value: vec![enum_value("RED", 0), enum_value("BLUE", 1)],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![
            FieldDescriptorProto {
                name: Some("name".to_string()),
                number: Some(1),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_STRING),
                json_name: Some("name".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("color".to_string()),
                number: Some(2),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_ENUM),
                type_name: Some(".pkg.Color".to_string()),
                json_name: Some("color".to_string()),
                ..Default::default()
            },
            FieldDescriptorProto {
                name: Some("which".to_string()),
                number: Some(3),
                label: Some(Label::LABEL_OPTIONAL),
                r#type: Some(Type::TYPE_INT32),
                oneof_index: Some(0),
                json_name: Some("which".to_string()),
                ..Default::default()
            },
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("kind".to_string()),
            ..Default::default()
        }],
        nested_type: vec![DescriptorProto {
            name: Some("Inner".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    // A message with `extensions N to M;` so the `__OuterExtJson` wrapper
    // and its serde impls are exercised.
    file.message_type.push(DescriptorProto {
        name: Some("Extendable".to_string()),
        extension_range: vec![
            crate::generated::descriptor::descriptor_proto::ExtensionRange {
                start: Some(100),
                end: Some(200),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    file
}

fn generate_gated(generate_text: bool) -> String {
    let cfg = CodeGenConfig {
        generate_json: true,
        generate_views: true,
        generate_text,
        preserve_unknown_fields: true,
        gate_impls_on_crate_features: true,
        ..CodeGenConfig::default()
    };
    let files =
        generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
    joined(&files)
}

fn generate_ungated() -> String {
    let cfg = CodeGenConfig {
        generate_json: true,
        generate_views: true,
        generate_text: true,
        preserve_unknown_fields: true,
        gate_impls_on_crate_features: false,
        ..CodeGenConfig::default()
    };
    let files =
        generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
    joined(&files)
}

#[test]
fn ungated_output_has_no_feature_cfgs() {
    let content = generate_ungated();
    // The ungated path is the default and must not introduce any
    // `cfg(feature = "json"|"views"|"text")` — only the pre-existing
    // `arbitrary` cfg_attr (which is *always* gated) is permitted.
    assert!(
        !content.contains(r#"feature = "json""#),
        "ungated output must not gate on the json feature: {content}"
    );
    assert!(
        !content.contains(r#"feature = "views""#),
        "ungated output must not gate on the views feature: {content}"
    );
    assert!(
        !content.contains(r#"feature = "text""#),
        "ungated output must not gate on the text feature: {content}"
    );
}

#[test]
fn custom_feature_names_replace_defaults_in_output() {
    let cfg = CodeGenConfig {
        generate_json: true,
        generate_views: true,
        generate_text: true,
        preserve_unknown_fields: true,
        gate_impls_on_crate_features: true,
        feature_gate_names: crate::FeatureGateNames {
            json: "serde".to_string(),
            views: "zero-copy".to_string(),
            text: "textproto".to_string(),
            ..crate::FeatureGateNames::default()
        },
        ..CodeGenConfig::default()
    };
    let files =
        generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
    let content = joined(&files);
    // Every gate uses the custom name...
    assert!(
        content.contains(r#"feature = "serde""#),
        "json gates must use the custom name: {content}"
    );
    assert!(
        content.contains(r#"feature = "zero-copy""#),
        "views gates must use the custom name: {content}"
    );
    assert!(
        content.contains(r#"feature = "textproto""#),
        "text gates must use the custom name: {content}"
    );
    // ...and the default names are gone entirely.
    assert!(
        !content.contains(r#"feature = "json""#),
        "no gate may use the default json name: {content}"
    );
    assert!(
        !content.contains(r#"feature = "views""#),
        "no gate may use the default views name: {content}"
    );
    assert!(
        !content.contains(r#"feature = "text""#),
        "no gate may use the default text name: {content}"
    );
}

#[test]
fn shared_custom_name_squashes_json_or_text_gate() {
    // json and text gated behind one feature → `register_types` (the
    // either-json-or-text surface) gets a single `feature = "serde"` gate,
    // never `any(feature = "serde", feature = "serde")`.
    let cfg = CodeGenConfig {
        generate_json: true,
        generate_text: true,
        gate_impls_on_crate_features: true,
        feature_gate_names: crate::FeatureGateNames {
            json: "serde".to_string(),
            text: "serde".to_string(),
            ..crate::FeatureGateNames::default()
        },
        ..CodeGenConfig::default()
    };
    let files =
        generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
    let content = joined(&files);
    let squashed = squash(&content);
    assert!(
        squashed.contains(r#"feature = "serde""#),
        "gates must use the shared custom name: {content}"
    );
    assert!(
        !squashed.contains("any(feature"),
        "a shared name must collapse to a single cfg predicate, not any(): {content}"
    );
}

#[test]
fn invalid_feature_name_is_a_generate_error() {
    // An active gate with a name that can't be a Cargo feature must fail
    // generation outright — the emitted `#[cfg]` would be permanently
    // false and the gated impls would silently compile away.
    let cfg = CodeGenConfig {
        generate_json: true,
        gate_impls_on_crate_features: true,
        feature_gate_names: crate::FeatureGateNames {
            json: "not a feature".to_string(),
            ..crate::FeatureGateNames::default()
        },
        ..CodeGenConfig::default()
    };
    let err = generate(&[fixture()], &["gated.proto".to_string()], &cfg)
        .expect_err("invalid feature name must fail generation");
    let msg = err.to_string();
    assert!(
        msg.contains("json") && msg.contains("not a feature"),
        "error must name the gate kind and the offending name: {msg}"
    );

    // The same name is inert — and therefore accepted — without gating.
    let ungated = CodeGenConfig {
        gate_impls_on_crate_features: false,
        ..cfg
    };
    generate(&[fixture()], &["gated.proto".to_string()], &ungated)
        .expect("invalid name without gating is inert and must not error");
}

#[test]
fn gated_message_serde_derive_is_cfg_attr() {
    let content = generate_gated(false);
    assert!(
        content.contains(r#"#[cfg_attr(feature = "json", derive(::serde::Serialize"#),
        "struct serde derive must be cfg_attr-gated: {content}"
    );
    assert!(
        content.contains(r#"#[cfg_attr(feature = "json", serde(default))]"#),
        "struct serde(default) must be cfg_attr-gated: {content}"
    );
    // Field-level serde attrs must also be cfg_attr-gated, otherwise they
    // are unrecognised when the derive is gated off. prettyplease may
    // line-wrap the cfg_attr body, so squash whitespace before matching.
    let squashed = squash(&content);
    assert!(
        squashed.contains(r#"#[cfg_attr( feature = "json", serde( rename"#),
        "field serde attrs must be cfg_attr-gated: {content}"
    );
    assert!(
        content.contains(r#"#[cfg_attr(feature = "json", serde(flatten))]"#),
        "oneof serde(flatten) must be cfg_attr-gated: {content}"
    );
    assert!(
        content.contains(r#"#[cfg_attr(feature = "json", serde(skip))]"#),
        "unknown-fields serde(skip) must be cfg_attr-gated: {content}"
    );
    // None of the above may appear ungated. `#[serde(...)]` only appears
    // for the ungated form — the gated form is `#[cfg_attr(..., serde(...))]`.
    assert!(
        !content.contains(r#"#[serde("#),
        "no serde attribute may be emitted ungated when gating is on: {content}"
    );
    assert!(
        !content.contains(r#"#[derive(::serde::"#),
        "no serde derive may be emitted ungated when gating is on: {content}"
    );
}

#[test]
fn gated_enum_serde_impls_are_cfg_blocked() {
    let content = generate_gated(false);
    // The enum gets a custom Serialize/Deserialize/ProtoElemJson impl set
    // wrapped in a `#[cfg(feature = "json")] const _: () = { ... };` block
    // (one outer cfg on the anonymous const covers all sibling impls).
    assert!(
        content.contains(r#"#[cfg(feature = "json")]"#),
        "enum serde impls must be cfg-gated: {content}"
    );
    // The enum's `Enumeration` impl is unconditional (binary codec needs it).
    assert!(
        content.contains("impl ::buffa::Enumeration for Color"),
        "Enumeration impl must remain unconditional: {content}"
    );
}

#[test]
fn gated_view_module_is_cfg_blocked() {
    let content = generate_gated(false);
    // The whole `__buffa::view` module is wrapped, not each impl.
    assert!(
        content.contains(r#"#[cfg(feature = "views")]"#),
        "view module must be cfg-gated: {content}"
    );
    let squashed = squash(&content);
    assert!(
        squashed.contains(r#"#[cfg(feature = "views")] pub mod view"#),
        "the views cfg must precede `pub mod view`: {content}"
    );
    // The view's `impl MessageName for FooView<'a>` lives in the
    // `<stem>.__view.rs` content file, which the stitcher `include!`s
    // *inside* the `#[cfg(feature = "views")] pub mod view` block, so
    // it disappears with the feature. The owned-message impl is in the
    // `<stem>.rs` content file (unconditional). Pin both: a refactor
    // that emits the view impl into a non-View content file would ship
    // a name-resolution error against a cfg'd-out type.
    let cfg = CodeGenConfig {
        generate_json: true,
        generate_views: true,
        generate_text: false,
        preserve_unknown_fields: true,
        gate_impls_on_crate_features: true,
        ..CodeGenConfig::default()
    };
    let files =
        generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
    for f in &files {
        let has_view_impl = f.content.contains("MessageName for OuterView");
        let has_owned_impl = squash(&f.content).contains("impl ::buffa::MessageName for Outer ");
        match f.kind {
            GeneratedFileKind::View => assert!(
                has_view_impl,
                "view MessageName impl must be in the view content file ({})",
                f.name
            ),
            GeneratedFileKind::Owned => assert!(
                has_owned_impl && !has_view_impl,
                "owned content file must have the owned (not view) MessageName impl ({})",
                f.name
            ),
            _ => assert!(
                !has_view_impl && !has_owned_impl,
                "MessageName impl leaked into a {:?} file ({})",
                f.kind,
                f.name
            ),
        }
    }
}

#[test]
fn gated_view_reexports_are_cfg_blocked() {
    // Every natural-path `pub use ...::view::FooView;` re-export must carry
    // the `views` gate — top-level message views (emitted in `lib.rs`),
    // nested-message views, and view-oneof enums (both in `message.rs`). A
    // missed gate is a silent name-resolution bug: the re-export survives
    // when the `view` module it points at has been cfg'd out, and the
    // compiler reports `could not find view in __buffa`.
    let content = generate_gated(false);
    let squashed = squash(&content);
    // `pub use ...::__buffa::view::...` re-exports live *outside* the gated
    // `view` module, so they need their own gate. References *inside* the
    // gated `view` module (cross-message view-oneof types, etc.) are
    // covered by the module gate and don't count here.
    let re_exports: Vec<&str> = squashed
        .match_indices("pub use ")
        .map(|(i, _)| &squashed[i..(i + 80).min(squashed.len())])
        .filter(|s| s.contains("__buffa::view::"))
        .collect();
    assert!(
        !re_exports.is_empty(),
        "fixture must produce at least one view re-export: {content}"
    );
    // Each must be immediately preceded by `#[cfg(feature = "views")]`
    // (with an optional `#[doc(inline)]` between the cfg and the `pub use`).
    for re in &re_exports {
        let prefix = format!(
            r#"#[cfg(feature = "views")] #[doc(inline)] {}"#,
            &re[..re.find('`').unwrap_or(re.len()).min(40)]
        );
        let alt = format!(r#"#[cfg(feature = "views")] {}"#, &re[..40.min(re.len())]);
        assert!(
            squashed.contains(&prefix) || squashed.contains(&alt),
            "view re-export `{re}` must be gated on `feature = \"views\"`: {content}"
        );
    }
}

#[test]
fn gated_text_impl_is_cfg_blocked() {
    let content = generate_gated(true);
    assert!(
        content.contains(r#"#[cfg(feature = "text")]"#),
        "TextFormat impl must be cfg-gated: {content}"
    );
    // The owned `Message` impl (binary codec) is unconditional.
    assert!(
        content.contains("impl ::buffa::Message for Outer"),
        "Message impl must remain unconditional: {content}"
    );
}

#[test]
fn gated_register_types_statements_are_cfg_blocked() {
    let content = generate_gated(true);
    // The `register_types` fn body has per-statement cfgs.
    let squashed = squash(&content);
    assert!(
        squashed.contains(r#"#[cfg(feature = "json")] reg.register_json_any"#),
        "register_json_any statement must be cfg-gated: {content}"
    );
    assert!(
        squashed.contains(r#"#[cfg(feature = "text")] reg.register_text_any"#),
        "register_text_any statement must be cfg-gated: {content}"
    );
    // The fn (and its package-root re-export) are gated on
    // `any(json, text)` — they're useless without at least one entry, and
    // `::buffa::type_registry::TypeRegistry` may itself become
    // feature-gated in a future runtime release. Both `register_types`
    // occurrences (the `pub fn` and the `pub use`) must carry the gate.
    let any_gate = r#"#[cfg(any(feature = "json", feature = "text"))]"#;
    assert_eq!(
        squashed.matches("register_types").count(),
        squashed.matches(any_gate).count(),
        "every register_types occurrence must carry an any(json, text) gate: {content}"
    );
    assert!(
        squashed.contains(any_gate),
        "register_types must be gated on any(json, text): {content}"
    );
    // `#[allow(unused_variables)]` is on the fn defensively for the
    // partial-feature case (json on, text off → text statements cfg'd out).
    assert!(
        content.contains("#[allow(unused_variables)]"),
        "register_types must allow unused `reg` for partial-feature builds: {content}"
    );
}

#[test]
fn gated_register_types_with_json_only() {
    // `generate_text = false` → only json entries → the fn is gated on a
    // single `feature = "json"` (no `any(...)`).
    let content = generate_gated(false);
    let squashed = squash(&content);
    assert!(
        !squashed.contains(r#"any(feature"#),
        "json-only register_types must be gated on a single feature, not any(): {content}"
    );
}

#[test]
fn gated_ext_json_wrapper_struct_is_unconditional_but_serde_impls_are_gated() {
    let content = generate_gated(false);
    // The `__ExtendableExtJson` wrapper struct (and its
    // Deref/DerefMut/From) are always present — encode/decode reach the
    // inner `UnknownFields` through `DerefMut`.
    assert!(
        content.contains("pub struct __ExtendableExtJson"),
        "ext-json wrapper struct must be unconditional: {content}"
    );
    assert!(
        content.contains("impl ::core::ops::Deref for __ExtendableExtJson"),
        "ext-json wrapper Deref must be unconditional: {content}"
    );
    // The Serialize / Deserialize impls reach into `extension_registry`,
    // which is `buffa/json`-only, so they're gated. Squash whitespace so
    // the assertion is robust to prettyplease line-wrapping.
    let squashed = squash(&content);
    assert!(
        squashed.contains(
            r#"#[cfg(feature = "json")] impl ::serde::Serialize for __ExtendableExtJson"#
        ),
        "ext-json wrapper Serialize must be cfg-gated: {content}"
    );
    assert!(
        squashed.contains(
            r#"#[cfg(feature = "json")] impl<'de> ::serde::Deserialize<'de> for __ExtendableExtJson"#
        ),
        "ext-json wrapper Deserialize must be cfg-gated: {content}"
    );
}

#[test]
fn gated_only_when_kind_is_enabled() {
    // `gate_impls_on_crate_features = true` does not change *whether* a
    // kind is emitted, only how. Disable text and verify nothing in the
    // output references the `text` feature.
    let content = generate_gated(false);
    assert!(
        !content.contains(r#"feature = "text""#),
        "text feature must not appear when generate_text is off: {content}"
    );
    assert!(
        !content.contains("TextFormat"),
        "no TextFormat impls when generate_text is off: {content}"
    );
}

#[test]
fn gating_with_json_disabled_emits_no_json_gates_or_impls() {
    // `gate_impls_on_crate_features` applies a gate to *enabled* impl
    // kinds; a kind that's off (`generate_json = false`) is simply not
    // emitted — no impl, no `cfg(feature = "json")`, exactly like the
    // ungated path with json off. Views are still on and gated.
    let cfg = CodeGenConfig {
        generate_json: false,
        generate_views: true,
        generate_text: false,
        preserve_unknown_fields: true,
        gate_impls_on_crate_features: true,
        ..CodeGenConfig::default()
    };
    let files =
        generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
    let content = joined(&files);
    assert!(
        !content.contains(r#"feature = "json""#),
        "json feature gate must not appear when generate_json is off: {content}"
    );
    assert!(
        !content.contains("::serde::"),
        "no serde impls when generate_json is off: {content}"
    );
    assert!(
        content.contains(r#"#[cfg(feature = "views")]"#),
        "views must still be gated when on: {content}"
    );
}

#[test]
fn gated_output_parses_as_valid_rust() {
    // A coarse compile-shape check: the gated output is still valid Rust
    // syntax. This catches structural cfg/cfg_attr misuse (a `#[cfg]` on a
    // trailing expression, a `cfg_attr` body that doesn't form an
    // attribute, etc.) without needing a full compile harness. Type
    // resolution and feature-combination compile coverage are deferred to
    // the `buffa-descriptor`/`buffa-types` regen PR (#113).
    for generate_text in [false, true] {
        let cfg = CodeGenConfig {
            generate_json: true,
            generate_views: true,
            generate_text,
            preserve_unknown_fields: true,
            gate_impls_on_crate_features: true,
            ..CodeGenConfig::default()
        };
        let files =
            generate(&[fixture()], &["gated.proto".to_string()], &cfg).expect("should generate");
        for f in &files {
            syn::parse_file(&f.content).unwrap_or_else(|e| {
                panic!(
                    "gated output for {} (text={generate_text}) is not valid Rust: {e}\n{}",
                    f.name, f.content
                )
            });
        }
    }
}

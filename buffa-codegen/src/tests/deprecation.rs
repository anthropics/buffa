//! `[deprecated = true]`: generated declarations carry `#[deprecated]` the way
//! prost-build emits it (fields and enum values), and the generated items that
//! must visit every field carry `#[allow(deprecated)]` so a deprecated member
//! does not flood the build with warnings from the generated code itself.

use super::*;
use crate::generated::descriptor::{EnumValueOptions, FieldOptions, MessageOptions};

/// Strip all whitespace so assertions are immune to prettyplease line wrapping.
fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn deprecated(mut field: FieldDescriptorProto) -> FieldDescriptorProto {
    field.options = FieldOptions {
        deprecated: Some(true),
        ..Default::default()
    }
    .into();
    field
}

fn deprecated_value(mut value: EnumValueDescriptorProto) -> EnumValueDescriptorProto {
    value.options = EnumValueOptions {
        deprecated: Some(true),
        ..Default::default()
    }
    .into();
    value
}

/// `Widget` with one deprecated and one live field, plus a `Size` enum whose
/// `SMALL` value is deprecated and aliased by `TINY`.
fn deprecated_file() -> FileDescriptorProto {
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Widget".to_string()),
        field: vec![
            deprecated(make_field(
                "legacy_name",
                1,
                Label::LABEL_OPTIONAL,
                Type::TYPE_STRING,
            )),
            make_field("label", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    });
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![
            enum_value("SIZE_UNSPECIFIED", 0),
            deprecated_value(enum_value("SMALL", 1)),
            deprecated_value(enum_value("TINY", 1)),
        ],
        ..Default::default()
    });
    file
}

/// A file with no deprecation anywhere — its output must be untouched.
fn clean_file() -> FileDescriptorProto {
    let mut file = proto3_file("clean.proto");
    file.package = Some("deprecate.test".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Widget".to_string()),
        field: vec![make_field(
            "legacy_name",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        ..Default::default()
    });
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![enum_value("SIZE_UNSPECIFIED", 0), enum_value("SMALL", 1)],
        ..Default::default()
    });
    file
}

fn generate_squashed(file: FileDescriptorProto, config: &CodeGenConfig) -> String {
    let name = file.name.clone().unwrap_or_default();
    let files = generate(&[file], &[name], config).expect("should generate");
    squash(&joined(&files))
}

// ── field markers ───────────────────────────────────────────────────

#[test]
fn deprecated_field_carries_marker() {
    let content = generate_squashed(deprecated_file(), &CodeGenConfig::default());
    assert!(
        content.contains(r#"#[deprecated]publegacy_name:"#),
        "deprecated field must carry #[deprecated]: {content}"
    );
    assert!(
        !content.contains(r#"#[deprecated]publabel:"#),
        "a live field must not carry the marker"
    );
}

#[test]
fn codec_and_debug_impls_allow_deprecated() {
    let config = CodeGenConfig {
        generate_text: true,
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    for impl_head in [
        "impl::buffa::MessageforWidget",
        "impl::core::fmt::DebugforWidget",
        "impl::buffa::text::TextFormatforWidget",
    ] {
        assert!(
            content.contains(&format!("#[allow(deprecated)]{impl_head}")),
            "{impl_head} visits every field, so it must be guarded: {content}"
        );
    }
}

#[test]
fn view_to_owned_allows_deprecated() {
    let content = generate_squashed(deprecated_file(), &CodeGenConfig::default());
    assert!(
        content.contains("#[allow(deprecated)]fnto_owned_from_source"),
        "the view's to_owned writes the owned struct's deprecated field: {content}"
    );
}

#[test]
fn setters_allow_deprecated() {
    // `with_*` setters exist only for explicit-presence fields, so the
    // deprecated field has to be `optional`.
    let mut file = deprecated_file();
    let msg = &mut file.message_type[0];
    msg.field[0].proto3_optional = Some(true);
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content.contains("#[allow(deprecated)]implWidget{"),
        "the with_* setter assigns the deprecated field: {content}"
    );
}

#[test]
fn reflection_impl_allows_deprecated() {
    let config = CodeGenConfig {
        generate_reflection: true,
        generate_reflection_vtable: true,
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    assert!(
        content.contains(
            "#[allow(deprecated)]impl::buffa_descriptor::reflect::ReflectMessageforWidget"
        ),
        "the vtable reads fields by index: {content}"
    );
}

#[test]
fn json_deserialize_impl_allows_deprecated() {
    // The hand-written Deserialize impl is emitted when a oneof forces it.
    let mut msg = DescriptorProto {
        name: Some("Widget".to_string()),
        field: vec![
            deprecated(make_field(
                "legacy_name",
                1,
                Label::LABEL_OPTIONAL,
                Type::TYPE_STRING,
            )),
            make_field("label", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    };
    let mut in_oneof = make_field("alias", 3, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    in_oneof.oneof_index = Some(0);
    msg.field.push(in_oneof);
    msg.oneof_decl.push(OneofDescriptorProto {
        name: Some("name_choice".to_string()),
        ..Default::default()
    });
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.message_type.push(msg);

    let config = CodeGenConfig {
        generate_json: true,
        ..Default::default()
    };
    let content = generate_squashed(file, &config);
    assert!(
        content.contains("#[allow(deprecated)]impl<'de>::serde::Deserialize<'de>forWidget"),
        "the visitor assigns the deprecated field: {content}"
    );
}

// ── enum value markers ──────────────────────────────────────────────

#[test]
fn deprecated_enum_value_carries_marker_on_variant_and_alias() {
    let content = generate_squashed(deprecated_file(), &CodeGenConfig::default());
    assert!(
        content.contains("#[deprecated]SMALL=1"),
        "deprecated enum value must mark its variant: {content}"
    );
    assert!(
        content
            .contains("#[deprecated]#[allow(non_upper_case_globals)]pubconstTINY:Self=Self::SMALL"),
        "an alias of a deprecated value is deprecated too: {content}"
    );
    assert!(
        !content.contains("#[deprecated]SIZE_UNSPECIFIED=0"),
        "a live value must not carry the marker"
    );
}

#[test]
fn idiomatic_alias_inherits_deprecation() {
    let content = generate_squashed(deprecated_file(), &CodeGenConfig::default());
    assert!(
        content.contains(
            "#[deprecated]#[allow(non_upper_case_globals)]pubconstSmall:Self=Self::SMALL"
        ),
        "the CamelCase alias must not be a quiet way to reach a deprecated value: {content}"
    );
}

#[test]
fn enum_impls_that_name_variants_allow_deprecated() {
    let content = generate_squashed(deprecated_file(), &CodeGenConfig::default());
    for impl_head in [
        // `values()` lists both variants, `from_i32`/`proto_name` name them.
        "impl::buffa::EnumerationforSize",
        // The `allow_alias` const and the idiomatic consts reference `SMALL`.
        "implSize{",
    ] {
        assert!(
            content.contains(&format!("#[allow(deprecated)]{impl_head}")),
            "{impl_head} names a deprecated variant, so it must be guarded: {content}"
        );
    }
    // `Default` returns only the first value, which is live here, so it must
    // not be guarded — the guard tracks what an item actually names.
    assert!(
        !content.contains("#[allow(deprecated)]impl::core::default::DefaultforSize"),
        "Default names SIZE_UNSPECIFIED only: {content}"
    );
}

#[test]
fn first_value_deprecation_does_not_change_default() {
    // The declared default stays the first value even when it is deprecated —
    // the guard is what keeps that legal without warnings.
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![
            deprecated_value(enum_value("SIZE_UNSPECIFIED", 0)),
            enum_value("SMALL", 1),
        ],
        ..Default::default()
    });
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content.contains("#[allow(deprecated)]impl::core::default::DefaultforSize{fndefault()"),
        "Default returns the deprecated first value: {content}"
    );
}

// ── guard reach: generated surfaces that must stay quiet ────────────

/// A proto2 file whose only deprecation is an enum value named by a field's
/// `[default = …]`: the message has no deprecated field, yet the generated
/// default expression spells out `Size::OLD`. `required` is the shape that
/// carries the default into generated code (an `optional` field is `None` until
/// set, so its proto default never appears as an expression).
fn enum_default_file() -> FileDescriptorProto {
    let mut field = make_field("size", 1, Label::LABEL_REQUIRED, Type::TYPE_ENUM);
    field.type_name = Some(".deprecate.test.Size".to_string());
    field.default_value = Some("OLD".to_string());

    let mut file = proto3_file("enum_default.proto");
    file.syntax = Some("proto2".to_string());
    file.package = Some("deprecate.test".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Holder".to_string()),
        field: vec![field],
        ..Default::default()
    });
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![deprecated_value(enum_value("OLD", 1)), enum_value("NEW", 2)],
        ..Default::default()
    });
    file
}

#[test]
fn enum_default_naming_deprecated_value_guards_codec() {
    let content = generate_squashed(enum_default_file(), &CodeGenConfig::default());
    assert!(
        content.contains("#[allow(deprecated)]impl::buffa::MessageforHolder"),
        "the codec's clear/default spells out Size::OLD, so it must be guarded: {content}"
    );
    assert!(
        content.contains("Size::OLD"),
        "fixture sanity: the default must reach the generated code: {content}"
    );
    assert!(
        content.contains("#[allow(deprecated)]impl::core::default::DefaultforHolder"),
        "the custom Default spells out Size::OLD: {content}"
    );
    // The field itself is live, so it must not be marked.
    assert!(
        !content.contains("#[deprecated]pubsize:"),
        "a live field must not carry the marker: {content}"
    );
}

#[test]
fn enum_default_naming_alias_of_deprecated_value_guards_codec() {
    // `SMALL` carries no option of its own; it is deprecated as an alias of
    // `OLD`, and the default expression names it.
    let mut file = enum_default_file();
    file.enum_type[0].value.insert(1, enum_value("SMALL", 1));
    file.message_type[0].field[0].default_value = Some("SMALL".to_string());
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content
            .contains("#[deprecated]#[allow(non_upper_case_globals)]pubconstSMALL:Self=Self::OLD"),
        "fixture sanity: the alias inherits the marker: {content}"
    );
    assert!(
        content.contains("Size::SMALL"),
        "fixture sanity: the default must reach the generated code: {content}"
    );
    assert!(
        content.contains("#[allow(deprecated)]impl::buffa::MessageforHolder"),
        "the codec spells out Size::SMALL, so it must be guarded: {content}"
    );
    assert!(
        content.contains("#[allow(deprecated)]impl::core::default::DefaultforHolder"),
        "the custom Default spells out Size::SMALL: {content}"
    );
}

#[test]
fn arbitrary_keeps_the_derive_when_only_an_alias_is_deprecated() {
    // The derive names variants only, and every variant here is live.
    let config = CodeGenConfig {
        generate_arbitrary: true,
        ..Default::default()
    };
    let mut file = enum_default_file();
    file.enum_type[0].value = vec![
        enum_value("BIG", 1),
        deprecated_value(enum_value("LARGE", 1)),
        enum_value("NEW", 2),
    ];
    file.message_type[0].field[0].default_value = Some("NEW".to_string());
    let content = generate_squashed(file, &config);
    assert!(
        !content.contains("impl<'a>::arbitrary::Arbitrary<'a>forSize"),
        "no variant is deprecated, so the derive stays: {content}"
    );
    assert_eq!(
        content
            .matches("#[cfg_attr(feature=\"arbitrary\",derive(::arbitrary::Arbitrary))]")
            .count(),
        2,
        "`Holder` and `Size` both derive: {content}"
    );
}

#[test]
fn idiomatic_const_of_an_alias_inherits_the_variant_marker() {
    // `SIZE_SMALL` has no option of its own; it is an alias of the deprecated
    // variant `SIZE_OLD`, so `Size::Small` is marked as `Size::SIZE_SMALL` is.
    let config = CodeGenConfig {
        idiomatic_enum_aliases: true,
        ..Default::default()
    };
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![
            deprecated_value(enum_value("SIZE_OLD", 0)),
            enum_value("SIZE_SMALL", 0),
        ],
        ..Default::default()
    });
    let content = generate_squashed(file, &config);
    assert!(
        content.contains(
            "#[deprecated]#[allow(non_upper_case_globals)]pubconstSmall:Self=Self::SIZE_OLD"
        ),
        "the idiomatic const of an inheriting alias carries the marker: {content}"
    );
}

#[test]
fn idiomatic_const_of_a_deprecated_alias_is_deprecated() {
    // `SIZE_LARGE` is deprecated by its own option while its variant
    // `SIZE_BIG` is live; `Size::Large` names the alias, so it is marked, and
    // `Size::Big` is not.
    let config = CodeGenConfig {
        idiomatic_enum_aliases: true,
        ..Default::default()
    };
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![
            enum_value("SIZE_BIG", 0),
            deprecated_value(enum_value("SIZE_LARGE", 0)),
        ],
        ..Default::default()
    });
    let content = generate_squashed(file, &config);
    assert!(
        content.contains(
            "#[deprecated]#[allow(non_upper_case_globals)]pubconstLarge:Self=Self::SIZE_BIG"
        ),
        "the idiomatic const of the deprecated alias carries the marker: {content}"
    );
    assert!(
        !content.contains(
            "#[deprecated]#[allow(non_upper_case_globals)]pubconstBig:Self=Self::SIZE_BIG"
        ),
        "the idiomatic const of the live variant stays unmarked: {content}"
    );
}

#[test]
fn enum_default_naming_a_deprecated_alias_of_a_live_variant_guards_codec() {
    let mut file = enum_default_file();
    file.enum_type[0].value = vec![
        enum_value("BIG", 1),
        deprecated_value(enum_value("LARGE", 1)),
        enum_value("NEW", 2),
    ];
    file.message_type[0].field[0].default_value = Some("LARGE".to_string());
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content.contains("#[allow(deprecated)]impl::buffa::MessageforHolder"),
        "the default names the deprecated alias const: {content}"
    );
}

#[test]
fn enum_default_naming_a_live_alias_beside_a_deprecated_one_is_unguarded() {
    // `C` inherits from the variant `A`, which is live; the deprecated alias
    // `B` between them does not pass its marker on.
    let mut file = enum_default_file();
    file.enum_type[0].value = vec![
        enum_value("A", 1),
        deprecated_value(enum_value("B", 1)),
        enum_value("C", 1),
    ];
    file.message_type[0].field[0].default_value = Some("C".to_string());
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content.contains("Size::C"),
        "fixture sanity: the default must reach the generated code: {content}"
    );
    assert!(
        !content.contains("#[allow(deprecated)]impl::buffa::MessageforHolder"),
        "`C` is live, so the default needs no guard: {content}"
    );
}

#[test]
fn enum_default_naming_live_variant_with_deprecated_alias_is_unguarded() {
    // The marker flows from a variant to its aliases, not back: `BIG` stays
    // live when only its alias `LARGE` is deprecated.
    let mut file = enum_default_file();
    file.enum_type[0].value = vec![
        enum_value("BIG", 1),
        deprecated_value(enum_value("LARGE", 1)),
        enum_value("NEW", 2),
    ];
    file.message_type[0].field[0].default_value = Some("BIG".to_string());
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content.contains("Size::BIG"),
        "fixture sanity: the default must reach the generated code: {content}"
    );
    assert!(
        !content.contains("#[allow(deprecated)]impl::buffa::MessageforHolder"),
        "a default that names a live variant needs no guard: {content}"
    );
}

#[test]
fn arbitrary_for_an_enum_with_a_deprecated_value_is_an_impl_not_a_derive() {
    let config = CodeGenConfig {
        generate_arbitrary: true,
        ..Default::default()
    };
    let mut file = enum_default_file();
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Live".to_string()),
        value: vec![enum_value("LIVE_A", 0), enum_value("LIVE_B", 1)],
        ..Default::default()
    });
    let content = generate_squashed(file, &config);
    // The derive would name `Size::OLD` outside any `#[allow(deprecated)]`.
    assert!(
        content.contains("#[cfg(feature=\"arbitrary\")]impl<'a>::arbitrary::Arbitrary<'a>forSize{"),
        "an enum with a deprecated value implements Arbitrary by hand: {content}"
    );
    assert!(
        content.contains(
            "letvalues=<Selfas::buffa::Enumeration>::values();\
             letdraw=<u32as::arbitrary::Arbitrary>::arbitrary(u)?;\
             letindex=(u64::from(draw)*values.len()asu64)>>32;\
             ::core::result::Result::Ok(values[indexasusize])"
        ),
        "the impl maps a `u32` onto the declared values as the derive does: {content}"
    );
    assert!(
        content.contains("<u32as::arbitrary::Arbitrary>::size_hint(depth)"),
        "the impl reports the derive's size hint: {content}"
    );
    assert!(
        !content.contains("impl<'a>::arbitrary::Arbitrary<'a>forLive"),
        "an enum without one keeps the derive: {content}"
    );
    assert_eq!(
        content
            .matches("#[cfg_attr(feature=\"arbitrary\",derive(::arbitrary::Arbitrary))]")
            .count(),
        2,
        "`Holder` and `Live` derive, `Size` does not: {content}"
    );
}

#[test]
fn lazy_view_to_owned_allows_deprecated() {
    let config = CodeGenConfig {
        lazy_views: true,
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    assert!(
        content.contains("#[allow(deprecated)]fnto_owned_message"),
        "the lazy view's to_owned writes the owned field: {content}"
    );
}

#[test]
fn table_static_allows_deprecated() {
    let config = CodeGenConfig {
        codec_strategy: crate::CodecStrategy::Table,
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    assert!(
        content.contains("#[allow(deprecated)]pub(crate)static__BUFFA_TABLE_Widget"),
        "the table entries take offsets of deprecated fields: {content}"
    );
}

#[test]
fn oneof_only_deprecation_is_out_of_scope() {
    // A deprecated oneof member is neither marked nor guarded by this change;
    // pinning it here makes the follow-up a deliberate widening.
    let mut member = make_field("legacy_choice", 3, Label::LABEL_OPTIONAL, Type::TYPE_STRING);
    member.oneof_index = Some(0);
    member.options = FieldOptions {
        deprecated: Some(true),
        ..Default::default()
    }
    .into();
    let mut msg = DescriptorProto {
        name: Some("Widget".to_string()),
        field: vec![
            member,
            make_field("label", 2, Label::LABEL_OPTIONAL, Type::TYPE_STRING),
        ],
        ..Default::default()
    };
    msg.oneof_decl.push(OneofDescriptorProto {
        name: Some("choice".to_string()),
        ..Default::default()
    });
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.message_type.push(msg);

    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        !content.contains("#[allow(deprecated)]impl::buffa::MessageforWidget"),
        "a oneof-only deprecation must not widen the owned guard: {content}"
    );
}

#[test]
fn view_fields_are_not_marked_yet() {
    // The zero-copy read path (`FooView` fields and `FooOwnedView` accessors)
    // does not carry the marker in this change, so a view read stays silent.
    // Pinned because that is a documented limit, not an oversight.
    let content = generate_squashed(deprecated_file(), &CodeGenConfig::default());
    assert!(
        !content.contains("#[deprecated]publegacy_name:&'astr"),
        "view fields are deliberately unmarked for now: {content}"
    );
}

// ── untouched output ────────────────────────────────────────────────

#[test]
fn file_without_deprecation_emits_no_attributes() {
    let content = generate_squashed(clean_file(), &CodeGenConfig::default());
    assert!(!content.contains("#[deprecated]"), "no option, no marker");
    assert!(
        !content.contains("#[allow(deprecated)]"),
        "guards must not leak into messages that need them: {content}"
    );
}

// ── caller-supplied marker wins ─────────────────────────────────────

#[test]
fn caller_deprecated_attribute_is_not_duplicated() {
    // rustc rejects two `deprecated` attributes on one item, so a build that
    // restored the marker by hand must keep exactly the caller's version.
    let config = CodeGenConfig {
        field_attributes: vec![(
            ".deprecate.test.Widget.legacy_name".to_string(),
            r#"#[deprecated(note = "use label")]"#.to_string(),
        )],
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    let field_pos = content.find("publegacy_name:").expect("legacy_name field");
    let start = content[..field_pos]
        .rfind("#[deprecated")
        .expect("a deprecated attribute above the field");
    let between = &content[start..field_pos];
    // Whitespace-insensitive like the rest of this module, so the note reads
    // `note="uselabel"` here.
    assert!(
        between.contains(r#"note="uselabel""#),
        "the caller's note survives: {between}"
    );
    assert_eq!(
        between.matches("#[deprecated").count(),
        1,
        "exactly one #[deprecated] on the field, got: {between}"
    );
    assert_eq!(
        content.matches(r#"note="uselabel""#).count(),
        1,
        "the note must not be duplicated onto other fields"
    );
}

#[test]
fn caller_deprecated_attribute_on_other_field_still_gets_marker() {
    // The hand-restored path only suppresses the option-derived marker for the
    // field its path matches: `label` gets the caller's note, `legacy_name`
    // still gets the option's marker.
    let config = CodeGenConfig {
        field_attributes: vec![(
            ".deprecate.test.Widget.label".to_string(),
            "#[deprecated(note = \"hand\")]".to_string(),
        )],
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    assert!(
        content.contains("#[deprecated]publegacy_name:"),
        "unmatched field keeps the option-derived marker: {content}"
    );
    assert!(
        content.contains(r#"note="hand""#),
        "the caller's attribute still lands on its own field: {content}"
    );
}

#[test]
fn caller_deprecated_attribute_in_a_multi_attribute_string_is_detected() {
    // `matching_attributes` accepts a stream of attributes, so the marker can
    // arrive anywhere in the string — not just first.
    let config = CodeGenConfig {
        field_attributes: vec![(
            ".deprecate.test.Widget.legacy_name".to_string(),
            "#[allow(non_snake_case)] #[deprecated(note = \"use label\")]".to_string(),
        )],
        ..Default::default()
    };
    let content = generate_squashed(deprecated_file(), &config);
    let field_pos = content.find("publegacy_name:").expect("legacy_name field");
    let start = content[..field_pos]
        .rfind("#[deprecated")
        .expect("a deprecated attribute above the field");
    assert_eq!(
        content[start..field_pos].matches("#[deprecated").count(),
        1,
        "the option marker must yield to a marker in a multi-attribute string, \
         two would be a hard error: {}",
        &content[start..field_pos]
    );
}

#[test]
fn deprecated_field_setter_is_also_deprecated() {
    // Otherwise `msg.with_legacy_name(v)` is a silent way to write a
    // deprecated field while `msg.legacy_name = v` warns.
    let mut file = deprecated_file();
    file.message_type[0].field[0].proto3_optional = Some(true);
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        content.contains("#[deprecated]pubfnwith_legacy_name"),
        "the setter for a deprecated field must be deprecated: {content}"
    );
    assert!(
        !content.contains("#[deprecated]pubfnwith_label"),
        "the setter for a live field stays unmarked"
    );
}

#[test]
fn alias_deprecated_while_primary_is_live_marks_only_the_alias() {
    // Direction check: the marker flows from the value's own option, so an
    // alias can be deprecated while its primary is live (and the idiomatic
    // alias of the primary then stays unmarked).
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    file.enum_type.push(EnumDescriptorProto {
        name: Some("Size".to_string()),
        value: vec![
            enum_value("BIG", 1),
            deprecated_value(enum_value("LARGE", 1)),
        ],
        ..Default::default()
    });
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        !content.contains("#[deprecated]BIG=1"),
        "the primary is live: {content}"
    );
    assert!(
        content
            .contains("#[deprecated]#[allow(non_upper_case_globals)]pubconstLARGE:Self=Self::BIG"),
        "the deprecated alias carries the marker: {content}"
    );
    assert!(
        !content
            .contains("#[deprecated]#[allow(non_upper_case_globals)]pubconstBig:Self=Self::BIG"),
        "the primary's idiomatic alias must not inherit from the alias: {content}"
    );
}

#[test]
fn attribute_shape_detection() {
    // The caller-marker probe decides whether codegen adds its own
    // `#[deprecated]`, and two of them on one item is a hard error — so the
    // accepted and rejected shapes matter.
    let cases: &[(&str, bool)] = &[
        ("#[deprecated]", true),
        ("#[deprecated(note = \"use x\")]", true),
        ("#[deprecated(note = \"a\")]\n#[allow(dead_code)]", true),
        ("#[allow(non_snake_case)] #[deprecated]", true),
        // cfg_attr expands to the same attribute in one of its arms.
        ("#[cfg_attr(feature = \"json\", deprecated)]", true),
        (
            "#[cfg_attr(feature = \"json\", deprecated, serde(skip))]",
            true,
        ),
        // Not the built-in attribute.
        ("#[deprecated_alias]", false),
        ("#[some_tool::deprecated]", false),
        ("#[cfg_attr(feature = \"json\", allow(deprecated))]", false),
        ("#[serde(skip)]", false),
        ("#[doc = \"marked deprecated in the proto\"]", false),
        ("not an attribute", false),
        // An inner attribute in the stream must not abort the scan.
        ("#![allow(deprecated)] #[deprecated]", true),
    ];
    for (attr, expected) in cases {
        assert_eq!(
            crate::message::is_deprecated_attr_str(attr),
            *expected,
            "shape {attr:?} should be {expected}"
        );
    }
}

#[test]
fn message_level_deprecated_option_is_not_emitted() {
    // Parity with prost-build, which marks fields, oneof variants and enum
    // values only. Pinned so widening the scope later is a deliberate change.
    let mut msg = DescriptorProto {
        name: Some("Widget".to_string()),
        field: vec![make_field(
            "label",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_STRING,
        )],
        options: MessageOptions {
            deprecated: Some(true),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let mut file = proto3_file("deprecated.proto");
    file.package = Some("deprecate.test".to_string());
    msg.options = MessageOptions {
        deprecated: Some(true),
        ..Default::default()
    }
    .into();
    file.message_type.push(msg);
    let content = generate_squashed(file, &CodeGenConfig::default());
    assert!(
        !content.contains("#[deprecated]pubstructWidget"),
        "message-level deprecation is not part of this change: {content}"
    );
}

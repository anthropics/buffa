//! Path-scoped editions feature overrides via descriptor feature injection.
//!
//! [`CodeGenConfig::feature_overrides`](crate::CodeGenConfig::feature_overrides)
//! is implemented as a preprocessing pass over the parsed descriptor set, run
//! once before [`CodeGenContext`](crate::context) construction. Each
//! [`FeatureOverride`] writes its feature slot into the matched descriptors,
//! and everything downstream — struct field types, decoders, JSON, text
//! format, reflection, the embedded descriptor pool — resolves features from
//! the mutated set, so no per-call-site configuration lookup exists anywhere
//! in the generation paths. The supported override set is the
//! [`FeatureOverride`] enum itself: a variant is added only once codegen
//! handles the descriptor states it can create.
//!
//! For [`FeatureOverride::EnumType`]: matching enums receive
//! `features.enum_type = OPEN` on their own descriptor (the same construct
//! protoc's proto2 → editions migration emits), and matching fields receive
//! a field-level override. Field-level `enum_type` is not a legal editions
//! target (protoc rejects it), so it can never appear in real input; it is
//! used purely as the carrier for field-scoped overrides, honored by the
//! carve-out in [`features::resolve_field`](crate::features::resolve_field).
//! Because the mutated set is also what
//! [`encode_fds_once`](crate::reflect::encode_fds_once) embeds, an
//! *enum-level* rule flows all the way to runtime: the embedded pool reports
//! the enum open and descriptor-driven dynamic codecs agree with the
//! generated types (a fully spec-valid descriptor). A *field-level* rule is
//! codegen-only: the runtime pool reads closedness from the enum descriptor,
//! so descriptor-driven codecs keep closed semantics for that enum — the
//! documented trade for field-scoped granularity.

use crate::context::matches_proto_prefix;
use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::generated::descriptor::{
    feature_set, DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use crate::{EnumTypeOverride, FeatureOverride};

/// The result of applying feature overrides: the mutated descriptor set,
/// plus any rules that matched nothing (surfaced as
/// [`CodeGenWarning::FeatureOverrideMatchedNothing`](crate::CodeGenWarning) —
/// a silently inert rule means the affected paths keep the semantics the
/// user was opting out of).
pub(crate) struct AppliedFeatureOverrides {
    pub(crate) files: Vec<FileDescriptorProto>,
    /// `(path, override)` pairs that matched nothing.
    pub(crate) unmatched: Vec<(String, FeatureOverride)>,
}

/// Apply configured feature overrides to a descriptor set, returning the
/// mutated copy, or `None` when none are configured (the common case —
/// callers keep using the borrowed input, so the default path never clones).
pub(crate) fn apply_feature_overrides(
    files: &[FileDescriptorProto],
    overrides: &[(String, FeatureOverride)],
) -> Option<AppliedFeatureOverrides> {
    if overrides.is_empty() {
        return None;
    }

    // Partition by override kind, keeping each rule's index into `overrides`
    // so unmatched reporting maps back to the caller's entries. Today the
    // only kind is EnumType(Open); future kinds add their own partition and
    // walker below.
    let mut enum_open_paths: Vec<String> = Vec::new();
    let mut enum_open_idx: Vec<usize> = Vec::new();
    for (i, (path, ovr)) in overrides.iter().enumerate() {
        match ovr {
            FeatureOverride::EnumType(EnumTypeOverride::Open) => {
                enum_open_paths.push(path.clone());
                enum_open_idx.push(i);
            }
        }
    }

    let mut matched = vec![false; overrides.len()];
    let mut files = files.to_vec();
    apply_open_enum_rules(&mut files, &enum_open_paths, &enum_open_idx, &mut matched);

    let unmatched = overrides
        .iter()
        .zip(&matched)
        .filter(|(_, m)| !**m)
        .map(|((path, ovr), _)| (path.clone(), *ovr))
        .collect();
    Some(AppliedFeatureOverrides { files, unmatched })
}

/// Apply `EnumType(Open)` rules: mutate matched enums' descriptors and
/// inject field-level carriers for field-scoped matches. `idx_map` maps each
/// rule's position in `rules` back to the caller's override index for
/// `matched` bookkeeping.
fn apply_open_enum_rules(
    files: &mut [FileDescriptorProto],
    rules: &[String],
    idx_map: &[usize],
    overall_matched: &mut [bool],
) {
    if rules.is_empty() {
        return;
    }

    // Enum FQNs present in this compilation set. Rules naming an enum that
    // *is* in the set open it at the enum level; rules matching the type of a
    // field whose enum is absent (extern_path) fall back to a field-level
    // override so the field representation still opens.
    let mut local_enums = std::collections::HashSet::new();
    for file in files.iter() {
        let prefix = package_prefix(file.package.as_deref());
        for e in &file.enum_type {
            collect_enum_fqn(&mut local_enums, &prefix, e);
        }
        for m in &file.message_type {
            collect_message_enum_fqns(&mut local_enums, &prefix, m);
        }
    }

    let mut matched = vec![false; rules.len()];
    for file in files.iter_mut() {
        let prefix = package_prefix(file.package.as_deref());
        for e in &mut file.enum_type {
            open_enum_if_matched(e, &prefix, rules, &mut matched);
        }
        for m in &mut file.message_type {
            apply_to_message(m, &prefix, rules, &local_enums, &mut matched);
        }
    }

    for (rule_i, &overall_i) in idx_map.iter().enumerate() {
        if matched[rule_i] {
            overall_matched[overall_i] = true;
        }
    }
}

/// `".pkg"` → `".pkg."`; empty package → `"."`.
fn package_prefix(package: Option<&str>) -> String {
    match package {
        Some(p) if !p.is_empty() => format!(".{p}."),
        _ => ".".to_string(),
    }
}

fn collect_enum_fqn(
    set: &mut std::collections::HashSet<String>,
    prefix: &str,
    e: &EnumDescriptorProto,
) {
    if let Some(name) = e.name.as_deref() {
        set.insert(format!("{prefix}{name}"));
    }
}

fn collect_message_enum_fqns(
    set: &mut std::collections::HashSet<String>,
    prefix: &str,
    msg: &DescriptorProto,
) {
    let Some(name) = msg.name.as_deref() else {
        return;
    };
    let child_prefix = format!("{prefix}{name}.");
    for e in &msg.enum_type {
        collect_enum_fqn(set, &child_prefix, e);
    }
    for nested in &msg.nested_type {
        collect_message_enum_fqns(set, &child_prefix, nested);
    }
}

/// Does any rule match this dotted FQN? Marks every matching rule in
/// `matched` (no short-circuit), so inert rules can be reported. Mirrors the
/// path-scoped option matching used elsewhere (`bytes_fields` et al): `"."`
/// matches everything, a leading dot is optional, trailing dots are ignored,
/// and prefixes only match on proto segment boundaries.
fn rule_matches(rules: &[String], fqn_dotted: &str, matched: &mut [bool]) -> bool {
    let mut any = false;
    for (i, rule) in rules.iter().enumerate() {
        if rule_matches_one(rule, fqn_dotted) {
            matched[i] = true;
            any = true;
        }
    }
    any
}

fn rule_matches_one(rule: &str, fqn_dotted: &str) -> bool {
    let rule = rule.trim();
    if rule == "." {
        return true;
    }
    let rule = rule.trim_end_matches('.');
    if rule.is_empty() {
        return false;
    }
    if rule.starts_with('.') {
        matches_proto_prefix(rule, fqn_dotted)
    } else if let Some(fqn_dotless) = fqn_dotted.strip_prefix('.') {
        matches_proto_prefix(rule, fqn_dotless)
    } else {
        matches_proto_prefix(rule, fqn_dotted)
    }
}

fn open_enum_if_matched(
    e: &mut EnumDescriptorProto,
    prefix: &str,
    rules: &[String],
    matched: &mut [bool],
) {
    let Some(name) = e.name.as_deref() else {
        return;
    };
    if rule_matches(rules, &format!("{prefix}{name}"), matched) {
        e.options
            .get_or_insert_default()
            .features
            .get_or_insert_default()
            .enum_type = Some(feature_set::EnumType::OPEN);
    }
}

fn open_field(field: &mut FieldDescriptorProto) {
    field
        .options
        .get_or_insert_default()
        .features
        .get_or_insert_default()
        .enum_type = Some(feature_set::EnumType::OPEN);
}

fn apply_to_message(
    msg: &mut DescriptorProto,
    prefix: &str,
    rules: &[String],
    local_enums: &std::collections::HashSet<String>,
    matched: &mut [bool],
) {
    let Some(name) = msg.name.as_deref() else {
        return;
    };
    let msg_prefix = format!("{prefix}{name}.");

    // Map fields whose outer path matches route the override to the synthetic
    // entry's value field (the map field itself is TYPE_MESSAGE, so the
    // direct-field pass below never fires for it). The entry is identified
    // through `find_map_entry` — the same helper the rest of codegen uses —
    // so a plain message field whose type's last segment happens to collide
    // with a sibling map-entry name can never route an override to the wrong
    // entry. Collected first (immutably, `find_map_entry` needs `&msg`),
    // applied to `nested_type` after the field loop releases the borrow. The
    // rule is only counted as matched once the entry's value is confirmed
    // enum-typed — a rule naming a non-enum map changes nothing and warns.
    let mut matched_entries: Vec<(String, String)> = Vec::new();
    for field in &msg.field {
        let Some(field_name) = field.name.as_deref() else {
            continue;
        };
        let Some(entry) = crate::message::find_map_entry(msg, field) else {
            continue;
        };
        let field_fqn = format!("{msg_prefix}{field_name}");
        if rules.iter().any(|rule| rule_matches_one(rule, &field_fqn)) {
            if let Some(entry_name) = entry.name.as_deref() {
                matched_entries.push((entry_name.to_string(), field_fqn));
            }
        }
    }

    for field in &mut msg.field {
        if field.r#type.unwrap_or_default() != Type::TYPE_ENUM {
            continue;
        }
        let (Some(field_name), Some(type_name)) =
            (field.name.as_deref(), field.type_name.as_deref())
        else {
            continue;
        };
        // A field-path match opens the field. An enum-type match on an enum
        // in this set is handled by the enum-level mutation (the field picks
        // it up through the referenced-enum overlay); for an extern enum the
        // field-level override is the only carrier. When the referenced local
        // enum is itself opened by a rule (e.g. a broad prefix matching both
        // sides), the field-level injection would be redundant — skip it so
        // the embedded descriptor set carries the non-spec field-level
        // feature only where it is load-bearing.
        let field_match = rule_matches(rules, &format!("{msg_prefix}{field_name}"), matched);
        let enum_is_local = local_enums.contains(type_name);
        let enum_type_match = rule_matches(rules, type_name, matched);
        let opened_via_enum = enum_is_local && enum_type_match;
        let extern_match = !enum_is_local && enum_type_match;
        if (field_match || extern_match) && !opened_via_enum {
            open_field(field);
        }
    }

    for e in &mut msg.enum_type {
        open_enum_if_matched(e, &msg_prefix, rules, matched);
    }
    for nested in &mut msg.nested_type {
        if let Some((_, field_fqn)) = matched_entries
            .iter()
            .find(|(entry, _)| nested.name.as_deref() == Some(entry.as_str()))
        {
            for f in &mut nested.field {
                if f.number == Some(2) && f.r#type.unwrap_or_default() == Type::TYPE_ENUM {
                    let value_enum_opened = f.type_name.as_deref().is_some_and(|tn| {
                        local_enums.contains(tn) && rule_matches(rules, tn, matched)
                    });
                    if !value_enum_opened {
                        open_field(f);
                    }
                    rule_matches(rules, field_fqn, matched);
                }
            }
        }
        apply_to_message(nested, &msg_prefix, rules, local_enums, matched);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::field_descriptor_proto::Label;

    /// Wrap paths as `EnumType(Open)` overrides — the shape every test uses.
    fn open_enums(paths: &[&str]) -> Vec<(String, FeatureOverride)> {
        paths
            .iter()
            .map(|p| {
                (
                    (*p).to_string(),
                    FeatureOverride::EnumType(EnumTypeOverride::Open),
                )
            })
            .collect()
    }

    fn unmatched_paths(applied: &AppliedFeatureOverrides) -> Vec<String> {
        applied
            .unmatched
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }

    fn enum_desc(name: &str) -> EnumDescriptorProto {
        EnumDescriptorProto {
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    fn enum_field(name: &str, number: i32, type_name: &str) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            number: Some(number),
            label: Some(Label::LABEL_OPTIONAL),
            r#type: Some(Type::TYPE_ENUM),
            type_name: Some(type_name.to_string()),
            ..Default::default()
        }
    }

    fn test_file() -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some("t.proto".to_string()),
            package: Some("p".to_string()),
            syntax: Some("proto2".to_string()),
            enum_type: vec![enum_desc("E")],
            message_type: vec![DescriptorProto {
                name: Some("M".to_string()),
                field: vec![
                    enum_field("a", 1, ".p.E"),
                    enum_field("b", 2, ".p.E"),
                    enum_field("ext", 3, ".other.Ext"),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn field_enum_type(f: &FieldDescriptorProto) -> Option<feature_set::EnumType> {
        f.options
            .as_option()
            .and_then(|o| o.features.as_option())
            .and_then(|fs| fs.enum_type)
    }

    fn matches(rules: &[String], fqn: &str) -> bool {
        let mut matched = vec![false; rules.len()];
        rule_matches(rules, fqn, &mut matched)
    }

    #[test]
    fn no_rules_returns_none() {
        assert!(apply_feature_overrides(&[test_file()], &[]).is_none());
    }

    #[test]
    fn enum_rule_mutates_enum_not_fields() {
        let applied = apply_feature_overrides(&[test_file()], &open_enums(&[".p.E"])).unwrap();
        assert!(applied.unmatched.is_empty());
        let e = &applied.files[0].enum_type[0];
        assert_eq!(
            e.options
                .as_option()
                .and_then(|o| o.features.as_option())
                .and_then(|fs| fs.enum_type),
            Some(feature_set::EnumType::OPEN)
        );
        // Fields referencing a local enum pick openness up via the enum;
        // no field-level injection needed.
        assert_eq!(
            field_enum_type(&applied.files[0].message_type[0].field[0]),
            None
        );
    }

    #[test]
    fn field_rule_mutates_only_that_field() {
        let applied = apply_feature_overrides(&[test_file()], &open_enums(&[".p.M.a"])).unwrap();
        assert!(applied.unmatched.is_empty());
        let msg = &applied.files[0].message_type[0];
        assert_eq!(
            field_enum_type(&msg.field[0]),
            Some(feature_set::EnumType::OPEN)
        );
        assert_eq!(field_enum_type(&msg.field[1]), None);
        assert!(applied.files[0].enum_type[0].options.as_option().is_none());
    }

    #[test]
    fn extern_enum_rule_falls_back_to_field_injection() {
        // `.other.Ext` is not declared in the set, so an enum-type rule can
        // only take effect through the referencing field.
        let applied =
            apply_feature_overrides(&[test_file()], &open_enums(&[".other.Ext"])).unwrap();
        assert!(applied.unmatched.is_empty());
        let msg = &applied.files[0].message_type[0];
        assert_eq!(field_enum_type(&msg.field[0]), None);
        assert_eq!(
            field_enum_type(&msg.field[2]),
            Some(feature_set::EnumType::OPEN)
        );
    }

    #[test]
    fn inert_rules_are_reported_unmatched() {
        let applied = apply_feature_overrides(
            &[test_file()],
            &open_enums(&[".p.M.a", ".p.Missing", ".p.M.a.typo"]),
        )
        .unwrap();
        assert_eq!(
            unmatched_paths(&applied),
            vec![".p.Missing".to_string(), ".p.M.a.typo".to_string()]
        );
    }

    #[test]
    fn broad_prefix_skips_redundant_field_injection() {
        // `.p` matches both the enum and every field path; the enum-level
        // mutation is sufficient, so no field-level features are injected.
        let applied = apply_feature_overrides(&[test_file()], &open_enums(&[".p"])).unwrap();
        assert!(applied.unmatched.is_empty());
        let file = &applied.files[0];
        assert_eq!(
            file.enum_type[0]
                .options
                .as_option()
                .and_then(|o| o.features.as_option())
                .and_then(|fs| fs.enum_type),
            Some(feature_set::EnumType::OPEN)
        );
        assert_eq!(field_enum_type(&file.message_type[0].field[0]), None);
        assert_eq!(field_enum_type(&file.message_type[0].field[1]), None);
        // The extern field has no local enum to carry the override, so the
        // field-level injection still applies there.
        assert_eq!(
            field_enum_type(&file.message_type[0].field[2]),
            Some(feature_set::EnumType::OPEN)
        );
    }

    #[test]
    fn nested_enum_and_empty_package_are_matchable() {
        let file = FileDescriptorProto {
            name: Some("np.proto".to_string()),
            syntax: Some("proto2".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("Outer".to_string()),
                enum_type: vec![enum_desc("Inner")],
                field: vec![enum_field("e", 1, ".Outer.Inner")],
                ..Default::default()
            }],
            ..Default::default()
        };
        let applied = apply_feature_overrides(&[file], &open_enums(&[".Outer.Inner"])).unwrap();
        assert!(applied.unmatched.is_empty());
        assert_eq!(
            applied.files[0].message_type[0].enum_type[0]
                .options
                .as_option()
                .and_then(|o| o.features.as_option())
                .and_then(|fs| fs.enum_type),
            Some(feature_set::EnumType::OPEN)
        );
        // The nested enum is local and directly opened, so its referencing
        // field needs no field-level injection.
        assert_eq!(
            field_enum_type(&applied.files[0].message_type[0].field[0]),
            None
        );
    }

    #[test]
    fn rule_matching_normalizes_and_keeps_boundaries() {
        assert!(matches(&[".".to_string()], ".my.pkg.Msg.status"));
        assert!(matches(
            &["my.pkg.Msg.status.".to_string()],
            ".my.pkg.Msg.status"
        ));
        assert!(matches(
            &["  .my.pkg.Status.  ".to_string()],
            ".my.pkg.Status"
        ));

        assert!(!matches(&["".to_string()], ".my.pkg.Msg.status"));
        assert!(!matches(&["...".to_string()], ".my.pkg.Msg.status"));
        assert!(!matches(&[".my.pk".to_string()], ".my.pkg.Msg.status"));
    }
}

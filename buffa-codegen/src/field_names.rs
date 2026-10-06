//! Pre-passes that plan the Rust source names of proto fields and oneofs
//! before any code is emitted: the snake_case conversion for
//! [`CodeGenConfig::idiomatic_field_names`], and the keyword-escape renames,
//! which apply with the option on or off.
//!
//! The snake_case conversion itself ([`idiomatic_snake_case`]) is context-free and
//! applied at ident-construction time by
//! [`CodeGenContext::field_rust_name`] /
//! [`CodeGenContext::oneof_rust_name`]; this pass only records the
//! *exceptions* — names whose context-free conversion would collide inside a
//! message's struct namespace and therefore need a deterministic adjustment.
//!
//! ## Conversion semantics
//!
//! [`idiomatic_snake_case`] is an *insertion-only* converter: it inserts an
//! underscore at every word boundary and lowercases, but never deletes or
//! collapses underscores the proto author wrote — so it is the identity on
//! every name that is already a valid snake_case identifier (including
//! `_foo` and `a__b`, which heck-style converters would rewrite to `foo` /
//! `a_b`). Word boundaries match heck's (and therefore prost-build's)
//! segmentation exactly: a boundary falls before an uppercase character
//! whose most recent *cased* predecessor is lowercase (digits are
//! transparent, so `v2Field` → `v2_field`), and before the last uppercase
//! character of an acronym run that is followed by a lowercase one
//! (`XMLHttpRequest` → `xml_http_request`). The Rails `underscore` /
//! Python `inflection` family behaves the same way on both counts.
//!
//! The net difference from prost is confined to names containing leading,
//! trailing, or doubled underscores: prost (via heck) normalizes those
//! (`_fieldName3` → `field_name3`), buffa preserves them
//! (`_fieldName3` → `_field_name3`).
//!
//! ## Collision rules
//!
//! Within one message, the struct namespace is the set of non-oneof-member
//! fields plus the non-synthetic oneof names (each oneof is one struct
//! field). When two or more members convert to the same candidate name:
//!
//! - A member whose name the conversion leaves **unchanged** (it was already
//!   snake_case) always keeps it. Proto names are unique per message, so at
//!   most one member of a group is unchanged.
//! - A **changed field** is suffixed with `_f<number>` (`userName = 12` →
//!   `user_name_f12`). Field numbers are unique and stable per message, so
//!   the suffix is stable under reordering and unrelated edits.
//! - A **changed oneof** falls back to its verbatim proto name (oneofs have
//!   no number to disambiguate with; the declaration index is not stable
//!   under reordering).
//!
//! If a suffixed/converted name *still* collides (e.g. a literal field named
//! `user_name_f12` next to `userName = 12`), every changed member of the
//! affected group reverts to its verbatim proto name — verbatim names are
//! unique by construction, so this final fallback always succeeds. Each
//! affected message records a [`CodeGenWarning::IdiomaticFieldNamesAdjusted`].
//!
//! ## Cross-message keying
//!
//! Exceptions are keyed by `(proto_name, field_number)` (and by name alone
//! for oneofs) rather than by containing message, so deeply nested emission
//! helpers can resolve a field's Rust name without threading the message FQN
//! through every signature. If two messages share the same `(name, number)`
//! pair but disagree on the outcome (one collides, the other doesn't), the
//! more conservative entry wins everywhere: verbatim fallback over suffix,
//! suffix over plain conversion. This leaks an adjustment from one message
//! into another when both messages reuse the exact same name *and* number;
//! a second planning pass ([`sweep_inherited`]) emits an
//! [`IdiomaticFieldNamesAdjusted`] warning for every message that inherits
//! an adjustment this way, so an adjusted Rust name never appears silently.
//! protoc rejects the underlying same-message collision for proto3 and
//! editions files outright (conflicting `json_name`s), so a collision can
//! only *originate* in a proto2 file — but a proto3 message compiled in the
//! same run can still inherit its adjustment through the shared key.
//!
//! ## Keyword-escaped names
//!
//! A second plan, [`plan_keyword_escapes`], runs with the option on or off.
//! `self`, `super`, `crate` and `Self` cannot be raw identifiers, so
//! [`make_field_ident`] appends `_` to them. When that name is also the name
//! of another member of the message (a field `self` next to a field `self_`,
//! or a field and a oneof in either order), the keyword member takes more
//! underscores until its name is free: `self__`. The member whose proto name
//! is `self_` keeps its name.
//!
//! The plan covers the messages of the files that the run generates, and uses
//! the keying described in [Cross-message keying](#cross-message-keying).
//! Every one of those messages that declares the same `(name, number)` field,
//! or a oneof of the same name, gets the same Rust name. The name is free in
//! all of them, and each one records a
//! [`CodeGenWarning::KeywordEscapedNamesAdjusted`]. A rename originates only
//! in a message that has both members.
//!
//! [`IdiomaticFieldNamesAdjusted`]: crate::CodeGenWarning::IdiomaticFieldNamesAdjusted
//! [`make_field_ident`]: crate::idents::make_field_ident
//! [`CodeGenWarning::KeywordEscapedNamesAdjusted`]: crate::CodeGenWarning::KeywordEscapedNamesAdjusted
//!
//! [`CodeGenConfig::idiomatic_field_names`]: crate::CodeGenConfig::idiomatic_field_names
//! [`CodeGenContext::field_rust_name`]: crate::context::CodeGenContext::field_rust_name
//! [`CodeGenContext::oneof_rust_name`]: crate::context::CodeGenContext::oneof_rust_name
//! [`CodeGenWarning::IdiomaticFieldNamesAdjusted`]: crate::CodeGenWarning::IdiomaticFieldNamesAdjusted

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::generated::descriptor::{DescriptorProto, FileDescriptorProto};
use crate::idents::{is_suffix_escaped, SUFFIX_ESCAPED_KEYWORDS};
use crate::impl_message::is_real_oneof_member;
use crate::CodeGenWarning;

/// Convert a proto field/oneof name to snake_case for
/// `idiomatic_field_names` (see the module docs for the exact semantics and
/// how they relate to heck/prost and the Rails `underscore` family).
///
/// This is deliberately a separate function from
/// [`crate::oneof::to_snake_case`], which names modules: that converter has
/// no digit-transparent boundary (`Msg2Part` → `msg2part`), and adding one
/// there would re-shape the generated module layout for existing users.
/// Field idents are a new, opt-in surface, so they can adopt the
/// ecosystem-standard boundaries from the start.
pub(crate) fn idiomatic_snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    // The most recent cased (upper/lowercase) character of the current word.
    // Digits are transparent; an underscore starts a new word (`None`), which
    // both suppresses insertion right after it and keeps `foo_2Bar` as
    // `foo_2bar` (matching heck's segmentation) rather than `foo_2_bar`.
    let mut last_cased: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            last_cased = None;
            out.push('_');
            continue;
        }
        if c.is_uppercase() && i > 0 {
            let boundary = match last_cased {
                Some(p) if p.is_lowercase() => true,
                Some(p) if p.is_uppercase() => {
                    // Acronym end: the last uppercase of a run starts the
                    // next word when followed by a lowercase character.
                    chars.get(i + 1).is_some_and(|n| n.is_lowercase())
                }
                _ => false,
            };
            if boundary {
                out.push('_');
            }
        }
        // `to_lowercase()` may yield multiple chars (e.g. `İ` → `i\u{307}`);
        // extend with the full sequence rather than truncating to the first.
        out.extend(c.to_lowercase());
        if c.is_lowercase() || c.is_uppercase() {
            last_cased = Some(c);
        }
    }
    out
}

/// The planned field/oneof rename exceptions for one generation run.
///
/// Empty maps mean "context-free conversion everywhere" — the common case.
#[derive(Debug, Default)]
pub(crate) struct FieldNamePlan {
    /// `(proto_name, field_number)` → final Rust source name (pre keyword
    /// escaping), for fields whose context-free conversion was adjusted.
    pub field_renames: HashMap<(String, i32), String>,
    /// Oneof proto names that must keep their verbatim spelling (their
    /// conversion collided somewhere).
    pub oneof_keep_verbatim: HashSet<String>,
    /// One warning per message that needed any adjustment.
    pub warnings: Vec<CodeGenWarning>,
}

/// The Rust source name of a field under the `idiomatic_field_names` plan,
/// before any keyword-escape rename: the proto name with the option off,
/// otherwise the plan's entry for `(name, number)` or the snake_case
/// conversion.
pub(crate) fn planned_field_name<'n>(
    idiomatic_field_names: bool,
    field_renames: &'n HashMap<(String, i32), String>,
    name: &'n str,
    number: i32,
) -> Cow<'n, str> {
    if !idiomatic_field_names {
        return Cow::Borrowed(name);
    }
    if !field_renames.is_empty() {
        if let Some(renamed) = field_renames.get(&(name.to_string(), number)) {
            return Cow::Borrowed(renamed);
        }
    }
    snake_case_or_borrowed(name)
}

/// The oneof counterpart of [`planned_field_name`]: a oneof in
/// `oneof_keep_verbatim` keeps its proto name.
pub(crate) fn planned_oneof_name<'n>(
    idiomatic_field_names: bool,
    oneof_keep_verbatim: &HashSet<String>,
    name: &'n str,
) -> Cow<'n, str> {
    if !idiomatic_field_names || oneof_keep_verbatim.contains(name) {
        return Cow::Borrowed(name);
    }
    snake_case_or_borrowed(name)
}

fn snake_case_or_borrowed(name: &str) -> Cow<'_, str> {
    let converted = idiomatic_snake_case(name);
    if converted == name {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(converted)
    }
}

/// Rust source names for the fields and oneofs that the keyword-escape plan
/// renames; see the module docs.
#[derive(Debug, Default)]
pub(crate) struct KeywordEscapePlan {
    /// `(proto_name, field_number)` → Rust source name.
    pub field_renames: HashMap<(String, i32), String>,
    /// Proto oneof name → Rust source name.
    pub oneof_renames: HashMap<String, String>,
    /// One warning per message that has a renamed member.
    pub warnings: Vec<CodeGenWarning>,
}

/// One struct member of a message that has a suffix-escaped member.
struct EscapeMember {
    proto_name: String,
    /// `Some(number)` for a field, `None` for a oneof.
    number: Option<i32>,
    /// The Rust source name before keyword escaping.
    rust_name: String,
}

struct EscapeScope {
    fqn: String,
    members: Vec<EscapeMember>,
}

/// Plan the names of members that [`make_field_ident`] suffixes with `_`
/// (`self`, `super`, `crate`, `Self`) where the suffixed name is also the
/// name of another member of the message: `self` next to `self_` becomes
/// `self__`.
///
/// `field_renames` and `oneof_keep_verbatim` are the `idiomatic_field_names`
/// plan, empty when the option is off.
///
/// Only messages of the files in `generated_files` take part (every file
/// when `None`). A message that this run does not emit has no Rust name to
/// plan, and a collision in an imported file must not rename a member of a
/// generated message.
///
/// [`make_field_ident`]: crate::idents::make_field_ident
pub(crate) fn plan_keyword_escapes(
    files: &[FileDescriptorProto],
    idiomatic_field_names: bool,
    field_renames: &HashMap<(String, i32), String>,
    oneof_keep_verbatim: &HashSet<String>,
    generated_files: Option<&HashSet<String>>,
) -> KeywordEscapePlan {
    let generated = files.iter().filter(|file| {
        generated_files.map_or(true, |names| {
            names.contains(file.name.as_deref().unwrap_or_default())
        })
    });
    let mut scopes: Vec<EscapeScope> = Vec::new();
    for_each_message(generated, |fqn, msg| {
        if !has_keyword_named_member(msg) {
            return;
        }
        let members: Vec<EscapeMember> = struct_members(msg)
            .into_iter()
            .map(|(name, number)| {
                let rust_name = match number {
                    Some(number) => {
                        planned_field_name(idiomatic_field_names, field_renames, name, number)
                    }
                    None => planned_oneof_name(idiomatic_field_names, oneof_keep_verbatim, name),
                };
                EscapeMember {
                    proto_name: name.to_string(),
                    number,
                    rust_name: rust_name.into_owned(),
                }
            })
            .collect();
        if members.iter().any(|m| is_suffix_escaped(&m.rust_name)) {
            scopes.push(EscapeScope {
                fqn: fqn.to_string(),
                members,
            });
        }
    });

    // Keys whose escaped identifier is another member's name in some message,
    // with the key's source name. Ordered, so the plan is deterministic.
    let mut colliding: BTreeMap<(&str, Option<i32>), &str> = BTreeMap::new();
    for scope in &scopes {
        for member in &scope.members {
            if !is_suffix_escaped(&member.rust_name) {
                continue;
            }
            let escaped = format!("{}_", member.rust_name);
            if scope.members.iter().any(|m| m.rust_name == escaped) {
                colliding.insert((&member.proto_name, member.number), &member.rust_name);
            }
        }
    }

    let mut plan = KeywordEscapePlan::default();
    for ((proto_name, number), rust_name) in colliding {
        // The rename applies to every message that has this key, so the new
        // name has to be free in all of them.
        let taken = |candidate: &str| {
            scopes
                .iter()
                .filter(|scope| {
                    scope
                        .members
                        .iter()
                        .any(|m| m.proto_name == proto_name && m.number == number)
                })
                .any(|scope| scope.members.iter().any(|m| m.rust_name == candidate))
        };
        let mut renamed = format!("{rust_name}__");
        while taken(&renamed) {
            renamed.push('_');
        }
        match number {
            Some(number) => {
                plan.field_renames
                    .insert((proto_name.to_string(), number), renamed);
            }
            None => {
                plan.oneof_renames.insert(proto_name.to_string(), renamed);
            }
        }
    }

    for scope in &scopes {
        let mut assignments: Vec<(String, String)> = scope
            .members
            .iter()
            .filter_map(|member| {
                let renamed = match member.number {
                    Some(number) => plan.field_renames.get(&(member.proto_name.clone(), number)),
                    None => plan.oneof_renames.get(&member.proto_name),
                }?;
                Some((member.proto_name.clone(), renamed.clone()))
            })
            .collect();
        if assignments.is_empty() {
            continue;
        }
        assignments.sort();
        plan.warnings
            .push(CodeGenWarning::KeywordEscapedNamesAdjusted {
                message_name: scope.fqn.clone(),
                assignments,
            });
    }

    plan
}

/// Whether a member of `msg` can have a suffix-escaped Rust source name (one
/// of [`SUFFIX_ESCAPED_KEYWORDS`]). The snake_case conversion only inserts
/// underscores and lowercases, so the proto name of such a member is one of
/// those words in some letter case.
fn has_keyword_named_member(msg: &DescriptorProto) -> bool {
    let fields = msg.field.iter().filter_map(|field| field.name.as_deref());
    let oneofs = msg.oneof_decl.iter().filter_map(|o| o.name.as_deref());
    fields.chain(oneofs).any(|name| {
        SUFFIX_ESCAPED_KEYWORDS
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
    })
}

/// The struct namespace of `msg`: each field that is not a member of a real
/// oneof (a proto3 `optional` field is such a field), and each real oneof, as
/// `(proto_name, field number)` with `None` for a oneof. The members of a real
/// oneof are enum variants, and a synthetic oneof has no struct field.
fn struct_members(msg: &DescriptorProto) -> Vec<(&str, Option<i32>)> {
    let mut members = Vec::new();
    let mut real_oneofs: HashSet<i32> = HashSet::new();
    for field in &msg.field {
        let Some(name) = field.name.as_deref() else {
            continue;
        };
        if is_real_oneof_member(field) {
            if let Some(idx) = field.oneof_index {
                real_oneofs.insert(idx);
            }
            continue;
        }
        members.push((name, Some(field.number.unwrap_or(0))));
    }
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let Some(name) = oneof.name.as_deref() else {
            continue;
        };
        if real_oneofs.contains(&i32::try_from(idx).unwrap_or(i32::MAX)) {
            members.push((name, None));
        }
    }
    members
}

/// Rank of an exception entry, for the cross-message conservative merge.
/// Higher rank wins: verbatim fallback (2) > suffix (1); plain conversion has
/// no entry at all (rank 0).
fn rank(name: &str, number: i32, value: &str) -> u8 {
    if value == name {
        2
    } else {
        debug_assert!(value.ends_with(&format!("_f{number}")));
        1
    }
}

/// Walk every message in `files` and plan the rename exceptions.
pub(crate) fn plan_field_names(files: &[FileDescriptorProto]) -> FieldNamePlan {
    let mut plan = FieldNamePlan::default();
    let mut assigned: HashSet<(String, String)> = HashSet::new();
    for_each_message(files, |fqn, msg| {
        plan_message(fqn, msg, &mut plan, &mut assigned)
    });
    // Second pass: the exception maps are keyed by `(name, number)` rather
    // than by containing message, so a message that shares a colliding
    // member's exact name and number *inherits* the adjustment without
    // having a collision of its own. Surface a warning for every such
    // message — an adjusted Rust name must never appear silently.
    for_each_message(files, |fqn, msg| {
        sweep_inherited(fqn, msg, &mut plan, &assigned)
    });
    plan
}

/// Apply `f` to every non-map-entry message in `files`, nested included,
/// with its dotted FQN (no leading dot).
fn for_each_message<'a>(
    files: impl IntoIterator<Item = &'a FileDescriptorProto>,
    mut f: impl FnMut(&str, &DescriptorProto),
) {
    fn recurse(fqn: &str, msg: &DescriptorProto, f: &mut impl FnMut(&str, &DescriptorProto)) {
        if msg
            .options
            .as_option()
            .and_then(|o| o.map_entry)
            .unwrap_or(false)
        {
            return;
        }
        for nested in &msg.nested_type {
            let nested_fqn = format!("{fqn}.{}", nested.name.as_deref().unwrap_or_default());
            recurse(&nested_fqn, nested, f);
        }
        f(fqn, msg);
    }
    for file in files {
        let package = file.package.as_deref().unwrap_or("");
        for msg in &file.message_type {
            let fqn = if package.is_empty() {
                msg.name.clone().unwrap_or_default()
            } else {
                format!("{package}.{}", msg.name.as_deref().unwrap_or_default())
            };
            recurse(&fqn, msg, &mut f);
        }
    }
}

/// Warn for adjustments `msg` inherits through the global `(name, number)`
/// keying without having a collision of its own (see [`plan_field_names`]).
fn sweep_inherited(
    fqn: &str,
    msg: &DescriptorProto,
    plan: &mut FieldNamePlan,
    assigned: &HashSet<(String, String)>,
) {
    let mut inherited: Vec<(String, String)> = Vec::new();
    for field in &msg.field {
        let Some(name) = field.name.as_deref() else {
            continue;
        };
        if is_real_oneof_member(field) {
            continue;
        }
        if assigned.contains(&(fqn.to_string(), name.to_string())) {
            continue;
        }
        let key = (name.to_string(), field.number.unwrap_or(0));
        if let Some(renamed) = plan.field_renames.get(&key) {
            inherited.push((name.to_string(), renamed.clone()));
        }
    }
    for oneof in &msg.oneof_decl {
        let Some(name) = oneof.name.as_deref() else {
            continue;
        };
        if assigned.contains(&(fqn.to_string(), name.to_string()))
            || !plan.oneof_keep_verbatim.contains(name)
            || idiomatic_snake_case(name) == name
        {
            continue;
        }
        inherited.push((name.to_string(), name.to_string()));
    }
    if !inherited.is_empty() {
        inherited.sort();
        plan.warnings
            .push(CodeGenWarning::IdiomaticFieldNamesAdjusted {
                message_name: fqn.to_string(),
                assignments: inherited,
            });
    }
}

/// One member of a message's struct namespace.
struct Member<'a> {
    proto_name: &'a str,
    /// `Some(number)` for a field, `None` for a oneof.
    number: Option<i32>,
    converted: String,
}

fn plan_message(
    fqn: &str,
    msg: &DescriptorProto,
    plan: &mut FieldNamePlan,
    assigned: &mut HashSet<(String, String)>,
) {
    let members: Vec<Member<'_>> = struct_members(msg)
        .into_iter()
        .map(|(proto_name, number)| Member {
            proto_name,
            number,
            converted: idiomatic_snake_case(proto_name),
        })
        .collect();

    // Group by converted candidate; resolve groups with more than one member.
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, m) in members.iter().enumerate() {
        groups.entry(m.converted.as_str()).or_default().push(i);
    }
    let colliding: Vec<&Vec<usize>> = groups.values().filter(|g| g.len() > 1).collect();
    if colliding.is_empty() {
        return;
    }

    // First resolution round: unchanged members keep their name; changed
    // fields get `_f<number>`; changed oneofs revert to verbatim.
    let mut finals: Vec<String> = members.iter().map(|m| m.converted.clone()).collect();
    let mut adjusted: Vec<usize> = Vec::new();
    for group in &colliding {
        for &i in group.iter() {
            let m = &members[i];
            if m.converted == m.proto_name {
                continue; // already snake_case: always keeps its name
            }
            finals[i] = match m.number {
                Some(number) => format!("{}_f{number}", m.converted),
                None => m.proto_name.to_string(),
            };
            adjusted.push(i);
        }
    }

    // Final uniqueness check. If a suffixed name still collides with another
    // final name, revert every *changed* member of the whole namespace to its
    // verbatim proto name — verbatim names are unique by construction.
    let unique: HashSet<&str> = finals.iter().map(String::as_str).collect();
    if unique.len() != finals.len() {
        for (i, m) in members.iter().enumerate() {
            if m.converted != m.proto_name {
                finals[i] = m.proto_name.to_string();
                if !adjusted.contains(&i) {
                    adjusted.push(i);
                }
            }
        }
    }

    // Record exceptions (conservative cross-message merge) and the warning.
    let mut assignments: Vec<(String, String)> = Vec::new();
    for &i in &adjusted {
        let m = &members[i];
        assignments.push((m.proto_name.to_string(), finals[i].clone()));
        assigned.insert((fqn.to_string(), m.proto_name.to_string()));
        match m.number {
            Some(number) => {
                let key = (m.proto_name.to_string(), number);
                let replace = match plan.field_renames.get(&key) {
                    None => true,
                    Some(existing) => {
                        rank(m.proto_name, number, &finals[i])
                            > rank(m.proto_name, number, existing)
                    }
                };
                if replace {
                    plan.field_renames.insert(key, finals[i].clone());
                }
            }
            None => {
                // Oneof adjustments are always "keep verbatim".
                plan.oneof_keep_verbatim.insert(m.proto_name.to_string());
            }
        }
    }
    assignments.sort();
    plan.warnings
        .push(CodeGenWarning::IdiomaticFieldNamesAdjusted {
            message_name: fqn.to_string(),
            assignments,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::{
        DescriptorProto, FieldDescriptorProto, OneofDescriptorProto,
    };

    fn field(name: &str, number: i32) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            number: Some(number),
            ..Default::default()
        }
    }

    fn msg(name: &str, fields: Vec<FieldDescriptorProto>) -> DescriptorProto {
        DescriptorProto {
            name: Some(name.to_string()),
            field: fields,
            ..Default::default()
        }
    }

    fn file(messages: Vec<DescriptorProto>) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some("test.proto".to_string()),
            package: Some("pkg".to_string()),
            message_type: messages,
            ..Default::default()
        }
    }

    #[test]
    fn idiomatic_snake_case_matches_heck_where_they_agree() {
        // prost-build's `to_snake` test vectors (which come from the
        // conformance suite's test_messages_proto3.proto), minus keyword
        // escaping (handled later by make_field_ident) and minus the
        // underscore-normalization class covered below.
        for (input, expected) in [
            ("FooBar", "foo_bar"),
            ("FooBarBAZ", "foo_bar_baz"),
            ("XMLHttpRequest", "xml_http_request"),
            ("While", "while"),
            ("FUZZ_BUSTER", "fuzz_buster"),
            ("foo_bar_baz", "foo_bar_baz"),
            ("FUZZ_buster", "fuzz_buster"),
            ("fieldname1", "fieldname1"),
            ("field_name2", "field_name2"),
            ("field0name5", "field0name5"),
            ("field_0_name6", "field_0_name6"),
            ("fieldName7", "field_name7"),
            ("FieldName8", "field_name8"),
            ("field_Name9", "field_name9"),
            ("Field_Name10", "field_name10"),
            ("FIELD_NAME11", "field_name11"),
            ("FIELD_name12", "field_name12"),
        ] {
            assert_eq!(idiomatic_snake_case(input), expected, "input {input}");
        }
    }

    #[test]
    fn idiomatic_snake_case_digit_transparent_boundaries() {
        // Digits are transparent to the boundary rules — both heck/prost and
        // the Rails `underscore` family split here; buffa's module-naming
        // converter does not.
        assert_eq!(idiomatic_snake_case("v2Field"), "v2_field");
        assert_eq!(idiomatic_snake_case("field0Name"), "field0_name");
        assert_eq!(idiomatic_snake_case("ipV6Address"), "ip_v6_address");
        assert_eq!(idiomatic_snake_case("HTTP2Server"), "http2_server");
        // An all-caps run with digits is one word (heck agrees).
        assert_eq!(idiomatic_snake_case("A2B"), "a2b");
        // An underscore starts a fresh word, so the digit carries no case
        // from before it (heck agrees: `foo_2bar`, not `foo_2_bar`).
        assert_eq!(idiomatic_snake_case("foo_2Bar"), "foo_2bar");
    }

    #[test]
    fn idiomatic_snake_case_preserves_authored_underscores() {
        // The deliberate divergence from heck/prost: insertion-only, so the
        // conversion is the identity on every valid snake_case identifier.
        // prost would produce field_name3 / field_name4 / fuzz here.
        assert_eq!(idiomatic_snake_case("_field_name3"), "_field_name3");
        assert_eq!(idiomatic_snake_case("field__name4_"), "field__name4_");
        assert_eq!(idiomatic_snake_case("_fuzz"), "_fuzz");
        assert_eq!(idiomatic_snake_case("fuzz_"), "fuzz_");
        // ...and conversion still applies around preserved underscores.
        assert_eq!(idiomatic_snake_case("_fieldName3"), "_field_name3");
        assert_eq!(idiomatic_snake_case("Fuzz_"), "fuzz_");
    }

    #[test]
    fn no_collisions_produces_empty_plan() {
        let f = file(vec![msg("M", vec![field("fooBar", 1), field("baz", 2)])]);
        let plan = plan_field_names(&[f]);
        assert!(plan.field_renames.is_empty());
        assert!(plan.oneof_keep_verbatim.is_empty());
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn changed_field_is_suffixed_unchanged_keeps_name() {
        let f = file(vec![msg(
            "M",
            vec![field("user_name", 11), field("userName", 12)],
        )]);
        let plan = plan_field_names(&[f]);
        // The already-snake field keeps its name (no entry needed).
        assert!(!plan
            .field_renames
            .contains_key(&("user_name".to_string(), 11)));
        assert_eq!(
            plan.field_renames.get(&("userName".to_string(), 12)),
            Some(&"user_name_f12".to_string())
        );
        assert_eq!(plan.warnings.len(), 1);
    }

    #[test]
    fn two_changed_fields_both_suffixed() {
        let f = file(vec![msg("M", vec![field("fooBar", 1), field("FooBar", 2)])]);
        let plan = plan_field_names(&[f]);
        assert_eq!(
            plan.field_renames.get(&("fooBar".to_string(), 1)),
            Some(&"foo_bar_f1".to_string())
        );
        assert_eq!(
            plan.field_renames.get(&("FooBar".to_string(), 2)),
            Some(&"foo_bar_f2".to_string())
        );
    }

    #[test]
    fn colliding_oneof_keeps_verbatim() {
        let mut m = msg("M", vec![field("data_value", 1)]);
        m.oneof_decl.push(OneofDescriptorProto {
            name: Some("dataValue".to_string()),
            ..Default::default()
        });
        let mut member = field("inner", 2);
        member.oneof_index = Some(0);
        m.field.push(member);
        let plan = plan_field_names(&[file(vec![m])]);
        assert!(plan.oneof_keep_verbatim.contains("dataValue"));
        assert!(plan.field_renames.is_empty());
    }

    #[test]
    fn suffix_collision_reverts_group_to_verbatim() {
        // `userName = 12` would suffix to `user_name_f12`, which collides
        // with the literal third field — everything changed reverts.
        let f = file(vec![msg(
            "M",
            vec![
                field("user_name", 11),
                field("userName", 12),
                field("user_name_f12", 13),
            ],
        )]);
        let plan = plan_field_names(&[f]);
        assert_eq!(
            plan.field_renames.get(&("userName".to_string(), 12)),
            Some(&"userName".to_string())
        );
    }

    #[test]
    fn inherited_adjustment_warns_for_contaminated_message() {
        // Message A collides (userName=12 vs user_name=11); message B has a
        // lone userName=12 and inherits the `_f12` suffix through the global
        // key — it must get its own warning, not a silent rename.
        let f = file(vec![
            msg("A", vec![field("user_name", 11), field("userName", 12)]),
            msg("B", vec![field("userName", 12)]),
        ]);
        let plan = plan_field_names(&[f]);
        let warned: Vec<&str> = plan
            .warnings
            .iter()
            .map(|w| match w {
                CodeGenWarning::IdiomaticFieldNamesAdjusted { message_name, .. } => {
                    message_name.as_str()
                }
                _ => unreachable!(),
            })
            .collect();
        assert!(warned.contains(&"pkg.A"), "{warned:?}");
        assert!(warned.contains(&"pkg.B"), "{warned:?}");
        // A message with the same name at a DIFFERENT number is untouched
        // and must not warn.
        let f2 = file(vec![
            msg("A", vec![field("user_name", 11), field("userName", 12)]),
            msg("C", vec![field("userName", 7)]),
        ]);
        let plan2 = plan_field_names(&[f2]);
        assert_eq!(plan2.warnings.len(), 1);
    }

    #[test]
    fn nested_messages_are_planned() {
        let mut outer = msg("Outer", vec![]);
        outer
            .nested_type
            .push(msg("Inner", vec![field("a_b", 1), field("aB", 2)]));
        let plan = plan_field_names(&[file(vec![outer])]);
        assert_eq!(
            plan.field_renames.get(&("aB".to_string(), 2)),
            Some(&"a_b_f2".to_string())
        );
        assert!(matches!(
            &plan.warnings[0],
            CodeGenWarning::IdiomaticFieldNamesAdjusted { message_name, .. }
                if message_name == "pkg.Outer.Inner"
        ));
    }

    #[test]
    fn synthetic_oneof_is_not_a_namespace_member() {
        // A proto3 optional field's synthetic oneof must not reserve a name.
        let mut m = msg("M", vec![]);
        let mut opt = field("fooBar", 1);
        opt.oneof_index = Some(0);
        opt.proto3_optional = Some(true);
        m.field.push(opt);
        m.field.push(field("foo_bar", 2));
        m.oneof_decl.push(OneofDescriptorProto {
            name: Some("_fooBar".to_string()),
            ..Default::default()
        });
        let plan = plan_field_names(&[file(vec![m])]);
        // The proto3-optional field is a struct member and collides with the
        // literal snake field; the synthetic oneof itself does not.
        assert_eq!(
            plan.field_renames.get(&("fooBar".to_string(), 1)),
            Some(&"foo_bar_f1".to_string())
        );
        assert!(plan.oneof_keep_verbatim.is_empty());
    }
}

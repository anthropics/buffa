//! Which messages are generated with [`CodecStrategy::Table`], and what each
//! table field looks like.
//!
//! A message can use the table if the interpreters in `buffa::table` cover
//! every field it has, with one exception that depends on the messages it
//! holds: a message with a bytes field of a non-default type would lose its
//! zero-copy decode inside a table message, so the messages that hold one,
//! directly or through other messages, stay unrolled. Any other child is
//! reached through its table or its `Message` impl, so it need not be a table
//! message itself.

use std::collections::{HashMap, HashSet};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, FileDescriptorProto};
use crate::impl_message::{
    effective_type, field_bytes_repr, is_explicit_presence_scalar, is_field_packed,
    is_real_oneof_member, is_required_field, map_value_bytes_repr,
};
use crate::message::{find_map_entry, is_closed_enum, map_entry_key_type, map_entry_value_type};
use crate::{CodeGenError, CodeGenWarning, CodecStrategy, TableCodecFallbackReason};

/// The cardinality half of a field's `buffa::table::Kind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Card {
    Implicit,
    Required,
    Optional,
    Repeated,
    Packed,
}

impl Card {
    fn name(self) -> &'static str {
        match self {
            Card::Implicit => "Implicit",
            Card::Required => "Required",
            Card::Optional => "Optional",
            Card::Repeated => "Repeated",
            Card::Packed => "Packed",
        }
    }
}

/// A field of a message that can use the table, in the terms the table uses.
pub(crate) struct TableField<'a> {
    pub(crate) field: &'a FieldDescriptorProto,
    pub(crate) number: u32,
    pub(crate) ty: Type,
    pub(crate) card: Card,
    /// The name of the `buffa::table::Kind` variant of this field's value. For
    /// a oneof member, the emitter wraps it in the member kind that `oneof`
    /// says it needs.
    pub(crate) kind: String,
    /// For an enum field: whether the enum is closed.
    pub(crate) closed_enum: bool,
    /// For a oneof member, the oneof it belongs to.
    pub(crate) oneof: Option<OneofMembership<'a>>,
}

/// The oneof that a [`TableField`] is a member of.
pub(crate) struct OneofMembership<'a> {
    /// The index of the oneof in the message's `oneof_decl`.
    pub(crate) index: usize,
    pub(crate) name: &'a str,
}

/// The `Kind` variant name of a field type, or `None` for a group, which has
/// no kind.
fn type_stem(ty: Type, card: Card) -> Option<&'static str> {
    Some(match ty {
        Type::TYPE_INT32 => "Int32",
        Type::TYPE_INT64 => "Int64",
        Type::TYPE_UINT32 => "Uint32",
        Type::TYPE_UINT64 => "Uint64",
        Type::TYPE_SINT32 => "Sint32",
        Type::TYPE_SINT64 => "Sint64",
        Type::TYPE_BOOL => "Bool",
        Type::TYPE_FIXED32 => "Fixed32",
        Type::TYPE_FIXED64 => "Fixed64",
        Type::TYPE_SFIXED32 => "Sfixed32",
        Type::TYPE_SFIXED64 => "Sfixed64",
        Type::TYPE_FLOAT => "Float",
        Type::TYPE_DOUBLE => "Double",
        Type::TYPE_STRING => "Str",
        Type::TYPE_BYTES => "Bytes",
        Type::TYPE_ENUM => "Enum",
        // A message has one kind per cardinality class, not per `Card`.
        Type::TYPE_MESSAGE if card == Card::Repeated => return Some("MsgRepeated"),
        Type::TYPE_MESSAGE => return Some("MsgSingular"),
        Type::TYPE_GROUP => return None,
    })
}

/// Why a message cannot use the table.
#[derive(Clone, Debug)]
pub(crate) struct Ineligible {
    /// The reason in a few words, which the summary warning groups messages
    /// by.
    pub(crate) reason: String,
    /// The reason for this message, naming the field or type.
    pub(crate) detail: String,
}

/// An [`Ineligible`] whose reason needs no more detail.
fn same(reason: &str) -> Ineligible {
    ineligible(reason, format!("it {reason}"))
}

fn ineligible(reason: impl Into<String>, detail: impl Into<String>) -> Ineligible {
    Ineligible {
        reason: reason.into(),
        detail: detail.into(),
    }
}

/// The table view of the fields of `msg`, or why it cannot use the table,
/// judged without regard to the other messages.
///
/// `fqn` is the message's proto path with a leading dot.
pub(crate) fn table_fields<'a>(
    ctx: &CodeGenContext,
    msg: &'a DescriptorProto,
    fqn: &str,
    features: &ResolvedFeatures,
) -> Result<Vec<TableField<'a>>, Ineligible> {
    if msg
        .options
        .as_option()
        .and_then(|o| o.message_set_wire_format)
        .unwrap_or(false)
    {
        return Err(same("uses the MessageSet wire format"));
    }
    // With JSON and extension ranges the unknown fields sit in a wrapper
    // struct that the table cannot address as `UnknownFields`.
    if ctx.config.generate_json
        && !msg.extension_range.is_empty()
        && ctx.preserve_unknown_fields(fqn)
    {
        return Err(same("has extension ranges and JSON code is generated"));
    }

    let mut fields = Vec::with_capacity(msg.field.len());
    for f in &msg.field {
        let name = f.name.as_deref().unwrap_or("");
        let oneof = if is_real_oneof_member(f) {
            let index = f.oneof_index.and_then(|i| usize::try_from(i).ok());
            let decl = index.and_then(|i| msg.oneof_decl.get(i));
            let Some((index, decl)) = index.zip(decl) else {
                return Err(ineligible(
                    "has a field in a oneof that does not exist",
                    format!("field `{name}` names a oneof that the message does not declare"),
                ));
            };
            Some((index, decl.name.as_deref().unwrap_or("")))
        } else {
            None
        };
        if find_map_entry(msg, f).is_some() {
            return Err(ineligible(
                "has a map field",
                format!("field `{name}` is a map"),
            ));
        }
        let ty = effective_type(ctx, f, features);
        let field_fqn = format!("{fqn}.{name}");
        let repeated = f.label.unwrap_or_default() == Label::LABEL_REPEATED;
        let custom = match ty {
            Type::TYPE_STRING => !ctx.string_repr(&field_fqn).is_default(),
            Type::TYPE_BYTES => !ctx.bytes_repr(&field_fqn).is_default(),
            _ => false,
        } || (repeated && !ctx.repeated_repr(&field_fqn).is_default());
        if custom {
            return Err(ineligible(
                "has a field with a custom string, bytes or collection type",
                format!("field `{name}` has a custom string, bytes or collection type"),
            ));
        }
        let number = crate::impl_message::validated_field_number(f)
            .map_err(|e| ineligible("has an invalid field number", e.to_string()))?;
        // A oneof member's value is written whenever the member is set.
        let card = if oneof.is_some() {
            Card::Required
        } else if repeated {
            if is_field_packed(f, features) {
                Card::Packed
            } else {
                Card::Repeated
            }
        } else if is_explicit_presence_scalar(f, ty, features) {
            Card::Optional
        } else if is_required_field(f, features) {
            Card::Required
        } else {
            Card::Implicit
        };
        let closed_enum = ty == Type::TYPE_ENUM
            && is_closed_enum(&crate::features::resolve_field(ctx, f, features));
        let Some(stem) = type_stem(ty, card) else {
            return Err(ineligible(
                "has a group field",
                format!("field `{name}` is a group"),
            ));
        };
        let kind = if ty == Type::TYPE_MESSAGE {
            stem.to_string()
        } else {
            format!("{stem}{}", card.name())
        };
        fields.push(TableField {
            field: f,
            number,
            ty,
            card,
            kind,
            closed_enum,
            oneof: oneof.map(|(index, name)| OneofMembership { index, name }),
        });
    }
    fields.sort_by_key(|f| f.number);
    Ok(fields)
}

/// One message of the run and what the plan needs to know about it.
struct Candidate<'a> {
    fqn: String,
    fields: Result<Vec<TableField<'a>>, Ineligible>,
    /// Whether a `bytes` field or map value of the message has a non-default
    /// type.
    own_bytes: bool,
    /// The proto paths of the message types of its fields: singular,
    /// repeated, in a oneof, as a map value, or a group.
    holds: Vec<String>,
}

/// Whether `msg` has a `bytes` field, or a map with `bytes` values, stored
/// as `bytes::Bytes` or a custom type.
fn has_non_default_bytes(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    fqn: &str,
    features: &ResolvedFeatures,
) -> bool {
    let proto_fqn = fqn.trim_start_matches('.');
    msg.field.iter().any(|f| {
        let name = f.name.as_deref().unwrap_or("");
        let repr = match find_map_entry(msg, f) {
            Some(entry) => map_value_bytes_repr(
                ctx,
                map_entry_key_type(ctx, entry, features),
                map_entry_value_type(ctx, entry, features),
                proto_fqn,
                name,
            ),
            None if effective_type(ctx, f, features) == Type::TYPE_BYTES => {
                field_bytes_repr(ctx, proto_fqn, name)
            }
            None => return false,
        };
        !repr.is_default()
    })
}

/// The proto paths of the message types `msg` has fields of.
fn held_messages(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    features: &ResolvedFeatures,
) -> Vec<String> {
    msg.field
        .iter()
        .filter_map(|f| {
            if let Some(entry) = find_map_entry(msg, f) {
                if map_entry_value_type(ctx, entry, features) != Some(Type::TYPE_MESSAGE) {
                    return None;
                }
                let value = entry.field.iter().find(|v| v.number == Some(2))?;
                return value.type_name.clone();
            }
            matches!(
                effective_type(ctx, f, features),
                Type::TYPE_MESSAGE | Type::TYPE_GROUP
            )
            .then(|| f.type_name.clone())
            .flatten()
        })
        .collect()
}

/// Every message of `messages` and the messages nested in them that has a
/// `Message` impl and is generated by this crate, with its parents' `scope`.
///
/// A message that another crate generates, though this run also holds its
/// descriptor, is not collected, because the types that refer to it name the
/// other crate's copy.
///
/// `features` is what `message.rs` gives each message's scope: the file's
/// features for a top-level message, whose own message-level features do not
/// apply to it, and the parent's features with the message's own for a nested
/// one. The emitter recomputes a message's fields under that scope, so the plan
/// must judge them under the same one.
fn collect<'a>(
    ctx: &CodeGenContext,
    messages: &'a [DescriptorProto],
    (package, scope): (&str, &str),
    parent_features: &ResolvedFeatures,
    top_level: bool,
    out: &mut Vec<Candidate<'a>>,
    group_types: &mut HashSet<String>,
) {
    for msg in messages {
        let is_map_entry = msg
            .options
            .as_option()
            .is_some_and(|o| o.map_entry.unwrap_or(false));
        let fqn = format!("{scope}.{}", msg.name.as_deref().unwrap_or(""));
        let features = crate::features::message_scope_features(parent_features, msg, top_level);
        let is_extern = ctx
            .rust_type_relative(&fqn, package, 0)
            .is_some_and(|path| path.starts_with("::") || path.starts_with("crate::"));
        if !is_map_entry && !is_extern {
            for f in &msg.field {
                if effective_type(ctx, f, &features) == Type::TYPE_GROUP {
                    group_types.extend(f.type_name.clone());
                }
            }
            out.push(Candidate {
                fields: table_fields(ctx, msg, &fqn, &features),
                own_bytes: has_non_default_bytes(ctx, msg, &fqn, &features),
                holds: held_messages(ctx, msg, &features),
                fqn: fqn.clone(),
            });
        }
        collect(
            ctx,
            &msg.nested_type,
            (package, &fqn),
            &features,
            false,
            out,
            group_types,
        );
    }
}

/// The messages that hold, directly or through other messages, one that this
/// run generates with a `bytes` field of a non-default type, each with the
/// message it holds that leads there.
///
/// The table decodes over one contiguous `&[u8]`, where `Buf::copy_to_bytes`
/// copies, so a `Bytes` field of such a child would stop being decoded
/// without a copy. A child of another crate is not a candidate and is not
/// inspected.
fn holders_of_non_default_bytes<'a>(candidates: &'a [Candidate<'_>]) -> HashMap<&'a str, &'a str> {
    let mut tainted: HashSet<&str> = candidates
        .iter()
        .filter(|c| c.own_bytes)
        .map(|c| c.fqn.as_str())
        .collect();
    let mut holders = HashMap::new();
    loop {
        let before = tainted.len();
        for c in candidates {
            if tainted.contains(c.fqn.as_str()) {
                continue;
            }
            if let Some(child) = c.holds.iter().find(|h| tainted.contains(h.as_str())) {
                tainted.insert(&c.fqn);
                holders.insert(c.fqn.as_str(), child.as_str());
            }
        }
        if tainted.len() == before {
            return holders;
        }
    }
}

/// The messages generated with the table codec in one run.
#[derive(Default)]
pub(crate) struct TablePlan {
    /// Proto paths with a leading dot.
    tables: HashSet<String>,
}

impl TablePlan {
    pub(crate) fn contains(&self, fqn: &str) -> bool {
        self.tables.contains(fqn)
    }
}

/// Decide which messages of `files_to_generate` use the table codec.
///
/// Returns the plan and a summary warning about the messages that asked for
/// the table and cannot have it.
///
/// # Errors
///
/// A `codec_strategy_in` rule that names, by its exact path, a message that
/// cannot use the table.
pub(crate) fn plan(
    ctx: &CodeGenContext,
    files: &[FileDescriptorProto],
    files_to_generate: &[String],
) -> Result<(TablePlan, Vec<CodeGenWarning>), CodeGenError> {
    let mut candidates = Vec::new();
    let mut group_types = HashSet::new();
    for file in files.iter().filter(|f| {
        f.name
            .as_deref()
            .is_some_and(|n| files_to_generate.iter().any(|g| g == n))
    }) {
        let package = file.package.as_deref().unwrap_or("");
        let scope = if package.is_empty() {
            String::new()
        } else {
            format!(".{package}")
        };
        collect(
            ctx,
            &file.message_type,
            (package, &scope),
            &crate::features::for_file(file),
            true,
            &mut candidates,
            &mut group_types,
        );
    }

    let bytes_holders = holders_of_non_default_bytes(&candidates);

    // For each message that asked for the table, the table if it can have
    // one, and otherwise the reason it cannot, which an exact rule turns into
    // an error and the summary counts.
    let mut tables = HashSet::new();
    let mut selected = 0;
    let mut errors = Vec::new();
    let mut summary: Vec<TableCodecFallbackReason> = Vec::new();
    let mut fallbacks = 0;
    for c in &candidates {
        if ctx.codec_strategy(&c.fqn) != CodecStrategy::Table {
            continue;
        }
        selected += 1;
        let why = match &c.fields {
            Err(why) => why.clone(),
            Ok(_) if group_types.contains(&c.fqn) => same("is the type of a group field"),
            Ok(_) if bytes_holders.contains_key(c.fqn.as_str()) => ineligible(
                "holds a message with bytes fields of a non-default type",
                format!(
                    "it holds `{}`, which has bytes fields of a non-default type or holds a \
                     message that has",
                    bytes_holders[c.fqn.as_str()]
                ),
            ),
            Ok(_) => {
                tables.insert(c.fqn.clone());
                continue;
            }
        };
        // A rule that names a message exactly and cannot be honoured is an
        // error, and all of them are reported together. The rest are counted
        // by reason in one warning.
        if let Some((rule, _)) = ctx.codec_strategy_rule(&c.fqn) {
            if *rule == c.fqn {
                errors.push(format!(
                    "codec_strategy_in rule '{rule}' selects the table codec for a message that \
                     cannot use it: {}. Select the unrolled codec for it instead (buffa-build: \
                     `.codec_strategy_in(CodecStrategy::Unrolled, &[\"{rule}\"])`; plugin: \
                     `codec_strategy_in={rule}=unrolled`), or remove the rule",
                    why.detail
                ));
                continue;
            }
        }
        fallbacks += 1;
        let entry = match summary.iter().position(|r| r.reason == why.reason) {
            Some(index) => &mut summary[index],
            None => {
                summary.push(TableCodecFallbackReason {
                    reason: why.reason.clone(),
                    messages: Vec::new(),
                });
                summary.last_mut().expect("just pushed")
            }
        };
        entry.messages.push(c.fqn.clone());
    }
    if !errors.is_empty() {
        return Err(CodeGenError::Other(errors.join("\n")));
    }
    let mut warnings = Vec::new();
    if fallbacks > 0 {
        summary.sort_by_key(|reason| std::cmp::Reverse(reason.messages.len()));
        warnings.push(CodeGenWarning::TableCodecFallbackSummary {
            fallbacks,
            selected,
            reasons: summary,
        });
    }
    Ok((TablePlan { tables }, warnings))
}

//! Which messages are generated with [`CodecStrategy::Table`], and what each
//! table field looks like.
//!
//! A message can use the table if the interpreters in `buffa::table` cover
//! every field and every message it holds is a table message too, because a
//! table records the tables of its children. The set of table messages is the
//! largest set of requested, locally eligible messages closed under that
//! rule, found by removing messages until none is left holding a non-table
//! child.

use std::collections::{HashMap, HashSet};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, FileDescriptorProto};
use crate::impl_message::{
    effective_type, is_explicit_presence_scalar, is_field_packed, is_real_oneof_member,
    is_required_field,
};
use crate::message::{find_map_entry, is_closed_enum};
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
    /// The name of the `buffa::table::Kind` variant of this field.
    pub(crate) kind: String,
    /// For an enum field: whether the enum is closed.
    pub(crate) closed_enum: bool,
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
    /// by. For a message that holds another that cannot use the table, this
    /// includes the other's reason.
    pub(crate) reason: String,
    /// The reason for this message, naming the field or type.
    pub(crate) detail: String,
    /// The message's own reason if it is the cause of a fallback, and
    /// otherwise the cause of the message it holds, which is followed down to
    /// the message that cannot use the table itself.
    pub(crate) root: String,
    /// Whether the fallback follows from a strategy the user chose (a message
    /// it holds is set to `Unrolled`), so that it needs no warning.
    pub(crate) silent: bool,
    /// What to do about it when a rule that names the message exactly asked
    /// for the table, if it differs from the general advice.
    pub(crate) hint: Option<String>,
}

/// An [`Ineligible`] whose reason needs no more detail.
fn same(reason: &str) -> Ineligible {
    ineligible(reason, format!("it {reason}"))
}

fn ineligible(reason: impl Into<String>, detail: impl Into<String>) -> Ineligible {
    let reason = reason.into();
    Ineligible {
        root: reason.clone(),
        reason,
        detail: detail.into(),
        silent: false,
        hint: None,
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
        if is_real_oneof_member(f) {
            return Err(ineligible(
                "has a oneof",
                format!("field `{name}` is in a oneof"),
            ));
        }
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
        let card = if repeated {
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
        });
    }
    fields.sort_by_key(|f| f.number);
    Ok(fields)
}

/// One message of the run and what the plan needs to know about it.
struct Candidate<'a> {
    fqn: String,
    /// The proto paths of the message types of its fields.
    children: Vec<String>,
    fields: Result<Vec<TableField<'a>>, Ineligible>,
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
            let mut children = Vec::new();
            for f in &msg.field {
                match effective_type(ctx, f, &features) {
                    Type::TYPE_MESSAGE => children.extend(f.type_name.clone()),
                    Type::TYPE_GROUP => group_types.extend(f.type_name.clone()),
                    _ => {}
                }
            }
            out.push(Candidate {
                fields: table_fields(ctx, msg, &fqn, &features),
                fqn: fqn.clone(),
                children,
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

/// Why a message that holds `child` cannot use the table, when `child` has no
/// table.
fn child_without_table(
    ctx: &CodeGenContext,
    child: &str,
    reasons: &HashMap<&str, Ineligible>,
    generated: &HashSet<&str>,
) -> Ineligible {
    if let Some(held) = reasons.get(child) {
        return Ineligible {
            reason: format!("holds a message that {}", held.root),
            detail: format!(
                "it has a field of message type `{child}`, which {}",
                held.root
            ),
            root: held.root.clone(),
            silent: held.silent,
            hint: held.hint.clone(),
        };
    }
    if !generated.contains(child) {
        return ineligible(
            "holds a message that another crate or run generates",
            format!("it has a field of message type `{child}`, which is not generated here"),
        );
    }
    // Not selected: the strategy for it is unrolled, by the user's rule or by
    // the global default.
    if ctx.codec_strategy_rule(child).is_some() {
        Ineligible {
            silent: true,
            ..ineligible(
                "holds a message set to the unrolled codec",
                format!(
                    "it has a field of message type `{child}`, which is set to the unrolled codec"
                ),
            )
        }
    } else {
        Ineligible {
            hint: Some(format!(
                "Select `{child}` as well, and every message it holds (buffa-build: \
                 `.codec_strategy_in(CodecStrategy::Table, &[\"{child}\"])`; plugin: \
                 `codec_strategy_in={child}=table`)"
            )),
            ..ineligible(
                "holds a message not selected for the table",
                format!("it has a field of message type `{child}`, which is not selected for the table codec"),
            )
        }
    }
}

/// Decide which messages of `files_to_generate` use the table codec.
///
/// Returns the plan and a summary warning about the messages that asked for
/// the table and cannot have it, unless the user's own choice of `Unrolled`
/// for a message they hold is the only reason.
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
    let generated: HashSet<&str> = candidates.iter().map(|c| c.fqn.as_str()).collect();

    // The messages that asked for the table, each with the reason it cannot
    // have it, if there is one.
    let mut reasons: HashMap<&str, Ineligible> = HashMap::new();
    let mut selected: Vec<&Candidate> = Vec::new();
    for c in &candidates {
        if ctx.codec_strategy(&c.fqn) != CodecStrategy::Table {
            continue;
        }
        selected.push(c);
        if let Err(why) = &c.fields {
            reasons.insert(&c.fqn, why.clone());
        } else if group_types.contains(&c.fqn) {
            reasons.insert(&c.fqn, same("is the type of a group field"));
        }
    }

    // Remove every message that holds a child without a table, until none is
    // left.
    let mut remaining: HashSet<&str> = selected
        .iter()
        .filter(|c| !reasons.contains_key(c.fqn.as_str()))
        .map(|c| c.fqn.as_str())
        .collect();
    loop {
        let mut removed = Vec::new();
        for c in selected
            .iter()
            .filter(|c| remaining.contains(c.fqn.as_str()))
        {
            // The reason for the first child without a table that the user did
            // not choose, if there is one, and otherwise for the first without
            // one at all.
            let whys: Vec<Ineligible> = c
                .children
                .iter()
                .filter(|ch| !remaining.contains(ch.as_str()))
                .map(|child| child_without_table(ctx, child, &reasons, &generated))
                .collect();
            if let Some(why) = whys.iter().find(|w| !w.silent).or(whys.first()) {
                removed.push((c.fqn.as_str(), why.clone()));
            }
        }
        if removed.is_empty() {
            break;
        }
        for (fqn, why) in removed {
            remaining.remove(fqn);
            reasons.insert(fqn, why);
        }
    }

    // A holder removed early may have looked like it fell back only because of
    // a message the user set to `Unrolled`, before a message it also holds was
    // itself removed for a reason of its own. Look again with the final
    // reasons, until no silent holder changes.
    loop {
        let mut changed = Vec::new();
        for c in selected.iter().filter(|c| {
            reasons
                .get(c.fqn.as_str())
                .is_some_and(|why| why.silent && !c.children.is_empty())
        }) {
            let loud = c
                .children
                .iter()
                .filter(|ch| !remaining.contains(ch.as_str()))
                .map(|child| child_without_table(ctx, child, &reasons, &generated))
                .find(|why| !why.silent);
            if let Some(why) = loud {
                changed.push((c.fqn.as_str(), why));
            }
        }
        if changed.is_empty() {
            break;
        }
        reasons.extend(changed);
    }

    // A rule that names a message exactly and cannot be honoured is an error,
    // and all of them are reported together. The rest are counted by reason in
    // one warning, except for the messages whose fallback the user chose.
    let mut errors = Vec::new();
    let mut summary: Vec<TableCodecFallbackReason> = Vec::new();
    let mut fallbacks = 0;
    let mut held_back = 0;
    for c in &selected {
        let Some(why) = reasons.get(c.fqn.as_str()) else {
            continue;
        };
        if let Some((rule, _)) = ctx.codec_strategy_rule(&c.fqn) {
            if *rule == c.fqn {
                let advice = why.hint.clone().unwrap_or_else(|| {
                    format!(
                        "Select the unrolled codec for it instead (buffa-build: \
                         `.codec_strategy_in(CodecStrategy::Unrolled, &[\"{rule}\"])`; plugin: \
                         `codec_strategy_in={rule}=unrolled`), or remove the rule"
                    )
                });
                errors.push(format!(
                    "codec_strategy_in rule '{rule}' selects the table codec for a message that \
                     cannot use it: {}. {advice}",
                    why.detail
                ));
                continue;
            }
        }
        if why.silent {
            held_back += 1;
            continue;
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
            selected: selected.len() - held_back,
            reasons: summary,
        });
    }

    let tables = remaining.into_iter().map(str::to_string).collect();
    Ok((TablePlan { tables }, warnings))
}

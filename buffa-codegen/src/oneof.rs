//! Oneof enum code generation.

use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, OneofDescriptorProto,
};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::impl_message::{field_bytes_repr, field_string_repr};
use crate::message::scalar_or_message_type_nested;
use crate::CodeGenError;

/// Returns `true` when a field's type is `google.protobuf.NullValue`.
///
/// NullValue requires special serde handling: JSON `null` is the valid
/// value (not "variant absent"), and serialization must emit `null`
/// rather than the enum string name.
pub(crate) fn is_null_value_field(field: &FieldDescriptorProto) -> bool {
    field.type_name.as_deref() == Some(".google.protobuf.NullValue")
}

/// Returns `true` when a field's type treats JSON `null` as a valid value
/// rather than "variant absent".
///
/// This covers:
/// - `google.protobuf.NullValue` (the enum): `null` is THE value
/// - `google.protobuf.Value` (the message): `null` represents `Kind::NullValue`
///
/// For these types, deserialization must NOT wrap in `NullableDeserializeSeed`
/// (which intercepts `null` as `None`), but instead forward `null` to the
/// type's own `Deserialize` impl.
pub(crate) fn null_is_valid_value(field: &FieldDescriptorProto) -> bool {
    matches!(
        field.type_name.as_deref(),
        Some(".google.protobuf.NullValue") | Some(".google.protobuf.Value")
    )
}

/// Returns `true` for oneof variant types that are heap-allocated via `Box`.
///
/// Message and group variants are always boxed so that recursive types
/// (e.g. `Type { oneof kind { Type type = 1; } }`) compile. This matches
/// Go's `protoc-gen-go` which emits pointers for message-typed oneof fields,
/// and is consistent with `MessageField<T>` being `Option<Box<T>>` for
/// singular message fields.
pub(crate) fn is_boxed_variant(ty: Type) -> bool {
    matches!(ty, Type::TYPE_MESSAGE | Type::TYPE_GROUP)
}

/// Returns `true` when a oneof variant is stored in a `Box`.
///
/// Message and group variants box by default (see [`is_boxed_variant`]); a
/// matching `config.unboxed_oneof_fields` rule opts the variant out, storing
/// it inline. `variant_fqn` is the leading-dot variant path, e.g.
/// `".my.pkg.MyMessage.body.small"`. Lookups go through the resolved set
/// built by [`resolve_unboxed_variants`], which never contains recursive
/// variants, so callers can store the value inline without a further
/// unsized-type check.
pub(crate) fn variant_boxed(ctx: &CodeGenContext, ty: Type, variant_fqn: &str) -> bool {
    is_boxed_variant(ty) && !ctx.oneof_unboxed(variant_fqn)
}

/// Resolve `config.unboxed_oneof_fields` rules into the concrete set of
/// variant paths (leading-dot form) that are stored inline.
///
/// A variant lands in the set when a rule matches it AND inlining it would
/// not create an unsized type. Recursive variants matched by a *prefix* rule
/// (including the `"."` blanket from `Config::unbox_oneof()`) are silently
/// kept boxed — the blanket is documented as "every non-recursive variant".
/// A rule that names a recursive variant *exactly* still errors, in
/// `collect_variant_info`, which detects "exact rule but not in this set".
///
/// Resolving once up front (at context construction) keeps every downstream
/// boxing site consistent with the enum declaration and builds the message
/// index a single time.
pub(crate) fn resolve_unboxed_variants(
    index: &std::collections::HashMap<String, &DescriptorProto>,
    rules: &[String],
    pointer_fields: &[(String, crate::PointerRepr)],
) -> std::collections::HashSet<String> {
    let mut resolved = std::collections::HashSet::new();
    if rules.is_empty() {
        return resolved;
    }
    for (msg_fqn, msg) in index {
        for_each_message_variant(msg, msg_fqn, |variant_fqn, type_name| {
            if rule_matches(rules, &variant_fqn)
                && !inline_is_recursive(index, rules, pointer_fields, msg_fqn, type_name)
            {
                resolved.insert(variant_fqn);
            }
        });
    }
    resolved
}

/// Resolve the set of singular message-field paths whose configured
/// [`PointerRepr`](crate::PointerRepr) is `Inline` and which can safely be
/// stored inline (no cycle through inline edges).
///
/// Mirrors [`resolve_unboxed_variants`]: a field where the raw last-match-wins
/// repr is `Inline` is added unless [`inline_is_recursive`] reports a cycle
/// through inlined oneof variants and/or other inline singular fields. The
/// resulting set is the source of truth for
/// [`CodeGenContext::pointer_repr`](crate::context::CodeGenContext::pointer_repr),
/// which demotes any `Inline` not in this set to `Box`.
///
/// Runs unconditionally now that `Inline` is the default. Cost is `O(F·(V+E))`
/// (a fresh DFS per candidate field); memoize per-target reachable sets if a
/// very large schema makes this noticeable.
pub(crate) fn resolve_inlined_fields(
    index: &std::collections::HashMap<String, &DescriptorProto>,
    rules: &[String],
    pointer_fields: &[(String, crate::PointerRepr)],
) -> std::collections::HashSet<String> {
    let mut resolved = std::collections::HashSet::new();
    for (msg_fqn, msg) in index {
        for_each_singular_message_field(msg, msg_fqn, |field_fqn, type_name| {
            if raw_pointer_repr(pointer_fields, &field_fqn) == crate::PointerRepr::Inline
                && !inline_is_recursive(index, rules, pointer_fields, msg_fqn, type_name)
            {
                resolved.insert(field_fqn);
            }
        });
    }
    resolved
}

/// Whether any `unboxed_oneof_fields` rule matches the variant path.
fn rule_matches(rules: &[String], variant_fqn: &str) -> bool {
    rules
        .iter()
        .any(|prefix| crate::context::matches_proto_prefix(prefix, variant_fqn))
}

/// Last-match-wins [`PointerRepr`](crate::PointerRepr) for `field_fqn` from the
/// raw `pointer_fields` config — no recursion demotion. The recursion check
/// uses this (not the post-demotion resolver) so the walk is order-independent.
fn raw_pointer_repr(
    pointer_fields: &[(String, crate::PointerRepr)],
    field_fqn: &str,
) -> crate::PointerRepr {
    pointer_fields
        .iter()
        .rev()
        .find(|(prefix, _)| crate::context::matches_proto_prefix(prefix, field_fqn))
        .map_or(crate::PointerRepr::default(), |(_, repr)| repr.clone())
}

/// Invoke `f` for every message/group-typed real oneof member of `msg`, with
/// the variant's leading-dot path and its target message name (no leading
/// dot). `msg_fqn` has no leading dot.
///
/// Fields with a missing name, oneof index, or type name are skipped rather
/// than surfaced as errors: protoc always populates them for real oneof
/// members, and `collect_variant_info` independently reports `MissingField`
/// for any descriptor malformed enough to hit this in practice.
fn for_each_message_variant(msg: &DescriptorProto, msg_fqn: &str, mut f: impl FnMut(String, &str)) {
    for field in &msg.field {
        if !crate::impl_message::is_real_oneof_member(field) {
            continue;
        }
        if !is_boxed_variant(field.r#type.unwrap_or_default()) {
            continue;
        }
        let (Some(oneof_idx), Some(field_name), Some(type_name)) = (
            field.oneof_index,
            field.name.as_deref(),
            field.type_name.as_deref(),
        ) else {
            continue;
        };
        let Some(oneof_name) = usize::try_from(oneof_idx)
            .ok()
            .and_then(|i| msg.oneof_decl.get(i))
            .and_then(|o| o.name.as_deref())
        else {
            continue;
        };
        f(
            format!(".{msg_fqn}.{oneof_name}.{field_name}"),
            type_name.trim_start_matches('.'),
        );
    }
}

/// Invoke `f` for every singular (non-repeated, non-oneof) message/group-typed
/// field of `msg`, with the field's leading-dot path and its target message
/// name (no leading dot). `msg_fqn` has no leading dot.
///
/// These are the fields whose [`PointerRepr`](crate::PointerRepr) governs
/// inline-vs-heap storage; repeated and map fields are heap-backed and break
/// cycles regardless of pointer config.
fn for_each_singular_message_field(
    msg: &DescriptorProto,
    msg_fqn: &str,
    mut f: impl FnMut(String, &str),
) {
    use crate::generated::descriptor::field_descriptor_proto::Label;
    for field in &msg.field {
        if crate::impl_message::is_real_oneof_member(field) {
            continue;
        }
        if !is_boxed_variant(field.r#type.unwrap_or_default()) {
            continue;
        }
        if field.label.unwrap_or_default() == Label::LABEL_REPEATED {
            continue;
        }
        let (Some(field_name), Some(type_name)) =
            (field.name.as_deref(), field.type_name.as_deref())
        else {
            continue;
        };
        f(
            format!(".{msg_fqn}.{field_name}"),
            type_name.trim_start_matches('.'),
        );
    }
}

/// Build a map from fully-qualified message name (no leading dot) to its
/// descriptor, walking every file and its nested types.
pub(crate) fn message_index(
    files: &[FileDescriptorProto],
) -> std::collections::HashMap<String, &DescriptorProto> {
    fn walk<'a>(
        map: &mut std::collections::HashMap<String, &'a DescriptorProto>,
        prefix: &str,
        msg: &'a DescriptorProto,
    ) {
        let Some(name) = msg.name.as_deref() else {
            return;
        };
        let fqn = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        for nested in &msg.nested_type {
            walk(map, &fqn, nested);
        }
        map.insert(fqn, msg);
    }

    let mut map = std::collections::HashMap::new();
    for file in files {
        let package = file.package.as_deref().unwrap_or("");
        for msg in &file.message_type {
            walk(&mut map, package, msg);
        }
    }
    map
}

/// Returns `true` when storing a value of message type `target` inline inside
/// `enclosing` would produce an unsized type.
///
/// `enclosing` and `target` are fully-qualified message names without a leading
/// dot. A cycle is reachable only through edges that store the target inline:
/// message-typed oneof variants matched by an `unboxed_oneof_fields` rule, and
/// singular message fields whose raw [`PointerRepr`](crate::PointerRepr)
/// resolves to `Inline`. Repeated fields, maps, and `Box`/custom-pointer fields
/// are heap-backed and break cycles. The walk follows every *rule-matched* edge
/// from `target`; if it reaches `enclosing`, inlining is recursive.
///
/// Following rule-matched (rather than finally-resolved) edges keeps the check
/// order-independent and conservative: an edge that resolution later keeps
/// boxed (because it is itself part of a cycle) can only cause a false `true`
/// here, which keeps more fields boxed — never an unsized type.
fn inline_is_recursive(
    index: &std::collections::HashMap<String, &DescriptorProto>,
    rules: &[String],
    pointer_fields: &[(String, crate::PointerRepr)],
    enclosing: &str,
    target: &str,
) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![target.to_string()];
    while let Some(current) = stack.pop() {
        if current == enclosing {
            return true;
        }
        if !seen.insert(current.clone()) {
            continue;
        }
        let Some(msg) = index.get(current.as_str()) else {
            continue;
        };
        for_each_message_variant(msg, &current, |variant_fqn, type_name| {
            if rule_matches(rules, &variant_fqn) {
                stack.push(type_name.to_string());
            }
        });
        for_each_singular_message_field(msg, &current, |field_fqn, type_name| {
            if raw_pointer_repr(pointer_fields, &field_fqn) == crate::PointerRepr::Inline {
                stack.push(type_name.to_string());
            }
        });
    }
    false
}

/// Metadata for a single oneof variant.
struct VariantInfo {
    variant_ident: proc_macro2::Ident,
    /// When `bytes_fields` config matches a bytes variant this is
    /// `::buffa::bytes::Bytes`, not `Vec<u8>` — see `collect_variant_info`.
    rust_type: TokenStream,
    json_name: String,
    field_type: Type,
    /// See [`is_null_value_field`].
    is_null_value: bool,
    /// Whether the variant is stored in a `Box` (see [`variant_boxed`]):
    /// message/group types are boxed unless opted out via
    /// `config.unboxed_oneof_fields`.
    is_boxed: bool,
    /// Custom attributes matched via `CodeGenConfig::field_attributes` on the
    /// variant's fully-qualified path (`{oneof_fqn}.{variant_proto_name}`).
    custom_attrs: TokenStream,
    /// Owned bytes representation for a `bytes` variant (default `Vec<u8>`).
    /// Drives both the variant type and the `arbitrary` shim selection.
    bytes_repr: crate::BytesRepr,
    /// Owned string representation for a `string` variant (default `String`).
    /// Drives both the variant type and the `arbitrary` shim selection.
    string_repr: crate::StringRepr,
    /// Owned pointer for a *boxed* message/group variant (default `Box`).
    /// Only consulted when `is_boxed` (an unboxed/inline variant has no
    /// pointer); a custom pointer is selected by the variant's path.
    pointer_repr: crate::PointerRepr,
    /// Variant's field carries `[debug_redact = true]`; the enum's `Debug`
    /// impl prints a placeholder instead of the payload.
    debug_redact: bool,
}

#[allow(clippy::too_many_arguments)]
fn collect_variant_info(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    oneof_name: &str,
    current_package: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    resolver: &crate::imports::ImportResolver,
    nesting: usize,
) -> Result<Vec<VariantInfo>, CodeGenError> {
    let oneof_index = msg
        .oneof_decl
        .iter()
        .position(|o| o.name.as_deref() == Some(oneof_name))
        .ok_or_else(|| CodeGenError::Other(format!("oneof '{oneof_name}' not found in message")))?;

    let fields: Vec<&FieldDescriptorProto> = msg
        .field
        .iter()
        .filter(|f| {
            f.oneof_index == Some(oneof_index as i32) && !f.proto3_optional.unwrap_or(false)
        })
        .collect();

    fields
        .iter()
        .map(|field| {
            let proto_name = field
                .name
                .as_deref()
                .ok_or(CodeGenError::MissingField("field.name"))?;
            let json_name = field.json_name.as_deref().unwrap_or(proto_name).to_string();
            let variant_ident = oneof_variant_ident(proto_name);
            let field_type = crate::impl_message::effective_type(ctx, field, features);
            // bytes_fields config override: scalar_or_message_type_nested goes
            // through scalar_rust_type which hardcodes Vec<u8> for TYPE_BYTES.
            // Encode/size/JSON-serialize all take &[u8] so Bytes deref-coerces
            // without codegen changes; only decode and JSON-deser need an
            // explicit Vec<u8>→Bytes conversion (see oneof_merge_arm and
            // oneof_variant_deser_arm).
            // Oneof enums live at `__buffa::oneof::<msg_path>::`, which is
            // `2 + (msg_nesting + 1)` levels below the package root
            // (sentinel + `oneof` + one snake-case segment per message in
            // the FQN path). `nesting` here is the owning message's
            // msg_nesting, so the enum body sits at `nesting + 3`.
            let bytes_repr = if field_type == Type::TYPE_BYTES {
                field_bytes_repr(ctx, proto_fqn, proto_name)
            } else {
                crate::BytesRepr::Vec
            };
            // Configurable owned string representation for a `string` variant.
            let string_repr = if field_type == Type::TYPE_STRING {
                field_string_repr(ctx, proto_fqn, proto_name)
            } else {
                crate::StringRepr::String
            };
            let rust_type = if field_type == Type::TYPE_BYTES && !bytes_repr.is_default() {
                bytes_repr.type_path(resolver, ctx, nesting + 3)?
            } else if field_type == Type::TYPE_STRING && !string_repr.is_default() {
                string_repr.type_path(resolver, ctx, nesting + 3)?
            } else {
                scalar_or_message_type_nested(
                    ctx,
                    field,
                    current_package,
                    nesting + 3,
                    features,
                    resolver,
                )?
            };
            let variant_fqn = format!("{proto_fqn}.{oneof_name}.{proto_name}");
            let custom_attrs =
                CodeGenContext::matching_attributes(&ctx.config.field_attributes, &variant_fqn)?;
            // Recursive variants never make it into the resolved unboxed set
            // (see `resolve_unboxed_variants`), so a variant that an
            // *exact-path* rule names but that is still boxed can only have
            // been excluded for recursion — reject loudly, the user asked for
            // something impossible. Prefix/blanket rules skip it silently.
            let dotted_fqn = format!(".{variant_fqn}");
            let is_boxed = variant_boxed(ctx, field_type, &dotted_fqn);
            if is_boxed
                && ctx
                    .config
                    .unboxed_oneof_fields
                    .iter()
                    .any(|r| r == &dotted_fqn)
            {
                return Err(CodeGenError::Other(format!(
                    "oneof variant `{variant_fqn}` is recursive and cannot be \
                     stored inline: it would make the generated enum unsized. \
                     Remove `\"{dotted_fqn}\"` from unbox_oneof_in, or use a \
                     broader prefix (or unbox_oneof()) to keep this variant \
                     boxed while inlining the rest."
                )));
            }
            Ok(VariantInfo {
                variant_ident,
                rust_type,
                json_name,
                field_type,
                is_boxed,
                is_null_value: is_null_value_field(field),
                custom_attrs,
                bytes_repr,
                string_repr,
                pointer_repr: ctx.pointer_repr(&dotted_fqn),
                debug_redact: crate::message::is_debug_redacted(field),
            })
        })
        .collect()
}

/// Generate a Rust enum for a protobuf oneof.
///
/// When JSON is enabled, the containing message always gets a hand-generated
/// `Deserialize` impl that handles oneof fields inline (`generate_custom_deserialize`
/// in `message.rs`), so the oneof enum only needs `Serialize`.
#[allow(clippy::too_many_arguments)]
pub fn generate_oneof_enum(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    idx: usize,
    oneof: &OneofDescriptorProto,
    current_package: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    resolver: &crate::imports::ImportResolver,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let rust_enum_ident = match oneof_idents.get(&idx) {
        Some(id) => id.clone(),
        None => return Ok(TokenStream::new()),
    };
    let oneof_name = oneof
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("oneof.name"))?;

    let variants_info = collect_variant_info(
        ctx,
        msg,
        oneof_name,
        current_package,
        proto_fqn,
        features,
        resolver,
        nesting,
    )?;
    if variants_info.is_empty() {
        return Ok(TokenStream::new());
    }

    let variants: Vec<_> = variants_info
        .iter()
        .map(|v| {
            let ident = &v.variant_ident;
            let ty = &v.rust_type;
            let attrs = &v.custom_attrs;
            // The arbitrary crate does not support #[arbitrary(...)] on
            // enum variants — the attribute must be on the inner field.
            // A non-default `bytes`/`string` variant attaches a type-agnostic
            // `Arbitrary` builder (singular form — a variant holds one value),
            // selected by kind rather than concrete type, so the substituted
            // type needs no native `Arbitrary` impl.
            let arbitrary_field_attr = if ctx.config.generate_arbitrary
                && !v.bytes_repr.is_default()
            {
                quote! { #[cfg_attr(feature = "arbitrary", arbitrary(with = ::buffa::__private::arbitrary_proto_bytes))] }
            } else if ctx.config.generate_arbitrary && !v.string_repr.is_default() {
                quote! { #[cfg_attr(feature = "arbitrary", arbitrary(with = ::buffa::__private::arbitrary_proto_string))] }
            } else {
                quote! {}
            };
            if v.is_boxed {
                // Boxed variants are message/group types (see is_boxed_variant),
                // never bytes — so there's no shim to lose here. Lock the
                // invariant in case is_boxed_variant ever broadens.
                debug_assert!(v.bytes_repr.is_default(), "boxed oneof variant cannot be bytes_fields-typed");
                // Default `Box<T>` is byte-identical; a custom pointer wraps the
                // message type. `unboxed_oneof_fields` wins — an unboxed variant
                // (the `else` arm) is stored inline and gets no pointer.
                let ptr = v.pointer_repr.pointer_type(ty)?;
                Ok(quote! { #attrs #ident(#ptr) })
            } else {
                Ok(quote! { #attrs #ident(#arbitrary_field_attr #ty) })
            }
        })
        .collect::<Result<Vec<_>, CodeGenError>>()?;

    // For boxed (message/group) variants, generate `From<T>` so callers can
    // write `Kind::from(msg)` instead of `Kind::Variant(Box::new(msg))`.
    // Skip types that appear as multiple variants (e.g. two `Empty` variants
    // in google.api.expr.v1alpha1.Type.type_kind) — `From` would be ambiguous.
    //
    // Keying by TokenStream::to_string() is safe here: all rust_type values
    // flow through scalar_or_message_type_nested -> rust_path_to_tokens,
    // which produces token streams with identical structure for identical
    // proto type names (so their string representations match).
    let mut type_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for v in variants_info
        .iter()
        .filter(|v| is_boxed_variant(v.field_type))
    {
        *type_counts.entry(v.rust_type.to_string()).or_insert(0) += 1;
    }
    let from_impls: Vec<_> = variants_info
        .iter()
        .filter(|v| is_boxed_variant(v.field_type) && type_counts[&v.rust_type.to_string()] == 1)
        .map(|v| {
            let ident = &v.variant_ident;
            let ty = &v.rust_type;
            let ty_str = ty.to_string();
            // Extern-path types (WKTs resolved to ::buffa_types, or any
            // user-mapped ::crate path) are from another crate — see
            // context.rs:rust_type_relative. For those, the Option<_> impl
            // would be E0117: Option is foreign (not fundamental) so doesn't
            // uncover the local Oneof inside, and T is foreign → no local
            // type in the impl header. `crate::…` is treated as local for
            // orphan purposes (it IS the current crate) so only `::` gates.
            let ty_is_extern = ty_str.trim_start().starts_with("::");
            // Unboxed variants store the value inline; boxed ones wrap it in the
            // configured pointer (`Box` by default).
            let wrapped = if v.is_boxed {
                v.pointer_repr.pointer_new(ty, &quote! { v })?
            } else {
                quote! { v }
            };
            // From<T> for Oneof — always legal (Oneof is local in T0 position).
            let from_oneof = quote! {
                impl From<#ty> for #rust_enum_ident {
                    fn from(v: #ty) -> Self {
                        Self::#ident(#wrapped)
                    }
                }
            };
            // From<T> for Option<Oneof> — legal only when T is local
            // (RFC 2451: T as trait param satisfies the orphan rule).
            // Collapses struct-literal construction to `field: Msg{..}.into()`.
            let from_option = if ty_is_extern {
                quote! {}
            } else {
                quote! {
                    impl From<#ty> for ::core::option::Option<#rust_enum_ident> {
                        fn from(v: #ty) -> Self {
                            Self::Some(#rust_enum_ident::from(v))
                        }
                    }
                }
            };
            Ok(quote! { #from_oneof #from_option })
        })
        .collect::<Result<Vec<_>, CodeGenError>>()?;

    let serde_impls = if ctx.config.generate_json {
        crate::feature_gates::cfg_block(
            generate_oneof_serialize(&rust_enum_ident, &variants_info),
            ctx.config.feature_gates().json,
        )
    } else {
        quote! {}
    };
    let arbitrary_derive = if ctx.config.generate_arbitrary {
        quote! { #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))] }
    } else {
        quote! {}
    };

    let oneof_fqn = format!("{}.{}", proto_fqn, oneof_name);
    let oneof_doc =
        crate::comments::doc_attrs_resolved(ctx.comment(&oneof_fqn), proto_fqn, &ctx.type_map);
    let custom_type_attrs =
        CodeGenContext::matching_attributes(&ctx.config.type_attributes, &oneof_fqn)?;
    // An inline (unboxed) message variant can dwarf its siblings, which trips
    // clippy::large_enum_variant in the consumer's crate on code they cannot
    // edit. The user explicitly chose inline storage, so allow the lint —
    // but only on enums that actually contain an unboxed message variant,
    // keeping default codegen output untouched.
    let large_variant_allow = if variants_info
        .iter()
        .any(|v| is_boxed_variant(v.field_type) && !v.is_boxed)
    {
        quote! { #[allow(clippy::large_enum_variant)] }
    } else {
        quote! {}
    };
    let custom_oneof_attrs =
        CodeGenContext::matching_attributes(&ctx.config.oneof_attributes, &oneof_fqn)?;

    // Variants whose field is `[debug_redact = true]` print a placeholder
    // instead of their payload. The `Debug` derive is swapped for a manual
    // impl only when at least one variant is redacted, so unaffected oneofs
    // keep byte-identical output.
    let any_redacted = variants_info.iter().any(|v| v.debug_redact);
    let (debug_derive, debug_impl) = if any_redacted {
        let placeholder = crate::message::DEBUG_REDACT_PLACEHOLDER;
        let arms: Vec<TokenStream> = variants_info
            .iter()
            .map(|v| {
                let ident = &v.variant_ident;
                let name = ident.to_string();
                if v.debug_redact {
                    quote! {
                        Self::#ident(_) => f
                            .debug_tuple(#name)
                            .field(&::core::format_args!(#placeholder))
                            .finish(),
                    }
                } else {
                    quote! {
                        Self::#ident(value) => f.debug_tuple(#name).field(value).finish(),
                    }
                }
            })
            .collect();
        (
            quote! { #[derive(Clone, PartialEq)] },
            quote! {
                impl ::core::fmt::Debug for #rust_enum_ident {
                    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        match self {
                            #(#arms)*
                        }
                    }
                }
            },
        )
    } else {
        (quote! { #[derive(Clone, PartialEq, Debug)] }, quote! {})
    };

    Ok(quote! {
        #oneof_doc
        #debug_derive
        #arbitrary_derive
        #large_variant_allow
        #custom_type_attrs
        #custom_oneof_attrs
        pub enum #rust_enum_ident {
            #(#variants,)*
        }

        #debug_impl

        impl ::buffa::Oneof for #rust_enum_ident {}

        #(#from_impls)*

        #serde_impls
    })
}

// ── Serde impl generation ────────────────────────────────────────────────────

/// Return the path to the serde helper `serialize` function for a field type,
/// or `None` if the type uses default serde serialization.
pub(crate) fn serde_helper_path(field_type: Type) -> Option<TokenStream> {
    match field_type {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            Some(quote! { ::buffa::json_helpers::int32 })
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => Some(quote! { ::buffa::json_helpers::uint32 }),
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            Some(quote! { ::buffa::json_helpers::int64 })
        }
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => Some(quote! { ::buffa::json_helpers::uint64 }),
        Type::TYPE_FLOAT => Some(quote! { ::buffa::json_helpers::float }),
        Type::TYPE_DOUBLE => Some(quote! { ::buffa::json_helpers::double }),
        Type::TYPE_BYTES => Some(quote! { ::buffa::json_helpers::bytes }),
        _ => None,
    }
}

fn generate_oneof_serialize(
    enum_ident: &proc_macro2::Ident,
    variants: &[VariantInfo],
) -> TokenStream {
    let arms: Vec<_> = variants
        .iter()
        .map(|v| {
            let ident = &v.variant_ident;
            let json_name = &v.json_name;

            // These arms live inside `impl Serialize for #enum_ident { fn
            // serialize(&self, ..) { match self { .. } } }`, so `Self`
            // resolves to the oneof enum and is the idiomatic spelling
            // (rustc's `clippy::use_self` flags the qualified form).
            // Contrast `oneof_variant_deser_arm` below, whose constructor
            // calls run inside the *message*'s Deserialize impl where
            // `Self` would be wrong.
            if v.is_null_value {
                // NullValue must serialize as JSON `null`, not "NULL_VALUE".
                // `&()` serializes as JSON `null` via serde_json.
                return quote! {
                    Self::#ident(_) => {
                        map.serialize_entry(#json_name, &())?;
                    }
                };
            }

            if serde_helper_path(v.field_type).is_some() {
                // Type needs special proto JSON encoding — route through the
                // runtime ProtoJson adapter (ProtoElemJson covers every type
                // serde_helper_path matches).
                quote! {
                    Self::#ident(v) => {
                        map.serialize_entry(
                            #json_name,
                            &::buffa::json_helpers::ProtoJson(v),
                        )?;
                    }
                }
            } else {
                quote! {
                    Self::#ident(v) => {
                        map.serialize_entry(#json_name, v)?;
                    }
                }
            }
        })
        .collect();

    quote! {
        impl serde::Serialize for #enum_ident {
            fn serialize<S: serde::Serializer>(&self, s: S) -> ::core::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeMap;
                let mut map = s.serialize_map(Some(1))?;
                match self {
                    #(#arms)*
                }
                map.end()
            }
        }
    }
}

/// Parameters for generating a single oneof variant deserialization match arm.
pub(crate) struct OneofVariantDeserInput<'a> {
    pub variant_ident: &'a Ident,
    pub variant_type: &'a TokenStream,
    pub json_name: &'a str,
    pub proto_name: &'a str,
    pub field_type: Type,
    /// See [`null_is_valid_value`] — includes both NullValue and Value types.
    pub null_forward: bool,
    /// Whether the variant is stored in a `Box` in the owned enum (see
    /// [`variant_boxed`]): message/group types are boxed unless opted out
    /// via `config.unboxed_oneof_fields`.
    pub is_boxed: bool,
    /// Owned pointer for a boxed variant (default `Box`); only consulted when
    /// `is_boxed`.
    pub pointer_repr: &'a crate::PointerRepr,
    pub enum_ident: &'a TokenStream,
    /// The identifier of the `Option<EnumIdent>` accumulator
    /// (e.g. `result` or `__oneof_foo`).
    pub result_var: &'a Ident,
    /// The proto name of the oneof, for error messages.
    pub oneof_name: &'a str,
}

/// Generate the deserialization match-arm body for one oneof variant.
///
/// Returns a `quote!` block that deserializes the value from a map entry and
/// sets the oneof result variable.
///
/// Handles:
/// - NullValue special case (JSON null IS the value, not "variant absent")
/// - Helper path dispatch (for types needing serde helpers like int64)
/// - NullableDeserializeSeed wrapping (null -> variant not set)
/// - Duplicate oneof field detection
pub(crate) fn oneof_variant_deser_arm(
    input: &OneofVariantDeserInput<'_>,
) -> Result<TokenStream, CodeGenError> {
    let OneofVariantDeserInput {
        variant_ident,
        variant_type,
        json_name,
        proto_name,
        field_type,
        null_forward,
        is_boxed,
        pointer_repr,
        enum_ident,
        result_var,
        oneof_name,
    } = input;
    let dup_err_msg = format!("multiple oneof fields set for '{oneof_name}'");
    // For boxed variants, the deserialized inner value must be wrapped in the
    // configured pointer (`Box` by default).
    let wrapped_v = if *is_boxed {
        pointer_repr.pointer_new(variant_type, &quote! { v })?
    } else {
        quote! { v }
    };
    // NullValue / Value: JSON `null` IS a valid value for these types,
    // not "variant absent". Deserialize directly without NullableDeserializeSeed
    // so `null` reaches the type's own Deserialize impl.
    let (deser, set_result) = if *null_forward {
        let deser = quote! {
            let v: #variant_type = map.next_value_seed(
                ::buffa::json_helpers::DefaultDeserializeSeed::<#variant_type>::new()
            )?;
        };
        let set = quote! {
            if #result_var.is_some() {
                return Err(serde::de::Error::custom(#dup_err_msg));
            }
            #result_var = Some(#enum_ident::#variant_ident(#wrapped_v));
        };
        (deser, set)
    } else {
        let deser = if let Some(helper) = serde_helper_path(*field_type) {
            // For bytes: json_helpers::bytes::deserialize is generic over
            // T: From<Vec<u8>>; the `-> Result<#variant_type, _>` return
            // type pins T to either Vec<u8> (default) or bytes::Bytes
            // (use_bytes_type). No shim needed.
            quote! {
                struct _DeserSeed;
                impl<'de> serde::de::DeserializeSeed<'de> for _DeserSeed {
                    type Value = #variant_type;
                    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> ::core::result::Result<#variant_type, D::Error> {
                        #helper::deserialize(d)
                    }
                }
                let v: ::core::option::Option<#variant_type> = map.next_value_seed(
                    ::buffa::json_helpers::NullableDeserializeSeed(_DeserSeed)
                )?;
            }
        } else {
            quote! {
                let v: ::core::option::Option<#variant_type> = map.next_value_seed(
                    ::buffa::json_helpers::NullableDeserializeSeed(
                        ::buffa::json_helpers::DefaultDeserializeSeed::<#variant_type>::new()
                    )
                )?;
            }
        };
        let set = quote! {
            if let Some(v) = v {
                if #result_var.is_some() {
                    return Err(serde::de::Error::custom(#dup_err_msg));
                }
                #result_var = Some(#enum_ident::#variant_ident(#wrapped_v));
            }
        };
        (deser, set)
    };

    // Accept both json_name and proto_name.
    if json_name == proto_name {
        Ok(quote! {
            #json_name => {
                #deser
                #set_result
            }
        })
    } else {
        Ok(quote! {
            #json_name | #proto_name => {
                #deser
                #set_result
            }
        })
    }
}

/// Build the Rust identifier for a oneof enum: `{PascalCase(oneof_name)}`.
///
/// No suffix and no collision check — oneof enums live in the dedicated
/// `__buffa::oneof::<msg>::` tree where they cannot collide with nested
/// types, nested enums, or view structs. Two sibling oneofs would only
/// produce the same ident if they share a proto name, which protoc
/// rejects at parse time.
fn oneof_enum_ident(oneof_name: &str) -> proc_macro2::Ident {
    format_ident!("{}", to_pascal_case(oneof_name))
}

/// Compute oneof enum identifiers for all non-synthetic oneofs in a message.
///
/// Returns a map from oneof declaration index to its Rust enum `Ident`.
/// Synthetic oneofs (proto3 `optional`) are omitted. Infallible: oneof
/// enums live in the `__buffa::oneof::` tree where collisions with
/// nested types are structurally impossible.
pub(crate) fn resolve_oneof_idents(
    msg: &DescriptorProto,
) -> std::collections::HashMap<usize, Ident> {
    let mut result = std::collections::HashMap::new();
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let has_real_fields = msg.field.iter().any(|f| {
            crate::impl_message::is_real_oneof_member(f) && f.oneof_index == Some(idx as i32)
        });
        if !has_real_fields {
            continue;
        }
        if let Some(oneof_name) = &oneof.name {
            result.insert(idx, oneof_enum_ident(oneof_name));
        }
    }
    result
}

/// Build the Rust variant identifier for a oneof field.
///
/// PascalCase the proto field name, then sanitize against reserved Rust
/// idents — the only lowercase Rust keyword whose PascalCase form is also
/// reserved is `self` → `Self`, which would otherwise produce
/// `pub enum Foo { Self(...) }` and fail to parse. `make_field_ident`
/// suffixes such names with `_` so the variant becomes `Self_`.
pub(crate) fn oneof_variant_ident(proto_name: &str) -> proc_macro2::Ident {
    crate::idents::make_field_ident(&to_pascal_case(proto_name))
}

/// Convert a snake_case identifier to PascalCase.
pub(crate) fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

/// Convert a PascalCase identifier to snake_case.
///
/// Inserts underscores at word boundaries:
/// - Before an uppercase letter that follows a lowercase letter (`fooBar` → `foo_bar`)
/// - Before the last uppercase in a consecutive run followed by lowercase
///   (`XMLHttp` → `xml_http`, `HTTPResponse` → `http_response`)
pub(crate) fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            // Insert `_` before an uppercase that follows lowercase (fooBar),
            // or before the start of a new word after an acronym run (XMLHttp).
            if prev.is_lowercase() || (prev.is_uppercase() && next_is_lower) {
                result.push('_');
            }
        }
        // `to_lowercase()` may yield multiple chars (e.g. `İ` → `i\u{307}`);
        // extend with the full sequence rather than truncating to the first.
        result.extend(c.to_lowercase());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{to_pascal_case, to_snake_case};

    #[test]
    fn test_to_pascal_case_basic() {
        assert_eq!(to_pascal_case("foo_bar"), "FooBar");
        assert_eq!(to_pascal_case("hello_world_baz"), "HelloWorldBaz");
        assert_eq!(to_pascal_case("single"), "Single");
    }

    #[test]
    fn test_to_pascal_case_leading_underscore() {
        // Leading underscore produces an empty first segment, which collapses.
        assert_eq!(to_pascal_case("_foo"), "Foo");
        assert_eq!(to_pascal_case("_foo_bar"), "FooBar");
    }

    #[test]
    fn test_to_pascal_case_consecutive_underscores() {
        // Consecutive underscores produce empty middle segments, which collapse.
        assert_eq!(to_pascal_case("foo__bar"), "FooBar");
        assert_eq!(to_pascal_case("a___b"), "AB");
    }

    #[test]
    fn test_to_pascal_case_empty() {
        assert_eq!(to_pascal_case(""), "");
    }

    #[test]
    fn test_to_snake_case_basic() {
        assert_eq!(to_snake_case("FooBar"), "foo_bar");
        assert_eq!(to_snake_case("HelloWorldBaz"), "hello_world_baz");
        assert_eq!(to_snake_case("Single"), "single");
    }

    #[test]
    fn test_to_snake_case_acronym_run() {
        assert_eq!(to_snake_case("XMLHttpRequest"), "xml_http_request");
        assert_eq!(to_snake_case("HTTPResponse"), "http_response");
        assert_eq!(to_snake_case("IOError"), "io_error");
    }

    #[test]
    fn test_to_snake_case_already_lower() {
        assert_eq!(to_snake_case("foo"), "foo");
    }

    #[test]
    fn test_to_snake_case_all_caps() {
        assert_eq!(to_snake_case("XML"), "xml");
        assert_eq!(to_snake_case("IO"), "io");
    }

    #[test]
    fn test_to_snake_case_proto_names() {
        // Typical proto message names we'll encounter.
        assert_eq!(to_snake_case("TestAllTypesProto3"), "test_all_types_proto3");
        assert_eq!(to_snake_case("NestedMessage"), "nested_message");
        assert_eq!(to_snake_case("ForeignMessage"), "foreign_message");
        assert_eq!(to_snake_case("ExtensionWithOneof"), "extension_with_oneof");
    }

    #[test]
    fn test_to_snake_case_empty() {
        assert_eq!(to_snake_case(""), "");
    }
}

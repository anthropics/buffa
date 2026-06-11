//! Code generation for `impl Message` and `impl DefaultInstance`.
//!
//! Generates `compute_size`, `write_to`, and `merge` implementations
//! covering all field types:
//!
//! - Numeric scalars: int32/64, uint32/64, sint32/64, fixed32/64, sfixed32/64,
//!   float, double, bool
//! - Length-delimited scalars: string, bytes
//! - Enum fields: open (`EnumValue<T>`) and closed (bare `E`) with
//!   unknown-value routing to `unknown_fields` for proto2
//! - Singular message fields: `MessageField<T>` (nested sub-message)
//! - Proto3 optional / proto2 optional scalars: `Option<T>`
//! - Repeated fields: `Vec<T>` — packed for numerics/enums, unpacked for
//!   string/bytes/message (both packed and unpacked accepted on decode)
//! - Map fields: `HashMap<K, V>` via synthetic map-entry messages
//! - Oneof fields: `Option<OneofEnum>` with per-variant encode/decode

use crate::context::CodeGenContext;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::features::ResolvedFeatures;
use crate::message::{find_map_entry, is_closed_enum, make_field_ident};
use crate::CodeGenError;
use buffa::encoding::MAX_FIELD_NUMBER;

/// Extract and validate the field number from a descriptor, returning a `u32`.
///
/// Protobuf field numbers must be in `[1, MAX_FIELD_NUMBER]`.
pub(crate) fn validated_field_number(field: &FieldDescriptorProto) -> Result<u32, CodeGenError> {
    let n = field
        .number
        .ok_or(CodeGenError::MissingField("field.number"))?;
    // FieldDescriptorProto.number is int32 in the descriptor schema, hence
    // the cast for the range check (2^29 − 1 fits comfortably in i32).
    if !(1..=MAX_FIELD_NUMBER as i32).contains(&n) {
        return Err(CodeGenError::Other(format!("invalid field number: {n}")));
    }
    Ok(n as u32)
}

/// Returns `true` when a non-repeated, non-message field has *explicit*
/// field presence and must be encoded as `Option<T>`.
///
/// - **Proto3**: only fields marked with the `optional` keyword
///   (`proto3_optional = true` in the descriptor, backed by a synthetic oneof).
/// - **Proto2**: any `optional`-labelled non-message field (proto2 `optional`
///   always confers explicit presence; `required` fields return `false` here
///   because they use always-encode semantics via `is_proto2_required`).
/// - **Editions**: fields with `field_presence = EXPLICIT` in resolved features.
/// - Message fields always use `MessageField<T>` regardless of syntax and are
///   excluded here.
pub(crate) fn is_explicit_presence_scalar(
    field: &FieldDescriptorProto,
    ty: Type,
    features: &ResolvedFeatures,
) -> bool {
    if ty == Type::TYPE_MESSAGE || ty == Type::TYPE_GROUP {
        return false;
    }
    // proto3_optional is a proto3-era flag; for editions, presence comes
    // from the resolved features.  Both paths converge here.
    if field.proto3_optional.unwrap_or(false) {
        return true;
    }
    // Proto2 required fields are always present (bare type, not Option).
    if field.label.unwrap_or_default() == Label::LABEL_REQUIRED {
        return false;
    }
    let field_features =
        crate::features::resolve_child(features, crate::features::field_features(field));
    field_features.field_presence == crate::features::FieldPresence::Explicit
        && field.oneof_index.is_none()
}

/// Does this field have required semantics (always encode regardless of value)?
///
/// True for proto2 `required` (LABEL_REQUIRED) and editions
/// `features.field_presence = LEGACY_REQUIRED` — both produce bare types
/// and must serialize even zero/empty values.
pub(crate) fn is_required_field(field: &FieldDescriptorProto, features: &ResolvedFeatures) -> bool {
    if field.label.unwrap_or_default() == Label::LABEL_REQUIRED {
        return true;
    }
    let field_features =
        crate::features::resolve_child(features, crate::features::field_features(field));
    field_features.field_presence == crate::features::FieldPresence::LegacyRequired
}

/// Returns the effective field type after applying `utf8_validation`.
///
/// When `ctx.config.strict_utf8_mapping` is `true` AND the per-field resolved
/// `utf8_validation` is `NONE`, string fields are treated as bytes fields:
/// the Rust type becomes `Vec<u8>` / `&[u8]`, decode uses `merge_bytes`
/// (no UTF-8 validation), and JSON encodes as base64. This is the only sound
/// mapping when strings may actually contain non-UTF-8 bytes — `&str` has a
/// type-level invariant that its contents are valid UTF-8.
///
/// When strict mapping is disabled (the default), string fields always map
/// to `String` / `&str` and decode validates UTF-8 regardless of the proto
/// `utf8_validation` feature. This is stricter than proto2 requires but
/// matches ecosystem expectations and avoids breaking existing proto2 code.
///
/// The per-field feature resolution happens here, so callers pass the
/// *message-level* resolved features and the field descriptor.
pub(crate) fn effective_type(
    ctx: &crate::context::CodeGenContext,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Type {
    let ty = field.r#type.unwrap_or_default();

    // Editions `features.message_encoding = DELIMITED` keeps the descriptor
    // field type as TYPE_MESSAGE (unlike proto2 `group` syntax which sets
    // TYPE_GROUP directly). Rewriting here routes DELIMITED fields through
    // the existing TYPE_GROUP encode/decode paths (wire types 3/4).
    if ty == Type::TYPE_MESSAGE {
        let field_features =
            crate::features::resolve_child(features, crate::features::field_features(field));
        if field_features.message_encoding == crate::features::MessageEncoding::Delimited {
            return Type::TYPE_GROUP;
        }
    }

    if !ctx.config.strict_utf8_mapping || ty != Type::TYPE_STRING {
        return ty;
    }
    // utf8_validation is field-local (not enum-dependent), so resolve_child
    // is sufficient here — no need for resolve_field's enum_type overlay.
    let field_features =
        crate::features::resolve_child(features, crate::features::field_features(field));
    if field_features.utf8_validation == crate::features::Utf8Validation::None {
        Type::TYPE_BYTES
    } else {
        ty
    }
}

/// [`effective_type`] for map-entry key/value fields.
///
/// The protobuf wire spec hard-codes map entries as length-prefixed,
/// independent of `features.message_encoding`. protoc does NOT stamp an
/// explicit `LENGTH_PREFIXED` feature on synthetic map-entry fields, so a
/// file-level `DELIMITED` default would otherwise inherit through and
/// [`effective_type`] would incorrectly rewrite a message-typed map value
/// to `TYPE_GROUP`. This wrapper forces the spec invariant.
pub(crate) fn effective_type_in_map_entry(
    ctx: &crate::context::CodeGenContext,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Type {
    let mut f = *features;
    f.message_encoding = crate::features::MessageEncoding::LengthPrefixed;
    effective_type(ctx, field, &f)
}

/// Generate the decode expression for a closed enum value.
///
/// Returns a `TokenStream` that decodes an i32 from `buf_expr`, checks
/// `from_i32`, and executes `on_known` with the decoded value bound as `__v`.
/// Unknown values are silently discarded.
///
/// Retained for the remaining silent-drop cases: view packed-repeated
/// (no per-element tag → no borrowable span) and map-entry (spec
/// requires the whole entry go to unknown fields — deferred).
/// All other paths (owned + view singular/optional/repeated-unpacked/oneof)
/// use [`closed_enum_decode_with_unknown`].
pub(crate) fn closed_enum_decode(buf_expr: &TokenStream, on_known: TokenStream) -> TokenStream {
    quote! {
        let __raw = ::buffa::types::decode_int32(#buf_expr)?;
        if let ::core::option::Option::Some(__v) = ::buffa::Enumeration::from_i32(__raw) {
            #on_known
        }
    }
}

/// Like [`closed_enum_decode`], but with an `else` branch for unknown values.
///
/// The `on_unknown` token stream is placed in the `else` block, with `__raw`
/// still in scope so it can be routed (e.g. to unknown fields storage).
pub(crate) fn closed_enum_decode_with_unknown(
    buf_expr: &TokenStream,
    on_known: TokenStream,
    on_unknown: TokenStream,
) -> TokenStream {
    quote! {
        let __raw = ::buffa::types::decode_int32(#buf_expr)?;
        if let ::core::option::Option::Some(__v) = ::buffa::Enumeration::from_i32(__raw) {
            #on_known
        } else {
            #on_unknown
        }
    }
}

/// Token stream that pushes a closed-enum unknown value (`__raw`, in scope)
/// to `self.__buffa_unknown_fields` as a varint with the given field number.
/// Returns empty tokens when `preserve_unknown_fields` is false (drop).
pub(crate) fn closed_enum_unknown_route(
    field_number: u32,
    preserve_unknown_fields: bool,
) -> TokenStream {
    if preserve_unknown_fields {
        quote! {
            self.__buffa_unknown_fields.push(::buffa::UnknownField {
                number: #field_number,
                data: ::buffa::UnknownFieldData::Varint(__raw as u64),
            });
        }
    } else {
        quote! {}
    }
}

/// A classified field (or oneof group) ready for per-kind codegen dispatch.
///
/// Produced by [`classify_fields_ordered`] in ascending field-number order so
/// that `compute_size` and `write_to` emit fields in the spec-recommended
/// sequence (matching the field order of prost / protoc-C++). A `Oneof` entry
/// stands in for all its member fields and is positioned at the group's
/// minimum member field number.
enum FieldKind<'a> {
    Scalar(&'a FieldDescriptorProto),
    Repeated(&'a FieldDescriptorProto),
    Map(&'a FieldDescriptorProto),
    Oneof {
        name: &'a str,
        enum_ident: proc_macro2::Ident,
        fields: Vec<&'a FieldDescriptorProto>,
    },
}

/// Classify every field of `msg` and return the result sorted by ascending
/// field number. Oneof members are folded into a single [`FieldKind::Oneof`]
/// per group, positioned at the lowest member number.
///
/// Shared between [`generate_message_impl`] (owned) and
/// [`build_view_encode_methods`] (view) so a new field category only needs
/// adding here.
fn classify_fields_ordered<'a>(
    msg: &'a DescriptorProto,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<Vec<FieldKind<'a>>, CodeGenError> {
    let mut entries: Vec<(u32, FieldKind<'a>)> = Vec::with_capacity(msg.field.len());
    let mut seen_oneof: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for f in &msg.field {
        let number = validated_field_number(f)?;
        if is_real_oneof_member(f) {
            let idx = f.oneof_index.expect("checked by is_real_oneof_member");
            if !seen_oneof.insert(idx) {
                continue;
            }
            let Some(enum_ident) = oneof_idents.get(&(idx as usize)) else {
                continue;
            };
            let Some(name) = msg
                .oneof_decl
                .get(idx as usize)
                .and_then(|o| o.name.as_deref())
            else {
                continue;
            };
            let members: Vec<_> = msg
                .field
                .iter()
                .filter(|m| is_real_oneof_member(m) && m.oneof_index == Some(idx))
                .collect();
            let min_number = members
                .iter()
                .map(|m| validated_field_number(m))
                .try_fold(number, |acc, n| Ok::<_, CodeGenError>(acc.min(n?)))?;
            entries.push((
                min_number,
                FieldKind::Oneof {
                    name,
                    enum_ident: enum_ident.clone(),
                    fields: members,
                },
            ));
        } else if f.label.unwrap_or_default() == Label::LABEL_REPEATED {
            if crate::message::is_map_field(msg, f) {
                entries.push((number, FieldKind::Map(f)));
            } else if is_supported_field_type(f.r#type.unwrap_or_default()) {
                entries.push((number, FieldKind::Repeated(f)));
            }
        } else if is_supported_field_type(f.r#type.unwrap_or_default()) {
            entries.push((number, FieldKind::Scalar(f)));
        }
    }
    entries.sort_by_key(|(n, _)| *n);
    Ok(entries.into_iter().map(|(_, k)| k).collect())
}

/// True if `compute_size` / `write_to` for this message reference the
/// threaded `SizeCache` — i.e. it has any sub-message-typed (LEN-delimited
/// or group) field, oneof variant, or map value. Leaf messages (scalars
/// only) take the cache as `_cache` to make the dead parameter explicit.
/// With `lazy_views` (lazy view family only), lazy message fields re-emit raw
/// fragments without a cache slot, so they don't count as cache users.
fn message_uses_size_cache(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    fields: &[FieldKind<'_>],
    features: &ResolvedFeatures,
    is_lazy: Option<&dyn Fn(&FieldDescriptorProto) -> bool>,
) -> bool {
    let is_nested = |f: &FieldDescriptorProto| match effective_type(ctx, f, features) {
        Type::TYPE_MESSAGE => !is_lazy.is_some_and(|p| p(f)),
        Type::TYPE_GROUP => true,
        _ => false,
    };
    let oneof_nested = |f: &FieldDescriptorProto| {
        matches!(
            effective_type(ctx, f, features),
            Type::TYPE_MESSAGE | Type::TYPE_GROUP
        )
    };
    fields.iter().any(|kind| match kind {
        FieldKind::Scalar(f) | FieldKind::Repeated(f) => is_nested(f),
        FieldKind::Oneof { fields, .. } => fields.iter().copied().any(oneof_nested),
        FieldKind::Map(f) => find_map_entry_fields(msg, f)
            .map(|(_, val_fd)| {
                effective_type_in_map_entry(ctx, val_fd, features) == Type::TYPE_MESSAGE
            })
            .unwrap_or(false),
    })
}

/// Generate `impl DefaultInstance` and `impl Message` for a message.
///
/// Emit `impl #generics ::buffa::MessageName for #ty { … }`.
///
/// `generics` is the impl-side generic parameter list (`<'a>` for the
/// view type, empty for the owned message). `ty` is the implementing
/// type *with* any generics applied (`Foo` or `FooView<'a>`).
///
/// All four consts are computed at codegen time as string literals so
/// `T::FULL_NAME` etc. are zero-cost at runtime — no `format!`,
/// `concat!`, or lazy static. `PACKAGE` and `NAME` are split here rather
/// than left to the consumer because the dotted `FULL_NAME` cannot be
/// re-split unambiguously: `foo.Bar.Baz` could be package `foo.Bar` +
/// message `Baz`, or package `foo` + nested `Bar.Baz`. Codegen knows
/// which.
pub(crate) fn message_name_impl(
    current_package: &str,
    proto_fqn: &str,
    generics: &TokenStream,
    ty: &TokenStream,
) -> TokenStream {
    let name = if current_package.is_empty() {
        proto_fqn.to_string()
    } else {
        // Strip `"<package>."` atomically — a two-step
        // `strip_prefix(package)` then `strip_prefix(".")` would
        // partial-match a prefix-overlapping package (`package = "foo"`
        // against `proto_fqn = "food.Bar"`) and silently violate the
        // documented `PACKAGE + "." + NAME == FULL_NAME` invariant.
        //
        // `proto_fqn` is always `"<package>.<rest>"` for a non-empty
        // package (it's built by joining message segments onto the
        // package), so the strip should never fail. Fall back
        // defensively rather than panic on a malformed descriptor.
        proto_fqn
            .strip_prefix(&format!("{current_package}."))
            .unwrap_or(proto_fqn)
            .to_string()
    };
    let type_url = format!("type.googleapis.com/{proto_fqn}");
    quote! {
        impl #generics ::buffa::MessageName for #ty {
            const PACKAGE: &'static str = #current_package;
            const NAME: &'static str = #name;
            const FULL_NAME: &'static str = #proto_fqn;
            const TYPE_URL: &'static str = #type_url;
        }
    }
}

/// `preserve_unknown_fields`: when `true`, the generated merge collects
/// unknown fields into `self.__buffa_unknown_fields` and both `compute_size` and
/// `write_to` include them.
#[allow(clippy::too_many_arguments)]
pub fn generate_message_impl(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    preserve_unknown_fields: bool,
    rust_name: &str,
    current_package: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
    oneof_prefix: &TokenStream,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let name_ident = format_ident!("{}", rust_name);

    let fields = classify_fields_ordered(msg, oneof_idents)?;
    // The lazy predicate applies to the lazy view family only; owned is eager.
    let cache_ident = if message_uses_size_cache(ctx, msg, &fields, features, None) {
        format_ident!("__cache")
    } else {
        format_ident!("_cache")
    };

    // Single pass in field-number order. `compute_stmts`/`write_stmts` MUST be
    // built in lockstep: SizeCache::consume_next() in write_to reads slots in
    // the same traversal order that compute_size's reserve()/set() filled them.
    let mut compute_stmts: Vec<TokenStream> = Vec::with_capacity(fields.len());
    let mut write_stmts: Vec<TokenStream> = Vec::with_capacity(fields.len());
    let mut merge_arms: Vec<TokenStream> = Vec::with_capacity(fields.len());
    let mut clear_stmts: Vec<TokenStream> = Vec::with_capacity(fields.len());
    for kind in &fields {
        match kind {
            FieldKind::Scalar(f) => {
                compute_stmts.push(scalar_compute_size_stmt(ctx, f, features)?);
                write_stmts.push(scalar_write_to_stmt(ctx, f, features)?);
                merge_arms.push(scalar_merge_arm(
                    ctx,
                    f,
                    proto_fqn,
                    features,
                    preserve_unknown_fields,
                )?);
                clear_stmts.push(scalar_clear_stmt(
                    f,
                    ctx,
                    current_package,
                    proto_fqn,
                    features,
                    nesting,
                )?);
            }
            FieldKind::Repeated(f) => {
                compute_stmts.push(repeated_compute_size_stmt(ctx, f, features)?);
                write_stmts.push(repeated_write_to_stmt(ctx, f, features)?);
                merge_arms.push(repeated_merge_arm(
                    ctx,
                    f,
                    proto_fqn,
                    features,
                    preserve_unknown_fields,
                )?);
                clear_stmts.push(vec_field_clear_stmt(f)?);
            }
            FieldKind::Map(f) => {
                compute_stmts.push(map_compute_size_stmt(ctx, msg, f, features)?);
                write_stmts.push(map_write_to_stmt(ctx, msg, f, features)?);
                merge_arms.push(map_merge_arm(ctx, msg, f, proto_fqn, features)?);
                clear_stmts.push(vec_field_clear_stmt(f)?);
            }
            FieldKind::Oneof {
                name,
                enum_ident,
                fields,
            } => {
                let (cs, ws, mas) = generate_oneof_impls(
                    ctx,
                    enum_ident,
                    name,
                    fields,
                    oneof_prefix,
                    proto_fqn,
                    features,
                    preserve_unknown_fields,
                )?;
                compute_stmts.push(cs);
                write_stmts.push(ws);
                merge_arms.extend(mas);
                let ident = make_field_ident(name);
                clear_stmts.push(quote! { self.#ident = ::core::option::Option::None; });
            }
        }
    }

    // MessageSet wire format: each LengthDelimited unknown field (i.e. each
    // extension payload) is wrapped in a group-at-field-1 Item on the wire,
    // but stored flat as `{number: type_id, data: LD(payload)}`. The gate
    // check (`CodeGenConfig::allow_message_set`) is in `message.rs`; by the
    // time we're here, the flag is set or the option was absent.
    let is_message_set = msg
        .options
        .as_option()
        .and_then(|o| o.message_set_wire_format)
        .unwrap_or(false);

    // Generate unknown-fields snippets based on config.
    let unknown_fields_size_stmt = if is_message_set {
        // LD records become Items; stray non-LD records (which shouldn't
        // normally exist on a MessageSet) re-emit as regular unknowns.
        quote! {
            for f in self.__buffa_unknown_fields.iter() {
                if let ::buffa::UnknownFieldData::LengthDelimited(ref bytes) = f.data {
                    size += ::buffa::message_set::item_encoded_len(f.number, bytes.len()) as u32;
                } else {
                    size += f.encoded_len() as u32;
                }
            }
        }
    } else if preserve_unknown_fields {
        quote! { size += self.__buffa_unknown_fields.encoded_len() as u32; }
    } else {
        quote! {}
    };
    let unknown_fields_write_stmt = if is_message_set {
        quote! {
            for f in self.__buffa_unknown_fields.iter() {
                if let ::buffa::UnknownFieldData::LengthDelimited(ref bytes) = f.data {
                    ::buffa::encoding::encode_varint(::buffa::message_set::ITEM_START_TAG, buf);
                    ::buffa::encoding::encode_varint(::buffa::message_set::TYPE_ID_TAG, buf);
                    ::buffa::encoding::encode_varint(f.number as u64, buf);
                    ::buffa::encoding::encode_varint(::buffa::message_set::MESSAGE_TAG, buf);
                    ::buffa::encoding::encode_varint(bytes.len() as u64, buf);
                    buf.put_slice(bytes);
                    ::buffa::encoding::encode_varint(::buffa::message_set::ITEM_END_TAG, buf);
                } else {
                    f.write_to(buf);
                }
            }
        }
    } else if preserve_unknown_fields {
        quote! { self.__buffa_unknown_fields.write_to(buf); }
    } else {
        quote! {}
    };
    let unknown_fields_merge_arm = if is_message_set {
        // Field 1 StartGroup is an Item wrapper: unwrap into a flat LD record
        // at the extension's field number. Everything else is preserved as a
        // regular unknown. The Item group itself consumes one depth level.
        quote! {
            _ => {
                if tag.field_number() == 1
                    && tag.wire_type() == ::buffa::encoding::WireType::StartGroup
                {
                    let (type_id, bytes) =
                        ::buffa::message_set::merge_item(buf, ctx.descend()?)?;
                    self.__buffa_unknown_fields.push(::buffa::UnknownField {
                        number: type_id,
                        data: ::buffa::UnknownFieldData::LengthDelimited(bytes),
                    });
                } else {
                    self.__buffa_unknown_fields.push(
                        ::buffa::encoding::decode_unknown_field(tag, buf, ctx)?
                    );
                }
            }
        }
    } else if preserve_unknown_fields {
        quote! {
            _ => {
                self.__buffa_unknown_fields.push(
                    ::buffa::encoding::decode_unknown_field(tag, buf, ctx)?
                );
            }
        }
    } else {
        quote! {
            _ => { ::buffa::encoding::skip_field_depth(tag, buf, ctx.depth())?; }
        }
    };

    let unknown_fields_clear_stmt = if preserve_unknown_fields {
        quote! { self.__buffa_unknown_fields.clear(); }
    } else {
        quote! {}
    };

    // Suppress lint warnings that fire on generated code for empty messages.
    let has_body = !fields.is_empty() || preserve_unknown_fields;
    let size_decl = if has_body {
        quote! { let mut size = 0u32; }
    } else {
        quote! { let size = 0u32; }
    };
    let buf_param = if has_body {
        quote! { buf: &mut impl ::buffa::bytes::BufMut }
    } else {
        quote! { _buf: &mut impl ::buffa::bytes::BufMut }
    };

    let extension_set_impl = if preserve_unknown_fields {
        let proto_fqn_lit = proto_fqn;
        quote! {
            impl ::buffa::ExtensionSet for #name_ident {
                const PROTO_FQN: &'static str = #proto_fqn_lit;
                fn unknown_fields(&self) -> &::buffa::UnknownFields {
                    &self.__buffa_unknown_fields
                }
                fn unknown_fields_mut(&mut self) -> &mut ::buffa::UnknownFields {
                    &mut self.__buffa_unknown_fields
                }
            }
        }
    } else {
        quote! {}
    };

    let message_name_impl = message_name_impl(
        current_package,
        proto_fqn,
        &quote! {},
        &quote! { #name_ident },
    );

    // Reflection: `impl Reflectable` resolving against the package's embedded
    // descriptor pool. Skipped for map entry synthetic messages — they're not
    // registered in the pool by name and consumers never reflect over them.
    //
    // In vtable mode this also emits `impl ReflectMessage` / `impl
    // ReflectElement` on the owned struct and makes `reflect()` borrow `self`
    // directly (no encode/decode round-trip). In bridge mode `reflect()` boxes
    // a `DynamicMessage` from a round-trip.
    let is_map_entry = msg
        .options
        .as_option()
        .is_some_and(|o| o.map_entry.unwrap_or(false));
    let reflectable_impl = if ctx.config.generate_reflection && !is_map_entry {
        let gate = ctx.config.feature_gates().reflect;
        if ctx.config.generate_reflection_vtable {
            let buffa_path = quote! { __buffa };
            let owned = crate::reflect_owned::reflect_owned_impls(
                &crate::reflect_owned::OwnedReflectScope {
                    ctx,
                    msg,
                    name_ident: &name_ident,
                    buffa_path: &buffa_path,
                    current_package,
                    proto_fqn,
                    features,
                    oneof_idents,
                    oneof_prefix,
                    nesting,
                },
            )?;
            let reflect_body = crate::reflect::reflectable_impl_vtable(&quote! { #name_ident });
            // Multiple sibling impls (ReflectMessage, ReflectElement, the memo
            // inherent impl, Reflectable) — gate them together with one cfg.
            crate::feature_gates::cfg_const_block(quote! { #owned #reflect_body }, gate)
        } else {
            // ReflectElement alongside Reflectable so a vtable-mode message
            // in another compilation can hold repeated/map fields of this
            // bridge-mode type (mixed-mode degradation at the boundary).
            let bridge =
                crate::reflect::reflectable_impl(&quote! { #name_ident }, &quote! { __buffa });
            let element = crate::reflect::reflect_element_impl_bridge(&quote! { #name_ident });
            crate::feature_gates::cfg_block(quote! { #bridge #element }, gate)
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl ::buffa::DefaultInstance for #name_ident {
            fn default_instance() -> &'static Self {
                static VALUE: ::buffa::__private::OnceBox<#name_ident>
                    = ::buffa::__private::OnceBox::new();
                VALUE.get_or_init(|| ::buffa::alloc::boxed::Box::new(Self::default()))
            }
        }

        #reflectable_impl

        #message_name_impl

        impl ::buffa::Message for #name_ident {
            /// Returns the total encoded size in bytes.
            ///
            /// The result is a `u32`; the protobuf specification requires all
            /// messages to fit within 2 GiB (2,147,483,647 bytes), so a
            /// compliant message will never overflow this type.
            #[allow(clippy::let_and_return)]
            fn compute_size(&self, #cache_ident: &mut ::buffa::SizeCache) -> u32 {
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                #size_decl
                #(#compute_stmts)*
                #unknown_fields_size_stmt
                size
            }

            fn write_to(
                &self,
                #cache_ident: &mut ::buffa::SizeCache,
                #buf_param,
            ) {
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                #(#write_stmts)*
                #unknown_fields_write_stmt
            }

            fn merge_field(
                &mut self,
                tag: ::buffa::encoding::Tag,
                buf: &mut impl ::buffa::bytes::Buf,
                ctx: ::buffa::DecodeContext<'_>,
            ) -> ::core::result::Result<(), ::buffa::DecodeError> {
                #[allow(unused_imports)]
                use ::buffa::bytes::Buf as _;
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                match tag.field_number() {
                    #(#merge_arms)*
                    #unknown_fields_merge_arm
                }
                ::core::result::Result::Ok(())
            }

            fn clear(&mut self) {
                #(#clear_stmts)*
                #unknown_fields_clear_stmt
            }
        }

        #extension_set_impl
    })
}

/// Build the `compute_size` / `write_to` method tokens for a
/// **view** type. Reuses the same per-field stmt builders as
/// [`generate_message_impl`] — they emit `&self.field`-relative code that is
/// duck-type-compatible with view field types (`&'a str`, `RepeatedView`,
/// `MapView`, `MessageFieldView`).
/// Compute-size / write-to pair for a lazy message field: each recorded
/// fragment is re-emitted as one LengthDelimited occurrence, byte-for-byte
/// and **without validation**. `accessor` is `fragments` (singular) or
/// `raw_elements` (repeated).
fn lazy_fragment_encode_stmts(
    field: &FieldDescriptorProto,
    accessor: TokenStream,
) -> Result<(TokenStream, TokenStream), CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(field_name);
    let ld_tag_len = tag_encoded_len(field_number, 2);
    let compute = quote! {
        for __frag in self.#ident.#accessor() {
            size += #ld_tag_len
                + ::buffa::encoding::varint_len(__frag.len() as u64) as u32
                + __frag.len() as u32;
        }
    };
    let write = quote! {
        for __frag in self.#ident.#accessor() {
            ::buffa::encoding::Tag::new(
                #field_number,
                ::buffa::encoding::WireType::LengthDelimited,
            ).encode(buf);
            ::buffa::encoding::encode_varint(__frag.len() as u64, buf);
            buf.put_slice(__frag);
        }
    };
    Ok((compute, write))
}

/// `is_lazy`: when building for the lazy view family, the per-field
/// deferral predicate — matching fields re-emit recorded fragments verbatim
/// (wire-equivalent to the merged value); all other field kinds share the
/// eager stmt builders. `None` builds the eager impl.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_view_encode_methods(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    preserve_unknown_fields: bool,
    features: &ResolvedFeatures,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
    view_oneof_prefix: &TokenStream,
    is_lazy: Option<&dyn Fn(&FieldDescriptorProto) -> bool>,
    vis: &TokenStream,
) -> Result<TokenStream, CodeGenError> {
    let fields = classify_fields_ordered(msg, oneof_idents)?;
    let cache_ident = if message_uses_size_cache(ctx, msg, &fields, features, is_lazy) {
        format_ident!("__cache")
    } else {
        format_ident!("_cache")
    };

    let lazy_message = |f: &FieldDescriptorProto| is_lazy.is_some_and(|p| p(f));

    let mut compute_stmts: Vec<TokenStream> = Vec::with_capacity(fields.len());
    let mut write_stmts: Vec<TokenStream> = Vec::with_capacity(fields.len());
    for kind in &fields {
        match kind {
            FieldKind::Scalar(f) if lazy_message(f) => {
                let (compute, write) = lazy_fragment_encode_stmts(f, quote! { fragments })?;
                compute_stmts.push(compute);
                write_stmts.push(write);
            }
            FieldKind::Repeated(f) if lazy_message(f) => {
                let (compute, write) = lazy_fragment_encode_stmts(f, quote! { raw_elements })?;
                compute_stmts.push(compute);
                write_stmts.push(write);
            }
            FieldKind::Scalar(f) => {
                compute_stmts.push(scalar_compute_size_stmt(ctx, f, features)?);
                write_stmts.push(scalar_write_to_stmt(ctx, f, features)?);
            }
            FieldKind::Repeated(f) => {
                compute_stmts.push(repeated_compute_size_stmt(ctx, f, features)?);
                write_stmts.push(repeated_write_to_stmt(ctx, f, features)?);
            }
            // map_{compute_size,write_to}_stmt emit `for (k, v) in &self.field
            // { ... }`. For owned `&HashMap<K,V>` that yields `(&K, &V)`
            // directly. For `&MapView<'_,K,V>` it yields `&(K,V)`, but
            // match-ergonomics binds the pattern `(k, v)` to `(&K, &V)` either
            // way — so the same generated body works on both.
            FieldKind::Map(f) => {
                compute_stmts.push(map_compute_size_stmt(ctx, msg, f, features)?);
                write_stmts.push(map_write_to_stmt(ctx, msg, f, features)?);
            }
            // The view-side oneof enum (in the parallel `__buffa::view::oneof::`
            // tree) has the same variant *names* as the owned enum but borrowed
            // payload types. The arm builders only emit the enum path + variant
            // name and call duck-typed primitives, so they work unchanged once
            // pointed at the view enum via `view_oneof_prefix`.
            FieldKind::Oneof {
                name,
                enum_ident,
                fields,
            } => {
                let field_ident = make_field_ident(name);
                let qualified: TokenStream = quote! { #view_oneof_prefix #enum_ident };
                let mut size_arms: Vec<TokenStream> = Vec::new();
                let mut write_arms: Vec<TokenStream> = Vec::new();
                for field in fields {
                    let field_number = validated_field_number(field)?;
                    let ty = effective_type(ctx, field, features);
                    let variant = crate::oneof::oneof_variant_ident(
                        field
                            .name
                            .as_deref()
                            .ok_or(CodeGenError::MissingField("field.name"))?,
                    );
                    let tag_len = tag_encoded_len(field_number, wire_type_byte(ty));
                    let wire_type = wire_type_token(ty);
                    size_arms.push(oneof_size_arm(&qualified, &variant, tag_len, ty));
                    write_arms.push(oneof_write_arm(
                        &qualified,
                        &variant,
                        field_number,
                        ty,
                        &wire_type,
                    ));
                }
                compute_stmts.push(quote! {
                    if let ::core::option::Option::Some(ref v) = self.#field_ident {
                        match v { #(#size_arms)* }
                    }
                });
                write_stmts.push(quote! {
                    if let ::core::option::Option::Some(ref v) = self.#field_ident {
                        match v { #(#write_arms)* }
                    }
                });
            }
        }
    }

    let unknown_fields_size_stmt = if preserve_unknown_fields {
        quote! { size += self.__buffa_unknown_fields.encoded_len() as u32; }
    } else {
        quote! {}
    };
    // MessageSet (option message_set_wire_format = true) needs no special
    // handling here: `UnknownFieldsView` stores raw verbatim wire spans, so the
    // Item-group framing is already in the bytes and `write_to` is a passthrough.
    // The owned path (see `generate_message_impl`) re-wraps because owned
    // `UnknownFields` stores parsed `(number, data)` pairs.
    let unknown_fields_write_stmt = if preserve_unknown_fields {
        quote! { self.__buffa_unknown_fields.write_to(buf); }
    } else {
        quote! {}
    };

    // Lazy bodies call ViewEncode methods on *eager* sub-views (groups, map
    // values, oneof variants); the eager bodies live inside the ViewEncode
    // impl itself where the trait is implicitly in scope.
    let lazy_use = if is_lazy.is_some() {
        quote! {
            #[allow(unused_imports)]
            use ::buffa::ViewEncode as _;
        }
    } else {
        quote! {}
    };
    let has_body = !fields.is_empty() || preserve_unknown_fields;
    let size_decl = if has_body {
        quote! { let mut size = 0u32; }
    } else {
        quote! { let size = 0u32; }
    };
    let buf_param = if has_body {
        quote! { buf: &mut impl ::buffa::bytes::BufMut }
    } else {
        quote! { _buf: &mut impl ::buffa::bytes::BufMut }
    };

    // On the lazy family these are inherent `pub fn`s, so they need doc
    // comments (a consumer crate denying `missing_docs` would otherwise fail
    // to build generated code). The eager family emits them as `ViewEncode`
    // trait methods, where docs come from the trait — keep those token
    // streams unchanged.
    let compute_size_doc = if is_lazy.is_some() {
        quote! {
            /// Compute the encoded byte size, filling `cache` with
            /// per-message sizes consumed by a following `write_to` call.
            /// Called for that side effect by `encode`; prefer `encoded_len`
            /// when only the size is needed.
        }
    } else {
        quote! {}
    };
    let write_to_doc = if is_lazy.is_some() {
        quote! {
            /// Write the encoded bytes to `buf`, reading per-message sizes
            /// from the `cache` filled by a preceding `compute_size` call.
            /// Prefer `encode` unless threading a shared cache.
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        // needless_borrow: stmt builders emit `&self.field` so they work on
        // owned `String`/`Vec<u8>`; on view fields (`&'a str`/`&'a [u8]`)
        // the borrow is redundant but harmless.
        #compute_size_doc
        #[allow(clippy::needless_borrow, clippy::let_and_return)]
        #vis fn compute_size(&self, #cache_ident: &mut ::buffa::SizeCache) -> u32 {
            #[allow(unused_imports)]
            use ::buffa::Enumeration as _;
            #lazy_use
            #size_decl
            #(#compute_stmts)*
            #unknown_fields_size_stmt
            size
        }

        #write_to_doc
        #[allow(clippy::needless_borrow)]
        #vis fn write_to(&self, #cache_ident: &mut ::buffa::SizeCache, #buf_param) {
            #[allow(unused_imports)]
            use ::buffa::Enumeration as _;
            #lazy_use
            #(#write_stmts)*
            #unknown_fields_write_stmt
        }
    })
}

/// `self.<field>.clear();` for repeated and map fields — both `Vec<T>` and
/// `HashMap<K,V>` retain their backing allocation on `.clear()`.
fn vec_field_clear_stmt(field: &FieldDescriptorProto) -> Result<TokenStream, CodeGenError> {
    let ident = make_field_ident(
        field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?,
    );
    Ok(quote! { self.#ident.clear(); })
}

/// Generate a clear statement for a scalar (non-repeated, non-oneof) field.
///
/// Returns a `TokenStream` that clears the field to its default value while
/// retaining heap allocations where possible (String, Vec, MessageField).
/// Check if a bytes-typed field should use `bytes::Bytes` instead of `Vec<u8>`.
///
/// `proto_fqn` is the fully-qualified message name (no leading dot), e.g.
/// `"my.pkg.Msg"`. Matched against `config.bytes_fields` as `".my.pkg.Msg.field"`.
pub(crate) fn field_uses_bytes(ctx: &CodeGenContext, proto_fqn: &str, field_name: &str) -> bool {
    let field_fqn = format!(".{}.{}", proto_fqn, field_name);
    ctx.use_bytes_type(&field_fqn)
}

/// Whether a `map<K, bytes>` value field should use `bytes::Bytes` instead of
/// `Vec<u8>`.
///
/// Single source of truth for the `bytes_fields` → map-value rule, shared by
/// `classify_field` (owned struct type, serde, and `arbitrary`), the binary and
/// text `map_merge_arm` decoders, and `view::map_to_owned_expr`. Centralizing it
/// keeps every site in agreement — a split decision would emit a `Bytes` value
/// on one side and `Vec<u8>` on another, surfacing only as a compile error in
/// the consuming crate.
///
/// `key_ty` / `val_ty` are the **effective** map-entry types (see
/// [`effective_type_in_map_entry`]); `None` means the entry lacks that field and
/// is treated as non-`bytes`, so a non-map caller naturally yields `false`. The
/// value becomes `Bytes` only when the value is proto `bytes`, the outer map
/// field matches `bytes_fields`, and the key is *not* effective-`bytes`. The key
/// carve-out tracks the one JSON helper (`bytes_key_bytes_val_map`) whose
/// signature is the concrete `HashMap<Vec<u8>, Vec<u8>>`; when the key is
/// effective-`bytes` the value must stay `Vec<u8>` to match it.
pub(crate) fn map_value_use_bytes(
    ctx: &CodeGenContext,
    key_ty: Option<Type>,
    val_ty: Option<Type>,
    proto_fqn: &str,
    field_name: &str,
) -> bool {
    val_ty == Some(Type::TYPE_BYTES)
        && key_ty != Some(Type::TYPE_BYTES)
        && field_uses_bytes(ctx, proto_fqn, field_name)
}

/// Resolve the [`StringRepr`](crate::StringRepr) for a `string`-typed field.
///
/// `proto_fqn` is the fully-qualified message name (no leading dot). Matched
/// against `config.string_fields` as `".my.pkg.Msg.field"`. Returns
/// [`StringRepr::String`](crate::StringRepr::String) for fields with no rule.
pub(crate) fn field_string_repr(
    ctx: &CodeGenContext,
    proto_fqn: &str,
    field_name: &str,
) -> crate::StringRepr {
    let field_fqn = format!(".{}.{}", proto_fqn, field_name);
    ctx.string_repr(&field_fqn)
}

fn scalar_clear_stmt(
    field: &FieldDescriptorProto,
    ctx: &CodeGenContext,
    current_package: &str,
    proto_fqn: &str,
    parent_features: &ResolvedFeatures,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);
    let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, field_name);

    // Explicit-presence fields (Option<T>): set to None.
    if is_explicit_presence_scalar(field, ty, features) {
        return Ok(quote! { self.#ident = ::core::option::Option::None; });
    }

    // If the field has a custom default value (proto2), use it instead of
    // the type's zero value so that clear() matches Default::default().
    if let Some(default_expr) = crate::defaults::parse_default_value(
        field,
        ctx,
        current_package,
        features,
        nesting,
        field_string_repr(ctx, proto_fqn, field_name),
    )? {
        return Ok(quote! { self.#ident = #default_expr; });
    }

    match ty {
        Type::TYPE_STRING => {
            // Non-default string types may be immutable (no `clear()`), so
            // reset to the default value uniformly.
            if field_string_repr(ctx, proto_fqn, field_name).is_default() {
                Ok(quote! { self.#ident.clear(); })
            } else {
                Ok(quote! { self.#ident = ::core::default::Default::default(); })
            }
        }
        Type::TYPE_BYTES => {
            // bytes::Bytes is immutable (no clear()), so reassign.
            if use_bytes {
                Ok(quote! { self.#ident = ::buffa::bytes::Bytes::new(); })
            } else {
                Ok(quote! { self.#ident.clear(); })
            }
        }
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            Ok(quote! { self.#ident = ::buffa::MessageField::none(); })
        }
        Type::TYPE_ENUM => {
            if is_closed_enum(features) {
                Ok(quote! { self.#ident = ::core::default::Default::default(); })
            } else {
                Ok(quote! { self.#ident = ::buffa::EnumValue::from(0); })
            }
        }
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            Ok(quote! { self.#ident = 0i32; })
        }
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            Ok(quote! { self.#ident = 0i64; })
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => Ok(quote! { self.#ident = 0u32; }),
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => Ok(quote! { self.#ident = 0u64; }),
        Type::TYPE_FLOAT => Ok(quote! { self.#ident = 0f32; }),
        Type::TYPE_DOUBLE => Ok(quote! { self.#ident = 0f64; }),
        Type::TYPE_BOOL => Ok(quote! { self.#ident = false; }),
    }
}

/// Generate an encoded-size expression for a value of the given numeric type.
///
/// `val` is the token stream for the value expression — typically either
/// `quote! { v }` (for a local binding) or `quote! { self.#field_ident }`
/// (for a struct field access). Only called for numeric scalar types;
/// string/bytes/enum are handled inline in the callers.
fn type_encoded_size_expr(ty: Type, val: &TokenStream) -> TokenStream {
    match ty {
        Type::TYPE_INT32 => quote! { ::buffa::types::int32_encoded_len(#val) as u32 },
        Type::TYPE_INT64 => quote! { ::buffa::types::int64_encoded_len(#val) as u32 },
        Type::TYPE_UINT32 => quote! { ::buffa::types::uint32_encoded_len(#val) as u32 },
        Type::TYPE_UINT64 => quote! { ::buffa::types::uint64_encoded_len(#val) as u32 },
        Type::TYPE_SINT32 => quote! { ::buffa::types::sint32_encoded_len(#val) as u32 },
        Type::TYPE_SINT64 => quote! { ::buffa::types::sint64_encoded_len(#val) as u32 },
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => {
            quote! { ::buffa::types::FIXED32_ENCODED_LEN as u32 }
        }
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => {
            quote! { ::buffa::types::FIXED64_ENCODED_LEN as u32 }
        }
        Type::TYPE_BOOL => quote! { ::buffa::types::BOOL_ENCODED_LEN as u32 },
        Type::TYPE_STRING
        | Type::TYPE_BYTES
        | Type::TYPE_ENUM
        | Type::TYPE_MESSAGE
        | Type::TYPE_GROUP => unreachable!(
            "type_encoded_size_expr called for non-numeric type {:?}",
            ty
        ),
    }
}

/// Returns `true` if the field is a real (non-synthetic) oneof member.
///
/// Distinguishes actual oneof fields from proto3 optional fields, which also
/// carry an `oneof_index` pointing at a synthetic single-field oneof.
pub(crate) fn is_real_oneof_member(field: &FieldDescriptorProto) -> bool {
    field.oneof_index.is_some() && !field.proto3_optional.unwrap_or(false)
}

/// Returns `true` for every field type that the code generator supports
/// (all types except the deprecated `group` encoding).
pub(crate) fn is_supported_field_type(ty: Type) -> bool {
    matches!(
        ty,
        Type::TYPE_INT32
            | Type::TYPE_INT64
            | Type::TYPE_UINT32
            | Type::TYPE_UINT64
            | Type::TYPE_SINT32
            | Type::TYPE_SINT64
            | Type::TYPE_FIXED32
            | Type::TYPE_FIXED64
            | Type::TYPE_SFIXED32
            | Type::TYPE_SFIXED64
            | Type::TYPE_FLOAT
            | Type::TYPE_DOUBLE
            | Type::TYPE_BOOL
            | Type::TYPE_STRING
            | Type::TYPE_BYTES
            | Type::TYPE_ENUM
            | Type::TYPE_MESSAGE
            | Type::TYPE_GROUP
    )
}

/// Returns the 3-bit wire type byte for the given proto field type.
///
/// Only called for types that pass [`is_supported_field_type`]; the catch-all
/// is therefore unreachable in practice.
pub(crate) fn wire_type_byte(ty: Type) -> u8 {
    match ty {
        Type::TYPE_INT32
        | Type::TYPE_INT64
        | Type::TYPE_UINT32
        | Type::TYPE_UINT64
        | Type::TYPE_SINT32
        | Type::TYPE_SINT64
        | Type::TYPE_BOOL
        | Type::TYPE_ENUM => 0,
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => 1,
        Type::TYPE_STRING | Type::TYPE_BYTES | Type::TYPE_MESSAGE => 2,
        Type::TYPE_GROUP => 3,
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => 5,
    }
}

pub(crate) fn wire_type_token(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_INT32
        | Type::TYPE_INT64
        | Type::TYPE_UINT32
        | Type::TYPE_UINT64
        | Type::TYPE_SINT32
        | Type::TYPE_SINT64
        | Type::TYPE_BOOL
        | Type::TYPE_ENUM => quote! { ::buffa::encoding::WireType::Varint },
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => {
            quote! { ::buffa::encoding::WireType::Fixed64 }
        }
        Type::TYPE_STRING | Type::TYPE_BYTES | Type::TYPE_MESSAGE => {
            quote! { ::buffa::encoding::WireType::LengthDelimited }
        }
        Type::TYPE_GROUP => {
            quote! { ::buffa::encoding::WireType::StartGroup }
        }
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => {
            quote! { ::buffa::encoding::WireType::Fixed32 }
        }
    }
}

fn encode_fn_token(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_INT32 => quote! { ::buffa::types::encode_int32 },
        Type::TYPE_INT64 => quote! { ::buffa::types::encode_int64 },
        Type::TYPE_UINT32 => quote! { ::buffa::types::encode_uint32 },
        Type::TYPE_UINT64 => quote! { ::buffa::types::encode_uint64 },
        Type::TYPE_SINT32 => quote! { ::buffa::types::encode_sint32 },
        Type::TYPE_SINT64 => quote! { ::buffa::types::encode_sint64 },
        Type::TYPE_FIXED32 => quote! { ::buffa::types::encode_fixed32 },
        Type::TYPE_FIXED64 => quote! { ::buffa::types::encode_fixed64 },
        Type::TYPE_SFIXED32 => quote! { ::buffa::types::encode_sfixed32 },
        Type::TYPE_SFIXED64 => quote! { ::buffa::types::encode_sfixed64 },
        Type::TYPE_FLOAT => quote! { ::buffa::types::encode_float },
        Type::TYPE_DOUBLE => quote! { ::buffa::types::encode_double },
        Type::TYPE_BOOL => quote! { ::buffa::types::encode_bool },
        Type::TYPE_STRING
        | Type::TYPE_BYTES
        | Type::TYPE_ENUM
        | Type::TYPE_MESSAGE
        | Type::TYPE_GROUP => unreachable!("encode_fn_token called for non-numeric type {:?}", ty),
    }
}

pub(crate) fn decode_fn_token(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_INT32 => quote! { ::buffa::types::decode_int32 },
        Type::TYPE_INT64 => quote! { ::buffa::types::decode_int64 },
        Type::TYPE_UINT32 => quote! { ::buffa::types::decode_uint32 },
        Type::TYPE_UINT64 => quote! { ::buffa::types::decode_uint64 },
        Type::TYPE_SINT32 => quote! { ::buffa::types::decode_sint32 },
        Type::TYPE_SINT64 => quote! { ::buffa::types::decode_sint64 },
        Type::TYPE_FIXED32 => quote! { ::buffa::types::decode_fixed32 },
        Type::TYPE_FIXED64 => quote! { ::buffa::types::decode_fixed64 },
        Type::TYPE_SFIXED32 => quote! { ::buffa::types::decode_sfixed32 },
        Type::TYPE_SFIXED64 => quote! { ::buffa::types::decode_sfixed64 },
        Type::TYPE_FLOAT => quote! { ::buffa::types::decode_float },
        Type::TYPE_DOUBLE => quote! { ::buffa::types::decode_double },
        Type::TYPE_BOOL => quote! { ::buffa::types::decode_bool },
        Type::TYPE_STRING
        | Type::TYPE_BYTES
        | Type::TYPE_ENUM
        | Type::TYPE_MESSAGE
        | Type::TYPE_GROUP => unreachable!("decode_fn_token called for non-numeric type {:?}", ty),
    }
}

pub(crate) fn is_non_default_expr(ty: Type, field_ident: &Ident) -> TokenStream {
    match ty {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            quote! { self.#field_ident != 0i32 }
        }
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            quote! { self.#field_ident != 0i64 }
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { self.#field_ident != 0u32 },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { self.#field_ident != 0u64 },
        // Float presence is by bit pattern: `to_bits() != 0` is true for NaN
        // (serialized) and for -0.0 (also serialized — the proto3 spec treats
        // only IEEE +0.0 as the default, and the conformance suite checks
        // that -0.0 round-trips through an implicit-presence field).
        Type::TYPE_FLOAT => quote! { self.#field_ident.to_bits() != 0u32 },
        Type::TYPE_DOUBLE => quote! { self.#field_ident.to_bits() != 0u64 },
        Type::TYPE_BOOL => quote! { self.#field_ident },
        Type::TYPE_STRING
        | Type::TYPE_BYTES
        | Type::TYPE_ENUM
        | Type::TYPE_MESSAGE
        | Type::TYPE_GROUP => {
            unreachable!("is_non_default_expr called for non-numeric type {:?}", ty)
        }
    }
}

/// Generate a wire-type guard for a merge/decode match arm.
///
/// Emits `if tag.wire_type() != <expected> { return Err(WireTypeMismatch { ... }) }`.
/// Shared by both owned-type merge (`impl_message.rs`) and view decode (`view.rs`).
pub(crate) fn wire_type_check(
    field_number: u32,
    wire_type: &TokenStream,
    expected_byte: u8,
) -> TokenStream {
    quote! {
        if tag.wire_type() != #wire_type {
            return ::core::result::Result::Err(
                ::buffa::DecodeError::WireTypeMismatch {
                    field_number: #field_number,
                    expected: #expected_byte,
                    actual: tag.wire_type() as u8,
                },
            );
        }
    }
}

/// Compute the varint length of a tag value at codegen time.
///
/// A tag encodes `(field_number << 3) | wire_type_byte` as a varint.
/// The result is always at least 1 byte since field numbers start at 1.
const fn tag_encoded_len(field_number: u32, wire_type: u8) -> u32 {
    let tag_value = ((field_number as u64) << 3) | wire_type as u64;
    // tag_value >= 8 (field_number >= 1), so leading_zeros <= 60 and bits >= 4.
    let bits = 64 - tag_value.leading_zeros();
    bits.div_ceil(7)
}

fn scalar_compute_size_stmt(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);
    let tag_len = tag_encoded_len(field_number, wire_type_byte(ty));
    // Proto2 `required` scalars must always be encoded, even when their value
    // equals the type default (zero / empty).  All other non-optional scalars
    // use proto3-style default-value suppression.
    let is_proto2_required = is_required_field(field, features);

    // Explicit-presence field (proto3 `optional` or proto2 `optional`): encoded as
    // Option<T>; always encode when Some regardless of the field value.
    if is_explicit_presence_scalar(field, ty, features) {
        return match ty {
            Type::TYPE_STRING => Ok(quote! {
                if let Some(ref v) = self.#ident {
                    size += #tag_len + ::buffa::types::string_encoded_len(v) as u32;
                }
            }),
            Type::TYPE_BYTES => Ok(quote! {
                if let Some(ref v) = self.#ident {
                    size += #tag_len + ::buffa::types::bytes_encoded_len(v) as u32;
                }
            }),
            Type::TYPE_ENUM => Ok(quote! {
                if let Some(ref v) = self.#ident {
                    size += #tag_len + ::buffa::types::int32_encoded_len(v.to_i32()) as u32;
                }
            }),
            _ => {
                // Fixed-size types (Fixed32, Float, Bool, …) use a constant;
                // no need to bind the value, which would trigger an unused-
                // variable warning in downstream generated code.
                let v = quote! { v };
                let size_expr = type_encoded_size_expr(ty, &v);
                if matches!(
                    ty,
                    Type::TYPE_FIXED32
                        | Type::TYPE_SFIXED32
                        | Type::TYPE_FLOAT
                        | Type::TYPE_FIXED64
                        | Type::TYPE_SFIXED64
                        | Type::TYPE_DOUBLE
                        | Type::TYPE_BOOL
                ) {
                    Ok(quote! {
                        if self.#ident.is_some() {
                            size += #tag_len + #size_expr;
                        }
                    })
                } else {
                    Ok(quote! {
                        if let Some(v) = self.#ident {
                            size += #tag_len + #size_expr;
                        }
                    })
                }
            }
        };
    }

    // Length-delimited and enum types need different size expressions.
    match ty {
        Type::TYPE_STRING => {
            return Ok(if is_proto2_required {
                quote! { size += #tag_len + ::buffa::types::string_encoded_len(&self.#ident) as u32; }
            } else {
                quote! {
                    if !self.#ident.is_empty() {
                        size += #tag_len + ::buffa::types::string_encoded_len(&self.#ident) as u32;
                    }
                }
            });
        }
        Type::TYPE_BYTES => {
            return Ok(if is_proto2_required {
                quote! { size += #tag_len + ::buffa::types::bytes_encoded_len(&self.#ident) as u32; }
            } else {
                quote! {
                    if !self.#ident.is_empty() {
                        size += #tag_len + ::buffa::types::bytes_encoded_len(&self.#ident) as u32;
                    }
                }
            });
        }
        Type::TYPE_ENUM => {
            return Ok(if is_proto2_required {
                quote! {
                    {
                        let val = self.#ident.to_i32();
                        size += #tag_len + ::buffa::types::int32_encoded_len(val) as u32;
                    }
                }
            } else {
                quote! {
                    {
                        let val = self.#ident.to_i32();
                        if val != 0 {
                            size += #tag_len + ::buffa::types::int32_encoded_len(val) as u32;
                        }
                    }
                }
            });
        }
        Type::TYPE_MESSAGE => {
            return Ok(quote! {
                if self.#ident.is_set() {
                    let __slot = __cache.reserve();
                    let inner_size = self.#ident.compute_size(__cache);
                    __cache.set(__slot, inner_size);
                    size += #tag_len
                        + ::buffa::encoding::varint_len(inner_size as u64) as u32
                        + inner_size;
                }
            });
        }
        Type::TYPE_GROUP => {
            // Groups: start_tag + body + end_tag (no length prefix).
            return Ok(quote! {
                if self.#ident.is_set() {
                    let inner_size = self.#ident.compute_size(__cache);
                    size += #tag_len + inner_size + #tag_len;
                }
            });
        }
        _ => {}
    }

    // Numeric scalars.
    let val = quote! { self.#ident };
    let size_expr = type_encoded_size_expr(ty, &val);
    Ok(if is_proto2_required {
        quote! { size += #tag_len + #size_expr; }
    } else {
        let is_non_default = is_non_default_expr(ty, &ident);
        quote! {
            if #is_non_default {
                size += #tag_len + #size_expr;
            }
        }
    })
}

fn scalar_write_to_stmt(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);
    let is_proto2_required = is_required_field(field, features);

    // Explicit-presence field: encoded as Option<T>; always encode when Some.
    if is_explicit_presence_scalar(field, ty, features) {
        return match ty {
            Type::TYPE_STRING => Ok(quote! {
                if let Some(ref v) = self.#ident {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::LengthDelimited,
                    ).encode(buf);
                    ::buffa::types::encode_string(v, buf);
                }
            }),
            Type::TYPE_BYTES => Ok(quote! {
                if let Some(ref v) = self.#ident {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::LengthDelimited,
                    ).encode(buf);
                    ::buffa::types::encode_bytes(v, buf);
                }
            }),
            Type::TYPE_ENUM => Ok(quote! {
                if let Some(ref v) = self.#ident {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::Varint,
                    ).encode(buf);
                    ::buffa::types::encode_int32(v.to_i32(), buf);
                }
            }),
            _ => {
                let wire_type = wire_type_token(ty);
                let encode_fn = encode_fn_token(ty);
                Ok(quote! {
                    if let Some(v) = self.#ident {
                        ::buffa::encoding::Tag::new(#field_number, #wire_type).encode(buf);
                        #encode_fn(v, buf);
                    }
                })
            }
        };
    }

    // Length-delimited and enum types need different encode calls.
    match ty {
        Type::TYPE_STRING => {
            return Ok(if is_proto2_required {
                quote! {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::LengthDelimited,
                    ).encode(buf);
                    ::buffa::types::encode_string(&self.#ident, buf);
                }
            } else {
                quote! {
                    if !self.#ident.is_empty() {
                        ::buffa::encoding::Tag::new(
                            #field_number,
                            ::buffa::encoding::WireType::LengthDelimited,
                        ).encode(buf);
                        ::buffa::types::encode_string(&self.#ident, buf);
                    }
                }
            });
        }
        Type::TYPE_BYTES => {
            return Ok(if is_proto2_required {
                quote! {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::LengthDelimited,
                    ).encode(buf);
                    ::buffa::types::encode_bytes(&self.#ident, buf);
                }
            } else {
                quote! {
                    if !self.#ident.is_empty() {
                        ::buffa::encoding::Tag::new(
                            #field_number,
                            ::buffa::encoding::WireType::LengthDelimited,
                        ).encode(buf);
                        ::buffa::types::encode_bytes(&self.#ident, buf);
                    }
                }
            });
        }
        Type::TYPE_ENUM => {
            return Ok(if is_proto2_required {
                quote! {
                    {
                        let val = self.#ident.to_i32();
                        ::buffa::encoding::Tag::new(
                            #field_number,
                            ::buffa::encoding::WireType::Varint,
                        ).encode(buf);
                        ::buffa::types::encode_int32(val, buf);
                    }
                }
            } else {
                quote! {
                    {
                        let val = self.#ident.to_i32();
                        if val != 0 {
                            ::buffa::encoding::Tag::new(
                                #field_number,
                                ::buffa::encoding::WireType::Varint,
                            ).encode(buf);
                            ::buffa::types::encode_int32(val, buf);
                        }
                    }
                }
            });
        }
        Type::TYPE_MESSAGE => {
            return Ok(quote! {
                if self.#ident.is_set() {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::LengthDelimited,
                    ).encode(buf);
                    ::buffa::encoding::encode_varint(__cache.consume_next() as u64, buf);
                    self.#ident.write_to(__cache, buf);
                }
            });
        }
        Type::TYPE_GROUP => {
            return Ok(quote! {
                if self.#ident.is_set() {
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::StartGroup,
                    ).encode(buf);
                    self.#ident.write_to(__cache, buf);
                    ::buffa::encoding::Tag::new(
                        #field_number,
                        ::buffa::encoding::WireType::EndGroup,
                    ).encode(buf);
                }
            });
        }
        _ => {}
    }

    // Numeric scalars: encode by value.
    let wire_type = wire_type_token(ty);
    let encode_fn = encode_fn_token(ty);
    Ok(if is_proto2_required {
        quote! {
            ::buffa::encoding::Tag::new(#field_number, #wire_type).encode(buf);
            #encode_fn(self.#ident, buf);
        }
    } else {
        let is_non_default = is_non_default_expr(ty, &ident);
        quote! {
            if #is_non_default {
                ::buffa::encoding::Tag::new(#field_number, #wire_type).encode(buf);
                #encode_fn(self.#ident, buf);
            }
        }
    })
}

/// Generate a merge match arm for a field with explicit presence (`Option<T>`).
///
/// Emits `field_number => { wire_check; self.field = Some(decoded_value); }`.
/// Proto3 optional fields and proto2 optional non-message fields use this path.
#[allow(clippy::too_many_arguments)]
fn explicit_presence_merge_arm(
    ident: &Ident,
    field_number: u32,
    ty: Type,
    features: &ResolvedFeatures,
    wire_check: &TokenStream,
    use_bytes: bool,
    string_repr: crate::StringRepr,
    preserve_unknown_fields: bool,
) -> TokenStream {
    match ty {
        Type::TYPE_STRING if string_repr.is_default() => quote! {
            #field_number => {
                #wire_check
                ::buffa::types::merge_string(
                    self.#ident.get_or_insert_with(::buffa::alloc::string::String::new),
                    buf,
                )?;
            }
        },
        Type::TYPE_STRING => quote! {
            #field_number => {
                #wire_check
                self.#ident = ::core::option::Option::Some(
                    ::buffa::types::decode_string_to(buf)?
                );
            }
        },
        Type::TYPE_BYTES => {
            if use_bytes {
                // bytes::Bytes is immutable — can't merge in place. Replace
                // via decode_bytes_to_bytes (zero-copy if buf is Bytes-backed).
                quote! {
                    #field_number => {
                        #wire_check
                        self.#ident = ::core::option::Option::Some(
                            ::buffa::types::decode_bytes_to_bytes(buf)?
                        );
                    }
                }
            } else {
                quote! {
                    #field_number => {
                        #wire_check
                        ::buffa::types::merge_bytes(
                            self.#ident.get_or_insert_with(::buffa::alloc::vec::Vec::new),
                            buf,
                        )?;
                    }
                }
            }
        }
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(features);
            if closed {
                let unknown_route =
                    closed_enum_unknown_route(field_number, preserve_unknown_fields);
                let decode = closed_enum_decode_with_unknown(
                    &quote! { buf },
                    quote! { self.#ident = ::core::option::Option::Some(__v); },
                    unknown_route,
                );
                quote! {
                    #field_number => {
                        #wire_check
                        #decode
                    }
                }
            } else {
                quote! {
                    #field_number => {
                        #wire_check
                        self.#ident = ::core::option::Option::Some(
                            ::buffa::EnumValue::from(::buffa::types::decode_int32(buf)?)
                        );
                    }
                }
            }
        }
        _ => {
            let decode_fn = decode_fn_token(ty);
            quote! {
                #field_number => {
                    #wire_check
                    self.#ident = ::core::option::Option::Some(#decode_fn(buf)?);
                }
            }
        }
    }
}

fn scalar_merge_arm(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    proto_fqn: &str,
    parent_features: &ResolvedFeatures,
    preserve_unknown_fields: bool,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, field_name);
    let string_repr = field_string_repr(ctx, proto_fqn, field_name);
    let ident = make_field_ident(field_name);
    let wire_type = wire_type_token(ty);
    let expected_byte = wire_type_byte(ty);

    let wire_check = wire_type_check(field_number, &wire_type, expected_byte);

    // Explicit-presence field: assign Some(decoded_value).
    if is_explicit_presence_scalar(field, ty, features) {
        return Ok(explicit_presence_merge_arm(
            &ident,
            field_number,
            ty,
            features,
            &wire_check,
            use_bytes,
            string_repr,
            preserve_unknown_fields,
        ));
    }

    // Length-delimited and enum types need different decode calls.
    // All arms below use proto3 last-wins semantics: the last occurrence of a
    // field on the wire wins.  Contrast with message fields, which use recursive
    // merge, and repeated fields, which append.
    //
    // For non-optional string and bytes fields, use merge_string / merge_bytes
    // to reuse the existing heap allocation rather than decode_string /
    // decode_bytes which always allocate a fresh Vec/String.
    match ty {
        Type::TYPE_STRING => {
            return Ok(if string_repr.is_default() {
                quote! {
                    #field_number => {
                        #wire_check
                        ::buffa::types::merge_string(&mut self.#ident, buf)?;
                    }
                }
            } else {
                // Non-default string types are constructed fresh on decode; the
                // in-place `merge_string` allocation reuse is `String`-only.
                quote! {
                    #field_number => {
                        #wire_check
                        self.#ident = ::buffa::types::decode_string_to(buf)?;
                    }
                }
            });
        }
        Type::TYPE_BYTES => {
            return Ok(if use_bytes {
                quote! {
                    #field_number => {
                        #wire_check
                        self.#ident = ::buffa::types::decode_bytes_to_bytes(buf)?;
                    }
                }
            } else {
                quote! {
                    #field_number => {
                        #wire_check
                        ::buffa::types::merge_bytes(&mut self.#ident, buf)?;
                    }
                }
            });
        }
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(features);
            if closed {
                let unknown_route =
                    closed_enum_unknown_route(field_number, preserve_unknown_fields);
                let decode = closed_enum_decode_with_unknown(
                    &quote! { buf },
                    quote! { self.#ident = __v; },
                    unknown_route,
                );
                return Ok(quote! {
                    #field_number => {
                        #wire_check
                        #decode
                    }
                });
            }
            return Ok(quote! {
                #field_number => {
                    #wire_check
                    self.#ident = ::buffa::EnumValue::from(::buffa::types::decode_int32(buf)?);
                }
            });
        }
        Type::TYPE_MESSAGE => {
            return Ok(quote! {
                #field_number => {
                    #wire_check
                    // Merge into the existing sub-message value (proto merge semantics).
                    ::buffa::Message::merge_length_delimited(
                        self.#ident.get_or_insert_default(),
                        buf,
                        ctx,
                    )?;
                }
            });
        }
        Type::TYPE_GROUP => {
            return Ok(quote! {
                #field_number => {
                    #wire_check
                    // Merge group: read fields until EndGroup tag.
                    ::buffa::Message::merge_group(
                        self.#ident.get_or_insert_default(),
                        buf,
                        ctx,
                        #field_number,
                    )?;
                }
            });
        }
        _ => {}
    }

    // Numeric scalars (proto3 last-wins: plain assignment overwrites any prior value).
    let decode_fn = decode_fn_token(ty);
    Ok(quote! {
        #field_number => {
            #wire_check
            self.#ident = #decode_fn(buf)?;
        }
    })
}

// ---------------------------------------------------------------------------
// Repeated field code generation
// ---------------------------------------------------------------------------

/// Returns `true` if `ty` is a type that can use packed repeated encoding
/// (all numeric scalars, bool, and enum).
pub(crate) fn is_packed_type(ty: Type) -> bool {
    matches!(
        ty,
        Type::TYPE_INT32
            | Type::TYPE_INT64
            | Type::TYPE_UINT32
            | Type::TYPE_UINT64
            | Type::TYPE_SINT32
            | Type::TYPE_SINT64
            | Type::TYPE_FIXED32
            | Type::TYPE_FIXED64
            | Type::TYPE_SFIXED32
            | Type::TYPE_SFIXED64
            | Type::TYPE_FLOAT
            | Type::TYPE_DOUBLE
            | Type::TYPE_BOOL
            | Type::TYPE_ENUM
    )
}

/// Returns `true` if this repeated field should be encoded as packed.
///
/// - **Proto3 / Editions**: packed by default for all numeric scalars and enums,
///   unless overridden by `[packed = false]` or
///   `[features.repeated_field_encoding = EXPANDED]`.
/// - **Proto2**: unpacked by default; packed only when `[packed = true]` or
///   `[features.repeated_field_encoding = PACKED]` is set on the field.
///
/// String, bytes, and message fields are always unpacked regardless of syntax.
pub(crate) fn is_field_packed(field: &FieldDescriptorProto, features: &ResolvedFeatures) -> bool {
    if !is_packed_type(field.r#type.unwrap_or_default()) {
        return false;
    }
    // field.options.packed (proto2/proto3 legacy) takes precedence over features.
    if let Some(packed) = field.options.as_option().and_then(|o| o.packed) {
        return packed;
    }
    // Resolve per-field features: editions protos use
    // FieldOptions.features.repeated_field_encoding instead of the legacy
    // packed option.
    let field_features =
        crate::features::resolve_child(features, crate::features::field_features(field));
    field_features.repeated_field_encoding == crate::features::RepeatedFieldEncoding::Packed
}

/// Generate the payload-size expression for a packed repeated field.
/// The expression evaluates to a `u32` at runtime.
fn repeated_payload_size_expr(ty: Type, ident: &Ident) -> TokenStream {
    match ty {
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => {
            quote! { self.#ident.len() as u32 * ::buffa::types::FIXED32_ENCODED_LEN as u32 }
        }
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => {
            quote! { self.#ident.len() as u32 * ::buffa::types::FIXED64_ENCODED_LEN as u32 }
        }
        Type::TYPE_BOOL => {
            quote! { self.#ident.len() as u32 * ::buffa::types::BOOL_ENCODED_LEN as u32 }
        }
        Type::TYPE_ENUM => {
            quote! {
                self.#ident
                    .iter()
                    .map(|v| ::buffa::types::int32_encoded_len(v.to_i32()) as u32)
                    .sum::<u32>()
            }
        }
        _ => {
            // Varint-sized numeric scalars (Int32, Int64, Uint32, Uint64, Sint32, Sint64):
            // element size depends on the encoded value, so compute per-element via map.
            let v = quote! { v };
            let size_expr = type_encoded_size_expr(ty, &v);
            quote! { self.#ident.iter().map(|&v| #size_expr).sum::<u32>() }
        }
    }
}

fn repeated_compute_size_stmt(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);
    // LengthDelimited tag (wire type 2): used for packed, message, string, bytes.
    let ld_tag_len = tag_encoded_len(field_number, 2);
    // Per-element tag using the field's own wire type: used for unpacked numerics.
    let elem_tag_len = tag_encoded_len(field_number, wire_type_byte(ty));

    if ty == Type::TYPE_MESSAGE {
        // Messages are always length-delimited (one tag per element).
        return Ok(quote! {
            for v in &self.#ident {
                let __slot = __cache.reserve();
                let inner_size = v.compute_size(__cache);
                __cache.set(__slot, inner_size);
                size += #ld_tag_len
                    + ::buffa::encoding::varint_len(inner_size as u64) as u32
                    + inner_size;
            }
        });
    }
    if ty == Type::TYPE_GROUP {
        // Groups: start_tag + body + end_tag per element (no length prefix).
        return Ok(quote! {
            for v in &self.#ident {
                let inner_size = v.compute_size(__cache);
                size += #elem_tag_len + inner_size + #elem_tag_len;
            }
        });
    }
    if !is_field_packed(field, features) {
        // Unpacked: each element emits its own tag + value.
        // String/bytes use LengthDelimited; numeric types use the element wire type.
        // Fixed-width types (float/fixed*/bool) have constant per-element size,
        // so use len()*const instead of a loop (avoids unused-`v` warning and
        // lets LLVM constant-fold).
        match ty {
            Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => {
                return Ok(quote! {
                    size += self.#ident.len() as u32
                        * (#elem_tag_len + ::buffa::types::FIXED32_ENCODED_LEN as u32);
                });
            }
            Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => {
                return Ok(quote! {
                    size += self.#ident.len() as u32
                        * (#elem_tag_len + ::buffa::types::FIXED64_ENCODED_LEN as u32);
                });
            }
            Type::TYPE_BOOL => {
                return Ok(quote! {
                    size += self.#ident.len() as u32
                        * (#elem_tag_len + ::buffa::types::BOOL_ENCODED_LEN as u32);
                });
            }
            _ => {}
        }
        let per_elem_size = match ty {
            Type::TYPE_STRING => {
                quote! { size += #ld_tag_len + ::buffa::types::string_encoded_len(v) as u32; }
            }
            Type::TYPE_BYTES => {
                quote! { size += #ld_tag_len + ::buffa::types::bytes_encoded_len(v) as u32; }
            }
            Type::TYPE_ENUM => {
                quote! { size += #elem_tag_len + ::buffa::types::int32_encoded_len(v.to_i32()) as u32; }
            }
            _ => {
                let deref_v = quote! { *v };
                let size_expr = type_encoded_size_expr(ty, &deref_v);
                quote! { size += #elem_tag_len + #size_expr; }
            }
        };
        return Ok(quote! {
            for v in &self.#ident { #per_elem_size }
        });
    }
    // Packed: single LengthDelimited tag + varint payload length + elements.
    let payload_expr = repeated_payload_size_expr(ty, &ident);
    Ok(quote! {
        if !self.#ident.is_empty() {
            let payload: u32 = #payload_expr;
            size += #ld_tag_len + ::buffa::encoding::varint_len(payload as u64) as u32 + payload;
        }
    })
}

fn repeated_write_to_stmt(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);

    if ty == Type::TYPE_MESSAGE {
        return Ok(quote! {
            for v in &self.#ident {
                ::buffa::encoding::Tag::new(
                    #field_number,
                    ::buffa::encoding::WireType::LengthDelimited,
                ).encode(buf);
                ::buffa::encoding::encode_varint(__cache.consume_next() as u64, buf);
                v.write_to(__cache, buf);
            }
        });
    }
    if ty == Type::TYPE_GROUP {
        return Ok(quote! {
            for v in &self.#ident {
                ::buffa::encoding::Tag::new(
                    #field_number,
                    ::buffa::encoding::WireType::StartGroup,
                ).encode(buf);
                v.write_to(__cache, buf);
                ::buffa::encoding::Tag::new(
                    #field_number,
                    ::buffa::encoding::WireType::EndGroup,
                ).encode(buf);
            }
        });
    }
    if !is_field_packed(field, features) {
        // Unpacked: each element emits its own tag + value.
        let wire_type = wire_type_token(ty);
        let per_elem_encode = match ty {
            Type::TYPE_STRING => quote! { ::buffa::types::encode_string(v, buf); },
            Type::TYPE_BYTES => quote! { ::buffa::types::encode_bytes(v, buf); },
            Type::TYPE_ENUM => quote! { ::buffa::types::encode_int32(v.to_i32(), buf); },
            _ => {
                let encode_fn = encode_fn_token(ty);
                quote! { #encode_fn(*v, buf); }
            }
        };
        return Ok(quote! {
            for v in &self.#ident {
                ::buffa::encoding::Tag::new(#field_number, #wire_type).encode(buf);
                #per_elem_encode
            }
        });
    }
    // Packed.
    let payload_expr = repeated_payload_size_expr(ty, &ident);
    let encode_loop = if ty == Type::TYPE_ENUM {
        quote! { for v in &self.#ident { ::buffa::types::encode_int32(v.to_i32(), buf); } }
    } else {
        let encode_fn = encode_fn_token(ty);
        quote! { for &v in &self.#ident { #encode_fn(v, buf); } }
    };
    Ok(quote! {
        if !self.#ident.is_empty() {
            let payload: u32 = #payload_expr;
            ::buffa::encoding::Tag::new(
                #field_number,
                ::buffa::encoding::WireType::LengthDelimited,
            ).encode(buf);
            ::buffa::encoding::encode_varint(payload as u64, buf);
            #encode_loop
        }
    })
}

fn repeated_merge_arm(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    proto_fqn: &str,
    parent_features: &ResolvedFeatures,
    preserve_unknown_fields: bool,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, field_name);
    let ident = make_field_ident(field_name);

    if ty == Type::TYPE_MESSAGE {
        let wire_check = wire_type_check(
            field_number,
            &quote! { ::buffa::encoding::WireType::LengthDelimited },
            2u8,
        );
        return Ok(quote! {
            #field_number => {
                #wire_check
                let mut elem = ::core::default::Default::default();
                ::buffa::Message::merge_length_delimited(&mut elem, buf, ctx)?;
                self.#ident.push(elem);
            }
        });
    }
    if ty == Type::TYPE_GROUP {
        let wire_check = wire_type_check(
            field_number,
            &quote! { ::buffa::encoding::WireType::StartGroup },
            3u8,
        );
        return Ok(quote! {
            #field_number => {
                #wire_check
                let mut elem = ::core::default::Default::default();
                ::buffa::Message::merge_group(&mut elem, buf, ctx, #field_number)?;
                self.#ident.push(elem);
            }
        });
    }
    if !is_packed_type(ty) {
        let wire_check = wire_type_check(
            field_number,
            &quote! { ::buffa::encoding::WireType::LengthDelimited },
            2u8,
        );
        let decode_expr = match ty {
            Type::TYPE_STRING if field_string_repr(ctx, proto_fqn, field_name).is_default() => {
                quote! { ::buffa::types::decode_string(buf)? }
            }
            Type::TYPE_STRING => quote! { ::buffa::types::decode_string_to(buf)? },
            Type::TYPE_BYTES => {
                if use_bytes {
                    quote! { ::buffa::types::decode_bytes_to_bytes(buf)? }
                } else {
                    quote! { ::buffa::types::decode_bytes(buf)? }
                }
            }
            // Message and group are handled by early returns above; the
            // remaining types satisfy `is_packed_type` and never reach this
            // unpacked branch. Enumerated so adding a `Type` variant is a
            // compile error here rather than a runtime panic during codegen.
            Type::TYPE_MESSAGE
            | Type::TYPE_GROUP
            | Type::TYPE_ENUM
            | Type::TYPE_BOOL
            | Type::TYPE_INT32
            | Type::TYPE_INT64
            | Type::TYPE_UINT32
            | Type::TYPE_UINT64
            | Type::TYPE_SINT32
            | Type::TYPE_SINT64
            | Type::TYPE_FIXED32
            | Type::TYPE_FIXED64
            | Type::TYPE_SFIXED32
            | Type::TYPE_SFIXED64
            | Type::TYPE_FLOAT
            | Type::TYPE_DOUBLE => {
                unreachable!("repeated_merge_arm: unhandled unpacked type {:?}", ty)
            }
        };
        return Ok(quote! {
            #field_number => {
                #wire_check
                self.#ident.push(#decode_expr);
            }
        });
    }
    // Packed: accept both packed (LengthDelimited) and unpacked (element wire type).
    let element_wire_type = wire_type_token(ty);
    // Packed path: decode from a length-limited sub-buffer.
    let closed = is_closed_enum(features);
    let push_known = quote! { self.#ident.push(__v); };
    let unknown_route = closed_enum_unknown_route(field_number, preserve_unknown_fields);
    let decode_packed_elem = if ty == Type::TYPE_ENUM {
        if closed {
            closed_enum_decode_with_unknown(
                &quote! { &mut limited },
                push_known.clone(),
                unknown_route.clone(),
            )
        } else {
            quote! { self.#ident.push(::buffa::EnumValue::from(::buffa::types::decode_int32(&mut limited)?)); }
        }
    } else {
        let decode_fn = decode_fn_token(ty);
        quote! { self.#ident.push(#decode_fn(&mut limited)?); }
    };
    // Unpacked path: decode a single element from the outer buffer.
    let decode_unpacked_elem = if ty == Type::TYPE_ENUM {
        if closed {
            closed_enum_decode_with_unknown(&quote! { buf }, push_known, unknown_route)
        } else {
            quote! { self.#ident.push(::buffa::EnumValue::from(::buffa::types::decode_int32(buf)?)); }
        }
    } else {
        let decode_fn = decode_fn_token(ty);
        quote! { self.#ident.push(#decode_fn(buf)?); }
    };
    // Pre-allocation hint for the packed decode loop: avoids repeated
    // reallocation. Fixed-size types get the exact element count; variable-
    // width types use the byte count as an upper bound (≥1 byte/element).
    // Pre-allocation hint. For bool and varint types the divisor is 1 (each
    // element takes at least 1 byte); skip the division to avoid the
    // `clippy::identity_op` lint in generated code.
    let reserve_divisor: usize = match ty {
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => 4,
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => 8,
        _ => 1,
    };
    let reserve_stmt = if reserve_divisor > 1 {
        quote! { self.#ident.reserve(len / #reserve_divisor); }
    } else {
        quote! { self.#ident.reserve(len); }
    };

    Ok(quote! {
        #field_number => {
            if tag.wire_type() == ::buffa::encoding::WireType::LengthDelimited {
                // Packed encoding.
                let len = ::buffa::encoding::decode_varint(buf)?;
                let len = usize::try_from(len)
                    .map_err(|_| ::buffa::DecodeError::MessageTooLarge)?;
                if buf.remaining() < len {
                    return ::core::result::Result::Err(::buffa::DecodeError::UnexpectedEof);
                }
                #reserve_stmt
                let mut limited = buf.take(len);
                while limited.has_remaining() {
                    #decode_packed_elem
                }
                // Advance past any trailing bytes left by the decode loop.
                // This fires when a malformed packed payload has a length not
                // aligned to the element size for fixed-size types; for varint
                // types, `decode_fn` above will already have returned an error
                // via `UnexpectedEof`, so this branch is dead for valid input.
                let leftover = limited.remaining();
                if leftover > 0 {
                    limited.advance(leftover);
                }
            } else if tag.wire_type() == #element_wire_type {
                // Unpacked (backward compatibility with older encoders).
                #decode_unpacked_elem
            } else {
                // This field accepts LengthDelimited (packed) or the element
                // wire type (unpacked); report the packed wire type as expected.
                return ::core::result::Result::Err(
                    ::buffa::DecodeError::WireTypeMismatch {
                        field_number: #field_number,
                        expected: 2u8,
                        actual: tag.wire_type() as u8,
                    },
                );
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Oneof field code generation
// ---------------------------------------------------------------------------

/// Generate a `compute_size` match arm for one oneof variant.
///
/// Emits `EnumIdent::VariantIdent(x) => { size += tag_len + encoded_len; }`.
fn oneof_size_arm(
    enum_ident: &TokenStream,
    variant_ident: &Ident,
    tag_len: u32,
    ty: Type,
) -> TokenStream {
    match ty {
        Type::TYPE_STRING => quote! {
            #enum_ident::#variant_ident(x) => {
                size += #tag_len + ::buffa::types::string_encoded_len(x) as u32;
            }
        },
        Type::TYPE_BYTES => quote! {
            #enum_ident::#variant_ident(x) => {
                size += #tag_len + ::buffa::types::bytes_encoded_len(x) as u32;
            }
        },
        Type::TYPE_ENUM => quote! {
            #enum_ident::#variant_ident(x) => {
                size += #tag_len + ::buffa::types::int32_encoded_len(x.to_i32()) as u32;
            }
        },
        Type::TYPE_MESSAGE => quote! {
            #enum_ident::#variant_ident(x) => {
                let __slot = __cache.reserve();
                let inner = x.compute_size(__cache);
                __cache.set(__slot, inner);
                size += #tag_len
                    + ::buffa::encoding::varint_len(inner as u64) as u32
                    + inner;
            }
        },
        Type::TYPE_GROUP => quote! {
            #enum_ident::#variant_ident(x) => {
                let inner = x.compute_size(__cache);
                size += #tag_len + inner + #tag_len;
            }
        },
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => quote! {
            #enum_ident::#variant_ident(_x) => {
                size += #tag_len + ::buffa::types::FIXED32_ENCODED_LEN as u32;
            }
        },
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => quote! {
            #enum_ident::#variant_ident(_x) => {
                size += #tag_len + ::buffa::types::FIXED64_ENCODED_LEN as u32;
            }
        },
        Type::TYPE_BOOL => quote! {
            #enum_ident::#variant_ident(_x) => {
                size += #tag_len + ::buffa::types::BOOL_ENCODED_LEN as u32;
            }
        },
        _ => {
            // Varint scalars (int32/64, uint32/64, sint32/64).
            // The oneof is matched via `if let Some(ref v) = self.field`
            // so the variant binding v: &T must be dereferenced.
            let deref_v = quote! { *v };
            let size_expr = type_encoded_size_expr(ty, &deref_v);
            quote! {
                #enum_ident::#variant_ident(v) => {
                    size += #tag_len + #size_expr;
                }
            }
        }
    }
}

/// Generate a `write_to` match arm for one oneof variant.
///
/// Emits `EnumIdent::VariantIdent(x) => { Tag::new(...).encode(buf); encode_*(x, buf); }`.
fn oneof_write_arm(
    enum_ident: &TokenStream,
    variant_ident: &Ident,
    field_number: u32,
    ty: Type,
    wire_type: &TokenStream,
) -> TokenStream {
    match ty {
        Type::TYPE_STRING => quote! {
            #enum_ident::#variant_ident(x) => {
                ::buffa::encoding::Tag::new(
                    #field_number, ::buffa::encoding::WireType::LengthDelimited,
                ).encode(buf);
                ::buffa::types::encode_string(x, buf);
            }
        },
        Type::TYPE_BYTES => quote! {
            #enum_ident::#variant_ident(x) => {
                ::buffa::encoding::Tag::new(
                    #field_number, ::buffa::encoding::WireType::LengthDelimited,
                ).encode(buf);
                ::buffa::types::encode_bytes(x, buf);
            }
        },
        Type::TYPE_ENUM => quote! {
            #enum_ident::#variant_ident(x) => {
                ::buffa::encoding::Tag::new(
                    #field_number, ::buffa::encoding::WireType::Varint,
                ).encode(buf);
                ::buffa::types::encode_int32(x.to_i32(), buf);
            }
        },
        Type::TYPE_MESSAGE => quote! {
            #enum_ident::#variant_ident(x) => {
                ::buffa::encoding::Tag::new(
                    #field_number, ::buffa::encoding::WireType::LengthDelimited,
                ).encode(buf);
                ::buffa::encoding::encode_varint(__cache.consume_next() as u64, buf);
                x.write_to(__cache, buf);
            }
        },
        Type::TYPE_GROUP => quote! {
            #enum_ident::#variant_ident(x) => {
                ::buffa::encoding::Tag::new(
                    #field_number, ::buffa::encoding::WireType::StartGroup,
                ).encode(buf);
                x.write_to(__cache, buf);
                ::buffa::encoding::Tag::new(
                    #field_number, ::buffa::encoding::WireType::EndGroup,
                ).encode(buf);
            }
        },
        _ => {
            let encode_fn = encode_fn_token(ty);
            quote! {
                #enum_ident::#variant_ident(x) => {
                    ::buffa::encoding::Tag::new(#field_number, #wire_type).encode(buf);
                    #encode_fn(*x, buf);
                }
            }
        }
    }
}

/// Generate a `merge` match arm for one oneof variant.
///
/// Emits `field_number => { wire_check; self.field = Some(EnumIdent::Variant(decoded)); }`.
/// Message variants use merge-into-existing semantics; closed enums with
/// unknown values are routed to `__buffa_unknown_fields` and the oneof is
/// left unset (matching Java's reference behavior and the singular-field spec).
#[allow(clippy::too_many_arguments)]
fn oneof_merge_arm(
    field_ident: &Ident,
    enum_ident: &TokenStream,
    variant_ident: &Ident,
    field_number: u32,
    ty: Type,
    features: &ResolvedFeatures,
    preserve_unknown_fields: bool,
    use_bytes: bool,
    string_repr: crate::StringRepr,
    boxed: bool,
) -> TokenStream {
    let wire_type = wire_type_token(ty);
    let wire_byte = wire_type_byte(ty);
    let wire_check = wire_type_check(field_number, &wire_type, wire_byte);
    // Message/group variants merge into the existing value. When boxed, the
    // binding is `&mut Box<M>` (deref once); when stored inline it is `&mut M`,
    // and the freshly decoded value is moved in without a `Box`.
    let existing_ref = if boxed {
        quote! { &mut **existing }
    } else {
        quote! { existing }
    };
    let wrapped_val = if boxed {
        quote! { ::buffa::alloc::boxed::Box::new(val) }
    } else {
        quote! { val }
    };
    match ty {
        Type::TYPE_STRING => {
            let decoded = if string_repr.is_default() {
                quote! { ::buffa::types::decode_string(buf)? }
            } else {
                quote! { ::buffa::types::decode_string_to(buf)? }
            };
            quote! {
                #field_number => {
                    #wire_check
                    self.#field_ident = ::core::option::Option::Some(
                        #enum_ident::#variant_ident(#decoded)
                    );
                }
            }
        }
        Type::TYPE_BYTES => {
            let decoded = if use_bytes {
                quote! { ::buffa::types::decode_bytes_to_bytes(buf)? }
            } else {
                quote! { ::buffa::types::decode_bytes(buf)? }
            };
            quote! {
                #field_number => {
                    #wire_check
                    self.#field_ident = ::core::option::Option::Some(
                        #enum_ident::#variant_ident(#decoded)
                    );
                }
            }
        }
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(features);
            if closed {
                let unknown_route =
                    closed_enum_unknown_route(field_number, preserve_unknown_fields);
                let decode = closed_enum_decode_with_unknown(
                    &quote! { buf },
                    quote! {
                        self.#field_ident = ::core::option::Option::Some(
                            #enum_ident::#variant_ident(__v)
                        );
                    },
                    unknown_route,
                );
                quote! {
                    #field_number => {
                        #wire_check
                        #decode
                    }
                }
            } else {
                quote! {
                    #field_number => {
                        #wire_check
                        self.#field_ident = ::core::option::Option::Some(
                            #enum_ident::#variant_ident(
                                ::buffa::EnumValue::from(::buffa::types::decode_int32(buf)?)
                            )
                        );
                    }
                }
            }
        }
        Type::TYPE_MESSAGE => quote! {
            #field_number => {
                #wire_check
                // Proto3 merge semantics: if this oneof variant is already
                // set, merge into the existing value rather than replacing it.
                if let ::core::option::Option::Some(
                    #enum_ident::#variant_ident(ref mut existing)
                ) = self.#field_ident {
                    ::buffa::Message::merge_length_delimited(#existing_ref, buf, ctx)?;
                } else {
                    let mut val = ::core::default::Default::default();
                    ::buffa::Message::merge_length_delimited(&mut val, buf, ctx)?;
                    self.#field_ident = ::core::option::Option::Some(
                        #enum_ident::#variant_ident(#wrapped_val)
                    );
                }
            }
        },
        Type::TYPE_GROUP => quote! {
            #field_number => {
                #wire_check
                if let ::core::option::Option::Some(
                    #enum_ident::#variant_ident(ref mut existing)
                ) = self.#field_ident {
                    ::buffa::Message::merge_group(#existing_ref, buf, ctx, #field_number)?;
                } else {
                    let mut val = ::core::default::Default::default();
                    ::buffa::Message::merge_group(&mut val, buf, ctx, #field_number)?;
                    self.#field_ident = ::core::option::Option::Some(
                        #enum_ident::#variant_ident(#wrapped_val)
                    );
                }
            }
        },
        _ => {
            let decode_fn = decode_fn_token(ty);
            quote! {
                #field_number => {
                    #wire_check
                    self.#field_ident = ::core::option::Option::Some(
                        #enum_ident::#variant_ident(#decode_fn(buf)?)
                    );
                }
            }
        }
    }
}

/// Generate compute_size, write_to, and merge tokens for one oneof group.
///
/// Returns `(compute_stmt, write_stmt, merge_arms)` where `merge_arms` is one
/// arm per field belonging to the oneof.
#[allow(clippy::too_many_arguments)]
fn generate_oneof_impls(
    ctx: &CodeGenContext,
    enum_ident: &proc_macro2::Ident,
    oneof_name: &str,
    fields: &[&FieldDescriptorProto],
    oneof_prefix: &TokenStream,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    preserve_unknown_fields: bool,
) -> Result<(TokenStream, TokenStream, Vec<TokenStream>), CodeGenError> {
    let field_ident = make_field_ident(oneof_name);
    let qualified_enum: TokenStream = quote! { #oneof_prefix #enum_ident };

    let mut size_arms: Vec<TokenStream> = Vec::new();
    let mut write_arms: Vec<TokenStream> = Vec::new();
    let mut merge_arm_list: Vec<TokenStream> = Vec::new();

    for field in fields {
        let field_name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let field_number = validated_field_number(field)?;
        let ty = effective_type(ctx, field, features);
        let variant_ident = crate::oneof::oneof_variant_ident(field_name);
        let tag_len = tag_encoded_len(field_number, wire_type_byte(ty));
        let wire_type = wire_type_token(ty);

        size_arms.push(oneof_size_arm(&qualified_enum, &variant_ident, tag_len, ty));
        write_arms.push(oneof_write_arm(
            &qualified_enum,
            &variant_ident,
            field_number,
            ty,
            &wire_type,
        ));
        let field_features = crate::features::resolve_field(ctx, field, features);
        let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, field_name);
        let string_repr = field_string_repr(ctx, proto_fqn, field_name);
        let boxed = crate::oneof::variant_boxed(
            ctx,
            ty,
            &format!(".{proto_fqn}.{oneof_name}.{field_name}"),
        );
        merge_arm_list.push(oneof_merge_arm(
            &field_ident,
            &qualified_enum,
            &variant_ident,
            field_number,
            ty,
            &field_features,
            preserve_unknown_fields,
            use_bytes,
            string_repr,
            boxed,
        ));
    }

    let compute_stmt = quote! {
        if let ::core::option::Option::Some(ref v) = self.#field_ident {
            match v {
                #(#size_arms)*
            }
        }
    };
    let write_stmt = quote! {
        if let ::core::option::Option::Some(ref v) = self.#field_ident {
            match v {
                #(#write_arms)*
            }
        }
    };

    Ok((compute_stmt, write_stmt, merge_arm_list))
}

// ---------------------------------------------------------------------------
// Map field code generation
// ---------------------------------------------------------------------------

/// Get the key and value field descriptors from a map-entry nested type.
pub(crate) fn find_map_entry_fields<'a>(
    msg: &'a DescriptorProto,
    field: &FieldDescriptorProto,
) -> Result<(&'a FieldDescriptorProto, &'a FieldDescriptorProto), CodeGenError> {
    let entry = find_map_entry(msg, field).ok_or_else(|| {
        let type_name = field.type_name.as_deref().unwrap_or("<unknown>");
        CodeGenError::Other(format!("map entry not found for {type_name}"))
    })?;
    let key = entry
        .field
        .iter()
        .find(|f| f.number == Some(1))
        .ok_or(CodeGenError::MissingField("map_entry.key"))?;
    let val = entry
        .field
        .iter()
        .find(|f| f.number == Some(2))
        .ok_or(CodeGenError::MissingField("map_entry.value"))?;
    Ok((key, val))
}

/// Generate the encoded-byte-size expression for a single map entry element
/// (key or value) bound to the variable named `var`. Uses `*var` for copy
/// scalars, `var` for string/bytes/enum, and `var.compute_size()` for messages.
fn map_element_size_expr(ty: Type, var: &Ident) -> TokenStream {
    match ty {
        Type::TYPE_STRING => quote! { ::buffa::types::string_encoded_len(#var) as u32 },
        Type::TYPE_BYTES => quote! { ::buffa::types::bytes_encoded_len(#var) as u32 },
        Type::TYPE_ENUM => quote! { ::buffa::types::int32_encoded_len(#var.to_i32()) as u32 },
        // Message values are phase-dependent (compute reserves a SizeCache
        // slot, write reads it) so callers handle them explicitly. Keys
        // cannot be message-typed per the proto spec.
        Type::TYPE_MESSAGE => {
            unreachable!("message map values are handled per-phase by callers")
        }
        Type::TYPE_FIXED32 | Type::TYPE_SFIXED32 | Type::TYPE_FLOAT => {
            quote! { ::buffa::types::FIXED32_ENCODED_LEN as u32 }
        }
        Type::TYPE_FIXED64 | Type::TYPE_SFIXED64 | Type::TYPE_DOUBLE => {
            quote! { ::buffa::types::FIXED64_ENCODED_LEN as u32 }
        }
        Type::TYPE_BOOL => quote! { ::buffa::types::BOOL_ENCODED_LEN as u32 },
        _ => {
            let deref_var = quote! { *#var };
            let size_expr = type_encoded_size_expr(ty, &deref_var);
            quote! { #size_expr }
        }
    }
}

/// True if `map_element_size_expr` for this type is a constant (ignores `var`).
fn map_element_size_is_constant(ty: Type) -> bool {
    matches!(
        ty,
        Type::TYPE_FIXED32
            | Type::TYPE_SFIXED32
            | Type::TYPE_FLOAT
            | Type::TYPE_FIXED64
            | Type::TYPE_SFIXED64
            | Type::TYPE_DOUBLE
            | Type::TYPE_BOOL
    )
}

/// Generate the write expression for a single map entry element.
fn map_element_encode_stmt(ty: Type, tag_num: u32, var: &Ident) -> TokenStream {
    let wire_type = wire_type_token(ty);
    let tag = quote! { ::buffa::encoding::Tag::new(#tag_num, #wire_type).encode(buf); };
    let payload = match ty {
        Type::TYPE_STRING => quote! { ::buffa::types::encode_string(#var, buf); },
        Type::TYPE_BYTES => quote! { ::buffa::types::encode_bytes(#var, buf); },
        Type::TYPE_ENUM => quote! { ::buffa::types::encode_int32(#var.to_i32(), buf); },
        Type::TYPE_MESSAGE => {
            quote! {
                ::buffa::encoding::encode_varint(__v_len as u64, buf);
                #var.write_to(__cache, buf);
            }
        }
        _ => {
            let encode_fn = encode_fn_token(ty);
            quote! { #encode_fn(*#var, buf); }
        }
    };
    quote! { #tag #payload }
}

/// Generate the decode statement for a single map entry element in the merge loop.
///
/// `buf_expr` is the token stream for the buffer expression — typically
/// `quote! { buf }` when the buffer is the outer `merge_to_limit` parameter
/// (already `&mut impl Buf`).
///
/// `use_bytes` switches a `bytes`-typed element from `decode_bytes` (→ `Vec<u8>`)
/// to `decode_bytes_to_bytes` (→ `bytes::Bytes`, zero-copy when `buf` is
/// `Bytes`-backed). Only meaningful for value reads on `map<K, bytes>` fields
/// when the outer field matches `bytes_fields`; keys are never bytes-typed.
fn map_element_decode_stmt(
    ty: Type,
    var: &Ident,
    buf_expr: &TokenStream,
    features: &ResolvedFeatures,
    use_bytes: bool,
) -> TokenStream {
    let wire_type = wire_type_token(ty);
    let wire_byte = wire_type_byte(ty);
    let tag_check = quote! {
        if entry_tag.wire_type() != #wire_type {
            return ::core::result::Result::Err(::buffa::DecodeError::WireTypeMismatch {
                field_number: entry_tag.field_number(),
                expected: #wire_byte,
                actual: entry_tag.wire_type() as u8,
            });
        }
    };
    let closed = is_closed_enum(features);
    let assign = match ty {
        Type::TYPE_STRING => quote! { #var = ::buffa::types::decode_string(#buf_expr)?; },
        Type::TYPE_BYTES if use_bytes => {
            quote! { #var = ::buffa::types::decode_bytes_to_bytes(#buf_expr)?; }
        }
        Type::TYPE_BYTES => quote! { #var = ::buffa::types::decode_bytes(#buf_expr)?; },
        Type::TYPE_ENUM => {
            if closed {
                closed_enum_decode(buf_expr, quote! { #var = __v; })
            } else {
                quote! { #var = ::buffa::EnumValue::from(::buffa::types::decode_int32(#buf_expr)?); }
            }
        }
        Type::TYPE_MESSAGE => {
            quote! { ::buffa::Message::merge_length_delimited(&mut #var, #buf_expr, ctx)?; }
        }
        _ => {
            let decode_fn = decode_fn_token(ty);
            quote! { #var = #decode_fn(#buf_expr)?; }
        }
    };
    quote! { #tag_check #assign }
}

fn map_compute_size_stmt(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(field_name);
    let outer_tag_len = tag_encoded_len(field_number, 2);
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;
    let key_ty = effective_type_in_map_entry(ctx, key_fd, features);
    let val_ty = effective_type_in_map_entry(ctx, val_fd, features);
    let key_tag_len = tag_encoded_len(1, wire_type_byte(key_ty));
    let val_tag_len = tag_encoded_len(2, wire_type_byte(val_ty));
    let k = format_ident!("k");
    let v = format_ident!("v");
    let key_size = map_element_size_expr(key_ty, &k);
    let val_size = if val_ty == Type::TYPE_MESSAGE {
        quote! {
            {
                let __slot = __cache.reserve();
                let inner = #v.compute_size(__cache);
                __cache.set(__slot, inner);
                ::buffa::encoding::varint_len(inner as u64) as u32 + inner
            }
        }
    } else {
        map_element_size_expr(val_ty, &v)
    };
    // Both passes iterate `for (k, v) in &self.#ident`, identical to
    // `map_write_to_stmt`, so SizeCache slot order matches by construction.
    // When both key and value are fixed-width (no cache slots reserved) the
    // entry size is constant and we fold to `len() * const`.
    if map_element_size_is_constant(key_ty) && map_element_size_is_constant(val_ty) {
        return Ok(quote! {
            {
                let entry_size: u32 = #key_tag_len + #key_size + #val_tag_len + #val_size;
                size += self.#ident.len() as u32 * (#outer_tag_len
                    + ::buffa::encoding::varint_len(entry_size as u64) as u32
                    + entry_size);
            }
        });
    }
    let k_bind = if map_element_size_is_constant(key_ty) {
        format_ident!("_{}", k)
    } else {
        k
    };
    let v_bind = if map_element_size_is_constant(val_ty) {
        format_ident!("_{}", v)
    } else {
        v
    };
    Ok(quote! {
        #[allow(clippy::for_kv_map)]
        for (#k_bind, #v_bind) in &self.#ident {
            let entry_size: u32 = #key_tag_len + #key_size + #val_tag_len + #val_size;
            size += #outer_tag_len
                + ::buffa::encoding::varint_len(entry_size as u64) as u32
                + entry_size;
        }
    })
}

fn map_write_to_stmt(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(field_name);
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;
    let key_ty = effective_type_in_map_entry(ctx, key_fd, features);
    let val_ty = effective_type_in_map_entry(ctx, val_fd, features);
    let key_tag_len = tag_encoded_len(1, wire_type_byte(key_ty));
    let val_tag_len = tag_encoded_len(2, wire_type_byte(val_ty));
    let k = format_ident!("k");
    let v = format_ident!("v");
    let key_size = map_element_size_expr(key_ty, &k);
    let (val_len_bind, val_size) = if val_ty == Type::TYPE_MESSAGE {
        (
            quote! { let __v_len = __cache.consume_next(); },
            quote! { (::buffa::encoding::varint_len(__v_len as u64) as u32 + __v_len) },
        )
    } else {
        (quote! {}, map_element_size_expr(val_ty, &v))
    };
    let encode_key = map_element_encode_stmt(key_ty, 1, &k);
    let encode_val = map_element_encode_stmt(val_ty, 2, &v);
    Ok(quote! {
        for (#k, #v) in &self.#ident {
            #val_len_bind
            let entry_size: u32 = #key_tag_len + #key_size + #val_tag_len + #val_size;
            ::buffa::encoding::Tag::new(
                #field_number,
                ::buffa::encoding::WireType::LengthDelimited,
            ).encode(buf);
            ::buffa::encoding::encode_varint(entry_size as u64, buf);
            #encode_key
            #encode_val
        }
    })
}

fn map_merge_arm(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
    proto_fqn: &str,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(field_name);
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;
    let key_ty = effective_type_in_map_entry(ctx, key_fd, features);
    let val_ty = effective_type_in_map_entry(ctx, val_fd, features);
    let k = format_ident!("key");
    let v = format_ident!("val");
    let buf_expr = quote! { buf };
    // Resolve features per map-entry field so enum_type reflects the
    // referenced enum's declaration (not the parent message's).
    let key_features = crate::features::resolve_field(ctx, key_fd, features);
    let val_features = crate::features::resolve_field(ctx, val_fd, features);
    // `bytes_fields` on `map<K, bytes>` → value decodes into `Bytes` (shared
    // carve-out in `map_value_use_bytes`; bytes-key + bytes-value stays `Vec<u8>`).
    let value_use_bytes =
        map_value_use_bytes(ctx, Some(key_ty), Some(val_ty), proto_fqn, field_name);
    let decode_key = map_element_decode_stmt(key_ty, &k, &buf_expr, &key_features, false);
    let decode_val = map_element_decode_stmt(val_ty, &v, &buf_expr, &val_features, value_use_bytes);
    let wire_check = wire_type_check(
        field_number,
        &quote! { ::buffa::encoding::WireType::LengthDelimited },
        2u8,
    );
    Ok(quote! {
        #field_number => {
            #wire_check
            let entry_len = ::buffa::encoding::decode_varint(buf)?;
            let entry_len = usize::try_from(entry_len)
                .map_err(|_| ::buffa::DecodeError::MessageTooLarge)?;
            if buf.remaining() < entry_len {
                return ::core::result::Result::Err(::buffa::DecodeError::UnexpectedEof);
            }
            let entry_limit = buf.remaining() - entry_len;
            let mut #k = ::core::default::Default::default();
            let mut #v = ::core::default::Default::default();
            while buf.remaining() > entry_limit {
                let entry_tag = ::buffa::encoding::Tag::decode(buf)?;
                match entry_tag.field_number() {
                    1 => { #decode_key }
                    2 => { #decode_val }
                    _ => { ::buffa::encoding::skip_field_depth(entry_tag, buf, ctx.depth())?; }
                }
            }
            // Correct the buffer position if the entry was not fully consumed.
            if buf.remaining() != entry_limit {
                let remaining = buf.remaining();
                if remaining > entry_limit {
                    buf.advance(remaining - entry_limit);
                } else {
                    return ::core::result::Result::Err(::buffa::DecodeError::UnexpectedEof);
                }
            }
            self.#ident.insert(#k, #v);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
    use crate::generated::descriptor::{FieldDescriptorProto, FieldOptions};

    fn make_field(ty: Type, label: Label) -> FieldDescriptorProto {
        FieldDescriptorProto {
            r#type: Some(ty),
            label: Some(label),
            ..Default::default()
        }
    }

    // ── classify_fields_ordered ──────────────────────────────────────────

    fn nfield(name: &str, num: i32, ty: Type, label: Label) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.into()),
            number: Some(num),
            r#type: Some(ty),
            label: Some(label),
            ..Default::default()
        }
    }

    fn kind_tag(k: &FieldKind<'_>) -> (&'static str, i32) {
        match k {
            FieldKind::Scalar(f) => ("scalar", f.number.unwrap()),
            FieldKind::Repeated(f) => ("repeated", f.number.unwrap()),
            FieldKind::Map(f) => ("map", f.number.unwrap()),
            FieldKind::Oneof { fields, .. } => (
                "oneof",
                fields.iter().map(|f| f.number.unwrap()).min().unwrap(),
            ),
        }
    }

    #[test]
    fn classify_orders_by_field_number_across_kinds() {
        use crate::generated::descriptor::OneofDescriptorProto;
        // Declared deliberately out of number order; oneof members at 5 & 6.
        let oneof_a = FieldDescriptorProto {
            oneof_index: Some(0),
            ..nfield("choice_a", 5, Type::TYPE_INT32, Label::LABEL_OPTIONAL)
        };
        let oneof_b = FieldDescriptorProto {
            oneof_index: Some(0),
            ..nfield("choice_b", 6, Type::TYPE_STRING, Label::LABEL_OPTIONAL)
        };
        let msg = DescriptorProto {
            field: vec![
                nfield("tail", 8, Type::TYPE_INT64, Label::LABEL_OPTIONAL),
                nfield("tags", 2, Type::TYPE_INT32, Label::LABEL_REPEATED),
                oneof_b,
                nfield("head", 1, Type::TYPE_STRING, Label::LABEL_OPTIONAL),
                oneof_a,
                nfield("names", 7, Type::TYPE_STRING, Label::LABEL_REPEATED),
            ],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("choice".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let oneof_idents = std::collections::HashMap::from([(0usize, format_ident!("Choice"))]);
        let ordered = classify_fields_ordered(&msg, &oneof_idents).unwrap();
        let got: Vec<_> = ordered.iter().map(kind_tag).collect();
        assert_eq!(
            got,
            vec![
                ("scalar", 1),
                ("repeated", 2),
                ("oneof", 5),
                ("repeated", 7),
                ("scalar", 8),
            ]
        );
    }

    #[test]
    fn classify_positions_oneof_at_min_member_number() {
        use crate::generated::descriptor::OneofDescriptorProto;
        // Higher-numbered oneof member appears first in declaration order.
        let hi = FieldDescriptorProto {
            oneof_index: Some(0),
            ..nfield("hi", 10, Type::TYPE_INT32, Label::LABEL_OPTIONAL)
        };
        let lo = FieldDescriptorProto {
            oneof_index: Some(0),
            ..nfield("lo", 3, Type::TYPE_INT32, Label::LABEL_OPTIONAL)
        };
        let msg = DescriptorProto {
            field: vec![
                hi,
                nfield("mid", 5, Type::TYPE_INT32, Label::LABEL_OPTIONAL),
                lo,
            ],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("o".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let idents = std::collections::HashMap::from([(0usize, format_ident!("O"))]);
        let got: Vec<_> = classify_fields_ordered(&msg, &idents)
            .unwrap()
            .iter()
            .map(kind_tag)
            .collect();
        assert_eq!(got, vec![("oneof", 3), ("scalar", 5)]);
    }

    #[test]
    fn classify_rejects_missing_field_number() {
        let msg = DescriptorProto {
            field: vec![FieldDescriptorProto {
                name: Some("x".into()),
                r#type: Some(Type::TYPE_INT32),
                label: Some(Label::LABEL_OPTIONAL),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(matches!(
            classify_fields_ordered(&msg, &Default::default()),
            Err(CodeGenError::MissingField("field.number"))
        ));
    }

    // ── is_explicit_presence_scalar ──────────────────────────────────────

    #[test]
    fn explicit_presence_proto3_non_optional_is_false() {
        let f = make_field(Type::TYPE_INT32, Label::LABEL_OPTIONAL);
        assert!(!is_explicit_presence_scalar(
            &f,
            Type::TYPE_INT32,
            &ResolvedFeatures::proto3_defaults()
        ));
    }

    #[test]
    fn explicit_presence_proto3_optional_is_true() {
        let f = FieldDescriptorProto {
            r#type: Some(Type::TYPE_INT32),
            label: Some(Label::LABEL_OPTIONAL),
            proto3_optional: Some(true),
            ..Default::default()
        };
        assert!(is_explicit_presence_scalar(
            &f,
            Type::TYPE_INT32,
            &ResolvedFeatures::proto3_defaults()
        ));
    }

    #[test]
    fn explicit_presence_proto2_optional_scalar_is_true() {
        let f = make_field(Type::TYPE_INT32, Label::LABEL_OPTIONAL);
        assert!(is_explicit_presence_scalar(
            &f,
            Type::TYPE_INT32,
            &ResolvedFeatures::proto2_defaults()
        ));
    }

    #[test]
    fn explicit_presence_proto2_optional_in_oneof_is_false() {
        // Oneof members are handled by the oneof enum, not as Option<T> scalars.
        let f = FieldDescriptorProto {
            r#type: Some(Type::TYPE_INT32),
            label: Some(Label::LABEL_OPTIONAL),
            oneof_index: Some(0),
            ..Default::default()
        };
        assert!(!is_explicit_presence_scalar(
            &f,
            Type::TYPE_INT32,
            &ResolvedFeatures::proto2_defaults()
        ));
    }

    #[test]
    fn explicit_presence_message_type_always_false() {
        let f = FieldDescriptorProto {
            r#type: Some(Type::TYPE_MESSAGE),
            label: Some(Label::LABEL_OPTIONAL),
            proto3_optional: Some(true),
            ..Default::default()
        };
        assert!(!is_explicit_presence_scalar(
            &f,
            Type::TYPE_MESSAGE,
            &ResolvedFeatures::proto3_defaults()
        ));
        assert!(!is_explicit_presence_scalar(
            &f,
            Type::TYPE_MESSAGE,
            &ResolvedFeatures::proto2_defaults()
        ));
    }

    // ── is_field_packed ──────────────────────────────────────────────────

    #[test]
    fn packed_proto3_scalar_default_is_packed() {
        let f = make_field(Type::TYPE_INT32, Label::LABEL_REPEATED);
        assert!(is_field_packed(&f, &ResolvedFeatures::proto3_defaults()));
    }

    #[test]
    fn packed_proto2_scalar_default_is_unpacked() {
        let f = make_field(Type::TYPE_INT32, Label::LABEL_REPEATED);
        assert!(!is_field_packed(&f, &ResolvedFeatures::proto2_defaults()));
    }

    #[test]
    fn packed_proto2_explicit_packed_true() {
        let f = FieldDescriptorProto {
            r#type: Some(Type::TYPE_INT32),
            label: Some(Label::LABEL_REPEATED),
            options: (FieldOptions {
                packed: Some(true),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        assert!(is_field_packed(&f, &ResolvedFeatures::proto2_defaults()));
    }

    #[test]
    fn packed_proto3_explicit_packed_false() {
        let f = FieldDescriptorProto {
            r#type: Some(Type::TYPE_INT32),
            label: Some(Label::LABEL_REPEATED),
            options: (FieldOptions {
                packed: Some(false),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        assert!(!is_field_packed(&f, &ResolvedFeatures::proto3_defaults()));
    }

    #[test]
    fn packed_string_always_false() {
        let f = make_field(Type::TYPE_STRING, Label::LABEL_REPEATED);
        assert!(!is_field_packed(&f, &ResolvedFeatures::proto3_defaults()));
        assert!(!is_field_packed(&f, &ResolvedFeatures::proto2_defaults()));
    }
}

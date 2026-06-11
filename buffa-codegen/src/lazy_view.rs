//! Code generation for the lazy view family (`FooLazyView<'a>`).
//!
//! Generated **additively** under the `lazy_views` option: the eager
//! `FooView` family is unchanged, and each message additionally gets a
//! `FooLazyView<'a>` implementing `buffa::LazyMessageView` — a single
//! non-recursive decode pass that records nested/repeated message fields as
//! undecoded byte ranges (`LazyMessageFieldView` / `LazyRepeatedView`) and
//! decodes them only on access.
//!
//! Everything that is not a singular/repeated message field reuses the eager
//! machinery verbatim: scalars/strings/bytes borrow identically, groups and
//! editions `DELIMITED` fields decode eagerly into the eager view types,
//! oneofs reuse the eager view-oneof enums, and map fields reuse `MapView`
//! with eager value views. The lazy structs are emitted into their own
//! `__buffa::lazy_view::` tree, beside the eager `__buffa::view::` tree.

use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::context::{ancillary_prefix, AncillaryKind, MessageScope};
use crate::impl_message::{
    effective_type, is_real_oneof_member, is_supported_field_type, validated_field_number,
    wire_type_check,
};
use crate::message::{is_map_field, make_field_ident, rust_path_to_tokens};
use crate::view::{
    map_decode_arm, map_to_owned_expr, message_view_has_borrowing_field, oneof_decode_arms,
    oneof_variant_to_owned, oneof_view_struct_fields, repeated_decode_arm, repeated_to_owned,
    resolve_lazy_view_path, resolve_lazy_view_ty_tokens, resolve_owned_path, scalar_decode_arm,
    singular_to_owned, view_field_serialize_stmt, view_map_type, view_repeated_type,
    view_singular_type,
};
use crate::CodeGenError;

/// Is this field stored lazily on the lazy view? Singular/repeated message
/// fields only — groups and editions `DELIMITED` fields resolve to
/// `TYPE_GROUP` via `effective_type` and stay eager (no length prefix to
/// defer), and map/oneof message values are reached through eager types.
/// Fields resolving to **extern** types (WKTs via `buffa-types`,
/// `extern_path`-mapped crates) also stay eager: the target crate may not
/// ship a lazy family, and WKT JSON needs the hand-written eager impls.
fn is_lazy_field(scope: MessageScope<'_>, field: &FieldDescriptorProto) -> bool {
    let features = crate::features::resolve_field(scope.ctx, field, scope.features);
    if effective_type(scope.ctx, field, &features) != Type::TYPE_MESSAGE {
        return false;
    }
    let Some(type_name) = field.type_name.as_deref() else {
        return false;
    };
    scope
        .ctx
        .rust_type_relative_split(type_name, scope.current_package, scope.nesting)
        .is_some_and(|split| !split.is_extern)
}

/// Generate the `FooLazyView` struct + impls for `msg`.
///
/// Returns the top-level token stream, destined for the mirrored
/// `__buffa::lazy_view::` position. The oneof view enums are shared with
/// the eager family, so no ancillary stream is produced.
pub(crate) fn generate_lazy_view_with_nesting(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    rust_name: &str,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        current_package,
        proto_fqn,
        features,
        nesting,
    } = scope;

    let oneof_idents = crate::oneof::resolve_oneof_idents(msg);
    let lazy_ident = format_ident!("{}LazyView", rust_name);

    let view_depth = nesting + 2;
    let view_scope = MessageScope {
        nesting: view_depth,
        ..scope
    };
    let view_oneof_prefix = ancillary_prefix(
        AncillaryKind::ViewOneof,
        current_package,
        proto_fqn,
        view_depth,
    );
    let owned_oneof_prefix =
        ancillary_prefix(AncillaryKind::Oneof, current_package, proto_fqn, view_depth);

    let lazy_fields = msg
        .field
        .iter()
        .filter(|f| is_supported_field_type(f.r#type.unwrap_or_default()))
        .map(|f| lazy_struct_field(view_scope, msg, f))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let direct_fields: Vec<&TokenStream> =
        lazy_fields.iter().map(|(tokens, _, _)| tokens).collect();

    let oneof_view_fields =
        oneof_view_struct_fields(ctx, msg, &view_oneof_prefix, features, &oneof_idents)?;
    let oneof_struct_fields: Vec<&TokenStream> =
        oneof_view_fields.iter().map(|(tokens, _)| tokens).collect();

    let (scalar_arms, repeated_arms, oneof_arms) =
        build_lazy_decode_arms(view_scope, msg, &view_oneof_prefix, &oneof_idents)?;

    let owned_fields = build_lazy_to_owned_fields(
        view_scope,
        msg,
        &view_oneof_prefix,
        &owned_oneof_prefix,
        &oneof_idents,
    )?;

    let unknown_fields_field = if ctx.config.preserve_unknown_fields {
        quote! { pub __buffa_unknown_fields: ::buffa::UnknownFieldsView<'a>, }
    } else {
        quote! {}
    };
    let is_lazy = |f: &FieldDescriptorProto| is_lazy_field(view_scope, f);
    let view_encode_methods = crate::impl_message::build_view_encode_methods(
        ctx,
        msg,
        ctx.config.preserve_unknown_fields,
        features,
        &oneof_idents,
        &view_oneof_prefix,
        Some(&is_lazy),
        &quote! { pub },
    )?;

    let message_name_impl = crate::impl_message::message_name_impl(
        current_package,
        proto_fqn,
        &quote! { <'a> },
        &quote! { #lazy_ident<'a> },
    );

    let serialize_impl = if ctx.config.generate_json {
        crate::feature_gates::cfg_block(
            generate_lazy_view_serialize(
                view_scope,
                msg,
                &lazy_ident,
                &view_oneof_prefix,
                &oneof_idents,
            )?,
            ctx.config.feature_gates().json,
        )
    } else {
        quote! {}
    };

    let before_tag_capture = if ctx.config.preserve_unknown_fields {
        quote! { let before_tag = cur; }
    } else {
        quote! {}
    };
    let unknown_field_handling = if ctx.config.preserve_unknown_fields {
        quote! {
            let span_len = before_tag.len() - cur.len();
            view.__buffa_unknown_fields.push_record(before_tag, span_len, ctx)?;
        }
    } else {
        quote! {}
    };

    let phantom_field =
        if message_view_has_borrowing_field(ctx, msg, features, ctx.config.preserve_unknown_fields)
        {
            quote! {}
        } else {
            quote! { #[doc(hidden)] pub __buffa_phantom: ::core::marker::PhantomData<&'a ()>, }
        };

    let owned_path: TokenStream = {
        let dotted = format!(".{proto_fqn}");
        let p = ctx
            .rust_type_relative(&dotted, current_package, view_depth)
            .ok_or_else(|| {
                CodeGenError::Other(format!(
                    "owned type for '{proto_fqn}' not resolvable from lazy view tree"
                ))
            })?;
        rust_path_to_tokens(&p)
    };

    let doc = format!(
        " Lazy view of `{proto_fqn}`: nested and repeated message fields are\n \
         recorded as undecoded byte ranges and decoded on access. See\n \
         [`::buffa::LazyMessageView`] for the deferred-validation contract;\n \
         the eager, whole-tree-validated counterpart is [`{rust_name}View`].\n\n \
         Oneof variants, map values, groups, and extern-typed fields (e.g.\n \
         well-known types) hold eagerly-decoded `{rust_name}View`-family\n \
         types; only singular/repeated message fields defer.\n\n \
         # Examples\n\n \
         ```rust,ignore\n \
         use buffa::LazyMessageView;\n\n \
         let view = {rust_name}LazyView::decode_lazy(&bytes)?;\n \
         ```"
    );

    // Lazy field types never leak redacted payloads through Debug (they print
    // fragment counts), but scalar/string/bytes fields can — reuse the same
    // redaction policy as the eager view.
    let any_redacted = lazy_fields.iter().any(|(_, _, redacted)| *redacted);
    let (debug_derive, debug_impl) = if any_redacted {
        let placeholder = crate::message::DEBUG_REDACT_PLACEHOLDER;
        let name_str = lazy_ident.to_string();
        let mut names: Vec<String> = Vec::new();
        let mut values: Vec<TokenStream> = Vec::new();
        for (_, ident, redacted) in &lazy_fields {
            names.push(ident.to_string().trim_start_matches("r#").to_string());
            values.push(if *redacted {
                quote! { &::core::format_args!(#placeholder) }
            } else {
                quote! { &self.#ident }
            });
        }
        for (_, ident) in &oneof_view_fields {
            names.push(ident.to_string().trim_start_matches("r#").to_string());
            values.push(quote! { &self.#ident });
        }
        (
            quote! { #[derive(Clone, Default)] },
            quote! {
                impl<'a> ::core::fmt::Debug for #lazy_ident<'a> {
                    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        f.debug_struct(#name_str)
                            #(.field(#names, #values))*
                            .finish()
                    }
                }
            },
        )
    } else {
        (quote! { #[derive(Clone, Debug, Default)] }, quote! {})
    };

    Ok(quote! {
        #[doc = #doc]
        #debug_derive
        pub struct #lazy_ident<'a> {
            #(#direct_fields)*
            #(#oneof_struct_fields)*
            #unknown_fields_field
            #phantom_field
        }

        #debug_impl

        impl<'a> #lazy_ident<'a> {
            /// Decode from `buf` under the limits carried by `ctx`, recording
            /// nested/repeated message fields as byte ranges.
            ///
            /// **Not part of the public API.**
            #[doc(hidden)]
            pub fn _decode_lazy_ctx(
                buf: &'a [u8],
                ctx: ::buffa::DecodeContext<'_>,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                let mut view = Self::default();
                view._merge_lazy(buf, ctx)?;
                ::core::result::Result::Ok(view)
            }

            /// Merge fields from `buf` into this view (proto merge semantics;
            /// deferred message fragments accumulate).
            ///
            /// **Not part of the public API.**
            #[doc(hidden)]
            pub fn _merge_lazy(
                &mut self,
                buf: &'a [u8],
                ctx: ::buffa::DecodeContext<'_>,
            ) -> ::core::result::Result<(), ::buffa::DecodeError> {
                let _ = ctx;
                #[allow(unused_variables)]
                let view = self;
                let mut cur: &'a [u8] = buf;
                while !cur.is_empty() {
                    #before_tag_capture
                    let tag = ::buffa::encoding::Tag::decode(&mut cur)?;
                    match tag.field_number() {
                        #(#scalar_arms)*
                        #(#repeated_arms)*
                        #(#oneof_arms)*
                        _ => {
                            ::buffa::encoding::skip_field_depth(tag, &mut cur, ctx.depth())?;
                            #unknown_field_handling
                        }
                    }
                }
                ::core::result::Result::Ok(())
            }
        }

        impl<'a> ::buffa::LazyMessageView<'a> for #lazy_ident<'a> {
            type Owned = #owned_path;

            fn decode_lazy(
                buf: &'a [u8],
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                let __limit = ::core::cell::Cell::new(::buffa::DEFAULT_UNKNOWN_FIELD_LIMIT);
                Self::_decode_lazy_ctx(
                    buf,
                    ::buffa::DecodeContext::new(::buffa::RECURSION_LIMIT, &__limit),
                )
            }

            fn decode_lazy_with_ctx(
                buf: &'a [u8],
                ctx: ::buffa::DecodeContext<'_>,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                Self::_decode_lazy_ctx(buf, ctx)
            }

            fn merge_lazy(
                &mut self,
                buf: &'a [u8],
                ctx: ::buffa::DecodeContext<'_>,
            ) -> ::core::result::Result<(), ::buffa::DecodeError> {
                self._merge_lazy(buf, ctx)
            }

            #[allow(clippy::useless_conversion, clippy::needless_update)]
            fn to_owned_message(
                &self,
            ) -> ::core::result::Result<#owned_path, ::buffa::DecodeError> {
                #[allow(unused_imports)]
                use ::buffa::alloc::string::ToString as _;
                #[allow(unused_imports)]
                use ::buffa::MessageView as _;
                // Lazy views have no backing `Bytes` source; eager to_owned
                // helpers are reused with the copy fallback.
                let __buffa_src: ::core::option::Option<&::buffa::bytes::Bytes> =
                    ::core::option::Option::None;
                let _ = __buffa_src;
                ::core::result::Result::Ok(#owned_path {
                    #(#owned_fields)*
                    ..::core::default::Default::default()
                })
            }
        }

        /// Re-encoding: recorded fragments are replayed byte-for-byte
        /// **without validation** — wire-equivalent to the merged value, and
        /// a never-accessed malformed deferred field round-trips silently.
        /// Inherent rather than [`::buffa::ViewEncode`] (whose `MessageView`
        /// supertrait carries the eager whole-tree-validated contract); the
        /// fuller `ViewEncode` set (`encode_length_delimited`,
        /// `encode_with_cache`) lives on the eager view.
        impl<'a> #lazy_ident<'a> {
            #view_encode_methods

            /// Compute size, then write. Primary encode entry point.
            pub fn encode(&self, buf: &mut impl ::buffa::bytes::BufMut) {
                let mut __cache = ::buffa::SizeCache::new();
                self.compute_size(&mut __cache);
                self.write_to(&mut __cache, buf);
            }

            /// Encoded byte size of this view.
            #[must_use]
            pub fn encoded_len(&self) -> u32 {
                self.compute_size(&mut ::buffa::SizeCache::new())
            }

            /// Encode this view to a new `Vec<u8>`.
            #[must_use]
            pub fn encode_to_vec(&self) -> ::buffa::alloc::vec::Vec<u8> {
                let mut __cache = ::buffa::SizeCache::new();
                let __size = self.compute_size(&mut __cache) as usize;
                let mut __buf = ::buffa::alloc::vec::Vec::with_capacity(__size);
                self.write_to(&mut __cache, &mut __buf);
                __buf
            }

            /// Encode this view to a new [`::buffa::bytes::Bytes`].
            #[must_use]
            pub fn encode_to_bytes(&self) -> ::buffa::bytes::Bytes {
                let mut __cache = ::buffa::SizeCache::new();
                let __size = self.compute_size(&mut __cache) as usize;
                let mut __buf = ::buffa::bytes::BytesMut::with_capacity(__size);
                self.write_to(&mut __cache, &mut __buf);
                __buf.freeze()
            }
        }

        #serialize_impl

        #message_name_impl
    })
}

/// One struct field declaration: lazy types for singular/repeated message
/// fields, the eager view types for everything else.
fn lazy_struct_field(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
) -> Result<Option<(TokenStream, proc_macro2::Ident, bool)>, CodeGenError> {
    let MessageScope { ctx, proto_fqn, .. } = scope;
    if is_real_oneof_member(field) {
        return Ok(None);
    }

    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(field_name);
    let number = field.number.unwrap_or(0);
    let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;
    let field_fqn = format!("{}.{}", proto_fqn, field_name);
    let proto_comment = ctx.comment(&field_fqn);

    if is_repeated && is_map_field(msg, field) {
        let tag_line = format!("Field {number}: `{field_name}` (map)");
        let doc = crate::comments::doc_attrs_with_tag_resolved(
            proto_comment,
            &tag_line,
            proto_fqn,
            &ctx.type_map,
        );
        let map_ty = view_map_type(scope, msg, field, &quote! { 'a })?;
        return Ok(Some((
            quote! { #doc pub #ident: #map_ty, },
            ident,
            crate::message::is_debug_redacted(field),
        )));
    }

    let tag_line = format!("Field {number}: `{field_name}`");
    let doc = crate::comments::doc_attrs_with_tag_resolved(
        proto_comment,
        &tag_line,
        proto_fqn,
        &ctx.type_map,
    );

    let lazy = is_lazy_field(scope, field);
    let self_fqn = format!(".{proto_fqn}");
    let is_self = field.type_name.as_deref() == Some(self_fqn.as_str());
    let rust_type = match (lazy, is_repeated) {
        (true, true) => {
            let elem = if is_self {
                quote! { Self }
            } else {
                resolve_lazy_view_ty_tokens(scope, field, &quote! { 'a })?
            };
            quote! { ::buffa::LazyRepeatedView<'a, #elem> }
        }
        (true, false) => {
            let sub = if is_self {
                quote! { Self }
            } else {
                resolve_lazy_view_ty_tokens(scope, field, &quote! { 'a })?
            };
            quote! { ::buffa::LazyMessageFieldView<'a, #sub> }
        }
        (false, true) => view_repeated_type(scope, field, &quote! { 'a })?,
        (false, false) => view_singular_type(scope, field, &quote! { 'a })?,
    };

    Ok(Some((
        quote! { #doc pub #ident: #rust_type, },
        ident,
        crate::message::is_debug_redacted(field),
    )))
}

/// Decode match arms: lazy record arms for message fields, the eager arms for
/// everything else (scalars, strings, bytes, enums, groups, maps, oneofs).
#[allow(clippy::type_complexity)]
fn build_lazy_decode_arms(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    view_oneof_prefix: &TokenStream,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<(Vec<TokenStream>, Vec<TokenStream>, Vec<TokenStream>), CodeGenError> {
    let scalar_arms = msg
        .field
        .iter()
        .filter(|f| {
            !is_real_oneof_member(f)
                && f.label.unwrap_or_default() != Label::LABEL_REPEATED
                && is_supported_field_type(f.r#type.unwrap_or_default())
        })
        .map(|f| {
            if is_lazy_field(scope, f) {
                lazy_singular_message_arm(scope, f)
            } else {
                scalar_decode_arm(scope, f)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut repeated_arms = msg
        .field
        .iter()
        .filter(|f| {
            f.label.unwrap_or_default() == Label::LABEL_REPEATED
                && !is_map_field(msg, f)
                && is_supported_field_type(f.r#type.unwrap_or_default())
        })
        .map(|f| {
            if is_lazy_field(scope, f) {
                lazy_repeated_message_arm(scope, f)
            } else {
                repeated_decode_arm(scope, f)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    let map_arms = msg
        .field
        .iter()
        .filter(|f| f.label.unwrap_or_default() == Label::LABEL_REPEATED && is_map_field(msg, f))
        .map(|f| map_decode_arm(scope, msg, f))
        .collect::<Result<Vec<_>, _>>()?;
    repeated_arms.extend(map_arms);

    let mut oneof_arms: Vec<TokenStream> = Vec::new();
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let base_ident = match oneof_idents.get(&idx) {
            Some(id) => id,
            None => continue,
        };
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let fields: Vec<_> = msg
            .field
            .iter()
            .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
            .collect();
        oneof_arms.extend(oneof_decode_arms(
            scope,
            base_ident,
            oneof_name,
            &fields,
            view_oneof_prefix,
        )?);
    }

    Ok((scalar_arms, repeated_arms, oneof_arms))
}

/// Lazy: record the byte range and remaining budgets; `get()` merges
/// multiple occurrences and charges the budgets on access.
fn lazy_singular_message_arm(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(
        field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?,
    );
    let _ = scope;
    let wire_check = wire_type_check(
        field_number,
        &quote! { ::buffa::encoding::WireType::LengthDelimited },
        2u8,
    );
    Ok(quote! {
        #field_number => {
            #wire_check
            let __sub_ctx = ctx.descend()?;
            let sub = ::buffa::types::borrow_bytes(&mut cur)?;
            view.#ident.push_fragment(sub, __sub_ctx);
        }
    })
}

fn lazy_repeated_message_arm(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(
        field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?,
    );
    let _ = scope;
    let wire_check = wire_type_check(
        field_number,
        &quote! { ::buffa::encoding::WireType::LengthDelimited },
        2u8,
    );
    Ok(quote! {
        #field_number => {
            #wire_check
            let __sub_ctx = ctx.descend()?;
            let sub = ::buffa::types::borrow_bytes(&mut cur)?;
            view.#ident.push_bytes(sub, __sub_ctx);
        }
    })
}

/// `to_owned_message` field initialisers: lazy fields decode-then-convert
/// (errors propagate); everything else reuses the eager helpers.
fn build_lazy_to_owned_fields(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    view_oneof_prefix: &TokenStream,
    owned_oneof_prefix: &TokenStream,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<Vec<TokenStream>, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let mut out = Vec::new();

    for field in &msg.field {
        if is_real_oneof_member(field) {
            continue;
        }
        let name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let ident = make_field_ident(name);
        let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;
        if is_repeated && is_map_field(msg, field) {
            let expr = map_to_owned_expr(scope, msg, field, &ident)?;
            out.push(quote! { #ident: #expr, });
            continue;
        }
        let ty = effective_type(ctx, field, features);
        let init = if is_lazy_field(scope, field) {
            if is_repeated {
                quote! {
                    {
                        let mut __out =
                            ::buffa::alloc::vec::Vec::with_capacity(self.#ident.len());
                        for __r in self.#ident.iter() {
                            __out.push(__r?.to_owned_message()?);
                        }
                        __out
                    }
                }
            } else {
                let owned_path = resolve_owned_path(scope, field)?;
                let owned_ty = rust_path_to_tokens(&owned_path);
                quote! {
                    match self.#ident.get()? {
                        ::core::option::Option::Some(v) => {
                            ::buffa::MessageField::<#owned_ty>::some(v.to_owned_message()?)
                        }
                        ::core::option::Option::None => ::buffa::MessageField::none(),
                    }
                }
            }
        } else if is_repeated {
            repeated_to_owned(scope, ty, &ident, name)
        } else {
            singular_to_owned(scope, field, ty, &ident, name)?
        };
        out.push(quote! { #ident: #init, });
    }

    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let base_ident = match oneof_idents.get(&idx) {
            Some(id) => id,
            None => continue,
        };
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let group: Vec<_> = msg
            .field
            .iter()
            .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
            .collect();
        if group.is_empty() {
            continue;
        }
        let field_ident = make_field_ident(oneof_name);
        let view_enum: TokenStream = quote! { #view_oneof_prefix #base_ident };
        let owned_enum: TokenStream = quote! { #owned_oneof_prefix #base_ident };

        let match_arms = group
            .iter()
            .map(|f| {
                let fname = f
                    .name
                    .as_deref()
                    .ok_or(CodeGenError::MissingField("field.name"))?;
                let variant = crate::oneof::oneof_variant_ident(fname);
                let ty = effective_type(ctx, f, features);
                let conv = oneof_variant_to_owned(scope, ty, oneof_name, fname);
                Ok(quote! {
                    #view_enum::#variant(v) => #owned_enum::#variant(#conv),
                })
            })
            .collect::<Result<Vec<_>, CodeGenError>>()?;

        out.push(quote! {
            #field_ident: match self.#field_ident.as_ref() {
                ::core::option::Option::Some(v) => {
                    ::core::option::Option::Some(match v { #(#match_arms)* })
                }
                ::core::option::Option::None => ::core::option::Option::None,
            },
        });
    }

    if ctx.config.preserve_unknown_fields {
        out.push(quote! {
            __buffa_unknown_fields: self
                .__buffa_unknown_fields
                .to_owned()?
                .into(),
        });
    }

    Ok(out)
}

/// `impl serde::Serialize`: lazy message fields decode-on-serialize (errors
/// become serde errors); everything else reuses the eager stmt builder.
fn generate_lazy_view_serialize(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    lazy_ident: &proc_macro2::Ident,
    view_oneof_prefix: &TokenStream,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<TokenStream, CodeGenError> {
    let mut stmts: Vec<TokenStream> = Vec::new();

    for field in &msg.field {
        if is_real_oneof_member(field) {
            continue;
        }
        if !is_supported_field_type(field.r#type.unwrap_or_default()) {
            continue;
        }
        let is_map =
            field.label.unwrap_or_default() == Label::LABEL_REPEATED && is_map_field(msg, field);
        if !is_map && is_lazy_field(scope, field) {
            stmts.push(lazy_field_serialize_stmt(scope, field)?);
        } else {
            stmts.push(view_field_serialize_stmt(scope, msg, field)?);
        }
    }

    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let base_ident = match oneof_idents.get(&idx) {
            Some(id) => id,
            None => continue,
        };
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let field_ident = make_field_ident(oneof_name);
        let view_enum = quote! { #view_oneof_prefix #base_ident };
        let fields: Vec<_> = msg
            .field
            .iter()
            .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
            .collect();
        if fields.is_empty() {
            continue;
        }
        let arms = fields
            .iter()
            .map(|f| crate::view::view_oneof_serialize_arm(scope, f, &view_enum))
            .collect::<Result<Vec<_>, _>>()?;
        stmts.push(quote! {
            if let ::core::option::Option::Some(ref __ov) = self.#field_ident {
                match __ov { #(#arms)* }
            }
        });
    }

    Ok(quote! {
        /// Serializes this lazy view as protobuf JSON, decoding deferred
        /// message fields on the fly. Malformed or over-budget deferred
        /// bytes surface as a serializer error.
        impl<'__a> ::serde::Serialize for #lazy_ident<'__a> {
            fn serialize<__S: ::serde::Serializer>(
                &self,
                __s: __S,
            ) -> ::core::result::Result<__S::Ok, __S::Error> {
                use ::serde::ser::SerializeMap as _;
                let mut __map = __s.serialize_map(::core::option::Option::None)?;
                #(#stmts)*
                __map.end()
            }
        }
    })
}

/// Lazy: decode-on-serialize; malformed bytes become a serde error.
fn lazy_field_serialize_stmt(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let json_name = field.json_name.as_deref().unwrap_or(field_name);
    let ident = make_field_ident(field_name);
    let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;

    if is_repeated {
        let path = resolve_lazy_view_path(scope, field)?;
        let elem_ty = quote! { #path <'__a> };
        return Ok(quote! {
            if !self.#ident.is_empty() {
                struct _WSeq<'__x, '__a>(
                    &'__x ::buffa::LazyRepeatedView<'__a, #elem_ty>,
                );
                impl<'__a> ::serde::Serialize for _WSeq<'_, '__a> {
                    fn serialize<__S: ::serde::Serializer>(&self, __s: __S) -> ::core::result::Result<__S::Ok, __S::Error> {
                        use ::serde::ser::SerializeSeq as _;
                        let mut __seq = __s.serialize_seq(::core::option::Option::Some(self.0.len()))?;
                        for __r in self.0.iter() {
                            match __r {
                                ::core::result::Result::Ok(__v) => __seq.serialize_element(&__v)?,
                                ::core::result::Result::Err(__e) => {
                                    return ::core::result::Result::Err(
                                        <__S::Error as ::serde::ser::Error>::custom(__e),
                                    );
                                }
                            }
                        }
                        __seq.end()
                    }
                }
                __map.serialize_entry(#json_name, &_WSeq(&self.#ident))?;
            }
        });
    }

    Ok(quote! {
        {
            match self.#ident.get() {
                ::core::result::Result::Ok(::core::option::Option::Some(__v)) => {
                    __map.serialize_entry(#json_name, &__v)?;
                }
                ::core::result::Result::Ok(::core::option::Option::None) => {}
                ::core::result::Result::Err(__e) => {
                    return ::core::result::Result::Err(
                        <__S::Error as ::serde::ser::Error>::custom(__e),
                    );
                }
            }
        }
    })
}

//! Code generation for zero-copy borrowed message view types.
//!
//! For each proto message `Foo` this module generates:
//!
//! - `FooView<'a>`: a struct whose string/bytes fields are `&'a str`/`&'a [u8]`,
//!   borrowing directly from the input buffer without allocation.
//! - A borrowed oneof enum at `view::oneofs::foo::Kind<'a>` for each oneof,
//!   mirroring the owned `oneofs::foo::Kind` but with borrowed variants.
//! - `impl MessageView<'a> for FooView<'a>`: provides `decode_view` (zero-copy
//!   decode) and `to_owned_message` (conversion to the owned type).

use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, OneofDescriptorProto};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::context::{CodeGenContext, MessageScope};
use crate::features::ResolvedFeatures;
use crate::impl_message::{
    closed_enum_decode, closed_enum_decode_with_unknown, decode_fn_token, effective_type,
    effective_type_in_map_entry, field_uses_bytes, find_map_entry_fields,
    is_explicit_presence_scalar, is_packed_type, is_real_oneof_member, is_supported_field_type,
    validated_field_number, wire_type_byte, wire_type_check, wire_type_token,
};
use crate::message::{is_closed_enum, is_map_field, make_field_ident, rust_path_to_tokens};
use crate::CodeGenError;

/// Token stream that pushes a closed-enum unknown value's raw wire span to
/// `view.__buffa_unknown_fields`. Requires `before_tag` and `cur` in scope
/// (the decode loop captures `before_tag` before reading the tag).
///
/// Returns empty tokens when `preserve_unknown_fields` is false. In that case
/// `closed_enum_decode_with_unknown` collapses to `closed_enum_decode`.
fn closed_enum_view_unknown_route(preserve_unknown_fields: bool) -> TokenStream {
    if preserve_unknown_fields {
        quote! {
            let __span_len = before_tag.len() - cur.len();
            view.__buffa_unknown_fields.push_raw(&before_tag[..__span_len]);
        }
    } else {
        quote! {}
    }
}

/// Convert a borrowed bytes view to the owned field type.
///
/// Emits `bytes::Bytes::copy_from_slice(expr)` when `use_bytes_type()`
/// is active for this field (the borrow isn't `'static` so `from` won't
/// work), otherwise `(expr).to_vec()`.
///
/// `expr` may be `&[u8]` (singular/optional) or `&&[u8]` (repeated-iter,
/// oneof match-ergonomics). Both branches accept either: `.to_vec()` via
/// method auto-deref, `copy_from_slice` via argument auto-deref.
fn bytes_to_owned(
    ctx: &CodeGenContext,
    proto_fqn: &str,
    field_name: &str,
    expr: TokenStream,
) -> TokenStream {
    if field_uses_bytes(ctx, proto_fqn, field_name) {
        quote! { ::bytes::Bytes::copy_from_slice(#expr) }
    } else {
        quote! { (#expr).to_vec() }
    }
}

/// Generate view items for a message and (recursively) all of its nested
/// messages, as a single `TokenStream`. The stream has the shape:
///
/// ```ignore
/// pub struct FooView<'a> { … }   // struct + decode/to_owned impls
/// impl<'a> FooView<'a> { … }
/// impl<'a> MessageView<'a> for FooView<'a> { … }
/// pub mod foo {                   // only if Foo has nested message types
///     use super::*;
///     pub struct InnerView<'a> { … }   // nested-view (recursive)
///     pub mod inner { … }
/// }
/// ```
///
/// Oneof view enums for `Foo`'s oneofs live in a separate stream destined
/// for `view::oneofs::foo::Kind<'a>` — see `ViewOutput::oneof_items`.
///
/// The caller wraps the outer stream in `pub mod view { … }` at the
/// package level and includes the contents from every file in the same
/// package — so `views::foo::InnerView` ends up at `my_pkg::view::foo::
/// InnerView<'a>`.
///
/// Nesting: `scope.nesting` counts module hops from package root to the
/// CURRENT point in the view tree. Top-level view items live at depth 2
/// (inside `pub mod __buffa { pub mod view { ... } }`). Nested views of
/// `Foo` live at depth 3 (inside `__buffa::view::foo`). Owned-type
/// references climb out via that many `super::` segments.
/// View generation result for a single proto message.
///
/// The content is split across two streams because the view `items` and
/// view-of-oneof `oneof_items` land in different trees
/// (`pkg::view::...` vs `pkg::view::oneofs::...`) — the latter is a
/// modifier of the `oneofs::` kind, stitched separately by
/// `generate_module_tree`.
pub struct ViewOutput {
    /// View struct + nested sub-module (for nested-message views).
    /// Destined for `pub mod view { ... }` at the package level.
    pub items: TokenStream,
    /// Contents of THIS message's sub-module within the view-of-oneofs
    /// tree: direct view-of-oneof enums + recursive `pub mod <nested>`
    /// wrappers. Destined for `pub mod view { pub mod oneofs { ... } }`
    /// at the package level, wrapped in `pub mod <self_mod>` by the
    /// caller (`generate_file`).
    pub oneof_items: TokenStream,
}

pub fn generate_view(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    current_package: &str,
    rust_name: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
) -> Result<ViewOutput, CodeGenError> {
    let scope = MessageScope {
        ctx,
        current_package,
        proto_fqn,
        features,
        // Two `super::` hops to escape `pub mod __buffa { pub mod view {
        // ... } }` back to the package root.
        nesting: 2,
        in_view_tree: true,
    };
    generate_view_items(scope, msg, rust_name)
}

/// Recursive helper shared by top-level and nested view generation.
fn generate_view_items(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    rust_name: &str,
) -> Result<ViewOutput, CodeGenError> {
    let MessageScope {
        ctx,
        proto_fqn,
        features,
        ..
    } = scope;
    let proto_name = msg.name.as_deref().unwrap_or(rust_name);
    let mod_name_str = crate::oneof::to_snake_case(proto_name);
    let mod_ident_raw = make_field_ident(&mod_name_str);

    // Sibling owned sub-module path. Used by `build_to_owned_fields` to
    // reach this message's OWNED oneof enum (`Kind`) when materializing
    // an owned message from a view. Resolves to the FULL owned module
    // path (package-relative, then prefixed with `super::`s to climb
    // out of the view tree). Computed below alongside `owned_ident`.
    // Build `super::super::…::` token stream for `scope.nesting` hops.
    // `syn::parse_str` handles a trailing `::` fine because it parses as
    // an unconstrained TokenStream, not a full path.
    let supers = "super::".repeat(scope.nesting);
    let supers_tokens: TokenStream = syn::parse_str(&supers).unwrap_or_default();

    let oneof_idents = crate::oneof::resolve_oneof_idents(msg, proto_fqn)?;

    // View struct name always keeps the `View` suffix (for co-import
    // ergonomics — callers commonly bind both `Foo` and `FooView` in the
    // same scope).
    let view_ident = format_ident!("{}View", rust_name);
    // Owned counterpart: lives in the owned tree at the mirrored
    // position. We resolve the full owned path (including any nested-
    // module prefix like `outer::`) via the context's type map, then
    // prefix with the right number of `super::` hops for the current
    // view-tree nesting.
    let proto_fqn_dotted = format!(".{proto_fqn}");
    let owned_path_from_pkg = ctx
        .rust_type(&proto_fqn_dotted)
        .and_then(|full| {
            // Strip the package prefix (e.g. `google::protobuf::outer::Middle`
            // → `outer::Middle`, or `google::protobuf::Any` → `Any`) so
            // we can re-prefix with `super::`s below. The package prefix
            // is `current_package` translated to `::` form.
            let pkg_rust = scope.current_package.replace('.', "::");
            if pkg_rust.is_empty() {
                Some(full.to_string())
            } else {
                full.strip_prefix(&format!("{pkg_rust}::"))
                    .map(|s| s.to_string())
            }
        })
        .unwrap_or_else(|| rust_name.to_string());
    // Use `rust_path_to_tokens` rather than `syn::parse_str`: the latter
    // chokes on keyword path segments like `google::r#type::LatLng`
    // (protoc allows `type` as a package name). The helper already
    // produces properly-escaped idents for each path segment.
    let owned_path_tokens: TokenStream = crate::idents::rust_path_to_tokens(&owned_path_from_pkg);
    let owned_ident: TokenStream = quote! { #supers_tokens #owned_path_tokens };

    // Owned sub-module path (mirror of `owned_path_from_pkg` with the
    // final Type segment replaced by its snake_case). For a top-level
    // `Foo` this is `foo`; for a nested `Outer.Middle` it's
    // `outer::middle`; etc.
    //
    // Parent prefix comes straight from `owned_path_from_pkg` by splitting
    // off the last segment; we then append the current message's own
    // snake-cased module name.
    let owned_mod_suffix = match owned_path_from_pkg.rsplit_once("::") {
        Some((parent, _leaf)) => format!("{parent}::{mod_name_str}"),
        None => mod_name_str.clone(),
    };
    let owned_mod_tokens: TokenStream = crate::idents::rust_path_to_tokens(&owned_mod_suffix);

    // Path prefix to this message's view-of-oneof sub-module within
    // the parallel `__buffa::view::oneofs::` tree, relative to the
    // current view-struct emission scope. For a view struct at
    // `__buffa::view::` (depth 2) targeting
    // `__buffa::view::oneofs::foo::Kind`: `oneofs::foo::` (sibling in
    // our `view::` module). For a nested view struct at
    // `__buffa::view::foo::InnerView` (depth 3) targeting
    // `__buffa::view::oneofs::foo::inner::Kind`:
    // `super::oneofs::foo::inner::` (climb out to `view::`, then
    // descend).
    //
    // Formally: (scope.nesting - 2) `super::`s + `oneofs::` + owned
    // module path chain (same as owned_mod_suffix).
    let view_oneofs_supers = "super::".repeat(scope.nesting.saturating_sub(2));
    let view_oneofs_supers_tokens: TokenStream =
        syn::parse_str(&view_oneofs_supers).unwrap_or_default();
    let view_oneofs_prefix: TokenStream = quote! {
        #view_oneofs_supers_tokens oneofs:: #owned_mod_tokens ::
    };

    // Path prefix to this message's OWNED oneof sub-module within the
    // parallel `__buffa::oneofs::` tree, reachable from the current
    // view-struct scope. From `__buffa::view::` at depth N, climb N-1
    // supers to `__buffa::`, then descend into
    // `oneofs::<owner_chain>::<self>::`.
    //
    // Used by `build_to_owned_fields` to reach the OWNED `Kind` enum
    // when materializing an owned message from a view.
    let owned_oneofs_supers = "super::".repeat(scope.nesting.saturating_sub(1));
    let owned_oneofs_supers_tokens: TokenStream =
        syn::parse_str(&owned_oneofs_supers).unwrap_or_default();
    let owned_oneofs_prefix: TokenStream = quote! {
        #owned_oneofs_supers_tokens oneofs:: #owned_mod_tokens ::
    };

    // View struct fields (excludes real-oneof members, map fields, and
    // unsupported types like groups).
    let direct_fields = msg
        .field
        .iter()
        .filter(|f| is_supported_field_type(f.r#type.unwrap_or_default()))
        .map(|f| view_struct_field(scope, msg, f))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    // One `Option<Kind<'a>>` (no `View` suffix — tree disambiguates)
    // per non-synthetic oneof, path-qualified to `view::oneofs::<…>`.
    let oneof_struct_fields =
        oneof_view_struct_fields(ctx, msg, &view_oneofs_prefix, features, &oneof_idents)?;

    // Oneof view enum definitions. They live in the parallel
    // `view::oneofs::<owner_chain>::<self_mod>::` tree — one extra
    // level deeper than the corresponding owned oneof enum. Relative
    // to the view struct emission scope (`scope.nesting`), the enum
    // sits two modules deeper: view::oneofs itself plus the self_mod
    // wrapper. Pass `scope` + 2 so variant-type path resolution inside
    // the enum produces the right number of `super::` hops for owned
    // and sibling-view targets.
    let enum_scope = MessageScope {
        nesting: scope.nesting + 2,
        ..scope
    };
    let oneof_view_enums = msg
        .oneof_decl
        .iter()
        .enumerate()
        .map(|(idx, oneof)| generate_oneof_view_enum(enum_scope, msg, idx, oneof, &oneof_idents))
        .collect::<Result<Vec<_>, _>>()?;

    // decode_view match arms. Oneof arms assign to view-of-oneof enum
    // variants living at `view::oneofs::<…>`, reached via
    // `view_oneofs_prefix`.
    let (scalar_arms, repeated_arms, oneof_arms) =
        build_decode_arms(scope, msg, &view_oneofs_prefix, &oneof_idents)?;

    // to_owned_message field initialisers. Oneof variants reference
    // the view-of-oneof enum (for pattern matching) at
    // `view::oneofs::<owner>::Kind` and the owned oneof enum (for
    // construction) at `oneofs::<owner>::Kind`, reached via the two
    // prefixes computed above.
    let owned_fields = build_to_owned_fields(
        scope,
        msg,
        &view_oneofs_prefix,
        &owned_oneofs_prefix,
        &oneof_idents,
    )?;

    let unknown_fields_field = if ctx.config.preserve_unknown_fields {
        quote! { pub __buffa_unknown_fields: ::buffa::UnknownFieldsView<'a>, }
    } else {
        quote! {}
    };

    // When preserving unknowns we capture `before_tag` so we can compute the
    // raw byte span after `skip_field` advances the cursor.
    let before_tag_capture = if ctx.config.preserve_unknown_fields {
        quote! { let before_tag = cur; }
    } else {
        quote! {}
    };
    let unknown_field_handling = if ctx.config.preserve_unknown_fields {
        quote! {
            let span_len = before_tag.len() - cur.len();
            view.__buffa_unknown_fields.push_raw(&before_tag[..span_len]);
        }
    } else {
        quote! {}
    };

    // If no field borrows from 'a (all-scalar message with unknown-fields
    // preservation disabled), inject PhantomData<&'a ()> so the struct's
    // lifetime param is used. _decode_depth(buf: &'a [u8]) requires 'a.
    let phantom_field =
        if message_view_has_borrowing_field(ctx, msg, features, ctx.config.preserve_unknown_fields)
        {
            quote! {}
        } else {
            quote! { #[doc(hidden)] pub __buffa_phantom: ::core::marker::PhantomData<&'a ()>, }
        };

    // Sub-module for this message's nested-view items. Oneof-view
    // enums are extracted into the separate oneofs:: tree, so only
    // recursive nested-view contents end up here (collected in
    // `nested_view_items` below).
    let mod_items = quote! {};

    let view_doc = crate::comments::doc_attrs(ctx.comment(proto_fqn));

    let top_level = quote! {
        #view_doc
        #[derive(Clone, Debug, Default)]
        pub struct #view_ident<'a> {
            #(#direct_fields)*
            #(#oneof_struct_fields)*
            #unknown_fields_field
            #phantom_field
        }

        impl<'a> #view_ident<'a> {
            /// Decode from `buf`, enforcing a recursion depth limit for nested messages.
            ///
            /// Called by [`::buffa::MessageView::decode_view`] with [`::buffa::RECURSION_LIMIT`]
            /// and by generated sub-message decode arms with `depth - 1`.
            ///
            /// **Not part of the public API.** Named with a leading underscore to
            /// signal that it is for generated-code use only.
            #[doc(hidden)]
            pub fn _decode_depth(
                buf: &'a [u8],
                depth: u32,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                let mut view = Self::default();
                view._merge_into_view(buf, depth)?;
                ::core::result::Result::Ok(view)
            }

            /// Merge fields from `buf` into this view (proto merge semantics).
            ///
            /// Repeated fields append; singular fields last-wins; singular
            /// MESSAGE fields merge recursively. Used by sub-message decode
            /// arms when the same field appears multiple times on the wire.
            ///
            /// **Not part of the public API.**
            #[doc(hidden)]
            pub fn _merge_into_view(
                &mut self,
                buf: &'a [u8],
                depth: u32,
            ) -> ::core::result::Result<(), ::buffa::DecodeError> {
                // `depth` may be unused for messages with no nested sub-message fields.
                let _ = depth;
                // Rebind as `view` so the arm-generating functions (which emit
                // `view.#ident`) work unchanged.
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
                            ::buffa::encoding::skip_field_depth(tag, &mut cur, depth)?;
                            #unknown_field_handling
                        }
                    }
                }
                ::core::result::Result::Ok(())
            }
        }

        impl<'a> ::buffa::MessageView<'a> for #view_ident<'a> {
            type Owned = #owned_ident;

            fn decode_view(
                buf: &'a [u8],
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                Self::_decode_depth(buf, ::buffa::RECURSION_LIMIT)
            }

            fn decode_view_with_limit(
                buf: &'a [u8],
                depth: u32,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                Self::_decode_depth(buf, depth)
            }

            /// Convert this view to the owned message type.
            // redundant_closure: bytes_to_owned() emits `|b| Bytes::copy_from_slice(b)`
            // for optional bytes — eta-reducible, but the non-bytes branch
            // `|b| (b).to_vec()` is NOT (no fn path for the method), so the
            // helper can't uniformly emit a fn path.
            // useless_conversion: __buffa_unknown_fields uses `.into()` to
            // unify the `UnknownFields` (no-wrapper) and `__<Name>ExtJson`
            // (generate_json wrapper) cases; no-op in the former.
            #[allow(clippy::redundant_closure, clippy::useless_conversion)]
            fn to_owned_message(&self) -> #owned_ident {
                #[allow(unused_imports)]
                use ::buffa::alloc::string::ToString as _;
                #owned_ident {
                    #(#owned_fields)*
                    ..::core::default::Default::default()
                }
            }
        }

        // SAFETY: The static default instance is lazily initialized via OnceBox
        // and never mutated after publication.
        unsafe impl ::buffa::DefaultViewInstance for #view_ident<'static> {
            fn default_view_instance() -> &'static Self {
                static VALUE: ::buffa::__private::OnceBox<#view_ident<'static>>
                    = ::buffa::__private::OnceBox::new();
                VALUE.get_or_init(|| ::buffa::alloc::boxed::Box::new(
                    Self::default(),
                ))
            }
        }

        // SAFETY: View types are covariant in `'a` (all fields are `&'a str`,
        // `&'a [u8]`, etc.) and layout-identical across lifetimes.
        unsafe impl<'a> ::buffa::HasDefaultViewInstance for #view_ident<'a> {
            type Static = #view_ident<'static>;
        }
    };

    // Recurse into nested messages — their view items live inside the
    // owner's sub-module within the view tree (`view::foo::InnerView`),
    // and their oneof-view items feed into THIS message's oneof_items
    // under a nested sub-module. Skip synthetic map-entry messages.
    let mut nested_view_items = TokenStream::new();
    let mut nested_oneof_view_items = TokenStream::new();
    for nested in &msg.nested_type {
        let is_map_entry = nested
            .options
            .as_option()
            .and_then(|o| o.map_entry)
            .unwrap_or(false);
        if is_map_entry {
            continue;
        }
        let nested_name = nested.name.as_deref().unwrap_or("");
        let nested_fqn = format!("{}.{}", proto_fqn, nested_name);
        let nested_features =
            crate::features::resolve_child(features, crate::features::message_features(nested));
        let nested_scope = MessageScope {
            proto_fqn: &nested_fqn,
            features: &nested_features,
            nesting: scope.nesting + 1,
            ..scope
        };
        let ViewOutput {
            items: nested_items,
            oneof_items: nested_oneofs,
        } = generate_view_items(nested_scope, nested, nested_name)?;
        nested_view_items.extend(nested_items);
        if !nested_oneofs.is_empty() {
            let nested_mod_ident = make_field_ident(&crate::oneof::to_snake_case(nested_name));
            nested_oneof_view_items.extend(quote! {
                pub mod #nested_mod_ident {
                    #[allow(unused_imports)]
                    use super::*;
                    #nested_oneofs
                }
            });
        }
    }

    // Combine recursive nested-view items into a single sub-module
    // (`view::foo::...`). Oneof-view enums moved to the separate
    // oneof_items stream, so this sub-module holds only nested-view
    // content; emit only when non-empty.
    let has_sub_items = !mod_items.is_empty() || !nested_view_items.is_empty();
    let sub_module = if has_sub_items {
        quote! {
            pub mod #mod_ident_raw {
                #[allow(unused_imports)]
                use super::*;
                #mod_items
                #nested_view_items
            }
        }
    } else {
        TokenStream::new()
    };

    // Assemble this message's view-of-oneofs sub-module contents:
    // direct oneof-view enums plus the recursive nested-message
    // wrappers collected above.
    let oneof_items = quote! {
        #(#oneof_view_enums)*
        #nested_oneof_view_items
    };

    Ok(ViewOutput {
        items: quote! { #top_level #sub_module },
        oneof_items,
    })
}

// ---------------------------------------------------------------------------
// View struct field declarations
// ---------------------------------------------------------------------------

fn view_struct_field(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
) -> Result<Option<TokenStream>, CodeGenError> {
    let MessageScope { ctx, proto_fqn, .. } = scope;
    // Real oneof members go into the oneof enum, not directly on the struct.
    if is_real_oneof_member(field) {
        return Ok(None);
    }

    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let label = field.label.unwrap_or_default();
    let is_repeated = label == Label::LABEL_REPEATED;
    let field_fqn = format!("{}.{}", proto_fqn, field_name);
    let proto_comment = ctx.comment(&field_fqn);

    if is_repeated && is_map_field(msg, field) {
        let ident = make_field_ident(field_name);
        let number = field.number.unwrap_or(0);
        let tag_line = format!("Field {number}: `{field_name}` (map)");
        let doc = crate::comments::doc_attrs_with_tag(proto_comment, &tag_line);
        let map_ty = view_map_type(scope, msg, field)?;
        return Ok(Some(quote! {
            #doc
            pub #ident: #map_ty,
        }));
    }

    let ident = make_field_ident(field_name);
    let number = field.number.unwrap_or(0);
    let tag_line = format!("Field {number}: `{field_name}`");
    let doc = crate::comments::doc_attrs_with_tag(proto_comment, &tag_line);

    let rust_type = if is_repeated {
        view_repeated_type(scope, field)?
    } else {
        view_singular_type(scope, field)?
    };

    // Self-referential view fields (e.g. HttpRuleView.additional_bindings)
    // can use `Self` — inside `struct FooView<'a>`, `Self` means `FooView<'a>`
    // with the lifetime applied. Override only for message-typed, non-map
    // struct fields; decode and to_owned paths use the resolved type as-is
    // via the helper functions so no conflict there.
    let self_fqn = format!(".{proto_fqn}");
    let struct_ty = if field.type_name.as_deref() == Some(self_fqn.as_str()) {
        if is_repeated {
            quote! { ::buffa::RepeatedView<'a, Self> }
        } else {
            quote! { ::buffa::MessageFieldView<Self> }
        }
    } else {
        rust_type
    };

    Ok(Some(quote! {
        #doc
        pub #ident: #struct_ty,
    }))
}

fn view_singular_type(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        features: parent_features,
        ..
    } = scope;
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let ty = effective_type(ctx, field, features);

    if is_explicit_presence_scalar(field, ty, features) {
        return Ok(match ty {
            Type::TYPE_STRING => quote! { ::core::option::Option<&'a str> },
            Type::TYPE_BYTES => quote! { ::core::option::Option<&'a [u8]> },
            Type::TYPE_ENUM => {
                let et = resolve_enum_ty(scope, field)?;
                if is_closed_enum(features) {
                    quote! { ::core::option::Option<#et> }
                } else {
                    quote! { ::core::option::Option<::buffa::EnumValue<#et>> }
                }
            }
            _ => {
                let st = scalar_ty(ty);
                quote! { ::core::option::Option<#st> }
            }
        });
    }

    match ty {
        Type::TYPE_STRING => Ok(quote! { &'a str }),
        Type::TYPE_BYTES => Ok(quote! { &'a [u8] }),
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            let view_ty = resolve_view_ty_tokens(scope, field)?;
            Ok(quote! { ::buffa::MessageFieldView<#view_ty> })
        }
        Type::TYPE_ENUM => {
            let et = resolve_enum_ty(scope, field)?;
            if is_closed_enum(features) {
                Ok(quote! { #et })
            } else {
                Ok(quote! { ::buffa::EnumValue<#et> })
            }
        }
        _ => Ok(scalar_ty(ty)),
    }
}

fn view_repeated_type(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        features: parent_features,
        ..
    } = scope;
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let ty = effective_type(ctx, field, features);
    match ty {
        Type::TYPE_STRING => Ok(quote! { ::buffa::RepeatedView<'a, &'a str> }),
        Type::TYPE_BYTES => Ok(quote! { ::buffa::RepeatedView<'a, &'a [u8]> }),
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            let view_ty = resolve_view_ty_tokens(scope, field)?;
            Ok(quote! { ::buffa::RepeatedView<'a, #view_ty> })
        }
        Type::TYPE_ENUM => {
            let et = resolve_enum_ty(scope, field)?;
            if is_closed_enum(features) {
                Ok(quote! { ::buffa::RepeatedView<'a, #et> })
            } else {
                Ok(quote! { ::buffa::RepeatedView<'a, ::buffa::EnumValue<#et>> })
            }
        }
        _ => {
            let st = scalar_ty(ty);
            Ok(quote! { ::buffa::RepeatedView<'a, #st> })
        }
    }
}

/// Build the `::buffa::MapView<'a, K, V>` type for a map field.
fn view_map_type(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;

    let key_ty = match effective_type_in_map_entry(ctx, key_fd, features) {
        Type::TYPE_STRING => quote! { &'a str },
        // utf8_validation = NONE on a string map key → &'a [u8].
        Type::TYPE_BYTES => quote! { &'a [u8] },
        ty => scalar_ty(ty),
    };

    let val_ty = match effective_type_in_map_entry(ctx, val_fd, features) {
        Type::TYPE_STRING => quote! { &'a str },
        Type::TYPE_BYTES => quote! { &'a [u8] },
        Type::TYPE_MESSAGE => {
            let view_ty = resolve_view_ty_tokens(scope, val_fd)?;
            quote! { #view_ty }
        }
        Type::TYPE_ENUM => {
            let et = resolve_enum_ty(scope, val_fd)?;
            let val_features = crate::features::resolve_field(ctx, val_fd, features);
            if is_closed_enum(&val_features) {
                quote! { #et }
            } else {
                quote! { ::buffa::EnumValue<#et> }
            }
        }
        ty => scalar_ty(ty),
    };

    Ok(quote! { ::buffa::MapView<'a, #key_ty, #val_ty> })
}

/// Does the oneof's view enum need a `'a` lifetime parameter?
///
/// String/bytes/message/group variants borrow from the input buffer;
/// scalar and enum variants don't. An all-scalar oneof must not emit
/// `<'a>` or the unused-lifetime check (E0392) fires.
fn oneof_view_needs_lifetime(
    ctx: &CodeGenContext,
    fields: &[&FieldDescriptorProto],
    features: &ResolvedFeatures,
) -> bool {
    fields.iter().any(|f| {
        matches!(
            effective_type(ctx, f, features),
            Type::TYPE_STRING | Type::TYPE_BYTES | Type::TYPE_MESSAGE | Type::TYPE_GROUP
        )
    })
}

/// Does the message's view struct have any field that borrows from `'a`?
///
/// Repeated, map, string, bytes, message, group fields all use `'a`.
/// Only an all-scalar/enum message with `preserve_unknown_fields=false`
/// has no borrowing fields — in that case a PhantomData marker is needed
/// to keep the `<'a>` lifetime valid for `_decode_depth(buf: &'a [u8])`.
fn message_view_has_borrowing_field(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    features: &ResolvedFeatures,
    preserve_unknown_fields: bool,
) -> bool {
    if preserve_unknown_fields {
        // UnknownFieldsView<'a> always uses 'a.
        return true;
    }
    for f in &msg.field {
        if is_real_oneof_member(f) {
            continue; // oneof members checked below via oneof_view_needs_lifetime
        }
        // Repeated and map fields always use 'a (RepeatedView<'a, T>, MapView<'a, K, V>).
        if f.label.unwrap_or_default()
            == crate::generated::descriptor::field_descriptor_proto::Label::LABEL_REPEATED
        {
            return true;
        }
        // Singular string/bytes/message/group borrow.
        if matches!(
            effective_type(ctx, f, features),
            Type::TYPE_STRING | Type::TYPE_BYTES | Type::TYPE_MESSAGE | Type::TYPE_GROUP
        ) {
            return true;
        }
    }
    // Check oneofs: an all-scalar oneof doesn't borrow, but one with a
    // string/bytes/message/group variant does.
    for (idx, _) in msg.oneof_decl.iter().enumerate() {
        let fields: Vec<_> = msg
            .field
            .iter()
            .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
            .collect();
        if oneof_view_needs_lifetime(ctx, &fields, features) {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn oneof_view_struct_fields(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    view_oneofs_prefix: &TokenStream,
    features: &ResolvedFeatures,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<Vec<TokenStream>, CodeGenError> {
    let mut out = Vec::new();
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let base_ident = match oneof_idents.get(&idx) {
            Some(id) => id,
            None => continue,
        };
        let fields: Vec<_> = msg
            .field
            .iter()
            .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
            .collect();
        if fields.is_empty() {
            continue;
        }
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let field_ident = make_field_ident(oneof_name);
        // View-of-oneof enum drops the `View` suffix — the
        // `view::oneofs::` path prefix is the disambiguator.
        let enum_ident = base_ident.clone();
        let generics = if oneof_view_needs_lifetime(ctx, &fields, features) {
            quote! { <'a> }
        } else {
            quote! {}
        };
        out.push(quote! {
            pub #field_ident: ::core::option::Option<#view_oneofs_prefix #enum_ident #generics>,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Oneof view enum
// ---------------------------------------------------------------------------

fn generate_oneof_view_enum(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    idx: usize,
    _oneof: &OneofDescriptorProto,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let base_ident = match oneof_idents.get(&idx) {
        Some(id) => id,
        None => return Ok(TokenStream::new()),
    };

    let fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
        .collect();

    if fields.is_empty() {
        return Ok(TokenStream::new());
    }

    // View-of-oneof enum drops the `View` suffix — the
    // `view::oneofs::` path prefix is the disambiguator (see
    // DESIGN.md → "Generated code layout").
    let view_enum = base_ident.clone();

    let variants = fields
        .iter()
        .map(|f| {
            let name = f
                .name
                .as_deref()
                .ok_or(CodeGenError::MissingField("field.name"))?;
            let variant = crate::oneof::oneof_variant_ident(name);
            let ty = effective_type(ctx, f, features);
            let f_features = crate::features::resolve_field(ctx, f, features);
            let vty = match ty {
                Type::TYPE_STRING => quote! { &'a str },
                Type::TYPE_BYTES => quote! { &'a [u8] },
                Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
                    // Oneof view enum lives in the view-oneofs tree
                    // (`view::oneofs::foo::Kind<'a>`). The caller already
                    // passed a `scope.deeper()` to reflect that depth, so
                    // standard view-path resolution applies.
                    let view_ty = resolve_view_ty_tokens(scope, f)?;
                    quote! { ::buffa::alloc::boxed::Box<#view_ty> }
                }
                Type::TYPE_ENUM => {
                    let et = resolve_enum_ty(scope, f)?;
                    if is_closed_enum(&f_features) {
                        quote! { #et }
                    } else {
                        quote! { ::buffa::EnumValue<#et> }
                    }
                }
                _ => scalar_ty(ty),
            };
            Ok(quote! { #variant(#vty) })
        })
        .collect::<Result<Vec<_>, CodeGenError>>()?;

    let generics = if oneof_view_needs_lifetime(ctx, &fields, features) {
        quote! { <'a> }
    } else {
        quote! {}
    };

    Ok(quote! {
        #[derive(Clone, Debug)]
        pub enum #view_enum #generics {
            #(#variants,)*
        }
    })
}

// ---------------------------------------------------------------------------
// decode_view match arms
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
fn build_decode_arms(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    mod_ident: &TokenStream,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<(Vec<TokenStream>, Vec<TokenStream>, Vec<TokenStream>), CodeGenError> {
    let scalar_fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| {
            if is_real_oneof_member(f) {
                return false;
            }
            f.label.unwrap_or_default() != Label::LABEL_REPEATED
                && is_supported_field_type(f.r#type.unwrap_or_default())
        })
        .collect();
    let scalar_arms = scalar_fields
        .iter()
        .map(|f| scalar_decode_arm(scope, f))
        .collect::<Result<Vec<_>, _>>()?;

    let repeated_fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| {
            f.label.unwrap_or_default() == Label::LABEL_REPEATED
                && !is_map_field(msg, f)
                && is_supported_field_type(f.r#type.unwrap_or_default())
        })
        .collect();
    let mut repeated_arms: Vec<_> = repeated_fields
        .iter()
        .map(|f| repeated_decode_arm(scope, f))
        .collect::<Result<Vec<_>, _>>()?;

    // Map fields: decode entries into MapView.
    let map_fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| f.label.unwrap_or_default() == Label::LABEL_REPEATED && is_map_field(msg, f))
        .collect();
    let map_arms = map_fields
        .iter()
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
            scope, base_ident, oneof_name, &fields, mod_ident,
        )?);
    }

    Ok((scalar_arms, repeated_arms, oneof_arms))
}

fn scalar_decode_arm(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        features: parent_features,
        ..
    } = scope;
    let preserve_unknown_fields = ctx.config.preserve_unknown_fields;
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);
    let wire_type = wire_type_token(ty);
    let expected_byte = wire_type_byte(ty);

    let wire_check = wire_type_check(field_number, &wire_type, expected_byte);

    if is_explicit_presence_scalar(field, ty, features) {
        let assign = match ty {
            Type::TYPE_STRING => {
                quote! { view.#ident = Some(::buffa::types::borrow_str(&mut cur)?); }
            }
            Type::TYPE_BYTES => {
                quote! { view.#ident = Some(::buffa::types::borrow_bytes(&mut cur)?); }
            }
            Type::TYPE_ENUM => {
                if is_closed_enum(features) {
                    let unknown_route = closed_enum_view_unknown_route(preserve_unknown_fields);
                    closed_enum_decode_with_unknown(
                        &quote! { &mut cur },
                        quote! { view.#ident = Some(__v); },
                        unknown_route,
                    )
                } else {
                    quote! {
                        view.#ident = Some(::buffa::EnumValue::from(::buffa::types::decode_int32(&mut cur)?));
                    }
                }
            }
            _ => {
                let dfn = decode_fn_token(ty);
                quote! { view.#ident = Some(#dfn(&mut cur)?); }
            }
        };
        return Ok(quote! { #field_number => { #wire_check #assign } });
    }

    let assign = match ty {
        Type::TYPE_STRING => quote! { view.#ident = ::buffa::types::borrow_str(&mut cur)?; },
        Type::TYPE_BYTES => quote! { view.#ident = ::buffa::types::borrow_bytes(&mut cur)?; },
        Type::TYPE_ENUM => {
            if is_closed_enum(features) {
                let unknown_route = closed_enum_view_unknown_route(preserve_unknown_fields);
                closed_enum_decode_with_unknown(
                    &quote! { &mut cur },
                    quote! { view.#ident = __v; },
                    unknown_route,
                )
            } else {
                quote! { view.#ident = ::buffa::EnumValue::from(::buffa::types::decode_int32(&mut cur)?); }
            }
        }
        Type::TYPE_MESSAGE => {
            let vt = resolve_view_decode_tokens(scope, field)?;
            quote! {
                if depth == 0 {
                    return Err(::buffa::DecodeError::RecursionLimitExceeded);
                }
                let sub = ::buffa::types::borrow_bytes(&mut cur)?;
                // Proto merge semantics: if this field appeared before,
                // merge the new bytes into the existing view.
                match view.#ident.as_mut() {
                    Some(existing) => existing._merge_into_view(sub, depth - 1)?,
                    None => view.#ident = ::buffa::MessageFieldView::set(
                        #vt::_decode_depth(sub, depth - 1)?
                    ),
                }
            }
        }
        Type::TYPE_GROUP => {
            let vt = resolve_view_decode_tokens(scope, field)?;
            quote! {
                if depth == 0 {
                    return Err(::buffa::DecodeError::RecursionLimitExceeded);
                }
                let sub = ::buffa::types::borrow_group(&mut cur, #field_number, depth - 1)?;
                match view.#ident.as_mut() {
                    Some(existing) => existing._merge_into_view(sub, depth - 1)?,
                    None => view.#ident = ::buffa::MessageFieldView::set(
                        #vt::_decode_depth(sub, depth - 1)?
                    ),
                }
            }
        }
        _ => {
            let dfn = decode_fn_token(ty);
            quote! { view.#ident = #dfn(&mut cur)?; }
        }
    };

    Ok(quote! { #field_number => { #wire_check #assign } })
}

fn repeated_decode_arm(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        features: parent_features,
        ..
    } = scope;
    let preserve_unknown_fields = ctx.config.preserve_unknown_fields;
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ty = effective_type(ctx, field, features);
    let ident = make_field_ident(field_name);

    // Message: always LengthDelimited, unpacked.
    if ty == Type::TYPE_MESSAGE {
        let ld_check = wire_type_check(
            field_number,
            &quote! { ::buffa::encoding::WireType::LengthDelimited },
            2u8,
        );
        let vt = resolve_view_decode_tokens(scope, field)?;
        return Ok(quote! {
            #field_number => {
                #ld_check
                if depth == 0 {
                    return Err(::buffa::DecodeError::RecursionLimitExceeded);
                }
                let sub = ::buffa::types::borrow_bytes(&mut cur)?;
                view.#ident.push(#vt::_decode_depth(sub, depth - 1)?);
            }
        });
    }

    // Group: StartGroup wire type, unpacked.
    if ty == Type::TYPE_GROUP {
        let sg_check = wire_type_check(
            field_number,
            &quote! { ::buffa::encoding::WireType::StartGroup },
            3u8,
        );
        let vt = resolve_view_decode_tokens(scope, field)?;
        return Ok(quote! {
            #field_number => {
                #sg_check
                if depth == 0 {
                    return Err(::buffa::DecodeError::RecursionLimitExceeded);
                }
                let sub = ::buffa::types::borrow_group(&mut cur, #field_number, depth - 1)?;
                view.#ident.push(#vt::_decode_depth(sub, depth - 1)?);
            }
        });
    }

    // String and bytes: unpacked only (no packed encoding for LD types).
    if !is_packed_type(ty) {
        let ld_check = wire_type_check(
            field_number,
            &quote! { ::buffa::encoding::WireType::LengthDelimited },
            2u8,
        );
        let borrow = match ty {
            Type::TYPE_STRING => quote! { ::buffa::types::borrow_str(&mut cur)? },
            Type::TYPE_BYTES => quote! { ::buffa::types::borrow_bytes(&mut cur)? },
            _ => unreachable!(),
        };
        return Ok(quote! {
            #field_number => {
                #ld_check
                view.#ident.push(#borrow);
            }
        });
    }

    // Packed numeric/enum: accept both packed (LengthDelimited) and unpacked.
    let elem_wire_type = wire_type_token(ty);
    let closed = is_closed_enum(features);
    let push_known = quote! { view.#ident.push(__v); };
    let packed_elem = if ty == Type::TYPE_ENUM {
        if closed {
            closed_enum_decode(&quote! { &mut pcur }, push_known.clone())
        } else {
            quote! { view.#ident.push(::buffa::EnumValue::from(::buffa::types::decode_int32(&mut pcur)?)); }
        }
    } else {
        let dfn = decode_fn_token(ty);
        quote! { view.#ident.push(#dfn(&mut pcur)?); }
    };
    let unpacked_elem = if ty == Type::TYPE_ENUM {
        if closed {
            // Unpacked: each element has its own tag, so `before_tag` captures
            // the per-element span. Packed (above) can't do this — the tag
            // covers the whole blob — so packed unknowns are still dropped.
            let unknown_route = closed_enum_view_unknown_route(preserve_unknown_fields);
            closed_enum_decode_with_unknown(&quote! { &mut cur }, push_known, unknown_route)
        } else {
            quote! { view.#ident.push(::buffa::EnumValue::from(::buffa::types::decode_int32(&mut cur)?)); }
        }
    } else {
        let dfn = decode_fn_token(ty);
        quote! { view.#ident.push(#dfn(&mut cur)?); }
    };

    Ok(quote! {
        #field_number => {
            if tag.wire_type() == ::buffa::encoding::WireType::LengthDelimited {
                // Packed: extract payload, decode elements via local cursor.
                let payload = ::buffa::types::borrow_bytes(&mut cur)?;
                let mut pcur: &[u8] = payload;
                while !pcur.is_empty() { #packed_elem }
            } else if tag.wire_type() == #elem_wire_type {
                // Unpacked (backward-compat with old encoders).
                #unpacked_elem
            } else {
                return Err(::buffa::DecodeError::WireTypeMismatch {
                    field_number: #field_number,
                    expected: 2u8,
                    actual: tag.wire_type() as u8,
                });
            }
        }
    })
}

fn map_decode_arm(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = validated_field_number(field)?;
    let ident = make_field_ident(field_name);
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;

    let ld_check = wire_type_check(
        field_number,
        &quote! { ::buffa::encoding::WireType::LengthDelimited },
        2u8,
    );

    // Default values for key and value when the entry sub-message omits them.
    let key_default = match effective_type_in_map_entry(ctx, key_fd, features) {
        Type::TYPE_STRING => quote! { "" },
        Type::TYPE_BYTES => quote! { &[][..] },
        _ => quote! { ::core::default::Default::default() },
    };
    let val_default = match effective_type_in_map_entry(ctx, val_fd, features) {
        Type::TYPE_STRING => quote! { "" },
        Type::TYPE_BYTES => quote! { &[][..] },
        _ => quote! { ::core::default::Default::default() },
    };

    let decode_key = map_view_entry_decode(scope, key_fd, &format_ident!("key"))?;
    let decode_val = map_view_entry_decode(scope, val_fd, &format_ident!("val"))?;

    Ok(quote! {
        #field_number => {
            #ld_check
            let entry_bytes = ::buffa::types::borrow_bytes(&mut cur)?;
            let mut entry_cur: &'a [u8] = entry_bytes;
            let mut key = #key_default;
            let mut val = #val_default;
            while !entry_cur.is_empty() {
                let entry_tag = ::buffa::encoding::Tag::decode(&mut entry_cur)?;
                match entry_tag.field_number() {
                    1 => { #decode_key }
                    2 => { #decode_val }
                    _ => { ::buffa::encoding::skip_field_depth(entry_tag, &mut entry_cur, depth)?; }
                }
            }
            view.#ident.push(key, val);
        }
    })
}

/// Generate the decode statement for one field inside a map-entry sub-message.
///
/// Uses zero-copy `borrow_str`/`borrow_bytes` for string/bytes fields and
/// decodes message values into view types.
fn map_view_entry_decode(
    scope: MessageScope<'_>,
    fd: &FieldDescriptorProto,
    var: &proc_macro2::Ident,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        features: parent_features,
        ..
    } = scope;
    let features = &crate::features::resolve_field(ctx, fd, parent_features);
    let ty = effective_type_in_map_entry(ctx, fd, features);
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

    let assign = match ty {
        Type::TYPE_STRING => quote! { #var = ::buffa::types::borrow_str(&mut entry_cur)?; },
        Type::TYPE_BYTES => quote! { #var = ::buffa::types::borrow_bytes(&mut entry_cur)?; },
        Type::TYPE_ENUM => {
            if is_closed_enum(features) {
                closed_enum_decode(&quote! { &mut entry_cur }, quote! { #var = __v; })
            } else {
                quote! { #var = ::buffa::EnumValue::from(::buffa::types::decode_int32(&mut entry_cur)?); }
            }
        }
        Type::TYPE_MESSAGE => {
            let vt = resolve_view_decode_tokens(scope, fd)?;
            quote! {
                if depth == 0 {
                    return Err(::buffa::DecodeError::RecursionLimitExceeded);
                }
                let sub = ::buffa::types::borrow_bytes(&mut entry_cur)?;
                #var = #vt::_decode_depth(sub, depth - 1)?;
            }
        }
        _ => {
            let dfn = decode_fn_token(ty);
            quote! { #var = #dfn(&mut entry_cur)?; }
        }
    };

    Ok(quote! { #tag_check #assign })
}

fn oneof_decode_arms(
    scope: MessageScope<'_>,
    base_ident: &proc_macro2::Ident,
    oneof_name: &str,
    fields: &[&FieldDescriptorProto],
    mod_ident: &TokenStream,
) -> Result<Vec<TokenStream>, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let preserve_unknown_fields = ctx.config.preserve_unknown_fields;
    let field_ident = make_field_ident(oneof_name);
    // View-of-oneof enum drops the `View` suffix. `mod_ident` here is
    // the `view::oneofs::<owner_chain>::` path prefix passed down from
    // `generate_view_items::view_oneofs_prefix`.
    let view_enum_simple = base_ident.clone();
    let view_enum: TokenStream = quote! { #mod_ident #view_enum_simple };

    fields
        .iter()
        .map(|field| {
            let name = field
                .name
                .as_deref()
                .ok_or(CodeGenError::MissingField("field.name"))?;
            let field_number = validated_field_number(field)?;
            let ty = effective_type(ctx, field, features);
            let field_features = crate::features::resolve_field(ctx, field, features);
            let variant = crate::oneof::oneof_variant_ident(name);
            let wire_type = wire_type_token(ty);
            let expected_byte = wire_type_byte(ty);
            let wire_check = wire_type_check(field_number, &wire_type, expected_byte);

            let value = match ty {
                Type::TYPE_STRING => quote! { ::buffa::types::borrow_str(&mut cur)? },
                Type::TYPE_BYTES => quote! { ::buffa::types::borrow_bytes(&mut cur)? },
                Type::TYPE_MESSAGE => {
                    let vt = resolve_view_decode_tokens(scope, field)?;
                    // Proto merge semantics: if this same variant is already set,
                    // merge into the existing boxed view rather than replacing.
                    // Uses an early `return Ok(...)` since the merge path doesn't
                    // fit the `value` expression shape used by scalar variants.
                    return Ok(quote! {
                        #field_number => {
                            #wire_check
                            if depth == 0 {
                                return Err(::buffa::DecodeError::RecursionLimitExceeded);
                            }
                            let sub = ::buffa::types::borrow_bytes(&mut cur)?;
                            if let Some(#view_enum::#variant(ref mut existing)) = view.#field_ident {
                                existing._merge_into_view(sub, depth - 1)?;
                            } else {
                                view.#field_ident = Some(#view_enum::#variant(
                                    ::buffa::alloc::boxed::Box::new(
                                        #vt::_decode_depth(sub, depth - 1)?
                                    )
                                ));
                            }
                        }
                    });
                }
                Type::TYPE_GROUP => {
                    let vt = resolve_view_decode_tokens(scope, field)?;
                    return Ok(quote! {
                        #field_number => {
                            #wire_check
                            if depth == 0 {
                                return Err(::buffa::DecodeError::RecursionLimitExceeded);
                            }
                            let sub = ::buffa::types::borrow_group(&mut cur, #field_number, depth - 1)?;
                            if let Some(#view_enum::#variant(ref mut existing)) = view.#field_ident {
                                existing._merge_into_view(sub, depth - 1)?;
                            } else {
                                view.#field_ident = Some(#view_enum::#variant(
                                    ::buffa::alloc::boxed::Box::new(
                                        #vt::_decode_depth(sub, depth - 1)?
                                    )
                                ));
                            }
                        }
                    });
                }
                Type::TYPE_ENUM => {
                    if is_closed_enum(&field_features) {
                        let unknown_route =
                            closed_enum_view_unknown_route(preserve_unknown_fields);
                        let decode = closed_enum_decode_with_unknown(
                            &quote! { &mut cur },
                            quote! { view.#field_ident = Some(#view_enum::#variant(__v)); },
                            unknown_route,
                        );
                        return Ok(quote! {
                            #field_number => {
                                #wire_check
                                #decode
                            }
                        });
                    }
                    quote! { ::buffa::EnumValue::from(::buffa::types::decode_int32(&mut cur)?) }
                }
                _ => {
                    let dfn = decode_fn_token(ty);
                    quote! { #dfn(&mut cur)? }
                }
            };

            Ok(quote! {
                #field_number => {
                    #wire_check
                    view.#field_ident = Some(#view_enum::#variant(#value));
                }
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// to_owned_message field initialisers
// ---------------------------------------------------------------------------

fn build_to_owned_fields(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    view_oneofs_prefix: &TokenStream,
    owned_mod_ident: &TokenStream,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<Vec<TokenStream>, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let preserve_unknown_fields = ctx.config.preserve_unknown_fields;
    let mut out = Vec::new();

    for field in &msg.field {
        // Real oneof members are handled below per-group.
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
        let init = if is_repeated {
            repeated_to_owned(scope, ty, &ident, name)?
        } else {
            singular_to_owned(scope, field, ty, &ident, name)?
        };
        out.push(quote! { #ident: #init, });
    }

    // Oneof groups.
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
        // View-of-oneof enum: lives at `view::oneofs::<owner_chain>::Kind`,
        // reached via `view_oneofs_prefix` (no `View` suffix — the
        // parallel tree disambiguates).
        let view_enum: TokenStream = quote! { #view_oneofs_prefix #base_ident };
        // Owned-side oneof enum lives at `oneofs::<owner_chain>::Kind`,
        // reached via `owned_mod_ident` (which is now the oneofs-tree
        // path: climb out of view::, descend into oneofs::<chain>).
        let owned_enum: TokenStream = quote! { #owned_mod_ident #base_ident };

        let match_arms = group
            .iter()
            .map(|f| {
                let fname = f
                    .name
                    .as_deref()
                    .ok_or(CodeGenError::MissingField("field.name"))?;
                let variant = crate::oneof::oneof_variant_ident(fname);
                let ty = effective_type(ctx, f, features);
                let conv = oneof_variant_to_owned(scope, ty, fname);
                Ok(quote! {
                    #view_enum::#variant(v) => #owned_enum::#variant(#conv),
                })
            })
            .collect::<Result<Vec<_>, CodeGenError>>()?;

        out.push(quote! {
            #field_ident: self.#field_ident.as_ref().map(|v| match v { #(#match_arms)* }),
        });
    }

    // Emit `unknown_fields` conversion so round-trip via decode_view +
    // to_owned_message preserves unknown fields. `.into()` is a no-op when
    // the owned field is `UnknownFields`; when generate_json is on it wraps
    // in the per-message `__<Name>ExtJson` newtype (which has `From<UnknownFields>`).
    if preserve_unknown_fields {
        out.push(quote! {
            __buffa_unknown_fields: self
                .__buffa_unknown_fields
                .to_owned()
                .unwrap_or_default()
                .into(),
        });
    }

    Ok(out)
}

fn singular_to_owned(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
    ty: Type,
    ident: &proc_macro2::Ident,
    field_name: &str,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        proto_fqn,
        features,
        ..
    } = scope;
    if is_explicit_presence_scalar(field, ty, features) {
        return Ok(match ty {
            Type::TYPE_STRING => quote! { self.#ident.map(|s| s.to_string()) },
            Type::TYPE_BYTES => {
                let conv = bytes_to_owned(ctx, proto_fqn, field_name, quote! { b });
                quote! { self.#ident.map(|b| #conv) }
            }
            _ => quote! { self.#ident },
        });
    }
    Ok(match ty {
        Type::TYPE_STRING => quote! { self.#ident.to_string() },
        Type::TYPE_BYTES => bytes_to_owned(ctx, proto_fqn, field_name, quote! { self.#ident }),
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            let owned_path = resolve_owned_path(scope, field)?;
            // Use rust_path_to_tokens, not syn::parse_str: the latter chokes
            // on keyword segments like `super::super::type::LatLng`.
            let owned_ty = crate::message::rust_path_to_tokens(&owned_path);
            quote! {
                match self.#ident.as_option() {
                    Some(v) => ::buffa::MessageField::<#owned_ty>::some(v.to_owned_message()),
                    None => ::buffa::MessageField::none(),
                }
            }
        }
        _ => quote! { self.#ident },
    })
}

fn repeated_to_owned(
    scope: MessageScope<'_>,
    ty: Type,
    ident: &proc_macro2::Ident,
    field_name: &str,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope { ctx, proto_fqn, .. } = scope;
    Ok(match ty {
        Type::TYPE_STRING => quote! { self.#ident.iter().map(|s| s.to_string()).collect() },
        Type::TYPE_BYTES => {
            // Vec<&[u8]>::iter() → b: &&[u8]. bytes_to_owned handles double-ref.
            let conv = bytes_to_owned(ctx, proto_fqn, field_name, quote! { b });
            quote! { self.#ident.iter().map(|b| #conv).collect() }
        }
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            quote! { self.#ident.iter().map(|v| v.to_owned_message()).collect() }
        }
        _ => quote! { self.#ident.to_vec() },
    })
}

fn map_to_owned_expr(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
    ident: &proc_macro2::Ident,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;

    let key_conv = match effective_type_in_map_entry(ctx, key_fd, features) {
        Type::TYPE_STRING => quote! { k.to_string() },
        // utf8_validation = NONE on a string map key: &[u8] → Vec<u8>.
        Type::TYPE_BYTES => quote! { k.to_vec() },
        _ => quote! { *k },
    };

    let val_conv = match effective_type_in_map_entry(ctx, val_fd, features) {
        Type::TYPE_STRING => quote! { v.to_string() },
        Type::TYPE_BYTES => quote! { v.to_vec() },
        Type::TYPE_MESSAGE => {
            // Verify the owned path resolves (catches missing imports at codegen time).
            let _owned_path = resolve_owned_path(scope, val_fd)?;
            quote! { v.to_owned_message() }
        }
        _ => quote! { *v },
    };

    Ok(quote! {
        self.#ident.iter().map(|(k, v)| (#key_conv, #val_conv)).collect()
    })
}

fn oneof_variant_to_owned(scope: MessageScope<'_>, ty: Type, field_name: &str) -> TokenStream {
    let MessageScope { ctx, proto_fqn, .. } = scope;
    match ty {
        Type::TYPE_STRING => quote! { v.to_string() },
        // match-ergonomics on &ViewEnum → v: &&[u8]. bytes_to_owned handles it.
        Type::TYPE_BYTES => bytes_to_owned(ctx, proto_fqn, field_name, quote! { v }),
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            quote! { ::buffa::alloc::boxed::Box::new(v.to_owned_message()) }
        }
        _ => quote! { *v },
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Scalar Rust type for view fields (same as owned scalars; no borrowing needed).
fn scalar_ty(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_DOUBLE => quote! { f64 },
        Type::TYPE_FLOAT => quote! { f32 },
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => quote! { i64 },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { u64 },
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => quote! { i32 },
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { u32 },
        Type::TYPE_BOOL => quote! { bool },
        _ => unreachable!("scalar_ty called for non-scalar {:?}", ty),
    }
}

/// Resolve the enum Rust type (same as owned — enums are Copy/Clone integers).
fn resolve_enum_ty(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let type_name = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.type_name"))?;
    let path = scope
        .ctx
        .rust_type_relative(type_name, scope.current_package, scope.nesting)
        .ok_or_else(|| CodeGenError::Other(format!("enum type '{type_name}' not found")))?;
    Ok(rust_path_to_tokens(&path))
}

/// Resolve the view type tokens for a message field
/// Resolve the view type tokens for a message field.
///
/// Branches on `scope.in_view_tree`:
/// - `true` (top-level view inside `pub mod view { ... }`): same-package
///   view refs are siblings; cross-package refs get a `view::` segment
///   inserted.
/// - `false` (nested view inside an owner's message sub-module, or
///   oneof-view-enum in owner's module): paths keep the legacy
///   `<Name>View` suffix in the owner's module.
fn resolve_view_ty_tokens(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let owned = resolve_owned_path(scope, field)?;
    let target_same_package = proto_type_same_package(field, scope);
    Ok(rewrite_to_view_path(
        &owned,
        target_same_package,
        /* with_lifetime */ true,
    ))
}

/// Resolve the view type tokens used for `decode_view` calls
/// (e.g. `"Address"` → `AddressView`).
fn resolve_view_decode_tokens(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let owned = resolve_owned_path(scope, field)?;
    let target_same_package = proto_type_same_package(field, scope);
    Ok(rewrite_to_view_path(
        &owned,
        target_same_package,
        /* with_lifetime */ false,
    ))
}

/// Determine if the target message referenced by `field.type_name` lives
/// in the SAME proto package as the scope emitting the reference.
///
/// Cross-package refs (including extern crates) need a `view::` segment
/// injected before the final ident in the generated view path, to
/// re-enter the target's package view tree. Same-package refs resolve
/// to a mirrored position inside our own view tree and don't need the
/// extra hop.
fn proto_type_same_package(field: &FieldDescriptorProto, scope: MessageScope<'_>) -> bool {
    let Some(type_name) = field.type_name.as_deref() else {
        return false;
    };
    match scope.ctx.package_of(type_name) {
        Some(pkg) => pkg == scope.current_package,
        // Extern types (not in our descriptor set) are by definition
        // not in our current package.
        None => false,
    }
}

fn resolve_owned_path(
    scope: MessageScope<'_>,
    field: &FieldDescriptorProto,
) -> Result<String, CodeGenError> {
    let type_name = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.type_name"))?;
    scope
        .ctx
        .rust_type_relative(type_name, scope.current_package, scope.nesting)
        .ok_or_else(|| CodeGenError::Other(format!("message type '{type_name}' not found")))
}

/// Rewrite an owned-type path string to the corresponding view-type path.
///
/// **Key insight**: the view tree mirrors the owned tree exactly. A
/// top-level `Foo` at `my_pkg::Foo` has its view at `my_pkg::view::Foo`
/// (sibling module path), and a nested `Outer.Inner` at
/// `my_pkg::outer::Inner` has its view at `my_pkg::view::outer::Inner`.
/// The spine (package path + nested-module path) is identical in both
/// trees.
///
/// Because the spines match, `rust_type_relative` — which computes the
/// view-tree-internal path from the current scope's nesting — already
/// produces exactly the owned-side path structure we want, modulo the
/// final ident. We just append `View` to the last segment and append
/// the lifetime param.
///
/// **Extern targets** (other crates, `::crate::…` or `crate::…`) follow
/// the same rule: the sibling crate's `view::` module mirrors its owned
/// tree, so the suffix swap suffices. Suppose buffa-types puts its WKT
/// views at `::buffa_types::google::protobuf::view::TimestampView`;
/// that path is what `resolve_owned_path` would produce for the owned
/// `Timestamp` plus the suffix swap.
///
/// **Actually**: extern crates may not yet have moved their views into
/// a `view::` sub-module — `buffa-types` is the canonical example, and
/// its `lib.rs` now does the module stitching. Nested-target handling
/// is identical for same-crate and extern.
///
/// `target_is_nested` is plumbed through for future use (view-tree
/// introspection) but the current implementation doesn't branch on it.
fn rewrite_to_view_path(
    owned: &str,
    target_same_package: bool,
    with_lifetime: bool,
) -> TokenStream {
    let (prefix_tokens, last_name) = split_path_last(owned);
    let view_ident = make_field_ident(&format!("{last_name}View"));
    let lifetime = if with_lifetime {
        quote! { <'a> }
    } else {
        quote! {}
    };

    // The view tree mirrors the owned tree inside each package. Rewrite
    // rules:
    //
    //   owned `super::super::Foo`          → `FooView<'a>`
    //   owned `super::super::super::Foo`   → `super::FooView<'a>`
    //   owned `super::super::outer::Inner` → `outer::InnerView<'a>`
    //   owned `super::super::super::pkg_b::Foo`
    //          → `super::super::super::pkg_b::__buffa::view::FooView<'a>`
    //   owned `::buffa_types::google::protobuf::Timestamp`
    //          → `::buffa_types::google::protobuf::__buffa::view::TimestampView<'a>`
    //
    // Algorithm:
    //
    // SAME-PACKAGE target (our own view tree mirrors the owned tree):
    //   Strip the two leading `super::`s (the hops that escape
    //   `__buffa::view::` to the package root, which we don't need when
    //   the target is in our own view tree). Keep the rest, swap the
    //   final ident.
    //
    // CROSS-PACKAGE target (same crate, different proto package):
    //   Keep the owned prefix verbatim (the climb reaches out through
    //   however many packages), then inject `__buffa::view::` before
    //   the final ident to re-enter the target package's view tree.
    //
    // EXTERN target (`::crate::…`):
    //   Same rewrite as cross-package: preserve prefix, inject
    //   `__buffa::view::` before the final ident. Matches
    //   `buffa-types`'s layout.
    if target_same_package {
        // Strip the two `super::` hops that escape `__buffa::view::`
        // back to the package root — within our own view tree the
        // mirrored position needs no climb.
        let stripped = owned
            .strip_prefix("super::super::")
            .or_else(|| owned.strip_prefix("super::"))
            .unwrap_or(owned);
        let (remainder_prefix, _) = split_path_last(stripped);
        return quote! { #remainder_prefix #view_ident #lifetime };
    }

    // Cross-package / extern: keep the full owned prefix, inject
    // `__buffa::view::` before the final ident to re-enter the target
    // package's view tree.
    quote! { #prefix_tokens __buffa::view:: #view_ident #lifetime }
}

/// Split `"a::b::Foo"` into (tokens for `a::b::`, `"Foo"`).
/// For `"Foo"` returns (empty tokens, `"Foo"`).
fn split_path_last(path: &str) -> (TokenStream, &str) {
    match path.rsplit_once("::") {
        Some((prefix, last)) => {
            let prefix_tokens = rust_path_to_tokens(prefix);
            (quote! { #prefix_tokens :: }, last)
        }
        None => (TokenStream::new(), path),
    }
}

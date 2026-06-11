//! Code generation for per-message owned-view wrapper types.
//!
//! For each proto message `Foo` (when views are generated) this module
//! generates `FooOwnedView`: a self-contained `'static` handle wrapping
//! `::buffa::OwnedView<FooView<'static>>` with one accessor method per field.
//! Every accessor takes `&self` and returns data borrowed from the wrapper's
//! internal buffer, so field borrows can never outlive the handle — the
//! ergonomic replacement for direct field access on `OwnedView` (which would
//! require exposing the view's synthetic `'static` lifetime and is therefore
//! not offered).

use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::context::MessageScope;
use crate::impl_message::{effective_type, is_real_oneof_member, is_supported_field_type};
use crate::message::{is_map_field, make_field_ident};
use crate::view::{
    oneof_view_needs_lifetime, view_map_type, view_repeated_type, view_singular_type,
};
use crate::{CodeGenError, CodeGenWarning};

/// Method names reserved by the wrapper itself. A proto field or oneof with
/// one of these names would collide with the wrapper's inherent methods (or,
/// for `clone`/`into`, silently shadow the standard trait method when called
/// with method syntax), so its accessor is suppressed with a
/// [`CodeGenWarning::OwnedViewAccessorSuppressed`] — the data stays reachable
/// through `view()`.
const RESERVED_WRAPPER_METHODS: &[&str] = &[
    "decode",
    "decode_with_options",
    "from_owned",
    "view",
    "to_owned_message",
    "bytes",
    "into_bytes",
    "clone",
    "into",
];

/// Generate the `FooOwnedView` wrapper for `msg`.
///
/// `scope` must be the **view scope** (the same `MessageScope` used to emit
/// the view struct's field types), so that relative type paths in accessor
/// return types resolve from the wrapper's module — the wrapper is emitted
/// beside the view struct.
pub(crate) fn generate_owned_view_wrapper(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    rust_name: &str,
    view_ident: &proc_macro2::Ident,
    owned_path: &TokenStream,
    view_oneof_prefix: &TokenStream,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        proto_fqn,
        features,
        ..
    } = scope;

    let wrapper_ident = format_ident!("{}OwnedView", rust_name);
    // Accessor return types use the anonymous lifetime: in return position
    // with a `&self` receiver it elides to the borrow of `self`.
    let lt = quote! { '_ };

    let mut accessors: Vec<TokenStream> = Vec::new();

    // Plain and map/repeated fields (real-oneof members are reached through
    // the oneof accessor below).
    for field in &msg.field {
        if !is_supported_field_type(field.r#type.unwrap_or_default()) {
            continue;
        }
        if is_real_oneof_member(field) {
            continue;
        }
        let field_name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        if RESERVED_WRAPPER_METHODS.contains(&field_name) {
            ctx.warn(CodeGenWarning::OwnedViewAccessorSuppressed {
                wrapper_name: wrapper_ident.to_string(),
                field_name: field_name.to_string(),
            });
            continue;
        }
        let ident = make_field_ident(field_name);
        let number = field.number.unwrap_or(0);
        let field_fqn = format!("{proto_fqn}.{field_name}");
        let proto_comment = ctx.comment(&field_fqn);

        let label = field.label.unwrap_or_default();
        let is_repeated = label == Label::LABEL_REPEATED;

        let (tag_line, ret_ty, body) = if is_repeated && is_map_field(msg, field) {
            let ty = view_map_type(scope, msg, field, &lt)?;
            (
                format!("Field {number}: `{field_name}` (map)"),
                quote! { &#ty },
                quote! { &self.0.reborrow().#ident },
            )
        } else if is_repeated {
            let ty = view_repeated_type(scope, field, &lt)?;
            (
                format!("Field {number}: `{field_name}`"),
                quote! { &#ty },
                quote! { &self.0.reborrow().#ident },
            )
        } else {
            let field_features = crate::features::resolve_field(ctx, field, features);
            let ty = view_singular_type(scope, field, &lt)?;
            let tag_line = format!("Field {number}: `{field_name}`");
            match effective_type(ctx, field, &field_features) {
                // Message fields are held in a `MessageFieldView` (not `Copy`);
                // hand out a reference.
                Type::TYPE_MESSAGE | Type::TYPE_GROUP => (
                    tag_line,
                    quote! { &#ty },
                    quote! { &self.0.reborrow().#ident },
                ),
                // Scalars, enums, `&str`/`&[u8]` and their `Option`s are all
                // `Copy`; return them by value.
                _ => (tag_line, ty, quote! { self.0.reborrow().#ident }),
            }
        };

        let doc = crate::comments::doc_attrs_with_tag_resolved(
            proto_comment,
            &tag_line,
            proto_fqn,
            &ctx.type_map,
        );
        accessors.push(quote! {
            #doc
            #[must_use]
            pub fn #ident(&self) -> #ret_ty {
                #body
            }
        });
    }

    // One accessor per non-synthetic oneof, returning `Option<&KindView>`.
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let Some(enum_ident) = oneof_idents.get(&idx) else {
            continue;
        };
        let fields: Vec<&FieldDescriptorProto> = msg
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
        if RESERVED_WRAPPER_METHODS.contains(&oneof_name) {
            ctx.warn(CodeGenWarning::OwnedViewAccessorSuppressed {
                wrapper_name: wrapper_ident.to_string(),
                field_name: oneof_name.to_string(),
            });
            continue;
        }
        let ident = make_field_ident(oneof_name);
        let generics = if oneof_view_needs_lifetime(ctx, &fields, features) {
            quote! { <'_> }
        } else {
            quote! {}
        };
        let doc = format!(" Oneof `{oneof_name}`.");
        accessors.push(quote! {
            #[doc = #doc]
            #[must_use]
            pub fn #ident(
                &self,
            ) -> ::core::option::Option<&#view_oneof_prefix #enum_ident #generics> {
                self.0.reborrow().#ident.as_ref()
            }
        });
    }

    let wrapper_doc = format!(
        " Self-contained, `'static` owned view of a `{rust_name}` message.\n\n \
         Wraps [`::buffa::OwnedView`]`<`[`{view}`]`<'static>>`: the decoded view and the \
         [`::buffa::bytes::Bytes`] buffer it borrows from travel together, so the handle is \
         `'static` and `Send + Sync` — suitable for async handlers, spawned tasks, and \
         anywhere a `'static` bound is required.\n\n \
         Field accessors return borrows tied to `&self`. Use [`Self::view`] to get the full \
         [`{view}`] when you need struct patterns, iteration helpers, or to pass the view to \
         lifetime-parameterised code.",
        view = view_ident,
    );
    let view_doc = format!(" Borrow the full [`{view_ident}`] with its lifetime tied to `&self`.");

    let serialize_impl = if ctx.config.generate_json {
        crate::feature_gates::cfg_block(
            quote! {
                impl ::serde::Serialize for #wrapper_ident {
                    fn serialize<__S: ::serde::Serializer>(
                        &self,
                        __s: __S,
                    ) -> ::core::result::Result<__S::Ok, __S::Error> {
                        ::serde::Serialize::serialize(&self.0, __s)
                    }
                }
            },
            ctx.config.feature_gates().json,
        )
    } else {
        quote! {}
    };

    Ok(quote! {
        #[doc = #wrapper_doc]
        #[derive(Clone, Debug)]
        pub struct #wrapper_ident(::buffa::OwnedView<#view_ident<'static>>);

        impl #wrapper_ident {
            /// Decode an owned view from a [`::buffa::bytes::Bytes`] buffer.
            ///
            /// The view borrows directly from the buffer's data; the buffer is
            /// retained inside the returned handle.
            ///
            /// # Errors
            ///
            /// Returns [`::buffa::DecodeError`] if the buffer contains invalid
            /// protobuf data.
            pub fn decode(
                bytes: ::buffa::bytes::Bytes,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                ::core::result::Result::Ok(#wrapper_ident(::buffa::OwnedView::decode(bytes)?))
            }

            /// Decode with custom [`::buffa::DecodeOptions`] (recursion limit,
            /// max message size).
            ///
            /// # Errors
            ///
            /// Returns [`::buffa::DecodeError`] if the buffer is invalid or
            /// exceeds the configured limits.
            pub fn decode_with_options(
                bytes: ::buffa::bytes::Bytes,
                opts: &::buffa::DecodeOptions,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                ::core::result::Result::Ok(#wrapper_ident(::buffa::OwnedView::decode_with_options(
                    bytes, opts,
                )?))
            }

            /// Build from an owned message via an encode → decode round-trip.
            ///
            /// # Errors
            ///
            /// Returns [`::buffa::DecodeError`] if the re-encoded bytes are
            /// somehow invalid (should not happen for well-formed messages).
            pub fn from_owned(
                msg: &#owned_path,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                ::core::result::Result::Ok(#wrapper_ident(::buffa::OwnedView::from_owned(msg)?))
            }

            #[doc = #view_doc]
            #[must_use]
            pub fn view(&self) -> &#view_ident<'_> {
                self.0.reborrow()
            }

            /// Convert to the owned message type.
            #[must_use]
            pub fn to_owned_message(&self) -> #owned_path {
                self.0.to_owned_message()
            }

            /// The underlying bytes buffer.
            #[must_use]
            pub fn bytes(&self) -> &::buffa::bytes::Bytes {
                self.0.bytes()
            }

            /// Consume the handle, returning the underlying bytes buffer.
            #[must_use]
            pub fn into_bytes(self) -> ::buffa::bytes::Bytes {
                self.0.into_bytes()
            }

            #(#accessors)*
        }

        impl ::core::convert::From<::buffa::OwnedView<#view_ident<'static>>> for #wrapper_ident {
            fn from(inner: ::buffa::OwnedView<#view_ident<'static>>) -> Self {
                #wrapper_ident(inner)
            }
        }

        impl ::core::convert::From<#wrapper_ident> for ::buffa::OwnedView<#view_ident<'static>> {
            fn from(wrapper: #wrapper_ident) -> Self {
                wrapper.0
            }
        }

        impl ::core::convert::AsRef<::buffa::OwnedView<#view_ident<'static>>> for #wrapper_ident {
            fn as_ref(&self) -> &::buffa::OwnedView<#view_ident<'static>> {
                &self.0
            }
        }

        // NOTE: this is the one generated impl whose `Self` is the owned
        // message path. Extern-mapped (`extern_path`) messages never reach
        // this code (they are not in the generation set, so no view/wrapper
        // is generated for them); if that invariant ever changes, this impl
        // must be skipped for absolute `owned_path`s to avoid an orphan impl.
        impl ::buffa::HasMessageView for #owned_path {
            type View<'a> = #view_ident<'a>;
            type ViewHandle = #wrapper_ident;
        }

        #serialize_impl
    })
}

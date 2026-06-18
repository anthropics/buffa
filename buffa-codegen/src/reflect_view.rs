//! Code generation for vtable-mode reflection on view types.
//!
//! When [`CodeGenConfig::generate_reflection`](crate::CodeGenConfig::generate_reflection)
//! and the internal `generate_reflection_vtable` flag are both set, each
//! generated view type gets:
//!
//! - `impl ::buffa_descriptor::reflect::ReflectMessage for FooView<'a>` — a
//!   zero-copy reflective accessor that reads struct fields directly, with no
//!   encode/decode round-trip and no `DynamicMessage`.
//! - `impl ::buffa_descriptor::reflect::ReflectElement for FooView<'a>` — so a
//!   `RepeatedView`/`MapView` of this message reflects through the generic
//!   container impls in `buffa-descriptor` (see that crate's `reflect::view`).
//! - A memoized per-message `MessageIndex` accessor.
//!
//! The bridge-mode `Reflectable` impl (on the owned message) is emitted
//! separately by [`reflect`](crate::reflect) and is unaffected; the
//! `Reflectable::reflect()` body switch to borrow the view directly is wired in
//! a later change. This module only adds the `ReflectMessage` surface.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::context::{MessageScope, SENTINEL_MOD};
use crate::features::resolve_field;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::DescriptorProto;
use crate::idents::make_field_ident;
use crate::impl_message::{
    effective_type, is_explicit_presence_scalar, is_real_oneof_member, is_required_field,
    is_supported_field_type,
};
use crate::message::{is_closed_enum, is_map_field};
use crate::oneof::oneof_variant_ident;
use crate::view::resolve_view_ty_tokens;
use crate::CodeGenError;

/// The `ValueRef` scalar variant for a wire-numeric proto type.
///
/// Mirrors the wire form, matching `DynamicMessage`: `int32`/`sint32`/`sfixed32`
/// all map to `I32`, etc. String, bytes, enum, and message types are handled by
/// the callers, not here.
pub(crate) fn scalar_variant(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => quote! { I32 },
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => quote! { I64 },
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { U32 },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { U64 },
        Type::TYPE_BOOL => quote! { Bool },
        Type::TYPE_FLOAT => quote! { F32 },
        Type::TYPE_DOUBLE => quote! { F64 },
        // Non-scalar types never reach this helper.
        _ => quote! { Bool },
    }
}

/// The default literal for a wire-numeric proto type (`0`, `0.0`, or `false`).
pub(crate) fn scalar_default(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_BOOL => quote! { false },
        Type::TYPE_FLOAT | Type::TYPE_DOUBLE => quote! { 0.0 },
        _ => quote! { 0 },
    }
}

/// Generate the vtable reflection impls for a single view type.
///
/// `view_scope` is the view struct's scope (`nesting + 2` below the package
/// root). `view_depth` is that same depth, used to climb back to the package
/// root for the `__buffa::reflect::descriptor_pool()` accessor. `oneof_idents`
/// and `view_oneof_prefix` come from the view-struct generation so oneof
/// members dispatch through the same view-oneof enum.
pub(crate) fn reflect_view_impls(
    view_scope: MessageScope<'_>,
    msg: &DescriptorProto,
    view_ident: &proc_macro2::Ident,
    view_depth: usize,
    view_oneof_prefix: &TokenStream,
    oneof_idents: &HashMap<usize, proc_macro2::Ident>,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope { ctx, .. } = view_scope;
    let features = view_scope.features;
    let vr = quote! { ::buffa_descriptor::reflect::ValueRef };
    let cow = quote! { ::buffa_descriptor::reflect::ReflectCow };

    let mut get_arms: Vec<TokenStream> = Vec::new();
    let mut has_arms: Vec<TokenStream> = Vec::new();

    // Direct (non-oneof) fields.
    for field in &msg.field {
        if is_real_oneof_member(field) {
            continue;
        }
        let ty = effective_type(ctx, field, features);
        if !is_supported_field_type(ty) {
            continue;
        }
        let name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let id = make_field_ident(name);
        // `FieldDescriptor::number()` (matched on below) returns `u32`; proto
        // field numbers are always positive.
        let number = field.number.unwrap_or(0) as u32;
        let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;

        if is_repeated && is_map_field(msg, field) {
            get_arms.push(quote! { #number => #vr::Map(&self.#id), });
            has_arms.push(quote! { #number => !::buffa::MapView::is_empty(&self.#id), });
            continue;
        }
        if is_repeated {
            get_arms.push(quote! { #number => #vr::List(&self.#id), });
            has_arms.push(quote! { #number => !::buffa::RepeatedView::is_empty(&self.#id), });
            continue;
        }

        let f_features = resolve_field(ctx, field, features);
        let (get_val, has_val) = if is_explicit_presence_scalar(field, ty, &f_features) {
            // Stored as `Option<T>`; absent singular returns the type default.
            match ty {
                Type::TYPE_STRING => (
                    quote! { #vr::String(self.#id.unwrap_or("")) },
                    quote! { self.#id.is_some() },
                ),
                Type::TYPE_BYTES => (
                    quote! { #vr::Bytes(self.#id.unwrap_or(&[])) },
                    quote! { self.#id.is_some() },
                ),
                Type::TYPE_ENUM => (
                    quote! { #vr::EnumNumber(self.#id.map_or(0, |e| e.to_i32())) },
                    quote! { self.#id.is_some() },
                ),
                _ => {
                    let variant = scalar_variant(ty);
                    let def = scalar_default(ty);
                    (
                        quote! { #vr::#variant(self.#id.unwrap_or(#def)) },
                        quote! { self.#id.is_some() },
                    )
                }
            }
        } else {
            // Implicit presence: absent is the default value, present is
            // non-default. proto2 `required` fields also fall here (they are
            // stored as bare types, not `Option`); their `has()` is overridden
            // below to consult the view's wire-presence tracking.
            match ty {
                Type::TYPE_STRING => (
                    quote! { #vr::String(self.#id) },
                    quote! { !self.#id.is_empty() },
                ),
                Type::TYPE_BYTES => (
                    quote! { #vr::Bytes(self.#id) },
                    quote! { !self.#id.is_empty() },
                ),
                Type::TYPE_MESSAGE | Type::TYPE_GROUP => (
                    // `MessageFieldView` derefs to the inner view, or the static
                    // default instance when unset — so the borrow is always
                    // valid and absent fields read as the empty message.
                    quote! { #vr::Message(#cow::Borrowed(&*self.#id)) },
                    quote! { self.#id.is_set() },
                ),
                Type::TYPE_ENUM => {
                    // A closed enum's default need not be zero (editions allows
                    // a non-zero first value), so "non-default" compares against
                    // the type default rather than `to_i32() != 0`. Open enums
                    // (`EnumValue`) always default to the zero wire value.
                    let has_val = if is_closed_enum(&f_features) {
                        quote! { self.#id != ::core::default::Default::default() }
                    } else {
                        quote! { self.#id.to_i32() != 0 }
                    };
                    (quote! { #vr::EnumNumber(self.#id.to_i32()) }, has_val)
                }
                _ => {
                    let variant = scalar_variant(ty);
                    let has_val = match ty {
                        Type::TYPE_BOOL => quote! { self.#id },
                        Type::TYPE_FLOAT | Type::TYPE_DOUBLE => quote! { self.#id != 0.0 },
                        _ => quote! { self.#id != 0 },
                    };
                    (quote! { #vr::#variant(self.#id) }, has_val)
                }
            }
        };
        // Required fields (#170): the view records wire presence (seen-bit
        // words / `is_set`), surfaced by the generated `has_*` accessor —
        // route reflection's `has()` through it so the two surfaces agree.
        let has_val = if is_required_field(field, features) && field.number.is_some() {
            let has_method = format_ident!("has_{}", name);
            quote! { self.#has_method() }
        } else {
            has_val
        };
        get_arms.push(quote! { #number => #get_val, });
        has_arms.push(quote! { #number => #has_val, });
    }

    // Oneof members dispatch through the `Option<KindView>` struct field.
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let Some(base_ident) = oneof_idents.get(&idx) else {
            continue;
        };
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let field_ident = make_field_ident(oneof_name);
        let view_enum = quote! { #view_oneof_prefix #base_ident };

        for field in msg
            .field
            .iter()
            .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
        {
            let name = field
                .name
                .as_deref()
                .ok_or(CodeGenError::MissingField("field.name"))?;
            let number = field.number.unwrap_or(0) as u32;
            let variant = oneof_variant_ident(name);
            let ty = effective_type(ctx, field, features);

            let (active, default) = match ty {
                Type::TYPE_STRING => (quote! { #vr::String(v) }, quote! { #vr::String("") }),
                Type::TYPE_BYTES => (quote! { #vr::Bytes(v) }, quote! { #vr::Bytes(&[]) }),
                Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
                    let view_ty = resolve_view_ty_tokens(view_scope, field, &quote! { 'a })?;
                    (
                        quote! { #vr::Message(#cow::Borrowed(&**v)) },
                        quote! {
                            #vr::Message(#cow::Borrowed(
                                <#view_ty as ::buffa::view::DefaultViewInstance>::default_view_instance(),
                            ))
                        },
                    )
                }
                Type::TYPE_ENUM => (
                    quote! { #vr::EnumNumber(v.to_i32()) },
                    quote! { #vr::EnumNumber(0) },
                ),
                _ => {
                    let variant_v = scalar_variant(ty);
                    let def = scalar_default(ty);
                    (
                        quote! { #vr::#variant_v(*v) },
                        quote! { #vr::#variant_v(#def) },
                    )
                }
            };

            get_arms.push(quote! {
                #number => match &self.#field_ident {
                    ::core::option::Option::Some(#view_enum::#variant(v)) => #active,
                    _ => #default,
                },
            });
            has_arms.push(quote! {
                #number => ::core::matches!(
                    &self.#field_ident,
                    ::core::option::Option::Some(#view_enum::#variant(_))
                ),
            });
        }
    }

    // Path from the view module back to `__buffa::reflect::descriptor_pool()`.
    let mut supers = TokenStream::new();
    for _ in 0..view_depth {
        supers.extend(quote! { super:: });
    }
    let sentinel = make_field_ident(SENTINEL_MOD);
    let pool = quote! { #supers #sentinel::reflect::descriptor_pool() };

    Ok(quote! {
        impl<'a> ::buffa_descriptor::reflect::ReflectMessage for #view_ident<'a> {
            fn message_descriptor(&self) -> &::buffa_descriptor::MessageDescriptor {
                #pool.message(Self::__buffa_reflect_message_index())
            }

            fn pool(&self) -> &::buffa::alloc::sync::Arc<::buffa_descriptor::DescriptorPool> {
                #pool
            }

            fn get(&self, field: &::buffa_descriptor::FieldDescriptor) -> #vr<'_> {
                // Closed enums are stored as the bare enum type, whose `to_i32`
                // is the `Enumeration` trait method (open enums use the inherent
                // `EnumValue::to_i32`, which needs no import). No-op for messages
                // without enum fields.
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                match field.number() {
                    #(#get_arms)*
                    _ => {
                        ::core::debug_assert!(
                            false,
                            "field number {} is not a member of this view's reflect get()",
                            field.number(),
                        );
                        #vr::Bool(false)
                    }
                }
            }

            fn has(&self, field: &::buffa_descriptor::FieldDescriptor) -> bool {
                match field.number() {
                    #(#has_arms)*
                    _ => false,
                }
            }

            fn for_each_set(
                &self,
                f: &mut dyn ::core::ops::FnMut(&::buffa_descriptor::FieldDescriptor, #vr<'_>),
            ) {
                let md = ::buffa_descriptor::reflect::ReflectMessage::message_descriptor(self);
                for fd in md.fields() {
                    if ::buffa_descriptor::reflect::ReflectMessage::has(self, fd) {
                        f(fd, ::buffa_descriptor::reflect::ReflectMessage::get(self, fd));
                    }
                }
            }

            fn to_dynamic(&self) -> ::buffa_descriptor::reflect::DynamicMessage {
                // The one allocating path in vtable mode (an explicit owned
                // snapshot — plain field reads never reach it). Encode the view
                // directly and decode into a `DynamicMessage`, skipping the
                // intermediate owned-message tree that `from_message` would build.
                let bytes = ::buffa::ViewEncode::encode_to_vec(self);
                ::buffa_descriptor::reflect::DynamicMessage::decode(
                    ::buffa::alloc::sync::Arc::clone(#pool),
                    Self::__buffa_reflect_message_index(),
                    &bytes,
                )
                .expect("view re-encodes to bytes decodable against its own descriptor")
            }
        }

        impl<'a> ::buffa_descriptor::reflect::ReflectElement for #view_ident<'a> {
            fn as_value_ref(&self) -> #vr<'_> {
                #vr::Message(#cow::Borrowed(self))
            }
        }

        impl<'a> #view_ident<'a> {
            /// Memoized `MessageIndex` for this view's message type, resolved
            /// once against the package's embedded descriptor pool. An inherent
            /// associated fn (not a free fn) so sibling views in the same module
            /// do not collide.
            #[doc(hidden)]
            fn __buffa_reflect_message_index() -> ::buffa_descriptor::MessageIndex {
                static IDX: ::std::sync::OnceLock<::buffa_descriptor::MessageIndex> =
                    ::std::sync::OnceLock::new();
                *IDX.get_or_init(|| {
                    #pool
                        .message_index(<Self as ::buffa::MessageName>::FULL_NAME)
                        .expect("generated view type is registered in the embedded descriptor pool")
                })
            }
        }
    })
}

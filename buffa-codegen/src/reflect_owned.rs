//! Code generation for vtable-mode reflection on owned message types.
//!
//! Parallel to [`reflect_view`](crate::reflect_view), but for the owned struct
//! rather than the zero-copy view. When vtable mode is on, each owned message
//! gets:
//!
//! - `impl ReflectMessage for Foo` — reads owned struct fields directly
//!   (`String`/`Vec<u8>`/`MessageField`/`Vec`/`HashMap`/owned oneof enum),
//!   backed by the owned-container reflect impls in `buffa-descriptor`.
//! - `impl ReflectElement for Foo` — so a `Vec<Foo>` / `HashMap<_, Foo>`
//!   reflects through the generic container impls.
//! - A memoized per-message `MessageIndex` accessor.
//!
//! The owned `Reflectable::reflect()` body is then `ReflectCow::Borrowed(self)`
//! (emitted by [`reflect`](crate::reflect)), so reflecting an in-memory message
//! costs no encode/decode round-trip — the interceptor use case. Bridge mode
//! keeps the round-trip body and emits none of this.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;

use crate::context::CodeGenContext;
use crate::features::{resolve_field, ResolvedFeatures};
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::DescriptorProto;
use crate::impl_message::{
    effective_type, is_explicit_presence_scalar, is_real_oneof_member, is_supported_field_type,
};
use crate::message::{is_closed_enum, is_map_field, rust_path_to_tokens};
use crate::oneof::oneof_variant_ident;
use crate::reflect_view::{scalar_default, scalar_variant};
use crate::CodeGenError;

/// Context needed to emit the owned-message vtable impls, mirroring the
/// arguments [`generate_message_impl`](crate::impl_message::generate_message_impl)
/// already has in hand.
pub(crate) struct OwnedReflectScope<'a> {
    pub ctx: &'a CodeGenContext<'a>,
    pub msg: &'a DescriptorProto,
    pub name_ident: &'a proc_macro2::Ident,
    pub buffa_path: &'a TokenStream,
    pub current_package: &'a str,
    /// Message path without a leading dot (e.g. `pkg.Outer.Inner`), used to
    /// build variant paths for `variant_boxed` lookups.
    pub proto_fqn: &'a str,
    pub features: &'a ResolvedFeatures,
    pub oneof_idents: &'a HashMap<usize, proc_macro2::Ident>,
    pub oneof_prefix: &'a TokenStream,
    pub nesting: usize,
}

/// Generate `impl ReflectMessage` + `impl ReflectElement` + the memoized
/// `MessageIndex` accessor for an owned message.
pub(crate) fn reflect_owned_impls(
    scope: &OwnedReflectScope<'_>,
) -> Result<TokenStream, CodeGenError> {
    let ctx = scope.ctx;
    let msg = scope.msg;
    let name_ident = scope.name_ident;
    let buffa_path = scope.buffa_path;
    let current_package = scope.current_package;
    let proto_fqn = scope.proto_fqn;
    let features = scope.features;
    let oneof_idents = scope.oneof_idents;
    let oneof_prefix = scope.oneof_prefix;
    let nesting = scope.nesting;
    let vr = quote! { ::buffa_descriptor::reflect::ValueRef };
    let cow = quote! { ::buffa_descriptor::reflect::ReflectCow };
    // Message-typed fields route through the field type's own
    // `Reflectable::reflect()` rather than `ReflectCow::Borrowed` directly:
    // a vtable-grade field returns `Borrowed` (zero-cost, same as before),
    // while a bridge-grade field from another compilation degrades to an
    // owned `DynamicMessage` snapshot at the boundary — the mixed-mode
    // behavior the reflection design promises.
    let reflectable = quote! { ::buffa_descriptor::reflect::Reflectable };

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
        let id = ctx.field_ident(name, field.number.unwrap_or(0));
        let number = field.number.unwrap_or(0) as u32;
        let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;

        if is_repeated && is_map_field(msg, field) {
            get_arms.push(quote! { #number => #vr::Map(&self.#id), });
            // The default `HashMap` keeps `.is_empty()` (byte-identical output);
            // a `BTreeMap` or custom map is checked through the generic
            // `MapStorage` surface, which any configured map implements.
            if crate::impl_message::field_map_repr(ctx, proto_fqn, name).is_default() {
                has_arms.push(quote! { #number => !self.#id.is_empty(), });
            } else {
                has_arms.push(quote! {
                    #number => ::buffa::map_codec::MapStorage::storage_len(&self.#id) != 0,
                });
            }
            continue;
        }
        if is_repeated {
            get_arms.push(quote! { #number => #vr::List(&self.#id), });
            has_arms.push(quote! { #number => !self.#id.is_empty(), });
            continue;
        }

        let f_features = resolve_field(ctx, field, features);
        let (get_val, has_val) = if is_explicit_presence_scalar(field, ty, &f_features) {
            // Stored as `Option<T>`; absent singular returns the type default.
            match ty {
                Type::TYPE_STRING => (
                    quote! { #vr::String(self.#id.as_deref().unwrap_or("")) },
                    quote! { self.#id.is_some() },
                ),
                Type::TYPE_BYTES => (
                    quote! { #vr::Bytes(self.#id.as_deref().unwrap_or(&[])) },
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
            // non-default. proto2 `required` fields also fall here (stored as
            // bare types), so a required field set to its default reflects as
            // `has() == false` — the same limitation the view path documents.
            match ty {
                Type::TYPE_STRING => (
                    quote! { #vr::String(&self.#id) },
                    quote! { !self.#id.is_empty() },
                ),
                Type::TYPE_BYTES => (
                    quote! { #vr::Bytes(&self.#id[..]) },
                    quote! { !self.#id.is_empty() },
                ),
                Type::TYPE_MESSAGE | Type::TYPE_GROUP => (
                    // `MessageField` derefs to the inner message, or the static
                    // default instance when unset, so the borrow is always valid.
                    quote! { #vr::Message(#reflectable::reflect(&*self.#id)) },
                    quote! { self.#id.is_set() },
                ),
                Type::TYPE_ENUM => {
                    // Closed enums compare against the type default (which
                    // need not be zero); enums opened by an enum-type override keep
                    // their declared default the same way.
                    let has_val = if is_closed_enum(&f_features) {
                        quote! { self.#id != ::core::default::Default::default() }
                    } else if let Some(default_expr) =
                        crate::defaults::open_enum_bare_default_value(
                            field,
                            ctx,
                            current_package,
                            &f_features,
                            nesting,
                        )?
                    {
                        quote! { self.#id != #default_expr }
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
        get_arms.push(quote! { #number => #get_val, });
        has_arms.push(quote! { #number => #has_val, });
    }

    // Oneof members dispatch through the `Option<Kind>` struct field.
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let Some(base_ident) = oneof_idents.get(&idx) else {
            continue;
        };
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let field_ident = ctx.oneof_ident(oneof_name);
        let oneof_enum = quote! { #oneof_prefix #base_ident };

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
                Type::TYPE_BYTES => (quote! { #vr::Bytes(&v[..]) }, quote! { #vr::Bytes(&[]) }),
                Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
                    let owned_ty = resolve_owned_message_ty(ctx, field, current_package, nesting)?;
                    // Boxed variants bind `v: &Box<M>` (deref twice to reach
                    // `&M`); variants opted out via unbox_oneof store the
                    // message inline and bind `v: &M` directly.
                    let borrowed = if crate::oneof::variant_boxed(
                        ctx,
                        ty,
                        &format!(".{proto_fqn}.{oneof_name}.{name}"),
                    ) {
                        quote! { &**v }
                    } else {
                        quote! { v }
                    };
                    (
                        quote! { #vr::Message(#reflectable::reflect(#borrowed)) },
                        quote! {
                            #vr::Message(#reflectable::reflect(
                                <#owned_ty as ::buffa::DefaultInstance>::default_instance(),
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
                    ::core::option::Option::Some(#oneof_enum::#variant(v)) => #active,
                    _ => #default,
                },
            });
            has_arms.push(quote! {
                #number => ::core::matches!(
                    &self.#field_ident,
                    ::core::option::Option::Some(#oneof_enum::#variant(_))
                ),
            });
        }
    }

    let pool = quote! { #buffa_path::reflect::descriptor_pool() };

    // Preserve unknown fields through reflection, matching the bridge path
    // (`DynamicMessage` carries them). Without this override the trait default
    // returns an empty set, so a recursive reflective walk over nested messages
    // would silently drop fields the local schema doesn't know — the exact
    // regression `ReflectMessage::unknown_fields`'s own doc warns against.
    let unknown_fields_method = if ctx.config.preserve_unknown_fields {
        quote! {
            fn unknown_fields(&self) -> &::buffa::UnknownFields {
                &self.__buffa_unknown_fields
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl ::buffa_descriptor::reflect::ReflectMessage for #name_ident {
            fn message_descriptor(&self) -> &::buffa_descriptor::MessageDescriptor {
                #pool.message(Self::__buffa_reflect_message_index())
            }

            fn pool(&self) -> &::buffa::alloc::sync::Arc<::buffa_descriptor::DescriptorPool> {
                #pool
            }

            #unknown_fields_method

            fn get(&self, field: &::buffa_descriptor::FieldDescriptor) -> #vr<'_> {
                // Closed enums use the `Enumeration` trait `to_i32`; open enums
                // (`EnumValue`) use the inherent one. No-op import for messages
                // without enum fields.
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                match field.number() {
                    #(#get_arms)*
                    _ => {
                        ::core::debug_assert!(
                            false,
                            "field number {} is not a member of this message's reflect get()",
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
                ::buffa_descriptor::reflect::DynamicMessage::from_message(
                    self,
                    ::buffa::alloc::sync::Arc::clone(#pool),
                    Self::__buffa_reflect_message_index(),
                )
            }
        }

        // `#[inline]` for the same cross-crate zero-cost reason as the
        // vtable `Reflectable::reflect()` body (see reflect.rs).
        impl ::buffa_descriptor::reflect::ReflectElement for #name_ident {
            #[inline]
            fn as_value_ref(&self) -> #vr<'_> {
                #vr::Message(#cow::Borrowed(self))
            }
        }

        impl #name_ident {
            /// Memoized `MessageIndex` for this message type, resolved once
            /// against the package's embedded descriptor pool.
            #[doc(hidden)]
            fn __buffa_reflect_message_index() -> ::buffa_descriptor::MessageIndex {
                static IDX: ::std::sync::OnceLock<::buffa_descriptor::MessageIndex> =
                    ::std::sync::OnceLock::new();
                *IDX.get_or_init(|| {
                    #pool
                        .message_index(<Self as ::buffa::MessageName>::FULL_NAME)
                        .expect("generated message is registered in the embedded descriptor pool")
                })
            }
        }
    })
}

/// Resolve the owned Rust type token for a message-typed oneof member, used for
/// the unset-member default (`<Ty as DefaultInstance>::default_instance()`).
fn resolve_owned_message_ty(
    ctx: &CodeGenContext,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    current_package: &str,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let dotted = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.type_name"))?;
    let path = ctx
        .rust_type_relative(dotted, current_package, nesting)
        .ok_or_else(|| {
            CodeGenError::Other(format!(
                "owned type for oneof message '{dotted}' not resolvable"
            ))
        })?;
    Ok(rust_path_to_tokens(&path))
}

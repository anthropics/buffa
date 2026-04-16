//! Message struct code generation.

use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::DescriptorProto;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::{CodeGenContext, MessageScope};
use crate::defaults::parse_default_value;
use crate::features::ResolvedFeatures;
use crate::impl_message::{is_explicit_presence_scalar, is_real_oneof_member};
use crate::CodeGenError;

/// Qualified paths to the per-message / per-extension registry `const` items
/// emitted alongside the structs, bubbled up to the file-level
/// `register_types` fn. Each vec is relative to the scope where the
/// corresponding consts land (see `generate_message`'s doc comment).
#[derive(Default)]
pub(crate) struct RegistryPaths {
    /// `__*_JSON_ANY` consts (relative to the struct's scope).
    pub json_any: Vec<TokenStream>,
    /// `__*_TEXT_ANY` consts (relative to the struct's scope).
    pub text_any: Vec<TokenStream>,
    /// `__*_JSON_EXT` consts (relative to the message's module scope).
    pub json_ext: Vec<TokenStream>,
    /// `__*_TEXT_EXT` consts (relative to the message's module scope).
    pub text_ext: Vec<TokenStream>,
}

impl RegistryPaths {
    pub(crate) fn is_empty(&self) -> bool {
        self.json_any.is_empty()
            && self.text_any.is_empty()
            && self.json_ext.is_empty()
            && self.text_ext.is_empty()
    }
}

/// Generate Rust code for a message type (and its nested types).
///
/// `current_package` is the proto package of the file being generated.
/// Types belonging to this package are referenced without the module prefix
/// since the generated code will be wrapped in `pub mod pkg { ... }`.
///
/// `rust_name` is the Rust struct name to emit.  For top-level messages this
/// is the proto message name; for nested messages it is the simple proto name
/// (e.g. `Inner`) since module nesting provides scoping.
///
/// `proto_fqn` is the fully-qualified proto type name without a leading dot
/// (e.g. `google.protobuf.Timestamp`, `my.package.Outer.Inner`).  It is used
/// to emit the `TYPE_URL` constant.
/// Returns `(top_level_items, module_items, registry_paths)` where
/// `top_level_items` contains the struct, its impls, and the custom
/// deserialize; `module_items` contains nested types and oneof enums to be
/// placed in `pub mod <name>`; and `registry_paths` collects qualified paths
/// to the per-message `__*_JSON_ANY` / `__*_TEXT_ANY` consts (relative to
/// the struct's scope) and per-extension `__*_JSON_EXT` / `__*_TEXT_EXT`
/// consts (relative to the `module_items` scope) for `register_types`.
pub fn generate_message(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    current_package: &str,
    rust_name: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    resolver: &crate::imports::ImportResolver,
) -> Result<(TokenStream, TokenStream, RegistryPaths), CodeGenError> {
    let scope = MessageScope {
        ctx,
        current_package,
        proto_fqn,
        features,
        nesting: 0,
    };
    generate_message_with_nesting(scope, msg, rust_name, resolver)
}

fn generate_message_with_nesting(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    rust_name: &str,
    resolver: &crate::imports::ImportResolver,
) -> Result<(TokenStream, TokenStream, RegistryPaths), CodeGenError> {
    let MessageScope {
        ctx,
        current_package,
        proto_fqn,
        features,
        nesting,
    } = scope;
    let name_ident = format_ident!("{}", rust_name);

    // MessageSet wire format: legacy Google encoding that wraps each extension
    // in a group at field 1. protoc enforces the "no regular fields" invariant
    // on the descriptor, so we don't re-check it here. The flag is read again
    // inside `generate_message_impl` to branch the unknown-fields snippets.
    let is_message_set = msg
        .options
        .as_option()
        .and_then(|o| o.message_set_wire_format)
        .unwrap_or(false);
    if is_message_set && !ctx.config.allow_message_set {
        return Err(CodeGenError::MessageSetNotSupported {
            message_name: proto_fqn.to_string(),
        });
    }

    // Nested enums — simple name, emitted inside the message's module.
    let nested_enums = msg
        .enum_type
        .iter()
        .map(|e| {
            let enum_name = e.name.as_deref().unwrap_or("");
            let enum_fqn = format!("{}.{}", proto_fqn, enum_name);
            crate::enumeration::generate_enum(ctx, e, enum_name, &enum_fqn, features, resolver)
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Nested messages (skip map entry synthetics) — simple name, emitted
    // inside the message's module.
    let nested_msgs = msg
        .nested_type
        .iter()
        .filter(|nested| {
            !nested
                .options
                .as_option()
                .and_then(|o| o.map_entry)
                .unwrap_or(false)
        })
        .map(|nested| {
            let nested_proto_name = nested.name.as_deref().unwrap_or("");
            let nested_fqn = format!("{}.{}", proto_fqn, nested_proto_name);
            let msg_features =
                crate::features::resolve_child(features, crate::features::message_features(nested));
            generate_message_with_nesting(
                scope.nested(&nested_fqn, &msg_features),
                nested,
                nested_proto_name,
                resolver,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Direct struct fields (real oneof fields are excluded; they go in the oneof enum).
    // Type resolution uses the package path since the struct sits at the
    // package level, not inside the message's module.
    let generated_fields: Vec<GeneratedField> = msg
        .field
        .iter()
        .map(|f| generate_field(scope, msg, f, resolver))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let direct_fields: Vec<&TokenStream> = generated_fields.iter().map(|f| &f.tokens).collect();

    // Collect field identifiers for the manual Debug impl (excludes __buffa_ internals).
    let mut debug_field_idents: Vec<&Ident> = generated_fields.iter().map(|f| &f.ident).collect();

    // Module name for this message (snake_case of proto name).
    let proto_name = msg.name.as_deref().unwrap_or(rust_name);
    let mod_name_str = crate::oneof::to_snake_case(proto_name);
    let mod_ident = make_field_ident(&mod_name_str);

    // Compute oneof enum identifiers for all non-synthetic oneofs up front.
    // Sequential allocation prevents sibling oneofs from claiming the same
    // suffixed name (see `resolve_oneof_idents`).
    let oneof_idents =
        crate::oneof::resolve_oneof_idents(msg, proto_fqn, ctx.config.generate_views)?;

    // One `Option<OneofEnum>` field in the struct per non-synthetic oneof.
    // Oneof enums live inside the message's module, so the type path is
    // `mod_name::EnumName`.
    let oneof_serde_attr = if ctx.config.generate_json {
        quote! { #[serde(flatten)] }
    } else {
        quote! {}
    };
    let oneof_generated: Vec<(TokenStream, Ident)> = msg
        .oneof_decl
        .iter()
        .enumerate()
        .filter_map(|(idx, oneof)| {
            let enum_ident = oneof_idents.get(&idx)?;
            let oneof_name = oneof.name.as_deref()?;
            let field_ident = make_field_ident(oneof_name);
            let opt = resolver.option();
            let tokens = quote! {
                #oneof_serde_attr
                pub #field_ident: #opt<#mod_ident::#enum_ident>,
            };
            Some((tokens, field_ident))
        })
        .collect();
    let oneof_struct_fields: Vec<&TokenStream> = oneof_generated.iter().map(|(t, _)| t).collect();
    debug_field_idents.extend(oneof_generated.iter().map(|(_, id)| id));

    // When JSON is on, `__buffa_unknown_fields` becomes a `#[serde(flatten)]`
    // newtype wrapper whose Serialize/Deserialize route through the extension
    // registry. The wrapper has `Deref<Target = UnknownFields>` so all binary
    // encode/decode paths in impl_message.rs are unaffected (method-call
    // auto-deref and `&wrapper` → `&UnknownFields` coercion both apply).
    //
    // Gated on `has_extension_ranges`: protoc rejects `extend Foo { ... }`
    // when `Foo` lacks an `extensions N to M;` declaration, so a message
    // without one can never have a registry entry naming it as extendee.
    // Without this gate, the wrapper is pure overhead — `#[serde(flatten)]`
    // on derive-Deserialize buffers every unknown key through serde's
    // `Content::Map` (String key + `serde_json::Value` DOM) before the
    // wrapper can discard it. With the gate, extension-range-free messages
    // keep the pre-extensions `#[serde(skip)]` behavior (zero-alloc
    // `IgnoredAny` skip for unknown keys).
    let has_extension_ranges = !msg.extension_range.is_empty();
    let use_ext_json_wrapper =
        ctx.config.generate_json && ctx.config.preserve_unknown_fields && has_extension_ranges;
    let ext_json_wrapper_ident = format_ident!("__{}ExtJson", rust_name);
    let (unknown_fields_field, ext_json_wrapper_def) = if use_ext_json_wrapper {
        let arbitrary_attr = if ctx.config.generate_arbitrary {
            quote! { #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))] }
        } else {
            quote! {}
        };
        let proto_fqn_lit = proto_fqn;
        let wrapper = quote! {
            #[doc(hidden)]
            #[derive(Clone, Debug, Default, PartialEq)]
            #[repr(transparent)]
            #arbitrary_attr
            pub struct #ext_json_wrapper_ident(pub ::buffa::UnknownFields);

            impl ::core::ops::Deref for #ext_json_wrapper_ident {
                type Target = ::buffa::UnknownFields;
                fn deref(&self) -> &::buffa::UnknownFields { &self.0 }
            }
            impl ::core::ops::DerefMut for #ext_json_wrapper_ident {
                fn deref_mut(&mut self) -> &mut ::buffa::UnknownFields { &mut self.0 }
            }
            impl ::core::convert::From<::buffa::UnknownFields> for #ext_json_wrapper_ident {
                fn from(u: ::buffa::UnknownFields) -> Self { Self(u) }
            }
            impl ::serde::Serialize for #ext_json_wrapper_ident {
                fn serialize<S: ::serde::Serializer>(&self, s: S)
                    -> ::core::result::Result<S::Ok, S::Error>
                {
                    ::buffa::extension_registry::serialize_extensions(#proto_fqn_lit, &self.0, s)
                }
            }
            impl<'de> ::serde::Deserialize<'de> for #ext_json_wrapper_ident {
                fn deserialize<D: ::serde::Deserializer<'de>>(d: D)
                    -> ::core::result::Result<Self, D::Error>
                {
                    ::buffa::extension_registry::deserialize_extensions(#proto_fqn_lit, d).map(Self)
                }
            }
        };
        let field = quote! {
            #[serde(flatten)]
            #[doc(hidden)]
            pub __buffa_unknown_fields: #ext_json_wrapper_ident,
        };
        (field, wrapper)
    } else if ctx.config.preserve_unknown_fields {
        // No wrapper — either generate_json is off, or this message has no
        // extension ranges. In the latter case the serde derive is present
        // and we must `#[serde(skip)]` to exclude the field from JSON; in
        // the former the attribute is harmless (no derive to read it).
        let skip_attr = if ctx.config.generate_json {
            quote! { #[serde(skip)] }
        } else {
            quote! {}
        };
        let field = quote! {
            #skip_attr
            #[doc(hidden)]
            pub __buffa_unknown_fields: ::buffa::UnknownFields,
        };
        (field, quote! {})
    } else {
        (quote! {}, quote! {})
    };

    // Does this message have real (non-synthetic) oneofs?
    let has_real_oneofs = !oneof_struct_fields.is_empty();

    // Messages declaring `extensions N to M;` accept `"[...]"` JSON keys.
    // With only `#[derive(Deserialize)]`, serde's flatten already routes them
    // to the wrapper's Deserialize — but that path buffers all unclaimed keys
    // into a serde_json::Value first. The custom impl matches them inline.

    // When serde is enabled and the message has oneofs, we generate a custom
    // Deserialize impl so that duplicate-oneof-field and null-value errors
    // propagate correctly (serde's #[serde(flatten)] + Option<T> swallows them).
    // Extension ranges also force the custom impl so `"[...]"` keys are
    // handled inline without buffering.
    let needs_custom_deserialize = ctx.config.generate_json
        && (has_real_oneofs || (has_extension_ranges && ctx.config.preserve_unknown_fields));

    // Oneof enum definitions — emitted inside the message's module.
    // Pass the file-level package as current_package, since
    // nesting=1 in the oneof codegen handles the module depth.
    let oneof_enums = msg
        .oneof_decl
        .iter()
        .enumerate()
        .map(|(idx, oneof)| {
            crate::oneof::generate_oneof_enum(
                ctx,
                msg,
                idx,
                oneof,
                current_package,
                proto_fqn,
                features,
                resolver,
                &oneof_idents,
                nesting,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let message_impl = crate::impl_message::generate_message_impl(
        ctx,
        msg,
        ctx.config.preserve_unknown_fields,
        rust_name,
        current_package,
        proto_fqn,
        features,
        &oneof_idents,
        nesting,
    )?;

    let text_impl = crate::impl_text::generate_text_impl(
        ctx,
        msg,
        rust_name,
        current_package,
        proto_fqn,
        features,
        has_extension_ranges,
        &oneof_idents,
        nesting,
    )?;

    let type_url = format!("type.googleapis.com/{proto_fqn}");
    let upper = crate::oneof::to_snake_case(rust_name).to_uppercase();

    // JSON Any entry — one per message with `generate_json`. Always
    // `is_wkt: false`: WKTs live in buffa-types and register themselves via
    // the hand-written `register_wkt_types` which knows which types get
    // `"value"` wrapping in Any JSON. The `any_to_json::<M>` /
    // `any_from_json::<M>` monomorphizations coerce to fn pointers in const
    // context (same pattern as enum_to_json<E> in extension_registry).
    let (json_any_const, json_any_ident) = if ctx.config.generate_json {
        let ident = format_ident!("__{}_JSON_ANY", upper);
        let tokens = quote! {
            #[doc(hidden)]
            pub const #ident: ::buffa::type_registry::JsonAnyEntry
                = ::buffa::type_registry::JsonAnyEntry {
                    type_url: #type_url,
                    to_json: ::buffa::type_registry::any_to_json::<#name_ident>,
                    from_json: ::buffa::type_registry::any_from_json::<#name_ident>,
                    is_wkt: false,
                };
        };
        (tokens, Some(ident))
    } else {
        (quote! {}, None)
    };

    // Text Any entry — one per message with `generate_text`. Independent of
    // `generate_json`: M implements TextFormat iff `generate_text` was on,
    // and the monomorphization only typechecks then. No `Option<fn>`
    // placeholder — JSON and text entries are separate consts in
    // feature-split maps, so presence in the text map means text-capable.
    let (text_any_const, text_any_ident) = if ctx.config.generate_text {
        let ident = format_ident!("__{}_TEXT_ANY", upper);
        let tokens = quote! {
            #[doc(hidden)]
            pub const #ident: ::buffa::type_registry::TextAnyEntry
                = ::buffa::type_registry::TextAnyEntry {
                    type_url: #type_url,
                    text_encode: ::buffa::type_registry::any_encode_text::<#name_ident>,
                    text_merge: ::buffa::type_registry::any_merge_text::<#name_ident>,
                };
        };
        (tokens, Some(ident))
    } else {
        (quote! {}, None)
    };

    let serde_struct_derive = if ctx.config.generate_json {
        if needs_custom_deserialize {
            // Only derive Serialize; Deserialize is generated separately.
            quote! {
                #[derive(::serde::Serialize)]
                #[serde(default)]
            }
        } else {
            quote! {
                #[derive(::serde::Serialize, ::serde::Deserialize)]
                #[serde(default)]
            }
        }
    } else {
        quote! {}
    };
    let arbitrary_derive = if ctx.config.generate_arbitrary {
        quote! { #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))] }
    } else {
        quote! {}
    };
    let custom_type_attrs =
        CodeGenContext::matching_attributes(&ctx.config.type_attributes, proto_fqn)?;
    let custom_message_attrs =
        CodeGenContext::matching_attributes(&ctx.config.message_attributes, proto_fqn)?;
    let custom_deserialize = if needs_custom_deserialize {
        generate_custom_deserialize(
            scope,
            msg,
            &name_ident,
            &mod_ident,
            resolver,
            has_extension_ranges,
            &oneof_idents,
        )?
    } else {
        quote! {}
    };

    // ProtoElemJson impl for use in proto_seq / proto_map containers.
    // Delegates to the derived/generated Serialize + Deserialize.
    let proto_elem_json_impl = if ctx.config.generate_json {
        quote! {
            impl ::buffa::json_helpers::ProtoElemJson for #name_ident {
                fn serialize_proto_json<S: ::serde::Serializer>(
                    v: &Self,
                    s: S,
                ) -> ::core::result::Result<S::Ok, S::Error> {
                    ::serde::Serialize::serialize(v, s)
                }
                fn deserialize_proto_json<'de, D: ::serde::Deserializer<'de>>(
                    d: D,
                ) -> ::core::result::Result<Self, D::Error> {
                    <Self as ::serde::Deserialize>::deserialize(d)
                }
            }
        }
    } else {
        quote! {}
    };

    let cached_size_serde_skip = if ctx.config.generate_json {
        quote! { #[serde(skip)] }
    } else {
        quote! {}
    };

    // Check if any non-optional field has a custom default value, which
    // requires a hand-written `impl Default` instead of `#[derive(Default)]`.
    let custom_default_impl =
        generate_custom_default(ctx, msg, &name_ident, current_package, features, nesting)?;
    let derive_default = if custom_default_impl.is_some() {
        quote! {}
    } else {
        quote! { Default, }
    };
    let custom_default_impl = custom_default_impl.unwrap_or_default();

    // Build module items from nested messages. Each nested message contributes:
    // - Its struct + impls (top_level) go directly into our module
    // - Its nested types (mod_items) + view types go into a sub-module
    // - Its registry const paths bubble up, prefixed appropriately
    let mut nested_items = TokenStream::new();
    // Any-entry paths are relative to THIS struct's scope (top_level).
    // Our own consts land alongside our struct; nested messages' consts land
    // in our mod_items, which the caller wraps in `pub mod #mod_ident`, so
    // their returned paths get prefixed with `#mod_ident::`.
    // Extension-entry paths are relative to the message's MODULE scope.
    let mut reg_paths = RegistryPaths::default();
    if let Some(id) = &json_any_ident {
        reg_paths.json_any.push(quote! { #id });
    }
    if let Some(id) = &text_any_ident {
        reg_paths.text_any.push(quote! { #id });
    }
    let non_map_nested: Vec<&DescriptorProto> = msg
        .nested_type
        .iter()
        .filter(|n| {
            !n.options
                .as_option()
                .and_then(|o| o.map_entry)
                .unwrap_or(false)
        })
        .collect();
    for (nested_desc, (top, mod_items, nested_reg)) in non_map_nested.iter().zip(nested_msgs) {
        nested_items.extend(top);
        let nested_name = nested_desc.name.as_deref().unwrap_or("");
        let nested_mod = make_field_ident(&crate::oneof::to_snake_case(nested_name));
        // Extension paths: nested's module-scope → our module-scope = prefix
        // with the nested message's own module ident.
        for p in nested_reg.json_ext {
            reg_paths.json_ext.push(quote! { #nested_mod :: #p });
        }
        for p in nested_reg.text_ext {
            reg_paths.text_ext.push(quote! { #nested_mod :: #p });
        }
        // Any paths: nested's struct-scope → our struct-scope = prefix
        // with our own module ident (where nested's top_level lands).
        for p in nested_reg.json_any {
            reg_paths.json_any.push(quote! { #mod_ident :: #p });
        }
        for p in nested_reg.text_any {
            reg_paths.text_any.push(quote! { #mod_ident :: #p });
        }

        // Also generate views for nested messages if enabled.
        // view_top (struct + impls) goes alongside the owned struct in the
        // parent module; view_mod (oneof view enums) goes in the sub-module.
        let view_mod_items = if ctx.config.generate_views {
            let nested_name = nested_desc.name.as_deref().unwrap_or("");
            let nested_fqn = format!("{}.{}", proto_fqn, nested_name);
            let (view_top, view_mod) = crate::view::generate_view_with_nesting(
                MessageScope {
                    proto_fqn: &nested_fqn,
                    nesting: nesting + 1,
                    ..scope
                },
                nested_desc,
                nested_name,
            )?;
            nested_items.extend(view_top);
            view_mod
        } else {
            quote! {}
        };

        if !mod_items.is_empty() || !view_mod_items.is_empty() {
            nested_items.extend(quote! {
                pub mod #nested_mod {
                    #[allow(unused_imports)]
                    use super::*;
                    #mod_items
                    #view_mod_items
                }
            });
        }
    }

    // `extend` declarations nested inside this message. The consts land in
    // the message's `pub mod`, one `super::` hop from package level.
    // `proto_fqn` is the scope for JSON full_name construction.
    let (nested_extensions, nested_ext_json, nested_ext_text) =
        crate::extension::generate_extensions(
            ctx,
            &msg.extension,
            current_package,
            // Extensions declared inside this message live in its module
            // (`pub mod {msg} { pub fn register_extensions() { ... } }`),
            // so type references inside them need one additional `super::`
            // hop beyond the current message's own nesting.
            nesting + 1,
            features,
            proto_fqn,
        )?;
    for id in nested_ext_json {
        reg_paths.json_ext.push(quote! { #id });
    }
    for id in nested_ext_text {
        reg_paths.text_ext.push(quote! { #id });
    }

    // Module items: nested enums, nested message structs + sub-modules,
    // and oneof enums.
    let mod_items = quote! {
        #(#nested_enums)*
        #nested_items
        #(#oneof_enums)*
        #nested_extensions
    };

    // Generate a manual Debug impl that excludes internal __buffa_ fields.
    let struct_name_str = name_ident.to_string();
    let debug_field_names: Vec<String> =
        debug_field_idents.iter().map(|id| id.to_string()).collect();
    let debug_impl = quote! {
        impl ::core::fmt::Debug for #name_ident {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(#struct_name_str)
                    #(.field(#debug_field_names, &self.#debug_field_idents))*
                    .finish()
            }
        }
    };

    let message_doc = crate::comments::doc_attrs(ctx.comment(proto_fqn));

    let top_level = quote! {
        #message_doc
        #[derive(Clone, PartialEq, #derive_default)]
        #serde_struct_derive
        #arbitrary_derive
        #custom_type_attrs
        #custom_message_attrs
        pub struct #name_ident {
            #(#direct_fields)*
            #(#oneof_struct_fields)*
            #unknown_fields_field
            #[doc(hidden)]
            #cached_size_serde_skip
            pub __buffa_cached_size: ::buffa::__private::CachedSize,
        }

        #debug_impl

        #custom_default_impl

        impl #name_ident {
            /// Protobuf type URL for this message, for use with `Any::pack` and
            /// `Any::unpack_if`.
            ///
            /// Format: `type.googleapis.com/<fully.qualified.TypeName>`
            pub const TYPE_URL: &'static str = #type_url;
        }

        #message_impl

        #text_impl

        #custom_deserialize

        #proto_elem_json_impl

        #ext_json_wrapper_def

        #json_any_const
        #text_any_const
    };

    Ok((top_level, mod_items, reg_paths))
}

// ── Custom Deserialize for messages with oneofs ──────────────────────────────
//
// serde's `#[serde(flatten)]` on `Option<T>` silently swallows deserialization
// errors from `T::deserialize`, converting them to `None`.  This prevents
// oneof duplicate-field rejection from propagating.  For messages that contain
// at least one real oneof, we generate a hand-written `Deserialize` impl that
// handles oneof fields inline in the message visitor.

/// Generate a custom `Deserialize` impl for a message that has oneofs.
///
/// Regular fields are deserialized using the same serde helpers as the
/// derive-based approach.  Oneof fields are handled inline with
/// `NullableDeserializeSeed` (null -> variant not set) and duplicate
/// detection (error on second non-null variant).
fn generate_custom_deserialize(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    name_ident: &proc_macro2::Ident,
    mod_ident: &proc_macro2::Ident,
    resolver: &crate::imports::ImportResolver,
    has_extension_ranges: bool,
    oneof_idents: &std::collections::HashMap<usize, Ident>,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        current_package,
        proto_fqn,
        features,
        nesting,
        ..
    } = scope;
    let mut field_vars = Vec::new();
    let mut match_arms = Vec::new();
    let mut field_inits = Vec::new();

    // Regular (non-oneof) fields.
    for field in &msg.field {
        if is_real_oneof_member(field) {
            continue;
        }
        let (var, arm, init) = custom_deser_regular_field(scope, msg, field, resolver)?;
        field_vars.push(var);
        match_arms.push(arm);
        field_inits.push(init);
    }

    // Oneof groups.
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let result = custom_deser_oneof_group(
            ctx,
            msg,
            idx,
            oneof,
            current_package,
            proto_fqn,
            mod_ident,
            features,
            resolver,
            oneof_idents,
            nesting,
        )?;
        let Some((var, arms, init)) = result else {
            continue;
        };
        field_vars.push(var);
        match_arms.extend(arms);
        field_inits.push(init);
    }

    // `"[pkg.ext]"` keys — collect the decoded UnknownField records in a
    // local Vec, then push into `__r.__buffa_unknown_fields` after `__r` is
    // built below (it doesn't exist yet inside the match loop).
    // Emitted only when the message declares `extensions N to M;` AND
    // preserve_unknown_fields is on (otherwise there's nowhere to store them).
    let (ext_var, ext_arm, ext_init) = if has_extension_ranges && ctx.config.preserve_unknown_fields
    {
        let proto_fqn_lit = proto_fqn;
        let var = quote! {
            let mut __ext_records: ::buffa::alloc::vec::Vec<::buffa::UnknownField>
                = ::buffa::alloc::vec::Vec::new();
        };
        let arm = quote! {
            __k if __k.starts_with('[') => {
                let __v: ::serde_json::Value = map.next_value()?;
                match ::buffa::extension_registry::deserialize_extension_key(
                    #proto_fqn_lit, __k, __v,
                ) {
                    ::core::option::Option::Some(::core::result::Result::Ok(__recs)) => {
                        for __rec in __recs {
                            __ext_records.push(__rec);
                        }
                    }
                    ::core::option::Option::Some(::core::result::Result::Err(__e)) => {
                        return ::core::result::Result::Err(
                            <A::Error as ::serde::de::Error>::custom(__e),
                        );
                    }
                    ::core::option::Option::None => {}
                }
            }
        };
        let init = quote! {
            for __rec in __ext_records {
                __r.__buffa_unknown_fields.push(__rec);
            }
        };
        (var, arm, init)
    } else {
        (quote! {}, quote! {}, quote! {})
    };

    // Assemble the impl block.
    let expecting_msg = format!("struct {name_ident}");

    Ok(quote! {
        impl<'de> serde::Deserialize<'de> for #name_ident {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> ::core::result::Result<Self, D::Error> {
                struct _V;
                impl<'de> serde::de::Visitor<'de> for _V {
                    type Value = #name_ident;

                    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        f.write_str(#expecting_msg)
                    }

                    #[allow(clippy::field_reassign_with_default)]
                    fn visit_map<A: serde::de::MapAccess<'de>>(
                        self,
                        mut map: A,
                    ) -> ::core::result::Result<#name_ident, A::Error> {
                        #(#field_vars)*
                        #ext_var

                        while let Some(key) = map.next_key::<::buffa::alloc::string::String>()? {
                            match key.as_str() {
                                #(#match_arms)*
                                #ext_arm
                                _ => { map.next_value::<serde::de::IgnoredAny>()?; }
                            }
                        }

                        // Start from the struct's Default (which may be a
                        // custom impl honouring proto2 [default = ...]
                        // annotations), then overwrite fields present in JSON.
                        let mut __r = <#name_ident as ::core::default::Default>::default();
                        #(#field_inits)*
                        #ext_init
                        Ok(__r)
                    }
                }
                d.deserialize_map(_V)
            }
        }
    })
}

/// Generate a `DeserializeSeed` wrapper that calls `inner` inside `deserialize`.
///
/// Produces a block expression:
/// ```ignore
/// { struct _S; impl DeserializeSeed for _S { ... } map.next_value_seed(_S)? }
/// ```
/// where the body of `deserialize` is `inner`, which should return
/// `Result<rust_type, D::Error>` using `d` as the deserializer binding.
fn deser_seed_expr(rust_type: &TokenStream, inner: TokenStream) -> TokenStream {
    quote! {{
        struct _S;
        impl<'de> serde::de::DeserializeSeed<'de> for _S {
            type Value = #rust_type;
            fn deserialize<D: serde::Deserializer<'de>>(self, d: D)
                -> ::core::result::Result<#rust_type, D::Error>
            {
                #inner
            }
        }
        map.next_value_seed(_S)?
    }}
}

/// Emit the variable declaration, match arm, and field initializer for one
/// regular (non-oneof) field in a custom `Deserialize` impl.
fn custom_deser_regular_field(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    resolver: &crate::imports::ImportResolver,
) -> Result<(TokenStream, TokenStream, TokenStream), CodeGenError> {
    let MessageScope { ctx, features, .. } = scope;
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let json_name = field.json_name.as_deref().unwrap_or(field_name);
    let var_ident = format_ident!("__f_{}", field_name);
    let field_ident = make_field_ident(field_name);

    let info = classify_field(scope, msg, field, resolver)?;
    let rust_type = &info.rust_type;

    let field_features = crate::features::resolve_field(ctx, field, features);
    let (with_module, null_deser) = field_deser_modules(
        crate::impl_message::effective_type(ctx, field, features),
        &info,
        &field_features,
    );

    // Deserialization expression for the field value.
    let deser_expr = if is_value_field(field, info.is_repeated, info.is_map) {
        // MessageField<Value> must forward null to Value::deserialize
        // rather than treating it as "field absent".
        let inner = quote! { ::buffa::json_helpers::message_field_always_present(d) };
        deser_seed_expr(rust_type, inner)
    } else if let Some(module) = with_module {
        let module_path: syn::Path = syn::parse_str(module)
            .map_err(|_| CodeGenError::InvalidTypePath(module.to_string()))?;
        let inner = quote! { #module_path::deserialize(d) };
        deser_seed_expr(rust_type, inner)
    } else if null_deser.is_some() {
        // repeated / map without a specific helper -> null_as_default
        let inner = quote! { ::buffa::json_helpers::null_as_default(d) };
        deser_seed_expr(rust_type, inner)
    } else {
        quote! { map.next_value::<#rust_type>()? }
    };

    // Match arm accepting both json_name and proto_name.
    let arm = if json_name != field_name {
        quote! { #json_name | #field_name => { #var_ident = Some(#deser_expr); } }
    } else {
        quote! { #json_name => { #var_ident = Some(#deser_expr); } }
    };

    let var_decl = quote! { let mut #var_ident: Option<#rust_type> = None; };
    // Overwrite only if present — missing fields keep the struct's Default
    // (which honours proto2 [default = X], unlike <T>::default()).
    let field_init = quote! {
        if let ::core::option::Option::Some(v) = #var_ident { __r.#field_ident = v; }
    };
    Ok((var_decl, arm, field_init))
}

/// Emit the variable declaration, match arms, and field initializer for one
/// oneof group in a custom `Deserialize` impl.
///
/// Returns `None` if the oneof has no real (non-synthetic) fields.
#[allow(clippy::too_many_arguments)]
fn custom_deser_oneof_group(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    idx: usize,
    oneof: &crate::generated::descriptor::OneofDescriptorProto,
    current_package: &str,
    proto_fqn: &str,
    mod_ident: &proc_macro2::Ident,
    features: &ResolvedFeatures,
    resolver: &crate::imports::ImportResolver,
    oneof_idents: &std::collections::HashMap<usize, Ident>,
    nesting: usize,
) -> Result<Option<(TokenStream, Vec<TokenStream>, TokenStream)>, CodeGenError> {
    let oneof_name = oneof
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("oneof.name"))?;

    let enum_ident = match oneof_idents.get(&idx) {
        Some(id) => id.clone(),
        None => return Ok(None),
    };

    let var_ident = format_ident!("__oneof_{}", oneof_name);
    let field_ident = make_field_ident(oneof_name);

    // Oneof enum lives in the message's module.
    let var_decl = quote! { let mut #var_ident: Option<#mod_ident::#enum_ident> = None; };
    let mut arms = Vec::new();

    for field in &msg.field {
        if !is_real_oneof_member(field) || field.oneof_index != Some(idx as i32) {
            continue;
        }
        let proto_name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let json_name = field.json_name.as_deref().unwrap_or(proto_name);
        let variant_ident = format_ident!("{}", crate::oneof::to_pascal_case(proto_name));
        let field_type = crate::impl_message::effective_type(ctx, field, features);
        // bytes_fields override: feeds #variant_type into the _DeserSeed
        // return type, which pins the generic T in json_helpers::bytes::
        // deserialize to Bytes (vs the Vec<u8> default). No downstream
        // shim needed — the helper is generic over T: From<Vec<u8>>.
        let variant_type = if field_type == Type::TYPE_BYTES
            && crate::impl_message::field_uses_bytes(ctx, proto_fqn, proto_name)
        {
            quote! { ::bytes::Bytes }
        } else {
            scalar_or_message_type_nested(ctx, field, current_package, nesting, features, resolver)?
        };

        // Module-qualified path for the oneof enum (lives in the message's module).
        let qualified_enum: TokenStream = quote! { #mod_ident::#enum_ident };
        let arm = crate::oneof::oneof_variant_deser_arm(&crate::oneof::OneofVariantDeserInput {
            variant_ident: &variant_ident,
            variant_type: &variant_type,
            json_name,
            proto_name,
            field_type,
            null_forward: crate::oneof::null_is_valid_value(field),
            is_boxed: crate::oneof::is_boxed_variant(field_type),
            enum_ident: &qualified_enum,
            result_var: &var_ident,
            oneof_name,
        });
        arms.push(arm);
    }

    let field_init = quote! { __r.#field_ident = #var_ident; };
    Ok(Some((var_decl, arms, field_init)))
}

/// Returns `true` for singular `google.protobuf.Value` fields.
///
/// For these fields, JSON `null` represents a valid `NullValue` rather than
/// "field absent", so deserialization must forward null to `Value::deserialize`.
fn is_value_field(
    field: &crate::generated::descriptor::FieldDescriptorProto,
    is_repeated: bool,
    is_map: bool,
) -> bool {
    field.r#type.unwrap_or_default() == Type::TYPE_MESSAGE
        && !is_repeated
        && !is_map
        && field.type_name.as_deref() == Some(".google.protobuf.Value")
}

/// Resolved Rust type and map-entry metadata for a single field.
#[derive(Debug)]
struct FieldInfo {
    rust_type: TokenStream,
    /// Type to use in the struct field declaration. Differs from `rust_type`
    /// only for self-referential message fields, where it uses `Self` instead
    /// of the concrete name. `rust_type` stays concrete for serde-deserialize
    /// codegen, which runs inside a local Visitor impl where `Self` binds to
    /// the wrong type.
    struct_field_type: TokenStream,
    is_repeated: bool,
    is_map: bool,
    /// Whether this field has explicit presence and uses `Option<T>` wrapping.
    /// True for proto3 `optional` scalars and proto2 `optional` (non-required)
    /// scalars. Not true for message fields (which use `MessageField<T>`),
    /// repeated fields, or proto2 `required` fields.
    is_optional: bool,
    /// Proto2 `required` (or editions `LEGACY_REQUIRED`). Required fields
    /// must always appear in JSON output regardless of value, matching the
    /// binary encoder's always-encode semantics.
    is_required: bool,
    map_key_type: Option<Type>,
    map_value_type: Option<Type>,
    /// Closedness of the **value enum** when `map_value_type == TYPE_ENUM`.
    /// Resolved from the map entry's value-field descriptor (which is
    /// TYPE_ENUM, so `resolve_field` correctly overlays the referenced
    /// enum's `enum_type`). Cannot be derived from the map field's own
    /// features — that field is TYPE_MESSAGE so the overlay doesn't fire.
    /// See `map_serde_module`.
    map_value_enum_closed: Option<bool>,
}

/// Resolve the Rust type and map-entry metadata for a single field.
///
/// Shared by `generate_field` (struct declaration) and the custom
/// deserialize codegen to avoid duplicating the type-resolution
/// if/else chain.
fn classify_field(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    resolver: &crate::imports::ImportResolver,
) -> Result<FieldInfo, CodeGenError> {
    let MessageScope {
        ctx,
        current_package,
        proto_fqn,
        features,
        nesting,
    } = scope;
    let label = field.label.unwrap_or_default();
    let field_type = crate::impl_message::effective_type(ctx, field, features);
    let is_repeated = label == Label::LABEL_REPEATED;
    let map_entry = if is_repeated {
        find_map_entry(msg, field)
    } else {
        None
    };
    let is_map = map_entry.is_some();
    let is_optional = is_explicit_presence_scalar(field, field_type, features);
    let is_required = crate::impl_message::is_required_field(field, features);

    // Check if this bytes field should use bytes::Bytes.
    let field_name = field.name.as_deref().unwrap_or("");
    let field_fqn = format!(".{}.{}", proto_fqn, field_name);
    let use_bytes = field_type == Type::TYPE_BYTES && ctx.use_bytes_type(&field_fqn);

    let bytes_type = if use_bytes {
        quote! { ::bytes::Bytes }
    } else {
        let vec = resolver.vec();
        quote! { #vec<u8> }
    };

    let rust_type = if let Some(entry) = map_entry {
        map_rust_type_from_entry(scope, entry, resolver)?
    } else if is_repeated {
        let elem = if field_type == Type::TYPE_BYTES {
            bytes_type.clone()
        } else {
            scalar_or_message_type_nested(ctx, field, current_package, nesting, features, resolver)?
        };
        {
            let vec = resolver.vec();
            quote! { #vec<#elem> }
        }
    } else if field_type == Type::TYPE_MESSAGE || field_type == Type::TYPE_GROUP {
        let inner = resolve_message_type(scope, field)?;
        {
            let mf = resolver.message_field();
            quote! { #mf<#inner> }
        }
    } else if is_optional {
        let inner = if field_type == Type::TYPE_ENUM {
            resolve_enum_type(scope, field, resolver)?
        } else if field_type == Type::TYPE_BYTES {
            bytes_type.clone()
        } else {
            scalar_rust_type(field_type, resolver)?
        };
        {
            let opt = resolver.option();
            quote! { #opt<#inner> }
        }
    } else if field_type == Type::TYPE_ENUM {
        resolve_enum_type(scope, field, resolver)?
    } else if field_type == Type::TYPE_BYTES {
        bytes_type
    } else {
        scalar_rust_type(field_type, resolver)?
    };

    // Self-referential struct fields (e.g. DescriptorProto.nested_type) can
    // use `Self` in the struct declaration. Only message-typed, non-map
    // fields qualify. `rust_type` stays concrete for the serde-deserialize
    // path — that codegen runs inside `impl Visitor for _V` where `Self`
    // means `_V`, not the message.
    let self_fqn = format!(".{proto_fqn}");
    let is_self_ref = field.type_name.as_deref() == Some(self_fqn.as_str()) && !is_map;
    let struct_field_type = if is_self_ref {
        if is_repeated {
            let vec = resolver.vec();
            quote! { #vec<Self> }
        } else {
            let mf = resolver.message_field();
            quote! { #mf<Self> }
        }
    } else {
        rust_type.clone()
    };

    let map_key_type = map_entry.and_then(|e| map_entry_key_type(ctx, e, features));
    let map_value_type = map_entry.and_then(|e| map_entry_value_type(ctx, e, features));

    // For enum-valued maps, resolve closedness via the MapEntry's value
    // field descriptor (TYPE_ENUM — resolve_field overlays the referenced
    // enum's enum_type). Matches what map_rust_type_from_entry →
    // resolve_enum_type does for the Rust type, so serde module selection
    // and Rust type agree even when a per-enum CLOSED override differs
    // from the file-level default (editions only).
    let map_value_enum_closed = if map_value_type == Some(Type::TYPE_ENUM) {
        map_entry
            .and_then(|e| e.field.iter().find(|f| f.number == Some(2)))
            .map(|val_fd| {
                let val_features = crate::features::resolve_field(ctx, val_fd, features);
                is_closed_enum(&val_features)
            })
    } else {
        None
    };

    Ok(FieldInfo {
        rust_type,
        struct_field_type,
        is_repeated,
        is_map,
        is_optional,
        is_required,
        map_key_type,
        map_value_type,
        map_value_enum_closed,
    })
}

/// Generate a single field declaration.
///
/// Returns `None` for fields that belong to a real oneof — those are
/// represented by the `Option<OneofEnum>` field added by `generate_message`.
/// Result of generating a single struct field: the field declaration tokens
/// and the field identifier (for use in the manual `Debug` impl).
struct GeneratedField {
    tokens: TokenStream,
    ident: Ident,
}

fn generate_field(
    scope: MessageScope<'_>,
    msg: &DescriptorProto,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    resolver: &crate::imports::ImportResolver,
) -> Result<Option<GeneratedField>, CodeGenError> {
    let MessageScope {
        ctx,
        proto_fqn,
        features,
        ..
    } = scope;
    let field_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let field_number = field.number.unwrap_or(0);

    // Real oneof fields are excluded from the struct body.
    if is_real_oneof_member(field) {
        return Ok(None);
    }

    let info = classify_field(scope, msg, field, resolver)?;
    let rust_name = make_field_ident(field_name);

    let field_fqn = format!("{}.{}", proto_fqn, field_name);
    let tag_line = format!("Field {field_number}: `{field_name}`");
    let doc = crate::comments::doc_attrs_with_tag(ctx.comment(&field_fqn), &tag_line);
    let serde_attr = if ctx.config.generate_json {
        serde_field_attr(ctx, field, field_name, &info, features)
    } else {
        quote! {}
    };
    let custom_field_attrs =
        CodeGenContext::matching_attributes(&ctx.config.field_attributes, &field_fqn)?;
    let rust_type = &info.struct_field_type;
    let tokens = quote! {
        #doc
        #serde_attr
        #custom_field_attrs
        pub #rust_name: #rust_type,
    };
    Ok(Some(GeneratedField {
        tokens,
        ident: rust_name,
    }))
}

pub(crate) fn is_map_field(
    msg: &DescriptorProto,
    field: &crate::generated::descriptor::FieldDescriptorProto,
) -> bool {
    field.r#type.unwrap_or_default() == Type::TYPE_MESSAGE && find_map_entry(msg, field).is_some()
}

/// Find the synthetic map-entry nested message for a map field.
///
/// Returns `None` if the field is not a map field (no matching nested type
/// with `map_entry = true`).  Used by all map-related helpers to avoid
/// duplicating the lookup predicate.
///
/// The match uses suffix comparison (`type_name.ends_with(".{name}")`)
/// rather than full FQN equality. This is safe because `msg.nested_type`
/// only contains types nested within this message, and protobuf does not
/// allow duplicate type names within a single message scope.
pub(crate) fn find_map_entry<'a>(
    msg: &'a DescriptorProto,
    field: &crate::generated::descriptor::FieldDescriptorProto,
) -> Option<&'a DescriptorProto> {
    let type_name = field.type_name.as_deref()?;
    msg.nested_type.iter().find(|nested| {
        nested
            .options
            .as_option()
            .and_then(|o| o.map_entry)
            .unwrap_or(false)
            && nested
                .name
                .as_deref()
                .is_some_and(|n| type_name.ends_with(&format!(".{n}")))
    })
}

/// Return the effective proto `Type` of a map entry's key field.
fn map_entry_key_type(
    ctx: &CodeGenContext,
    entry: &DescriptorProto,
    features: &ResolvedFeatures,
) -> Option<Type> {
    let key_field = entry.field.iter().find(|f| f.number == Some(1))?;
    Some(crate::impl_message::effective_type_in_map_entry(
        ctx, key_field, features,
    ))
}

/// Return the effective proto `Type` of a map entry's value field.
fn map_entry_value_type(
    ctx: &CodeGenContext,
    entry: &DescriptorProto,
    features: &ResolvedFeatures,
) -> Option<Type> {
    let value_field = entry.field.iter().find(|f| f.number == Some(2))?;
    Some(crate::impl_message::effective_type_in_map_entry(
        ctx,
        value_field,
        features,
    ))
}

/// Build the `HashMap<K, V>` Rust type from an already-resolved map entry descriptor.
fn map_rust_type_from_entry(
    scope: MessageScope<'_>,
    entry: &DescriptorProto,
    resolver: &crate::imports::ImportResolver,
) -> Result<TokenStream, CodeGenError> {
    let MessageScope {
        ctx,
        current_package,
        features,
        nesting,
        ..
    } = scope;
    let key_field = entry
        .field
        .iter()
        .find(|f| f.number == Some(1))
        .ok_or(CodeGenError::MissingField("map_entry.key"))?;
    let value_field = entry
        .field
        .iter()
        .find(|f| f.number == Some(2))
        .ok_or(CodeGenError::MissingField("map_entry.value"))?;

    let key_type = scalar_or_message_type_nested(
        ctx,
        key_field,
        current_package,
        nesting,
        features,
        resolver,
    )?;
    let value_type = scalar_or_message_type_nested(
        ctx,
        value_field,
        current_package,
        nesting,
        features,
        resolver,
    )?;

    let hm = resolver.hashmap();
    Ok(quote! { #hm<#key_type, #value_type> })
}

/// Resolve the Rust type for a scalar, message, or enum field.
///
/// `current_package` is used to produce unqualified names for types in the
/// same proto package (they will be in the same generated `pub mod`).
/// `nesting` is the module depth of the *consumer* of this type (every
/// `pub mod` step away from the package root adds one hop). Used by both
/// this module and `oneof.rs`.
pub(crate) fn scalar_or_message_type_nested(
    ctx: &CodeGenContext,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    current_package: &str,
    nesting: usize,
    features: &ResolvedFeatures,
    resolver: &crate::imports::ImportResolver,
) -> Result<TokenStream, CodeGenError> {
    let scope = MessageScope {
        ctx,
        current_package,
        proto_fqn: "",
        features,
        nesting,
    };
    match crate::impl_message::effective_type(ctx, field, features) {
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => resolve_message_type(scope, field),
        Type::TYPE_ENUM => resolve_enum_type(scope, field, resolver),
        other => scalar_rust_type(other, resolver),
    }
}

fn resolve_message_type(
    scope: MessageScope<'_>,
    field: &crate::generated::descriptor::FieldDescriptorProto,
) -> Result<TokenStream, CodeGenError> {
    let type_name = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.type_name"))?;
    let path_str = scope
        .ctx
        .rust_type_relative(type_name, scope.current_package, scope.nesting)
        .ok_or_else(|| {
            CodeGenError::Other(format!(
                "message type '{type_name}' not found in descriptor set; \
                 ensure all imports are included with --include_imports"
            ))
        })?;
    let ty = rust_path_to_tokens(&path_str);
    Ok(quote! { #ty })
}

fn resolve_enum_type(
    scope: MessageScope<'_>,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    resolver: &crate::imports::ImportResolver,
) -> Result<TokenStream, CodeGenError> {
    let type_name = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.type_name"))?;
    let path_str = scope
        .ctx
        .rust_type_relative(type_name, scope.current_package, scope.nesting)
        .ok_or_else(|| {
            CodeGenError::Other(format!(
                "enum type '{type_name}' not found in descriptor set; \
                 ensure all imports are included with --include_imports"
            ))
        })?;
    let ty = rust_path_to_tokens(&path_str);
    let field_features = crate::features::resolve_field(scope.ctx, field, scope.features);
    if is_closed_enum(&field_features) {
        Ok(quote! { #ty })
    } else {
        let ev = resolver.enum_value();
        Ok(quote! { #ev<#ty> })
    }
}

/// Returns `true` when `features.enum_type` is CLOSED.
///
/// **Important:** `enum_type` is a property of the ENUM DECLARATION, not the
/// field. For this to return the correct value, the caller must have already
/// resolved the enum's own features into the passed `features` — see
/// [`crate::features::resolve_field`] which does this automatically for
/// enum-typed fields by looking up the referenced enum's closedness.
pub(crate) fn is_closed_enum(features: &ResolvedFeatures) -> bool {
    features.enum_type == crate::features::EnumType::Closed
}

fn scalar_rust_type(
    t: Type,
    resolver: &crate::imports::ImportResolver,
) -> Result<TokenStream, CodeGenError> {
    match t {
        Type::TYPE_DOUBLE => Ok(quote! { f64 }),
        Type::TYPE_FLOAT => Ok(quote! { f32 }),
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => Ok(quote! { i64 }),
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => Ok(quote! { u64 }),
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => Ok(quote! { i32 }),
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => Ok(quote! { u32 }),
        Type::TYPE_BOOL => Ok(quote! { bool }),
        Type::TYPE_STRING => Ok(resolver.string()),
        Type::TYPE_BYTES => {
            let vec = resolver.vec();
            Ok(quote! { #vec<u8> })
        }
        Type::TYPE_GROUP | Type::TYPE_MESSAGE | Type::TYPE_ENUM => Err(CodeGenError::Other(
            format!("scalar_rust_type called for non-scalar type {:?}", t),
        )),
    }
}

/// Determine the `with` module and `null_as_default` deserializer for a
/// non-oneof field.  Shared between `serde_field_attr` (for derive-based
/// deserialization) and `generate_custom_deserialize` (for hand-generated
/// Deserialize impls on messages with oneofs).
fn field_deser_modules(
    field_type: Type,
    info: &FieldInfo,
    features: &ResolvedFeatures,
) -> (Option<&'static str>, Option<&'static str>) {
    let with_module = if info.is_map {
        map_serde_module(info)
    } else if info.is_repeated {
        repeated_serde_module(field_type, features)
    } else if info.is_optional {
        optional_serde_module(field_type, features)
    } else {
        singular_serde_module(field_type, features)
    };

    let null_deser = if with_module.is_none() && (info.is_repeated || info.is_map) {
        Some("::buffa::json_helpers::null_as_default")
    } else {
        None
    };

    (with_module, null_deser)
}

/// Does this scalar type need proto3-JSON special encoding in containers?
///
/// int64/uint64 → quoted strings; float/double → NaN/Inf tokens; bytes →
/// base64. For bool/string/int32/uint32/sint32/sfixed32/fixed32, derive
/// serde is already proto3-JSON compliant — routing through ProtoElemJson
/// adds trait-dispatch overhead (and for proto_map, a `.to_string()` alloc
/// per key) for no correctness benefit.
fn value_needs_proto_json(ty: Type) -> bool {
    matches!(
        ty,
        Type::TYPE_INT64
            | Type::TYPE_SINT64
            | Type::TYPE_SFIXED64
            | Type::TYPE_UINT64
            | Type::TYPE_FIXED64
            | Type::TYPE_FLOAT
            | Type::TYPE_DOUBLE
            | Type::TYPE_BYTES
    )
}

/// Serde module for map fields (keyed by key/value types).
///
/// Uses `proto_map` (generic over `V: ProtoElemJson`) only when the value
/// type needs proto3-JSON special encoding (int64→quoted, float→NaN token,
/// bytes→base64). For simple values (string, bool, 32-bit ints) with string
/// keys, returns `None` to use derive — zero overhead. Non-string keys still
/// use `string_key_map` for key stringification.
///
/// Open-enum map values keep `map_enum` for its ignore-unknown-values
/// filtering behavior (a `JsonParseOptions` feature proto_map doesn't have).
///
/// Key stringification: serde_json's `MapKeySerializer` auto-stringifies
/// all proto map key types (i32/i64/u32/u64/bool → `"42"`/`"true"`/etc.)
/// and parses them back, so `map_enum`/`map_closed_enum` delegating to
/// `HashMap`'s default serde is correct for non-string keys without an
/// explicit `string_key_map` wrapper.
fn map_serde_module(info: &FieldInfo) -> Option<&'static str> {
    // Bytes key (from strict_utf8_mapping normalizing string→bytes):
    // keys are base64-encoded, not Display-stringified. proto_map's
    // Display-based key serialization doesn't work here.
    if matches!(info.map_key_type, Some(Type::TYPE_BYTES)) {
        return Some(if matches!(info.map_value_type, Some(Type::TYPE_BYTES)) {
            "::buffa::json_helpers::bytes_key_bytes_val_map"
        } else {
            "::buffa::json_helpers::bytes_key_map"
        });
    }

    // Enum values (both open and closed) need the unknown-value filtering
    // behavior of map_enum / map_closed_enum (ignore_unknown_enum_values
    // option). proto_map lacks this.
    //
    // Closedness MUST come from info.map_value_enum_closed (resolved from
    // the MapEntry value field, which is TYPE_ENUM), NOT from the map
    // field's own features (TYPE_MESSAGE → resolve_field skips the
    // enum_type overlay → stale file-level default). See classify_field.
    if let Some(closed) = info.map_value_enum_closed {
        return Some(if closed {
            "::buffa::json_helpers::map_closed_enum"
        } else {
            "::buffa::json_helpers::map_enum"
        });
    }

    // Message values: derived Serialize/Deserialize is already proto-JSON.
    // Default serde for HashMap<String, Message> works. Non-string keys
    // still need stringification via string_key_map.
    if matches!(info.map_value_type, Some(Type::TYPE_MESSAGE)) {
        let is_string_key = matches!(info.map_key_type, Some(Type::TYPE_STRING));
        return if is_string_key {
            None
        } else {
            Some("::buffa::json_helpers::string_key_map")
        };
    }

    // Scalar value types: only route through proto_map if the value needs
    // proto-JSON encoding. For simple values with string keys, derive is
    // correct and avoids proto_map's per-key `.to_string()` allocation.
    let value_ty = info.map_value_type.unwrap_or(Type::TYPE_STRING);
    let is_string_key = matches!(info.map_key_type, Some(Type::TYPE_STRING));
    if value_needs_proto_json(value_ty) {
        // Value needs special encoding (int64 quoted, bytes base64, etc.).
        Some("::buffa::json_helpers::proto_map")
    } else if is_string_key {
        // String key + simple value: derive is proto-JSON compliant, zero overhead.
        None
    } else {
        // Non-string key + simple value: need key stringification only.
        Some("::buffa::json_helpers::string_key_map")
    }
}

/// Serde module for repeated fields.
///
/// Uses `proto_seq` (generic over `T: ProtoElemJson`) only for element types
/// that need proto3-JSON special encoding. For string/bool/32-bit ints,
/// derive is correct and avoids trait-dispatch overhead.
///
/// Enums keep the `_enum` / `_closed_enum` modules for their
/// ignore-unknown-values filtering behavior (JsonParseOptions).
fn repeated_serde_module(field_type: Type, features: &ResolvedFeatures) -> Option<&'static str> {
    match field_type {
        // Enums need ignore_unknown_enum_values filtering.
        Type::TYPE_ENUM => Some(if is_closed_enum(features) {
            "::buffa::json_helpers::repeated_closed_enum"
        } else {
            "::buffa::json_helpers::repeated_enum"
        }),
        // Messages/groups: derived Serialize is already proto-JSON.
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => None,
        // Simple scalar types (string, bool, 32-bit ints): derive is
        // proto-JSON compliant. Only route through proto_seq for types
        // that need special encoding (int64 quoted, bytes base64, etc.).
        ty if value_needs_proto_json(ty) => Some("::buffa::json_helpers::proto_seq"),
        _ => None,
    }
}

/// Serde module for explicit-presence (optional) fields.
fn optional_serde_module(field_type: Type, features: &ResolvedFeatures) -> Option<&'static str> {
    match field_type {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            Some("::buffa::json_helpers::opt_int32")
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => Some("::buffa::json_helpers::opt_uint32"),
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            Some("::buffa::json_helpers::opt_int64")
        }
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => Some("::buffa::json_helpers::opt_uint64"),
        Type::TYPE_FLOAT => Some("::buffa::json_helpers::opt_float"),
        Type::TYPE_DOUBLE => Some("::buffa::json_helpers::opt_double"),
        Type::TYPE_BYTES => Some("::buffa::json_helpers::opt_bytes"),
        Type::TYPE_ENUM => Some(if is_closed_enum(features) {
            "::buffa::json_helpers::opt_closed_enum"
        } else {
            "::buffa::json_helpers::opt_enum"
        }),
        _ => None,
    }
}

/// Serde module for singular (non-optional, non-repeated) fields.
fn singular_serde_module(field_type: Type, features: &ResolvedFeatures) -> Option<&'static str> {
    match field_type {
        Type::TYPE_BOOL => Some("::buffa::json_helpers::proto_bool"),
        Type::TYPE_STRING => Some("::buffa::json_helpers::proto_string"),
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            Some("::buffa::json_helpers::int32")
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => Some("::buffa::json_helpers::uint32"),
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            Some("::buffa::json_helpers::int64")
        }
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => Some("::buffa::json_helpers::uint64"),
        Type::TYPE_FLOAT => Some("::buffa::json_helpers::float"),
        Type::TYPE_DOUBLE => Some("::buffa::json_helpers::double"),
        Type::TYPE_BYTES => Some("::buffa::json_helpers::bytes"),
        Type::TYPE_ENUM => Some(if is_closed_enum(features) {
            "::buffa::json_helpers::closed_enum"
        } else {
            "::buffa::json_helpers::proto_enum"
        }),
        _ => None,
    }
}

/// Determine the `skip_serializing_if` predicate for a field.
fn skip_serializing_predicate(
    field_type: Type,
    info: &FieldInfo,
    features: &ResolvedFeatures,
) -> Option<&'static str> {
    if info.is_required {
        // Proto2 required fields must always be present in JSON, even at
        // their default value — mirrors the binary encoder's always-encode
        // semantics (impl_message.rs is_proto2_required check).
        None
    } else if info.is_map {
        Some("::buffa::__private::HashMap::is_empty")
    } else if info.is_repeated {
        Some("::buffa::json_helpers::skip_if::is_empty_vec")
    } else if info.is_optional {
        Some("Option::is_none")
    } else {
        singular_skip_predicate(field_type, features)
    }
}

/// Determine the `skip_serializing_if` predicate for a singular field.
fn singular_skip_predicate(field_type: Type, features: &ResolvedFeatures) -> Option<&'static str> {
    match field_type {
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            Some("::buffa::json_helpers::skip_if::is_unset_message_field")
        }
        Type::TYPE_ENUM => Some(if is_closed_enum(features) {
            "::buffa::json_helpers::skip_if::is_default_closed_enum"
        } else {
            "::buffa::json_helpers::skip_if::is_default_enum_value"
        }),
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            Some("::buffa::json_helpers::skip_if::is_zero_i64")
        }
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => {
            Some("::buffa::json_helpers::skip_if::is_zero_u64")
        }
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            Some("::buffa::json_helpers::skip_if::is_zero_i32")
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => {
            Some("::buffa::json_helpers::skip_if::is_zero_u32")
        }
        Type::TYPE_BOOL => Some("::buffa::json_helpers::skip_if::is_false"),
        Type::TYPE_FLOAT => Some("::buffa::json_helpers::skip_if::is_zero_f32"),
        Type::TYPE_DOUBLE => Some("::buffa::json_helpers::skip_if::is_zero_f64"),
        Type::TYPE_STRING => Some("::buffa::json_helpers::skip_if::is_empty_str"),
        Type::TYPE_BYTES => Some("::buffa::json_helpers::skip_if::is_empty_bytes"),
    }
}

/// Build a `#[serde(...)]` attribute for a direct (non-oneof) field.
///
/// Emits `rename` using the proto JSON name, `skip_serializing_if` for
/// default-value suppression (proto3 JSON omits fields at their default),
/// and `with` for types that require special proto JSON encoding (int64,
/// uint64, float, double, bytes).
/// Repeated, map, and optional wrappers dispatch to container-specific
/// helper modules that handle per-element encoding.
fn serde_field_attr(
    ctx: &CodeGenContext,
    field: &crate::generated::descriptor::FieldDescriptorProto,
    field_name: &str,
    info: &FieldInfo,
    features: &ResolvedFeatures,
) -> TokenStream {
    let field_type = crate::impl_message::effective_type(ctx, field, features);
    let field_features = crate::features::resolve_field(ctx, field, features);
    let json_name = field.json_name.as_deref().unwrap_or(field_name);
    let (with_module, null_deser) = field_deser_modules(field_type, info, &field_features);

    let skip_if = skip_serializing_predicate(field_type, info, &field_features);

    // Proto3 JSON spec: parsers must accept both the camelCase json_name
    // and the original proto field name.  Emit `alias` when they differ.
    let needs_alias = json_name != field_name;

    // Build the attribute parts list to avoid a combinatorial match.
    let alias_part = if needs_alias {
        quote! { , alias = #field_name }
    } else {
        quote! {}
    };
    let with_part = if let Some(module) = with_module {
        quote! { , with = #module }
    } else {
        quote! {}
    };
    let skip_part = if let Some(skip) = skip_if {
        quote! { , skip_serializing_if = #skip }
    } else {
        quote! {}
    };
    let deser_part = if is_value_field(field, info.is_repeated, info.is_map) {
        quote! { , deserialize_with = "::buffa::json_helpers::message_field_always_present" }
    } else if let Some(deser) = null_deser {
        quote! { , deserialize_with = #deser }
    } else {
        quote! {}
    };

    quote! {
        #[serde(rename = #json_name #alias_part #with_part #skip_part #deser_part)]
    }
}

/// Generate a custom `impl Default` for a message when any non-optional field
/// has a custom `default_value`.
///
/// Returns `Some(impl_block)` if a custom default is needed, `None` otherwise
/// (in which case the struct should `#[derive(Default)]`).
fn generate_custom_default(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    name_ident: &Ident,
    current_package: &str,
    features: &ResolvedFeatures,
    nesting: usize,
) -> Result<Option<TokenStream>, CodeGenError> {
    // Custom defaults only apply when field presence is explicit (proto2,
    // or editions with explicit presence).
    if features.field_presence != crate::features::FieldPresence::Explicit {
        return Ok(None);
    }

    // First pass: check if any field has a custom default that matters.
    let mut has_custom = false;
    for field in &msg.field {
        if is_real_oneof_member(field) {
            continue;
        }
        let field_type = crate::impl_message::effective_type(ctx, field, features);
        let is_optional = is_explicit_presence_scalar(field, field_type, features);
        let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;
        if is_optional
            || is_repeated
            || field_type == Type::TYPE_MESSAGE
            || field_type == Type::TYPE_GROUP
        {
            continue;
        }
        if field
            .default_value
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        {
            has_custom = true;
            break;
        }
    }

    if !has_custom {
        return Ok(None);
    }

    // Second pass: build field initializers.
    let mut field_inits = Vec::new();

    for field in &msg.field {
        if is_real_oneof_member(field) {
            continue;
        }
        let field_name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let field_ident = make_field_ident(field_name);
        let field_type = crate::impl_message::effective_type(ctx, field, features);
        let is_optional = is_explicit_presence_scalar(field, field_type, features);
        let is_repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;

        if is_optional
            || is_repeated
            || field_type == Type::TYPE_MESSAGE
            || field_type == Type::TYPE_GROUP
        {
            field_inits.push(quote! { #field_ident: ::core::default::Default::default(), });
            continue;
        }

        if let Some(expr) = parse_default_value(field, ctx, current_package, features, nesting)? {
            field_inits.push(quote! { #field_ident: #expr, });
        } else {
            field_inits.push(quote! { #field_ident: ::core::default::Default::default(), });
        }
    }

    // Oneof fields default to None.
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let oneof_name = oneof
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("oneof.name"))?;
        let has_real = msg
            .field
            .iter()
            .any(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32));
        if has_real {
            let ident = make_field_ident(oneof_name);
            field_inits.push(quote! { #ident: ::core::default::Default::default(), });
        }
    }

    let unknown_fields_init = if ctx.config.preserve_unknown_fields {
        quote! { __buffa_unknown_fields: ::core::default::Default::default(), }
    } else {
        quote! {}
    };

    Ok(Some(quote! {
        impl ::core::default::Default for #name_ident {
            fn default() -> Self {
                Self {
                    #(#field_inits)*
                    #unknown_fields_init
                    __buffa_cached_size: ::core::default::Default::default(),
                }
            }
        }
    }))
}

// Ident/path helpers re-exported from the public `idents` module so existing
// `crate::message::*` imports continue to work unchanged.
pub(crate) use crate::idents::{make_field_ident, rust_path_to_tokens};

//! Oneof enum code generation.

use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, OneofDescriptorProto};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::impl_message::field_uses_bytes;
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

/// Metadata for a single oneof variant.
struct VariantInfo {
    variant_ident: proc_macro2::Ident,
    /// When `bytes_fields` config matches a bytes variant this is
    /// `::bytes::Bytes`, not `Vec<u8>` — see `collect_variant_info`.
    rust_type: TokenStream,
    json_name: String,
    field_type: Type,
    /// See [`is_null_value_field`].
    is_null_value: bool,
    /// True for message/group types (boxed in the owned enum).
    is_boxed: bool,
    /// Custom attributes matched via `CodeGenConfig::field_attributes` on the
    /// variant's fully-qualified path (`{oneof_fqn}.{variant_proto_name}`).
    custom_attrs: TokenStream,
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
            let rust_type =
                if field_type == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, proto_name) {
                    quote! { ::bytes::Bytes }
                } else {
                    scalar_or_message_type_nested(
                        ctx,
                        field,
                        current_package,
                        nesting + 1,
                        features,
                        resolver,
                    )?
                };
            let variant_fqn = format!("{proto_fqn}.{oneof_name}.{proto_name}");
            let custom_attrs =
                CodeGenContext::matching_attributes(&ctx.config.field_attributes, &variant_fqn)?;
            Ok(VariantInfo {
                variant_ident,
                rust_type,
                json_name,
                field_type,
                is_boxed: is_boxed_variant(field_type),
                is_null_value: is_null_value_field(field),
                custom_attrs,
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
            if v.is_boxed {
                quote! { #attrs #ident(::buffa::alloc::boxed::Box<#ty>) }
            } else {
                quote! { #attrs #ident(#ty) }
            }
        })
        .collect();

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
    for v in variants_info.iter().filter(|v| v.is_boxed) {
        *type_counts.entry(v.rust_type.to_string()).or_insert(0) += 1;
    }
    let from_impls: Vec<_> = variants_info
        .iter()
        .filter(|v| v.is_boxed && type_counts[&v.rust_type.to_string()] == 1)
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
            // From<T> for Oneof — always legal (Oneof is local in T0 position).
            let from_oneof = quote! {
                impl From<#ty> for #rust_enum_ident {
                    fn from(v: #ty) -> Self {
                        Self::#ident(::buffa::alloc::boxed::Box::new(v))
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
            quote! { #from_oneof #from_option }
        })
        .collect();

    let serde_impls = if ctx.config.generate_json {
        generate_oneof_serialize(&rust_enum_ident, &variants_info)
    } else {
        quote! {}
    };
    let arbitrary_derive = if ctx.config.generate_arbitrary {
        quote! { #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))] }
    } else {
        quote! {}
    };

    let oneof_fqn = format!("{}.{}", proto_fqn, oneof_name);
    let oneof_doc = crate::comments::doc_attrs(ctx.comment(&oneof_fqn));
    let custom_type_attrs =
        CodeGenContext::matching_attributes(&ctx.config.type_attributes, &oneof_fqn)?;

    Ok(quote! {
        #oneof_doc
        #[derive(Clone, PartialEq, Debug)]
        #arbitrary_derive
        #custom_type_attrs
        pub enum #rust_enum_ident {
            #(#variants,)*
        }

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

            if v.is_null_value {
                // NullValue must serialize as JSON `null`, not "NULL_VALUE".
                // `&()` serializes as JSON `null` via serde_json.
                return quote! {
                    #enum_ident::#ident(_) => {
                        map.serialize_entry(#json_name, &())?;
                    }
                };
            }

            let rust_type = &v.rust_type;
            if let Some(helper) = serde_helper_path(v.field_type) {
                // Type needs special proto JSON encoding — wrap in a newtype
                // that delegates to the helper's serialize function.
                quote! {
                    #enum_ident::#ident(v) => {
                        struct _W<'a>(&'a #rust_type);
                        impl serde::Serialize for _W<'_> {
                            fn serialize<S2: serde::Serializer>(&self, s: S2) -> ::core::result::Result<S2::Ok, S2::Error> {
                                #helper::serialize(self.0, s)
                            }
                        }
                        map.serialize_entry(#json_name, &_W(v))?;
                    }
                }
            } else {
                quote! {
                    #enum_ident::#ident(v) => {
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
    /// True for message/group types (boxed in the owned enum).
    pub is_boxed: bool,
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
pub(crate) fn oneof_variant_deser_arm(input: &OneofVariantDeserInput<'_>) -> TokenStream {
    let OneofVariantDeserInput {
        variant_ident,
        variant_type,
        json_name,
        proto_name,
        field_type,
        null_forward,
        is_boxed,
        enum_ident,
        result_var,
        oneof_name,
    } = input;
    let dup_err_msg = format!("multiple oneof fields set for '{oneof_name}'");
    // For boxed variants, the deserialized inner value must be wrapped.
    let wrapped_v = if *is_boxed {
        quote! { ::buffa::alloc::boxed::Box::new(v) }
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
                let v: Option<#variant_type> = map.next_value_seed(
                    ::buffa::json_helpers::NullableDeserializeSeed(_DeserSeed)
                )?;
            }
        } else {
            quote! {
                let v: Option<#variant_type> = map.next_value_seed(
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
        quote! {
            #json_name => {
                #deser
                #set_result
            }
        }
    } else {
        quote! {
            #json_name | #proto_name => {
                #deser
                #set_result
            }
        }
    }
}

/// Collect the names already claimed in a message's Rust module that a
/// oneof enum must not collide with: nested message names, nested enum
/// names, and — when view generation is enabled — each nested message's
/// `{name}View` struct (emitted in the same module).
fn reserved_names_for_msg(
    msg: &DescriptorProto,
    generate_views: bool,
) -> std::collections::HashSet<String> {
    let mut reserved = std::collections::HashSet::new();
    for nested in &msg.nested_type {
        if let Some(name) = &nested.name {
            reserved.insert(name.clone());
            if generate_views {
                reserved.insert(format!("{name}View"));
            }
        }
    }
    for nested_enum in &msg.enum_type {
        if let Some(name) = &nested_enum.name {
            reserved.insert(name.clone());
        }
    }
    reserved
}

/// Build the Rust identifier for a oneof enum.
///
/// With module-based nesting the enum lives inside the owning message's
/// module (`pub mod msg_name { pub enum FooOneof { ... } }`), so no
/// message prefix is needed. The enum name is always
/// `{PascalCase(oneof_name)}Oneof` regardless of whether siblings would
/// collide — uniform naming makes the generated type discoverable from
/// the `.proto` alone and prevents source-breaking renames when nested
/// types are added later.
///
/// # Errors
///
/// Returns [`CodeGenError::OneofNameConflict`] when a nested type or a
/// prior oneof in the same message already claims the suffixed name
/// (e.g. a nested message literally named `FooOneof` alongside
/// `oneof foo`). Users resolve these by renaming in the `.proto`.
fn oneof_enum_ident(
    oneof_name: &str,
    reserved: &std::collections::HashSet<String>,
    views_enabled: bool,
    scope: &str,
) -> Result<proc_macro2::Ident, CodeGenError> {
    let pascal = to_pascal_case(oneof_name);
    let name = format!("{pascal}Oneof");
    if reserved.contains(&name) || (views_enabled && reserved.contains(&format!("{name}View"))) {
        return Err(CodeGenError::OneofNameConflict {
            scope: scope.to_string(),
            oneof_name: oneof_name.to_string(),
            attempted: name,
        });
    }
    Ok(format_ident!("{}", name))
}

/// Compute oneof enum identifiers for all non-synthetic oneofs in a message.
///
/// Every oneof enum is named `{PascalCase(oneof_name)}Oneof`; the reserved
/// set is grown after each allocation so two sibling oneofs cannot both
/// claim the same name (which could happen if the user declared e.g.
/// `oneof foo` alongside `oneof foo` — disallowed by protoc — or via a
/// hand-crafted descriptor).
///
/// `scope` is the parent message's fully-qualified proto name, used only
/// in error diagnostics. `generate_views` must match
/// [`CodeGenContext::config.generate_views`](crate::context::CodeGenContext);
/// when true, nested `{n}View` names are added to the reserved set so the
/// view-side oneof enum (`{Name}OneofView`) also avoids collisions.
///
/// Returns a map from oneof declaration index to its Rust enum `Ident`.
/// Synthetic oneofs (proto3 `optional`) are omitted.
///
/// # Errors
///
/// Propagates [`CodeGenError::OneofNameConflict`] from
/// [`oneof_enum_ident`].
pub(crate) fn resolve_oneof_idents(
    msg: &DescriptorProto,
    scope: &str,
    generate_views: bool,
) -> Result<std::collections::HashMap<usize, Ident>, CodeGenError> {
    let mut reserved = reserved_names_for_msg(msg, generate_views);
    let mut result = std::collections::HashMap::new();
    for (idx, oneof) in msg.oneof_decl.iter().enumerate() {
        let has_real_fields = msg.field.iter().any(|f| {
            crate::impl_message::is_real_oneof_member(f) && f.oneof_index == Some(idx as i32)
        });
        if !has_real_fields {
            continue;
        }
        if let Some(oneof_name) = &oneof.name {
            let ident = oneof_enum_ident(oneof_name, &reserved, generate_views, scope)?;
            let owned = ident.to_string();
            if generate_views {
                reserved.insert(format!("{owned}View"));
            }
            reserved.insert(owned);
            result.insert(idx, ident);
        }
    }
    Ok(result)
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
        result.push(c.to_lowercase().next().unwrap());
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

//! Codegen for `extend` declarations — emits `pub const` extension descriptors.
//!
//! `extend google.protobuf.FieldOptions { optional Foo my_opt = 50001; }`
//! produces:
//!
//! ```ignore
//! /// Extension `my_opt` on `.google.protobuf.FieldOptions` (field 50001).
//! pub const MY_OPT: ::buffa::Extension<::buffa::extension::codecs::MessageCodec<Foo>>
//!     = ::buffa::Extension::new(50001);
//! ```

use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::FieldDescriptorProto;
use crate::idents::rust_path_to_tokens;
use crate::CodeGenError;

/// Generate `pub const` extension descriptors for a list of `extend` declarations.
///
/// `nesting` follows the same convention as [`crate::context::CodeGenContext::rust_type_relative`]:
/// `0` for file-level `extend` blocks, `1` for `extend` nested inside a message
/// (the const lands in that message's `pub mod`).
///
/// `scope_fqn` is the proto FQN prefix for building extension JSON full names
/// (the package for file-level, `<package>.<Message>` for message-nested).
///
/// Returns the generated tokens plus two lists of registry const identifiers
/// emitted in the same scope: JSON entries (all types except `TYPE_GROUP`)
/// and text entries (message/group only).
pub(crate) fn generate_extensions(
    ctx: &CodeGenContext,
    extensions: &[FieldDescriptorProto],
    current_package: &str,
    nesting: usize,
    features: &ResolvedFeatures,
    scope_fqn: &str,
) -> Result<(TokenStream, Vec<Ident>, Vec<Ident>), CodeGenError> {
    let mut out = TokenStream::new();
    let mut json_consts = Vec::new();
    let mut text_consts = Vec::new();
    for ext in extensions {
        if let Some((tokens, json_id, text_id)) =
            generate_one(ctx, ext, current_package, nesting, features, scope_fqn)?
        {
            out.extend(tokens);
            if let Some(id) = json_id {
                json_consts.push(id);
            }
            if let Some(id) = text_id {
                text_consts.push(id);
            }
        }
    }
    Ok((out, json_consts, text_consts))
}

#[allow(clippy::type_complexity)]
fn generate_one(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    current_package: &str,
    nesting: usize,
    features: &ResolvedFeatures,
    scope_fqn: &str,
) -> Result<Option<(TokenStream, Option<Ident>, Option<Ident>)>, CodeGenError> {
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("extension.name"))?;
    let number = crate::impl_message::validated_field_number(field)?;
    let ty = crate::impl_message::effective_type(ctx, field, features);
    let extendee = field.extendee.as_deref().unwrap_or("<unknown>");
    // Strip leading dot for both the runtime extendee check (PROTO_FQN is
    // emitted without a dot too) and the JSON registry entry.
    let extendee_no_dot = extendee.strip_prefix('.').unwrap_or(extendee);
    let repeated = field.label.unwrap_or_default() == Label::LABEL_REPEATED;
    let const_ident = extension_const_ident(proto_name);

    let inner_codec = codec_for_type(ctx, field, ty, current_package, nesting)?;

    let codec = if repeated {
        if crate::impl_message::is_field_packed(field, features) {
            quote! { ::buffa::extension::codecs::PackedRepeated<#inner_codec> }
        } else {
            quote! { ::buffa::extension::codecs::Repeated<#inner_codec> }
        }
    } else {
        inner_codec
    };

    // Proto2 `[default = ...]`. The proto spec only allows defaults on singular
    // scalar/enum fields; protoc rejects message/group/repeated at parse time.
    // We check defensively because a hand-crafted descriptor could slip through.
    let default_fn = if field
        .default_value
        .as_deref()
        .filter(|s| !s.is_empty())
        .is_some()
    {
        if repeated {
            return Err(CodeGenError::Other(format!(
                "extension `{proto_name}`: repeated fields cannot have a default value"
            )));
        }
        if matches!(ty, Type::TYPE_MESSAGE | Type::TYPE_GROUP) {
            return Err(CodeGenError::Other(format!(
                "extension `{proto_name}`: message/group fields cannot have a default value"
            )));
        }
        Some(default_fn_tokens(
            ctx,
            field,
            ty,
            proto_name,
            &const_ident,
            current_package,
            nesting,
            features,
        )?)
    } else {
        None
    };

    // The doc comment includes the extendee and proto name so users can
    // grep from the proto source to the generated const. For enum
    // extensions, also name the proto enum type (the codec is just
    // `EnumI32`, which loses that information).
    let encoding_note = if ty == Type::TYPE_GROUP {
        ", group-encoded"
    } else {
        ""
    };
    let mut doc =
        format!("Extension `{proto_name}` on `{extendee}` (field {number}{encoding_note}).");
    if ty == Type::TYPE_ENUM {
        if let Some(enum_name) = field.type_name.as_deref() {
            doc.push_str(&format!(
                "\n\nProto enum type: `{enum_name}`. Cast via `EnumValue::from_i32`."
            ));
        }
    }

    // Registry entries are feature-split into two separate consts. JSON
    // entries cover every type except TYPE_GROUP (proto3 JSON form
    // undefined — groups are a proto2-only legacy construct). Text entries
    // cover message/group only (the conformance-exercised `[pkg.ext] { ... }`
    // form). The two consts are independent: a group gets a text entry but
    // no JSON entry; a scalar gets a JSON entry but no text entry.
    let full_name = if scope_fqn.is_empty() {
        proto_name.to_owned()
    } else {
        format!("{scope_fqn}.{proto_name}")
    };

    let (json_const, json_ident) = if ctx.config.generate_json {
        match json_helper_tokens(ctx, field, ty, repeated, current_package, nesting)? {
            Some((to_fn, from_fn)) => {
                let ident = format_ident!("__{}_JSON_EXT", const_ident);
                let tokens = quote! {
                    #[doc(hidden)]
                    pub const #ident: ::buffa::type_registry::JsonExtEntry
                        = ::buffa::type_registry::JsonExtEntry {
                            number: #number,
                            full_name: #full_name,
                            extendee: #extendee_no_dot,
                            to_json: #to_fn,
                            from_json: #from_fn,
                        };
                };
                (tokens, Some(ident))
            }
            None => (quote! {}, None),
        }
    } else {
        (quote! {}, None)
    };

    let (text_const, text_ident) = if ctx.config.generate_text {
        match text_helper_tokens(ctx, field, ty, current_package, nesting)? {
            Some((te, tm)) => {
                let ident = format_ident!("__{}_TEXT_EXT", const_ident);
                let tokens = quote! {
                    #[doc(hidden)]
                    pub const #ident: ::buffa::type_registry::TextExtEntry
                        = ::buffa::type_registry::TextExtEntry {
                            number: #number,
                            full_name: #full_name,
                            extendee: #extendee_no_dot,
                            text_encode: #te,
                            text_merge: #tm,
                        };
                };
                (tokens, Some(ident))
            }
            None => (quote! {}, None),
        }
    } else {
        (quote! {}, None)
    };

    let (default_fn_def, ext_const) = match default_fn {
        Some((fn_ident, fn_def)) => (
            fn_def,
            quote! {
                pub const #const_ident: ::buffa::Extension<#codec>
                    = ::buffa::Extension::with_default(#number, #extendee_no_dot, #fn_ident);
            },
        ),
        None => (
            quote! {},
            quote! {
                pub const #const_ident: ::buffa::Extension<#codec>
                    = ::buffa::Extension::new(#number, #extendee_no_dot);
            },
        ),
    };

    Ok(Some((
        quote! {
            #default_fn_def
            #[doc = #doc]
            #ext_const
            #json_const
            #text_const
        },
        json_ident,
        text_ident,
    )))
}

/// Build the `#[doc(hidden)] fn __<name>_default() -> T { ... }` that backs
/// `Extension::with_default`. Returns the fn ident and its definition tokens.
///
/// Scalars and enums get a `const fn`; `string`/`bytes` get a regular `fn`
/// (their default expressions allocate). The enum case resolves the variant
/// path locally (with correct `nesting`) instead of going through
/// `parse_default_value`, because the `EnumI32` codec's `Value` is `i32`,
/// not the Rust enum type.
#[allow(clippy::too_many_arguments)]
fn default_fn_tokens(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    ty: Type,
    proto_name: &str,
    const_ident: &Ident,
    current_package: &str,
    nesting: usize,
    features: &ResolvedFeatures,
) -> Result<(Ident, TokenStream), CodeGenError> {
    let fn_ident = format_ident!("__{}_default", const_ident.to_string().to_lowercase());

    // Enum: `parse_default_value` would emit `Color::RED`, but the EnumI32
    // codec wants `i32`. Resolve the enum path here (with correct nesting)
    // and cast the variant. Buffa enums are `#[repr(i32)]` so `as i32` is
    // a direct bit-cast of the discriminant.
    if ty == Type::TYPE_ENUM {
        let enum_path = resolve_type_path(ctx, field, current_package, nesting, "enum")?;
        // default_value for enums is the proto variant name, e.g. "RED".
        // Same ident escaping as `enumeration.rs` uses for variant names.
        let variant =
            crate::idents::make_field_ident(field.default_value.as_deref().unwrap_or_default());
        return Ok((
            fn_ident.clone(),
            quote! {
                #[doc(hidden)]
                const fn #fn_ident() -> i32 {
                    #enum_path::#variant as i32
                }
            },
        ));
    }

    let value_ty = codec_value_type(ty);
    let default_expr =
        crate::defaults::parse_default_value(field, ctx, current_package, features, nesting)?
            .ok_or_else(|| {
                // default_value was non-empty but parse returned None —
                // happens when field_presence ≠ Explicit (shouldn't for
                // extensions, which are always explicit-presence per
                // protocolbuffers/protobuf#8234) or for an unhandled type.
                CodeGenError::Other(format!(
                    "extension `{proto_name}`: could not parse default_value `{}` for type {ty:?}",
                    field.default_value.as_deref().unwrap_or_default()
                ))
            })?;

    // String::from / vec![...] allocate → can't be const. Everything else
    // (integer/float/bool literals) is const-evaluable.
    let const_kw = if matches!(ty, Type::TYPE_STRING | Type::TYPE_BYTES) {
        quote! {}
    } else {
        quote! { const }
    };

    Ok((
        fn_ident.clone(),
        quote! {
            #[doc(hidden)]
            #const_kw fn #fn_ident() -> #value_ty {
                #default_expr
            }
        },
    ))
}

/// Map a proto scalar/enum type to its `ExtensionCodec::Value` Rust type.
///
/// Only called for types that can carry a `[default = ...]` — message/group
/// are rejected upstream and never reach here.
fn codec_value_type(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => quote! { i32 },
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => quote! { i64 },
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { u32 },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { u64 },
        Type::TYPE_BOOL => quote! { bool },
        Type::TYPE_FLOAT => quote! { f32 },
        Type::TYPE_DOUBLE => quote! { f64 },
        Type::TYPE_ENUM => quote! { i32 },
        Type::TYPE_STRING => quote! { ::buffa::alloc::string::String },
        Type::TYPE_BYTES => quote! { ::buffa::alloc::vec::Vec<u8> },
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            unreachable!("message/group defaults rejected before codec_value_type")
        }
    }
}

/// Map a `(proto type, repeated)` pair to its
/// `::buffa::extension_registry::helpers::*` function-path token pair.
///
/// Returns `None` for `TYPE_GROUP` (JSON form undefined; groups are a
/// proto2-only legacy construct that doesn't appear in real custom options).
/// For `TYPE_ENUM` / `TYPE_MESSAGE`, resolves the Rust type path so the
/// generic helper can be monomorphized to a concrete `fn` pointer.
fn json_helper_tokens(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    ty: Type,
    repeated: bool,
    current_package: &str,
    nesting: usize,
) -> Result<Option<(TokenStream, TokenStream)>, CodeGenError> {
    let h = |name: &str| {
        let ident = format_ident!("{}", name);
        quote! { ::buffa::extension_registry::helpers::#ident }
    };
    // Scalar cases: pick the helper name by type, prefix with `repeated_`
    // when applicable. The helper signature is identical across all scalars,
    // so codegen just points the registry entry's fn pointer at it.
    let (to, from): (&str, &str) = match ty {
        Type::TYPE_INT32 => ("int32_to_json", "int32_from_json"),
        Type::TYPE_INT64 => ("int64_to_json", "int64_from_json"),
        Type::TYPE_UINT32 => ("uint32_to_json", "uint32_from_json"),
        Type::TYPE_UINT64 => ("uint64_to_json", "uint64_from_json"),
        Type::TYPE_SINT32 => ("sint32_to_json", "sint32_from_json"),
        Type::TYPE_SINT64 => ("sint64_to_json", "sint64_from_json"),
        Type::TYPE_FIXED32 => ("fixed32_to_json", "fixed32_from_json"),
        Type::TYPE_FIXED64 => ("fixed64_to_json", "fixed64_from_json"),
        Type::TYPE_SFIXED32 => ("sfixed32_to_json", "sfixed32_from_json"),
        Type::TYPE_SFIXED64 => ("sfixed64_to_json", "sfixed64_from_json"),
        Type::TYPE_BOOL => ("bool_to_json", "bool_from_json"),
        Type::TYPE_STRING => ("string_to_json", "string_from_json"),
        Type::TYPE_BYTES => ("bytes_to_json", "bytes_from_json"),
        Type::TYPE_FLOAT => ("float_to_json", "float_from_json"),
        Type::TYPE_DOUBLE => ("double_to_json", "double_from_json"),
        Type::TYPE_ENUM => {
            // The enum helper is generic: resolve the Rust enum type so the
            // monomorphized `enum_to_json::<E>` coerces to a concrete fn ptr.
            let enum_ty = resolve_type_path(ctx, field, current_package, nesting, "enum")?;
            let (to, from) = if repeated {
                (h("repeated_enum_to_json"), h("repeated_enum_from_json"))
            } else {
                (h("enum_to_json"), h("enum_from_json"))
            };
            return Ok(Some((
                quote! { #to::<#enum_ty> },
                quote! { #from::<#enum_ty> },
            )));
        }
        Type::TYPE_MESSAGE => {
            let msg_ty = resolve_type_path(ctx, field, current_package, nesting, "message")?;
            let (to, from) = if repeated {
                (
                    h("repeated_message_to_json"),
                    h("repeated_message_from_json"),
                )
            } else {
                (h("message_to_json"), h("message_from_json"))
            };
            return Ok(Some((
                quote! { #to::<#msg_ty> },
                quote! { #from::<#msg_ty> },
            )));
        }
        Type::TYPE_GROUP => return Ok(None),
    };
    Ok(Some(if repeated {
        (h(&format!("repeated_{to}")), h(&format!("repeated_{from}")))
    } else {
        (h(to), h(from))
    }))
}

/// Map a message/group extension to its `type_registry::*_{encode,merge}_text<M>`
/// function-path token pair. Returns `None` for scalars and enums — textproto
/// extension support currently covers only the `[pkg.ext] { ... }` form that
/// conformance exercises.
fn text_helper_tokens(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    ty: Type,
    current_package: &str,
    nesting: usize,
) -> Result<Option<(TokenStream, TokenStream)>, CodeGenError> {
    let h = quote! { ::buffa::type_registry };
    match ty {
        Type::TYPE_MESSAGE => {
            let msg_ty = resolve_type_path(ctx, field, current_package, nesting, "message")?;
            Ok(Some((
                quote! { #h::message_encode_text::<#msg_ty> },
                quote! { #h::message_merge_text::<#msg_ty> },
            )))
        }
        Type::TYPE_GROUP => {
            let msg_ty = resolve_type_path(ctx, field, current_package, nesting, "group")?;
            Ok(Some((
                quote! { #h::group_encode_text::<#msg_ty> },
                quote! { #h::group_merge_text::<#msg_ty> },
            )))
        }
        _ => Ok(None),
    }
}

/// Resolve `field.type_name` (a `.pkg.Type` proto FQN) to a Rust type path
/// token stream via [`CodeGenContext::rust_type_relative`]. Shared by the
/// enum and message arms of [`json_helper_tokens`].
fn resolve_type_path(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    current_package: &str,
    nesting: usize,
    kind: &str,
) -> Result<TokenStream, CodeGenError> {
    let type_name = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("extension.type_name"))?;
    let path_str = ctx
        .rust_type_relative(type_name, current_package, nesting)
        .ok_or_else(|| {
            CodeGenError::Other(format!(
                "extension {kind} type '{type_name}' not found in descriptor set"
            ))
        })?;
    Ok(rust_path_to_tokens(&path_str))
}

/// Map a proto scalar/message type to its `::buffa::extension::codecs::*` codec path.
fn codec_for_type(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    ty: Type,
    current_package: &str,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let c = |name: &str| {
        let ident = format_ident!("{}", name);
        quote! { ::buffa::extension::codecs::#ident }
    };
    Ok(match ty {
        Type::TYPE_INT32 => c("Int32"),
        Type::TYPE_INT64 => c("Int64"),
        Type::TYPE_UINT32 => c("Uint32"),
        Type::TYPE_UINT64 => c("Uint64"),
        Type::TYPE_SINT32 => c("Sint32"),
        Type::TYPE_SINT64 => c("Sint64"),
        Type::TYPE_BOOL => c("Bool"),
        Type::TYPE_ENUM => c("EnumI32"),
        Type::TYPE_FIXED32 => c("Fixed32"),
        Type::TYPE_FIXED64 => c("Fixed64"),
        Type::TYPE_SFIXED32 => c("Sfixed32"),
        Type::TYPE_SFIXED64 => c("Sfixed64"),
        Type::TYPE_FLOAT => c("Float"),
        Type::TYPE_DOUBLE => c("Double"),
        Type::TYPE_STRING => c("StringCodec"),
        Type::TYPE_BYTES => c("BytesCodec"),
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            // Same type resolution for both — the value is a message type,
            // only the wire encoding (length-delimited vs group-framed)
            // differs. The codec picked below encapsulates that.
            let type_name = field
                .type_name
                .as_deref()
                .ok_or(CodeGenError::MissingField("extension.type_name"))?;
            let path_str = ctx
                .rust_type_relative(type_name, current_package, nesting)
                .ok_or_else(|| {
                    CodeGenError::Other(format!(
                        "extension message type '{type_name}' not found in descriptor set"
                    ))
                })?;
            let msg_ty = rust_path_to_tokens(&path_str);
            if ty == Type::TYPE_GROUP {
                quote! { ::buffa::extension::codecs::GroupCodec<#msg_ty> }
            } else {
                quote! { ::buffa::extension::codecs::MessageCodec<#msg_ty> }
            }
        }
    })
}

/// `field_info` → `FIELD_INFO`.
///
/// Protobuf extension field names are already `lower_snake_case` by convention
/// (enforced by most linters), so uppercasing is sufficient. Pass through
/// `to_snake_case` first to handle oddball camelCase names.
fn extension_const_ident(proto_name: &str) -> proc_macro2::Ident {
    let upper = crate::oneof::to_snake_case(proto_name).to_uppercase();
    crate::idents::make_field_ident(&upper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::ResolvedFeatures;
    use crate::generated::descriptor::{
        DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    };
    use crate::CodeGenConfig;

    fn ext_field(name: &str, number: i32, ty: Type) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            number: Some(number),
            r#type: Some(ty),
            label: Some(Label::LABEL_OPTIONAL),
            extendee: Some(".google.protobuf.FieldOptions".to_string()),
            ..Default::default()
        }
    }

    /// Run `generate_one` against an empty context with `generate_json` off.
    /// Suitable for tests that don't resolve message types (scalar/enum/group).
    fn gen(field: &FieldDescriptorProto) -> Option<String> {
        gen_with(field, false).map(|(t, _)| t)
    }

    /// Returns `(tokens, json_ident)` — text is off in these helpers, so
    /// `text_ident` is dropped.
    fn gen_with(field: &FieldDescriptorProto, json: bool) -> Option<(String, Option<String>)> {
        let files: [FileDescriptorProto; 0] = [];
        let config = CodeGenConfig {
            generate_json: json,
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &[]);
        let features = ResolvedFeatures::proto2_defaults();
        generate_one(&ctx, field, "", 0, &features, "my.pkg")
            .unwrap()
            .map(|(t, json, _text)| (t.to_string(), json.map(|i| i.to_string())))
    }

    #[test]
    fn const_ident_conversion() {
        assert_eq!(
            extension_const_ident("field_info").to_string(),
            "FIELD_INFO"
        );
        assert_eq!(extension_const_ident("http").to_string(), "HTTP");
        assert_eq!(extension_const_ident("myOption").to_string(), "MY_OPTION");
    }

    #[test]
    fn scalar_extension_tokens() {
        let tokens = gen(&ext_field("weight", 50002, Type::TYPE_SINT32)).unwrap();
        assert!(tokens.contains("WEIGHT"), "{tokens}");
        assert!(tokens.contains("Sint32"), "{tokens}");
        assert!(tokens.contains("50002"), "{tokens}");
        assert!(!tokens.contains("Repeated"), "{tokens}");
    }

    #[test]
    fn repeated_string_extension_tokens() {
        let mut field = ext_field("tags", 50003, Type::TYPE_STRING);
        field.label = Some(Label::LABEL_REPEATED);
        let tokens = gen(&field).unwrap();
        assert!(tokens.contains("TAGS"), "{tokens}");
        assert!(tokens.contains("Repeated"), "{tokens}");
        assert!(tokens.contains("StringCodec"), "{tokens}");
        // String is not packable — no PackedRepeated.
        assert!(!tokens.contains("PackedRepeated"), "{tokens}");
    }

    #[test]
    fn repeated_int32_proto2_unpacked_by_default() {
        let mut field = ext_field("nums", 50004, Type::TYPE_INT32);
        field.label = Some(Label::LABEL_REPEATED);
        let tokens = gen(&field).unwrap();
        // Proto2: unpacked by default. Tokens stringify with a space between
        // `Repeated` and `<`.
        assert!(tokens.contains("Repeated <"), "{tokens}");
        assert!(!tokens.contains("PackedRepeated"), "{tokens}");
    }

    #[test]
    fn repeated_int32_explicit_packed() {
        use crate::generated::descriptor::FieldOptions;
        let mut field = ext_field("nums", 50004, Type::TYPE_INT32);
        field.label = Some(Label::LABEL_REPEATED);
        field.options = FieldOptions {
            packed: Some(true),
            ..Default::default()
        }
        .into();
        let tokens = gen(&field).unwrap();
        assert!(tokens.contains("PackedRepeated"), "{tokens}");
    }

    /// Build a `CodeGenContext` containing message `Ann` and enum `Color` in
    /// package `my.pkg`, then run `generate_one` for an extension in that
    /// package. Needed for the enum/message JSON paths, which resolve
    /// `field.type_name` against the context's type map.
    fn gen_in_pkg(field: &FieldDescriptorProto) -> Option<(String, Option<String>)> {
        let files = [FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("my.pkg".into()),
            message_type: vec![DescriptorProto {
                name: Some("Ann".into()),
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("Color".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let config = CodeGenConfig {
            generate_json: true,
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &[]);
        let features = ResolvedFeatures::proto2_defaults();
        generate_one(&ctx, field, "my.pkg", 0, &features, "my.pkg")
            .unwrap()
            .map(|(t, json, _text)| (t.to_string(), json.map(|i| i.to_string())))
    }

    #[test]
    fn group_extension_no_json_helper() {
        // TYPE_GROUP extensions share TYPE_MESSAGE's codec resolution path
        // (integration test `group_extension_codec_type` in buffa-test covers
        // that). Here verify the JSON-registry exclusion: no helper for groups
        // — their JSON form is undefined and they don't appear in real custom
        // options (groups are proto2-only legacy).
        let files: [FileDescriptorProto; 0] = [];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &[]);
        let field = ext_field("g", 1, Type::TYPE_GROUP);
        let got = json_helper_tokens(&ctx, &field, Type::TYPE_GROUP, false, "", 0).unwrap();
        assert!(got.is_none());
        let got = json_helper_tokens(&ctx, &field, Type::TYPE_GROUP, true, "", 0).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn enum_extension_doc_mentions_type() {
        let mut field = ext_field("level", 50006, Type::TYPE_ENUM);
        field.type_name = Some(".my.pkg.LogLevel".to_string());
        let tokens = gen(&field).unwrap();
        assert!(tokens.contains("EnumI32"), "{tokens}");
        assert!(tokens.contains("my.pkg.LogLevel"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_scalar() {
        let (tokens, json_ident) =
            gen_with(&ext_field("weight", 50002, Type::TYPE_SINT32), true).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__WEIGHT_JSON_EXT"));
        assert!(tokens.contains("JsonExtEntry"), "{tokens}");
        assert!(tokens.contains("sint32_to_json"), "{tokens}");
        assert!(tokens.contains("sint32_from_json"), "{tokens}");
        assert!(tokens.contains("\"my.pkg.weight\""), "{tokens}");
        assert!(
            tokens.contains("\"google.protobuf.FieldOptions\""),
            "{tokens}"
        );
        // Binary Extension const still present.
        assert!(tokens.contains("Sint32"), "{tokens}");
        // Scalar extensions don't get text entries (message/group only).
        assert!(!tokens.contains("TextExtEntry"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_repeated_scalar() {
        let mut field = ext_field("tags", 50003, Type::TYPE_STRING);
        field.label = Some(Label::LABEL_REPEATED);
        let (tokens, json_ident) = gen_with(&field, true).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__TAGS_JSON_EXT"));
        assert!(tokens.contains("repeated_string_to_json"), "{tokens}");
        assert!(tokens.contains("repeated_string_from_json"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_repeated_int64() {
        // int64 is the interesting case: elements stringify in JSON.
        let mut field = ext_field("nums", 50004, Type::TYPE_INT64);
        field.label = Some(Label::LABEL_REPEATED);
        let (tokens, json_ident) = gen_with(&field, true).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__NUMS_JSON_EXT"));
        assert!(tokens.contains("repeated_int64_to_json"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_enum() {
        let mut field = ext_field("color", 50005, Type::TYPE_ENUM);
        field.type_name = Some(".my.pkg.Color".to_string());
        let (tokens, json_ident) = gen_in_pkg(&field).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__COLOR_JSON_EXT"));
        // Tokens stringify with spaces around `::<` — match loosely.
        assert!(tokens.contains("enum_to_json"), "{tokens}");
        assert!(tokens.contains("enum_from_json"), "{tokens}");
        assert!(tokens.contains("Color"), "{tokens}");
        assert!(!tokens.contains("repeated_enum"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_repeated_enum() {
        let mut field = ext_field("colors", 50006, Type::TYPE_ENUM);
        field.type_name = Some(".my.pkg.Color".to_string());
        field.label = Some(Label::LABEL_REPEATED);
        let (tokens, json_ident) = gen_in_pkg(&field).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__COLORS_JSON_EXT"));
        assert!(tokens.contains("repeated_enum_to_json"), "{tokens}");
        assert!(tokens.contains("repeated_enum_from_json"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_message() {
        let mut field = ext_field("ann", 50007, Type::TYPE_MESSAGE);
        field.type_name = Some(".my.pkg.Ann".to_string());
        let (tokens, json_ident) = gen_in_pkg(&field).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__ANN_JSON_EXT"));
        assert!(tokens.contains("message_to_json"), "{tokens}");
        assert!(tokens.contains("message_from_json"), "{tokens}");
        assert!(tokens.contains("Ann"), "{tokens}");
        assert!(!tokens.contains("repeated_message"), "{tokens}");
    }

    #[test]
    fn json_const_emitted_for_repeated_message() {
        let mut field = ext_field("anns", 50008, Type::TYPE_MESSAGE);
        field.type_name = Some(".my.pkg.Ann".to_string());
        field.label = Some(Label::LABEL_REPEATED);
        let (tokens, json_ident) = gen_in_pkg(&field).unwrap();
        assert_eq!(json_ident.as_deref(), Some("__ANNS_JSON_EXT"));
        assert!(tokens.contains("repeated_message_to_json"), "{tokens}");
        assert!(tokens.contains("repeated_message_from_json"), "{tokens}");
    }

    #[test]
    fn json_const_skipped_when_json_off() {
        let (tokens, json_ident) =
            gen_with(&ext_field("weight", 50002, Type::TYPE_SINT32), false).unwrap();
        assert!(json_ident.is_none());
        assert!(!tokens.contains("JsonExtEntry"), "{tokens}");
    }

    #[test]
    fn text_const_emitted_for_message_independent_of_json() {
        // text on, json OFF — message extension gets a TextExtEntry but no
        // JsonExtEntry. This is the decoupling that the feature-split enables.
        let files = [FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("my.pkg".into()),
            message_type: vec![DescriptorProto {
                name: Some("Ann".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let config = CodeGenConfig {
            generate_text: true,
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &[]);
        let features = ResolvedFeatures::proto2_defaults();
        let mut field = ext_field("ann", 50007, Type::TYPE_MESSAGE);
        field.type_name = Some(".my.pkg.Ann".to_string());

        let (tokens, json_id, text_id) =
            generate_one(&ctx, &field, "my.pkg", 0, &features, "my.pkg")
                .unwrap()
                .unwrap();
        let tokens = tokens.to_string();
        assert!(json_id.is_none());
        assert_eq!(
            text_id.map(|i| i.to_string()).as_deref(),
            Some("__ANN_TEXT_EXT")
        );
        assert!(tokens.contains("TextExtEntry"), "{tokens}");
        assert!(tokens.contains("message_encode_text"), "{tokens}");
        assert!(tokens.contains("message_merge_text"), "{tokens}");
        assert!(!tokens.contains("JsonExtEntry"), "{tokens}");
    }

    #[test]
    fn json_helpers_cover_all_scalars() {
        // All scalar types produce a JSON const (both singular and repeated).
        let scalars = [
            Type::TYPE_INT32,
            Type::TYPE_INT64,
            Type::TYPE_UINT32,
            Type::TYPE_UINT64,
            Type::TYPE_SINT32,
            Type::TYPE_SINT64,
            Type::TYPE_FIXED32,
            Type::TYPE_FIXED64,
            Type::TYPE_SFIXED32,
            Type::TYPE_SFIXED64,
            Type::TYPE_BOOL,
            Type::TYPE_STRING,
            Type::TYPE_BYTES,
            Type::TYPE_FLOAT,
            Type::TYPE_DOUBLE,
        ];
        for ty in scalars {
            let (_, id) = gen_with(&ext_field("x", 1, ty), true).unwrap();
            assert!(id.is_some(), "singular {ty:?}");
            let mut f = ext_field("x", 1, ty);
            f.label = Some(Label::LABEL_REPEATED);
            let (tokens, id) = gen_with(&f, true).unwrap();
            assert!(id.is_some(), "repeated {ty:?}");
            assert!(tokens.contains("repeated_"), "repeated {ty:?}: {tokens}");
        }
    }

    // ── [default = ...] on extensions ───────────────────────────────────────

    fn ext_with_default(name: &str, ty: Type, default: &str) -> FieldDescriptorProto {
        let mut f = ext_field(name, 50030, ty);
        f.default_value = Some(default.into());
        f
    }

    /// Defensive rejects: protoc already blocks these at parse time, but
    /// a hand-crafted descriptor could carry them.
    #[test]
    fn default_rejected_on_repeated_and_message() {
        let files: [FileDescriptorProto; 0] = [];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &[]);
        let features = ResolvedFeatures::proto2_defaults();

        let mut f = ext_with_default("x", Type::TYPE_INT32, "7");
        f.label = Some(Label::LABEL_REPEATED);
        let err = generate_one(&ctx, &f, "", 0, &features, "").unwrap_err();
        assert!(format!("{err:?}").contains("repeated"));

        // Message case needs a resolvable type_name so codec_for_type
        // succeeds and the default-value check is the one that fires.
        let mut f = ext_with_default("x", Type::TYPE_MESSAGE, "?");
        f.type_name = Some(".my.pkg.Ann".into());
        let files = [FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("my.pkg".into()),
            message_type: vec![DescriptorProto {
                name: Some("Ann".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let ctx = CodeGenContext::new(&files, &config, &[]);
        let err = generate_one(&ctx, &f, "my.pkg", 0, &features, "my.pkg").unwrap_err();
        assert!(format!("{err:?}").contains("message/group"));
    }

    #[test]
    fn default_int32_emits_const_fn_and_with_default() {
        let f = ext_with_default("priority", Type::TYPE_INT32, "7");
        let tokens = gen(&f).unwrap();
        assert!(tokens.contains("with_default"), "{tokens}");
        assert!(tokens.contains("__priority_default"), "{tokens}");
        assert!(tokens.contains("const fn __priority_default"), "{tokens}");
        assert!(tokens.contains("-> i32"), "{tokens}");
        assert!(tokens.contains("7i32"), "{tokens}");
        assert!(tokens.contains("PRIORITY"), "{tokens}");
    }

    #[test]
    fn default_string_emits_non_const_fn() {
        let f = ext_with_default("label", Type::TYPE_STRING, "none");
        let tokens = gen(&f).unwrap();
        assert!(tokens.contains("with_default"), "{tokens}");
        // Non-const: `fn` not immediately preceded by `const`. Token
        // stringification puts a space between tokens so match loosely.
        assert!(tokens.contains("fn __label_default"), "{tokens}");
        assert!(!tokens.contains("const fn __label_default"), "{tokens}");
        assert!(
            tokens.contains(":: buffa :: alloc :: string :: String"),
            "{tokens}"
        );
    }

    #[test]
    fn default_bytes_emits_non_const_fn() {
        let f = ext_with_default("blob", Type::TYPE_BYTES, r"\xDE\xAD");
        let tokens = gen(&f).unwrap();
        assert!(tokens.contains("with_default"), "{tokens}");
        assert!(!tokens.contains("const fn __blob_default"), "{tokens}");
        assert!(tokens.contains("Vec < u8 >"), "{tokens}");
        // Byte literals from unescape.
        assert!(tokens.contains("222u8"), "{tokens}"); // 0xDE
        assert!(tokens.contains("173u8"), "{tokens}"); // 0xAD
    }

    #[test]
    fn default_bool_emits_const_fn() {
        let f = ext_with_default("active", Type::TYPE_BOOL, "true");
        let tokens = gen(&f).unwrap();
        assert!(tokens.contains("const fn __active_default"), "{tokens}");
        assert!(tokens.contains("-> bool"), "{tokens}");
        assert!(tokens.contains("true"), "{tokens}");
    }

    #[test]
    fn default_float_emits_const_fn() {
        let f = ext_with_default("ratio", Type::TYPE_FLOAT, "1.5");
        let tokens = gen(&f).unwrap();
        assert!(tokens.contains("const fn __ratio_default"), "{tokens}");
        assert!(tokens.contains("-> f32"), "{tokens}");
    }

    #[test]
    fn default_enum_resolves_path_with_nesting() {
        // `gen_in_pkg` puts the context at nesting=0 in package `my.pkg` with
        // enum `Color`. Verify the enum path resolves and is cast to i32.
        let mut f = ext_field("color", 50030, Type::TYPE_ENUM);
        f.type_name = Some(".my.pkg.Color".into());
        f.default_value = Some("RED".into());
        let (tokens, _) = gen_in_pkg(&f).unwrap();
        assert!(tokens.contains("with_default"), "{tokens}");
        assert!(tokens.contains("const fn __color_default"), "{tokens}");
        assert!(tokens.contains("-> i32"), "{tokens}");
        assert!(tokens.contains("Color :: RED as i32"), "{tokens}");
    }

    #[test]
    fn default_value_absent_emits_plain_new() {
        let f = ext_field("weight", 50002, Type::TYPE_SINT32);
        let tokens = gen(&f).unwrap();
        assert!(tokens.contains(":: new"), "{tokens}");
        assert!(!tokens.contains("with_default"), "{tokens}");
        assert!(!tokens.contains("__weight_default"), "{tokens}");
    }

    #[test]
    fn default_value_empty_string_treated_as_absent() {
        // An empty `default_value` is semantically absent (protoc sets it to
        // "" rather than omitting the field in some paths).
        let mut f = ext_field("weight", 50002, Type::TYPE_SINT32);
        f.default_value = Some(String::new());
        let tokens = gen(&f).unwrap();
        assert!(!tokens.contains("with_default"), "{tokens}");
    }

    #[test]
    fn codec_value_type_covers_all_scalars() {
        // Sanity: every type that can carry a default has a value-type mapping
        // whose stringification matches what the default-fn return type needs.
        let cases = [
            (Type::TYPE_INT32, "i32"),
            (Type::TYPE_SINT32, "i32"),
            (Type::TYPE_SFIXED32, "i32"),
            (Type::TYPE_INT64, "i64"),
            (Type::TYPE_SINT64, "i64"),
            (Type::TYPE_SFIXED64, "i64"),
            (Type::TYPE_UINT32, "u32"),
            (Type::TYPE_FIXED32, "u32"),
            (Type::TYPE_UINT64, "u64"),
            (Type::TYPE_FIXED64, "u64"),
            (Type::TYPE_BOOL, "bool"),
            (Type::TYPE_FLOAT, "f32"),
            (Type::TYPE_DOUBLE, "f64"),
            (Type::TYPE_ENUM, "i32"),
        ];
        for (ty, expected) in cases {
            assert_eq!(codec_value_type(ty).to_string(), expected, "{ty:?}");
        }
        assert!(codec_value_type(Type::TYPE_STRING)
            .to_string()
            .contains("String"));
        assert!(codec_value_type(Type::TYPE_BYTES)
            .to_string()
            .contains("Vec < u8 >"));
    }
}

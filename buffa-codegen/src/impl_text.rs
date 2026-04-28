//! Code generation for `impl TextFormat`.
//!
//! Emits `encode_text` and `merge_text` implementations that call into the
//! `buffa::text::{TextEncoder, TextDecoder}` runtime. The generated shape
//! mirrors `impl_message.rs`: iterate scalar/repeated/oneof/map field groups,
//! emit a `quote!` block per field, assemble into the final impl.
//!
//! `encode_text` is a straight walk: for each set field, emit
//! `enc.write_field_name("x")?; enc.write_*(...)?;`. Presence is the same
//! as binary encode — implicit-presence scalars skip when zero/empty,
//! explicit-presence (`Option<T>`) skip when `None`, `MessageField<T>` skip
//! when `!is_set()`.
//!
//! `merge_text` is a `while let Some(name) = dec.read_field_name()?` loop
//! with a `match name { ... }` dispatch. The match key is the proto field
//! name as it appears in the `.proto` (not `json_name`, not the Rust ident).
//! Unknown names call `dec.skip_value()`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto};
use crate::idents::rust_path_to_tokens;
use crate::impl_message::{
    effective_type, effective_type_in_map_entry, field_uses_bytes, find_map_entry_fields,
    is_explicit_presence_scalar, is_non_default_expr, is_real_oneof_member, is_required_field,
    is_supported_field_type,
};
use crate::message::{is_closed_enum, is_map_field, make_field_ident};
use crate::oneof::is_boxed_variant;
use crate::CodeGenError;

/// Generate `impl ::buffa::text::TextFormat for #name_ident { ... }`.
///
/// Returns an empty `TokenStream` when `generate_text` is disabled, the
/// message is `google.protobuf.Any` (hand-written in `buffa-types` — consults
/// the type registry for `[type_url] { fields }` expansion), or the message
/// uses MessageSet wire format.
#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_text_impl(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    rust_name: &str,
    current_package: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    has_extension_ranges: bool,
    oneof_idents: &std::collections::HashMap<usize, proc_macro2::Ident>,
    oneof_prefix: &TokenStream,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    if !ctx.config.generate_text {
        return Ok(TokenStream::new());
    }

    // Any's textproto impl is hand-written in buffa-types/src/any_ext.rs —
    // it consults the global type registry for `[type_url] { fields }`
    // expansion, mirroring the hand-written serde impl. The generated
    // field-by-field impl would be wrong (it'd just do type_url/value
    // literally, skipping the expanded form).
    if proto_fqn == "google.protobuf.Any" {
        return Ok(TokenStream::new());
    }

    // MessageSet wire format: the message has no regular fields, only an
    // extension range. Textproto representation is all `[type.url]` brackets
    // which needs the extension registry (PR 4). Emit a stub so containing
    // messages still compile — encode writes nothing, decode skips. A
    // MessageSet field shows as `name {}` until the registry is wired.
    let is_message_set = msg
        .options
        .as_option()
        .and_then(|o| o.message_set_wire_format)
        .unwrap_or(false);
    if is_message_set {
        let name_ident = format_ident!("{}", rust_name);
        return Ok(quote! {
            impl ::buffa::text::TextFormat for #name_ident {
                fn encode_text(
                    &self,
                    _enc: &mut ::buffa::text::TextEncoder<'_>,
                ) -> ::core::fmt::Result {
                    ::core::result::Result::Ok(())
                }
                fn merge_text(
                    &mut self,
                    dec: &mut ::buffa::text::TextDecoder<'_>,
                ) -> ::core::result::Result<(), ::buffa::text::ParseError> {
                    while dec.read_field_name()?.is_some() {
                        dec.skip_value()?;
                    }
                    ::core::result::Result::Ok(())
                }
            }
        });
    }

    let name_ident = format_ident!("{}", rust_name);

    // ── field grouping (mirrors generate_message_impl) ──────────────────────

    let scalar_fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| {
            !is_real_oneof_member(f)
                && f.label.unwrap_or_default() != Label::LABEL_REPEATED
                && is_supported_field_type(f.r#type.unwrap_or_default())
        })
        .collect();

    let repeated_fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| {
            f.label.unwrap_or_default() == Label::LABEL_REPEATED
                && !is_map_field(msg, f)
                && is_supported_field_type(f.r#type.unwrap_or_default())
        })
        .collect();

    let map_fields: Vec<_> = msg
        .field
        .iter()
        .filter(|f| f.label.unwrap_or_default() == Label::LABEL_REPEATED && is_map_field(msg, f))
        .collect();

    let oneof_groups: Vec<(String, proc_macro2::Ident, Vec<&FieldDescriptorProto>)> = msg
        .oneof_decl
        .iter()
        .enumerate()
        .filter_map(|(idx, oneof)| {
            let enum_ident = oneof_idents.get(&idx)?;
            let fields: Vec<_> = msg
                .field
                .iter()
                .filter(|f| is_real_oneof_member(f) && f.oneof_index == Some(idx as i32))
                .collect();
            if fields.is_empty() {
                return None;
            }
            Some((
                oneof.name.as_deref()?.to_string(),
                enum_ident.clone(),
                fields,
            ))
        })
        .collect();

    // ── encode_text body ────────────────────────────────────────────────────

    let scalar_encode: Vec<_> = scalar_fields
        .iter()
        .map(|f| scalar_encode_stmt(ctx, f, features))
        .collect::<Result<_, _>>()?;
    let repeated_encode: Vec<_> = repeated_fields
        .iter()
        .map(|f| repeated_encode_stmt(ctx, f, features))
        .collect::<Result<_, _>>()?;
    let oneof_encode: Vec<_> = oneof_groups
        .iter()
        .map(|(name, enum_ident, fields)| {
            oneof_encode_stmt(ctx, enum_ident, name, fields, oneof_prefix, features)
        })
        .collect::<Result<_, _>>()?;
    let map_encode: Vec<_> = map_fields
        .iter()
        .map(|f| map_encode_stmt(ctx, msg, f, features))
        .collect::<Result<_, _>>()?;

    // ── merge_text arms ─────────────────────────────────────────────────────

    let scalar_merge: Vec<_> = scalar_fields
        .iter()
        .map(|f| scalar_merge_arm(ctx, f, current_package, proto_fqn, features, nesting))
        .collect::<Result<_, _>>()?;
    let repeated_merge: Vec<_> = repeated_fields
        .iter()
        .map(|f| repeated_merge_arm(ctx, f, current_package, proto_fqn, features, nesting))
        .collect::<Result<_, _>>()?;
    let mut oneof_merge: Vec<TokenStream> = Vec::new();
    for (name, enum_ident, fields) in &oneof_groups {
        oneof_merge.extend(oneof_merge_arms(
            ctx,
            enum_ident,
            name,
            fields,
            oneof_prefix,
            current_package,
            proto_fqn,
            features,
            nesting,
        )?);
    }
    let map_merge: Vec<_> = map_fields
        .iter()
        .map(|f| map_merge_arm(ctx, msg, f, current_package, features, nesting))
        .collect::<Result<_, _>>()?;

    // Extension bracket syntax `[pkg.ext] { ... }` — encode side writes
    // registered extensions from unknown fields; decode side consults the
    // text extension map installed via `set_type_registry`. The lookup
    // methods live under `buffa/text` (where this codegen path already is).
    // Gated on `has_extension_ranges`: protoc rejects `extend Foo { ... }`
    // when Foo has no `extensions N to M;` declaration, so a message
    // without one never has a matching registry entry.
    let use_ext_text = ctx.config.preserve_unknown_fields && has_extension_ranges;
    let proto_fqn_lit = proto_fqn;
    let (ext_encode, ext_merge_arm) = if use_ext_text {
        (
            quote! {
                enc.write_extension_fields(#proto_fqn_lit, &self.__buffa_unknown_fields)?;
            },
            quote! {
                __name if __name.starts_with('[') => {
                    for __r in dec.read_extension(__name, #proto_fqn_lit)? {
                        self.__buffa_unknown_fields.push(__r);
                    }
                }
            },
        )
    } else {
        (quote! {}, quote! {})
    };

    // Unknown-field printing is a no-op by default (the encoder checks its
    // `emit_unknown` flag), so it's fine to call unconditionally. Deref
    // coercion handles the JSON `__<Name>ExtJson` wrapper when generate_json
    // is also on.
    let unknown_encode = if ctx.config.preserve_unknown_fields {
        quote! { enc.write_unknown_fields(&self.__buffa_unknown_fields)?; }
    } else {
        quote! {}
    };

    // Unused-variable suppression when the message has no fields at all
    // AND no unknown-fields storage.
    let has_encode = !scalar_encode.is_empty()
        || !repeated_encode.is_empty()
        || !oneof_encode.is_empty()
        || !map_encode.is_empty()
        || ctx.config.preserve_unknown_fields;
    let enc_param = if has_encode {
        quote! { enc }
    } else {
        quote! { _enc }
    };

    Ok(quote! {
        impl ::buffa::text::TextFormat for #name_ident {
            fn encode_text(
                &self,
                #enc_param: &mut ::buffa::text::TextEncoder<'_>,
            ) -> ::core::fmt::Result {
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                #(#scalar_encode)*
                #(#repeated_encode)*
                #(#oneof_encode)*
                #(#map_encode)*
                #ext_encode
                #unknown_encode
                ::core::result::Result::Ok(())
            }

            fn merge_text(
                &mut self,
                dec: &mut ::buffa::text::TextDecoder<'_>,
            ) -> ::core::result::Result<(), ::buffa::text::ParseError> {
                #[allow(unused_imports)]
                use ::buffa::Enumeration as _;
                while let ::core::option::Option::Some(__name) = dec.read_field_name()? {
                    match __name {
                        #(#scalar_merge)*
                        #(#repeated_merge)*
                        #(#oneof_merge)*
                        #(#map_merge)*
                        #ext_merge_arm
                        _ => dec.skip_value()?,
                    }
                }
                ::core::result::Result::Ok(())
            }
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-type write/read method mapping
// ─────────────────────────────────────────────────────────────────────────────

/// `enc.write_*(#val)?` for a scalar value expression.
///
/// `val` should already be by-ref/deref as appropriate for the type: pass
/// `*v` for Copy numerics, `v` for `&str` / `&[u8]` / `&M`. Enum and message
/// are handled by the callers (they need more structure than a single call).
fn write_call(ty: Type, val: &TokenStream) -> TokenStream {
    match ty {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            quote! { enc.write_i32(#val)?; }
        }
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            quote! { enc.write_i64(#val)?; }
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { enc.write_u32(#val)?; },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { enc.write_u64(#val)?; },
        Type::TYPE_FLOAT => quote! { enc.write_f32(#val)?; },
        Type::TYPE_DOUBLE => quote! { enc.write_f64(#val)?; },
        Type::TYPE_BOOL => quote! { enc.write_bool(#val)?; },
        Type::TYPE_STRING => quote! { enc.write_string(#val)?; },
        Type::TYPE_BYTES => quote! { enc.write_bytes(#val)?; },
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => quote! { enc.write_message(#val)?; },
        Type::TYPE_ENUM => {
            unreachable!("write_call: enum handled by caller (closed vs open split)")
        }
    }
}

/// `dec.read_*()?` returning the Rust scalar type.
///
/// String returns `Cow<str>` — caller wraps in `.into_owned()`. Bytes returns
/// `Vec<u8>` — caller wraps in `.into()` when targeting `bytes::Bytes`.
/// Enum and message are handled by callers.
fn read_call(ty: Type) -> TokenStream {
    match ty {
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => quote! { dec.read_i32()? },
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => quote! { dec.read_i64()? },
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { dec.read_u32()? },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { dec.read_u64()? },
        Type::TYPE_FLOAT => quote! { dec.read_f32()? },
        Type::TYPE_DOUBLE => quote! { dec.read_f64()? },
        Type::TYPE_BOOL => quote! { dec.read_bool()? },
        Type::TYPE_STRING => quote! { dec.read_string()? },
        Type::TYPE_BYTES => quote! { dec.read_bytes()? },
        Type::TYPE_ENUM | Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
            unreachable!("read_call: enum/message handled by caller")
        }
    }
}

/// True for Copy-scalar types where `*v` (deref) is needed to get the value
/// out of an iterator or Option binding.
fn is_copy_scalar(ty: Type) -> bool {
    !matches!(
        ty,
        Type::TYPE_STRING | Type::TYPE_BYTES | Type::TYPE_MESSAGE | Type::TYPE_GROUP
    )
}

/// Text-format encode name and decode match pattern for a field.
///
/// Proto2 `group Data = N { ... }` creates a field named `data` (protoc
/// lowercases the type name) but text format uses the TYPE NAME:
/// `Data { ... }`. This is the one place where textproto's identifier
/// diverges from the `.proto` field name.
///
/// Returns `(encode_name, decode_pattern)`. For group-like fields, encode
/// emits the type name and decode accepts both (`"Data" | "data"`) —
/// matching protobuf-go's `ByTextName` which indexes the type name and its
/// lowercase (`internal/filedesc/desc_list_gen.go`). For everything else
/// both are the plain field name.
///
/// A field is group-like when its effective type is `TYPE_GROUP` and the
/// field name equals the lowercased simple name of its type. Proto2 `group`
/// syntax always satisfies this (protoc enforces it). Editions DELIMITED
/// fields satisfy it only when the user chose matching names — as the
/// editions-proto2 conformance golden does to mirror proto2 semantics —
/// so a generic `SomeMsg foo = 1 [DELIMITED]` keeps its field name.
fn text_field_name(
    proto_name: &str,
    field: &FieldDescriptorProto,
    ty: Type,
) -> (String, TokenStream) {
    if ty == Type::TYPE_GROUP {
        if let Some(type_name) = field.type_name.as_deref() {
            // `.pkg.Parent.GroupType` → `GroupType`
            let simple = type_name.rsplit('.').next().unwrap_or(type_name);
            // `simple != proto_name` guards against all-lowercase type names
            // (possible in editions) producing a duplicate `"x" | "x"` pattern.
            if simple != proto_name && simple.to_ascii_lowercase() == proto_name {
                return (simple.to_string(), quote! { #simple | #proto_name });
            }
        }
    }
    (proto_name.to_string(), quote! { #proto_name })
}

/// Write an enum value: named variant when known, numeric fallback when not.
///
/// `val` is a reference expression (e.g. `&self.status` or `__v` where `__v`
/// is already a reference from a `for` / `if let` binding). For closed enums
/// the underlying type is the bare `E` so `proto_name()` is always available.
/// For open enums it's `EnumValue<E>`, so match Known/Unknown.
fn enum_write(closed: bool, val: &TokenStream) -> TokenStream {
    if closed {
        // Autoref on method call: `proto_name` takes `&self`, so both
        // `self.field.proto_name()` and `__v.proto_name()` (where `__v: &E`)
        // work without explicit `&`, avoiding clippy::needless_borrow.
        quote! { enc.write_enum_name(#val.proto_name())?; }
    } else {
        quote! {
            match #val {
                ::buffa::EnumValue::Known(__e) => enc.write_enum_name(__e.proto_name())?,
                ::buffa::EnumValue::Unknown(__n) => enc.write_enum_number(*__n)?,
            }
        }
    }
}

/// Resolve a field's enum type path relative to the message impl scope.
/// The impl block sits at the struct's own depth (`nesting`); a nested
/// message referencing a cross-package enum therefore needs the same
/// number of `super::` hops as its containing struct.
fn enum_type_path(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    current_package: &str,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let type_name = field
        .type_name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.type_name"))?;
    let path = ctx
        .rust_type_relative(type_name, current_package, nesting)
        .ok_or_else(|| CodeGenError::Other(format!("enum type '{type_name}' not found")))?;
    Ok(rust_path_to_tokens(&path))
}

/// Read an enum value; produces a `Result<FieldType, ParseError>` expression.
///
/// Closed enums call `read_closed_enum_by_name::<E>` → `Result<E, _>`
/// directly — unknown numeric values are a parse error per the proto2 text
/// format spec. Open enums call `read_enum_by_name::<E>` → `Result<i32, _>`
/// and `.map(EnumValue::from)` into `Result<EnumValue<E>, _>`.
///
/// `dec` is the decoder ident — `dec` for outer-loop scalars, `__d` inside
/// `read_repeated_into` / `merge_map_entry` closures. Callers add `?` when
/// they need the value; `read_repeated_into` closures pass it through as-is.
fn enum_read(closed: bool, enum_ty: &TokenStream, dec: &proc_macro2::Ident) -> TokenStream {
    if closed {
        quote! { #dec.read_closed_enum_by_name::<#enum_ty>() }
    } else {
        quote! {
            #dec.read_enum_by_name::<#enum_ty>().map(::buffa::EnumValue::from)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scalar (singular, non-oneof) fields
// ─────────────────────────────────────────────────────────────────────────────

fn scalar_encode_stmt(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    parent_features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(proto_name);
    let ty = effective_type(ctx, field, features);
    let (name_lit, _) = text_field_name(proto_name, field, ty);
    let required = is_required_field(field, features);

    // Explicit-presence (Option<T>): emit when Some, regardless of value.
    if is_explicit_presence_scalar(field, ty, features) {
        let body = match ty {
            Type::TYPE_ENUM => {
                let closed = is_closed_enum(features);
                enum_write(closed, &quote! { __v })
            }
            _ if is_copy_scalar(ty) => write_call(ty, &quote! { *__v }),
            _ => write_call(ty, &quote! { __v }),
        };
        return Ok(quote! {
            if let ::core::option::Option::Some(ref __v) = self.#ident {
                enc.write_field_name(#name_lit)?;
                #body
            }
        });
    }

    // Singular message: MessageField<T>, skip when !is_set().
    if matches!(ty, Type::TYPE_MESSAGE | Type::TYPE_GROUP) {
        return Ok(quote! {
            if self.#ident.is_set() {
                enc.write_field_name(#name_lit)?;
                enc.write_message(&*self.#ident)?;
            }
        });
    }

    // Enum: closed → bare E, open → EnumValue<E>. Both expose to_i32()
    // via the Enumeration trait import. Closed passes the field directly
    // (autoref handles `&self` receiver); open passes `&` for the match.
    if ty == Type::TYPE_ENUM {
        let closed = is_closed_enum(features);
        let val = if closed {
            quote! { self.#ident }
        } else {
            quote! { &self.#ident }
        };
        let write = enum_write(closed, &val);
        return Ok(if required {
            quote! {
                enc.write_field_name(#name_lit)?;
                #write
            }
        } else {
            quote! {
                if self.#ident.to_i32() != 0 {
                    enc.write_field_name(#name_lit)?;
                    #write
                }
            }
        });
    }

    // String / bytes: skip when empty (implicit presence). Both `String`
    // and `Vec<u8>` / `bytes::Bytes` expose `.is_empty()` and deref to
    // `&str` / `&[u8]`.
    if matches!(ty, Type::TYPE_STRING | Type::TYPE_BYTES) {
        let write = write_call(ty, &quote! { &self.#ident });
        return Ok(if required {
            quote! {
                enc.write_field_name(#name_lit)?;
                #write
            }
        } else {
            quote! {
                if !self.#ident.is_empty() {
                    enc.write_field_name(#name_lit)?;
                    #write
                }
            }
        });
    }

    // Numeric scalar (implicit presence): skip when zero.
    let write = write_call(ty, &quote! { self.#ident });
    if required {
        return Ok(quote! {
            enc.write_field_name(#name_lit)?;
            #write
        });
    }
    let check = is_non_default_expr(ty, &ident);
    Ok(quote! {
        if #check {
            enc.write_field_name(#name_lit)?;
            #write
        }
    })
}

fn scalar_merge_arm(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    current_package: &str,
    proto_fqn: &str,
    parent_features: &ResolvedFeatures,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(proto_name);
    let ty = effective_type(ctx, field, features);
    let (_, name_pat) = text_field_name(proto_name, field, ty);
    let explicit = is_explicit_presence_scalar(field, ty, features);
    let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, proto_name);

    // Message: merge into existing (proto merge semantics).
    if matches!(ty, Type::TYPE_MESSAGE | Type::TYPE_GROUP) {
        return Ok(quote! {
            #name_pat => dec.merge_message(self.#ident.get_or_insert_default())?,
        });
    }

    // Enum.
    if ty == Type::TYPE_ENUM {
        let closed = is_closed_enum(features);
        let enum_ty = enum_type_path(ctx, field, current_package, nesting)?;
        let read = enum_read(closed, &enum_ty, &format_ident!("dec"));
        return Ok(if explicit {
            quote! { #name_pat => self.#ident = ::core::option::Option::Some(#read?), }
        } else {
            quote! { #name_pat => self.#ident = #read?, }
        });
    }

    // String: `read_string()` returns `Cow<str>`, need `.into_owned()`.
    // Bytes: `read_bytes()` returns `Vec<u8>`; `bytes::Bytes: From<Vec<u8>>`.
    let read = match ty {
        Type::TYPE_STRING => quote! { dec.read_string()?.into_owned() },
        Type::TYPE_BYTES if use_bytes => quote! { ::buffa::bytes::Bytes::from(dec.read_bytes()?) },
        _ => read_call(ty),
    };
    Ok(if explicit {
        quote! { #name_pat => self.#ident = ::core::option::Option::Some(#read), }
    } else {
        quote! { #name_pat => self.#ident = #read, }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Repeated fields
// ─────────────────────────────────────────────────────────────────────────────

fn repeated_encode_stmt(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    parent_features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(proto_name);
    let ty = effective_type(ctx, field, features);
    let (name_lit, _) = text_field_name(proto_name, field, ty);

    let body = match ty {
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(features);
            enum_write(closed, &quote! { __v })
        }
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => write_call(ty, &quote! { __v }),
        _ if is_copy_scalar(ty) => write_call(ty, &quote! { *__v }),
        _ => write_call(ty, &quote! { __v }),
    };
    Ok(quote! {
        for __v in &self.#ident {
            enc.write_field_name(#name_lit)?;
            #body
        }
    })
}

fn repeated_merge_arm(
    ctx: &CodeGenContext,
    field: &FieldDescriptorProto,
    current_package: &str,
    proto_fqn: &str,
    parent_features: &ResolvedFeatures,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let features = &crate::features::resolve_field(ctx, field, parent_features);
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(proto_name);
    let ty = effective_type(ctx, field, features);
    let (_, name_pat) = text_field_name(proto_name, field, ty);
    let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, proto_name);

    // read_repeated_into handles both `f: [a, b]` and `f: a` forms. The
    // closure takes `&mut TextDecoder` as `__d` (not `dec`, which is already
    // borrowed by the outer loop).
    let elem = match ty {
        Type::TYPE_MESSAGE | Type::TYPE_GROUP => quote! {
            {
                let mut __m = ::core::default::Default::default();
                __d.merge_message(&mut __m)?;
                ::core::result::Result::Ok(__m)
            }
        },
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(features);
            let enum_ty = enum_type_path(ctx, field, current_package, nesting)?;
            enum_read(closed, &enum_ty, &format_ident!("__d"))
        }
        Type::TYPE_STRING => {
            quote! { ::core::result::Result::Ok(__d.read_string()?.into_owned()) }
        }
        Type::TYPE_BYTES if use_bytes => {
            quote! { ::core::result::Result::Ok(::buffa::bytes::Bytes::from(__d.read_bytes()?)) }
        }
        Type::TYPE_BYTES => {
            quote! { __d.read_bytes() }
        }
        _ => {
            // Numeric: re-dispatch read_call but against __d.
            let call = match ty {
                Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
                    quote! { __d.read_i32() }
                }
                Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
                    quote! { __d.read_i64() }
                }
                Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { __d.read_u32() },
                Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { __d.read_u64() },
                Type::TYPE_FLOAT => quote! { __d.read_f32() },
                Type::TYPE_DOUBLE => quote! { __d.read_f64() },
                Type::TYPE_BOOL => quote! { __d.read_bool() },
                _ => unreachable!(),
            };
            quote! { #call }
        }
    };
    Ok(quote! {
        #name_pat => dec.read_repeated_into(&mut self.#ident, |__d| #elem)?,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Oneof fields
// ─────────────────────────────────────────────────────────────────────────────

fn oneof_encode_stmt(
    ctx: &CodeGenContext,
    enum_ident: &proc_macro2::Ident,
    oneof_name: &str,
    fields: &[&FieldDescriptorProto],
    oneof_prefix: &TokenStream,
    parent_features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let field_ident = make_field_ident(oneof_name);
    let qualified: TokenStream = quote! { #oneof_prefix #enum_ident };

    let mut arms: Vec<TokenStream> = Vec::new();
    for field in fields {
        let features = crate::features::resolve_field(ctx, field, parent_features);
        let proto_name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let variant = crate::oneof::oneof_variant_ident(proto_name);
        let ty = effective_type(ctx, field, &features);
        let (name_lit, _) = text_field_name(proto_name, field, ty);
        let boxed = is_boxed_variant(ty);

        // Box<M> auto-derefs through `&**__v` → `&M`. For string/bytes,
        // `__v: &String` / `&Vec<u8>` / `&bytes::Bytes` deref-coerces.
        let body = match ty {
            Type::TYPE_ENUM => {
                let closed = is_closed_enum(&features);
                enum_write(closed, &quote! { __v })
            }
            Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
                let val = if boxed {
                    quote! { &**__v }
                } else {
                    quote! { __v }
                };
                write_call(ty, &val)
            }
            _ if is_copy_scalar(ty) => write_call(ty, &quote! { *__v }),
            _ => write_call(ty, &quote! { __v }),
        };
        arms.push(quote! {
            #qualified::#variant(__v) => {
                enc.write_field_name(#name_lit)?;
                #body
            }
        });
    }

    Ok(quote! {
        if let ::core::option::Option::Some(ref __v) = self.#field_ident {
            match __v {
                #(#arms)*
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn oneof_merge_arms(
    ctx: &CodeGenContext,
    enum_ident: &proc_macro2::Ident,
    oneof_name: &str,
    fields: &[&FieldDescriptorProto],
    oneof_prefix: &TokenStream,
    current_package: &str,
    proto_fqn: &str,
    parent_features: &ResolvedFeatures,
    nesting: usize,
) -> Result<Vec<TokenStream>, CodeGenError> {
    let field_ident = make_field_ident(oneof_name);
    let qualified: TokenStream = quote! { #oneof_prefix #enum_ident };

    let mut arms: Vec<TokenStream> = Vec::new();
    for field in fields {
        let features = crate::features::resolve_field(ctx, field, parent_features);
        let proto_name = field
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("field.name"))?;
        let variant = crate::oneof::oneof_variant_ident(proto_name);
        let ty = effective_type(ctx, field, &features);
        let (_, name_pat) = text_field_name(proto_name, field, ty);
        let use_bytes = ty == Type::TYPE_BYTES && field_uses_bytes(ctx, proto_fqn, proto_name);

        // Message/group variants are boxed. Merge-into-existing matches
        // binary oneof semantics (oneof_merge_arm in impl_message.rs).
        let assign = match ty {
            Type::TYPE_MESSAGE | Type::TYPE_GROUP => {
                quote! {
                    if let ::core::option::Option::Some(
                        #qualified::#variant(ref mut __existing)
                    ) = self.#field_ident {
                        dec.merge_message(&mut **__existing)?;
                    } else {
                        let mut __m = ::core::default::Default::default();
                        dec.merge_message(&mut __m)?;
                        self.#field_ident = ::core::option::Option::Some(
                            #qualified::#variant(::buffa::alloc::boxed::Box::new(__m))
                        );
                    }
                }
            }
            Type::TYPE_ENUM => {
                let closed = is_closed_enum(&features);
                let enum_ty = enum_type_path(ctx, field, current_package, nesting)?;
                let read = enum_read(closed, &enum_ty, &format_ident!("dec"));
                quote! {
                    self.#field_ident = ::core::option::Option::Some(
                        #qualified::#variant(#read?)
                    );
                }
            }
            Type::TYPE_STRING => quote! {
                self.#field_ident = ::core::option::Option::Some(
                    #qualified::#variant(dec.read_string()?.into_owned())
                );
            },
            Type::TYPE_BYTES => {
                let read = if use_bytes {
                    quote! { ::buffa::bytes::Bytes::from(dec.read_bytes()?) }
                } else {
                    quote! { dec.read_bytes()? }
                };
                quote! {
                    self.#field_ident = ::core::option::Option::Some(
                        #qualified::#variant(#read)
                    );
                }
            }
            _ => {
                let read = read_call(ty);
                quote! {
                    self.#field_ident = ::core::option::Option::Some(
                        #qualified::#variant(#read)
                    );
                }
            }
        };
        arms.push(quote! { #name_pat => { #assign } });
    }
    Ok(arms)
}

// ─────────────────────────────────────────────────────────────────────────────
// Map fields
// ─────────────────────────────────────────────────────────────────────────────

/// Encode a map as one `field_name { key: K value: V }` entry per pair.
///
/// The entry body reuses [`write_call`] for key and value, routed directly
/// at the encoder with hard-coded `"key"` / `"value"` field names. Map keys
/// are never enum-typed (proto spec restricts keys to integral/bool/string);
/// values may be any type including message and enum.
fn map_encode_stmt(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
    features: &ResolvedFeatures,
) -> Result<TokenStream, CodeGenError> {
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(proto_name);
    let name_lit = proto_name;
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;
    let key_ty = effective_type_in_map_entry(ctx, key_fd, features);
    let val_ty = effective_type_in_map_entry(ctx, val_fd, features);
    let val_features = crate::features::resolve_field(ctx, val_fd, features);

    // Key: integral/bool/string only. `__k` is a reference from the iterator.
    let key_write = if key_ty == Type::TYPE_STRING {
        write_call(key_ty, &quote! { __k })
    } else {
        write_call(key_ty, &quote! { *__k })
    };

    // Value: `__v` is a reference. Message values are `&M` directly (map
    // values are never boxed). Enum values need the closed/open split.
    let val_write = match val_ty {
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(&val_features);
            enum_write(closed, &quote! { __v })
        }
        Type::TYPE_MESSAGE => write_call(val_ty, &quote! { __v }),
        Type::TYPE_STRING | Type::TYPE_BYTES => write_call(val_ty, &quote! { __v }),
        _ => write_call(val_ty, &quote! { *__v }),
    };

    // `write_map_entry` takes a closure directly — no `TextFormat` (and hence
    // no `Message: Default + 'static + Clone + ...`) bound to satisfy. The
    // closure captures `__k`, `__v` with their concrete types so the
    // `#key_write` / `#val_write` bodies (which contain type-specific calls
    // like `enc.write_i32(*__k)`) typecheck.
    Ok(quote! {
        for (__k, __v) in &self.#ident {
            enc.write_field_name(#name_lit)?;
            enc.write_map_entry(|enc| {
                enc.write_field_name("key")?;
                #key_write
                enc.write_field_name("value")?;
                #val_write
                ::core::result::Result::Ok(())
            })?;
        }
    })
}

/// Decode one map entry. Opens `{`, reads `key` / `value` until `}`, then
/// inserts into the map with defaults for whichever was absent.
fn map_merge_arm(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    field: &FieldDescriptorProto,
    current_package: &str,
    features: &ResolvedFeatures,
    nesting: usize,
) -> Result<TokenStream, CodeGenError> {
    let proto_name = field
        .name
        .as_deref()
        .ok_or(CodeGenError::MissingField("field.name"))?;
    let ident = make_field_ident(proto_name);
    let name_lit = proto_name;
    let (key_fd, val_fd) = find_map_entry_fields(msg, field)?;
    let key_ty = effective_type_in_map_entry(ctx, key_fd, features);
    let val_ty = effective_type_in_map_entry(ctx, val_fd, features);
    let val_features = crate::features::resolve_field(ctx, val_fd, features);

    // Key read (never enum, never message per proto spec).
    let key_read = match key_ty {
        Type::TYPE_STRING => quote! { __d.read_string()?.into_owned() },
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => quote! { __d.read_i32()? },
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => quote! { __d.read_i64()? },
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { __d.read_u32()? },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { __d.read_u64()? },
        Type::TYPE_BOOL => quote! { __d.read_bool()? },
        _ => {
            return Err(CodeGenError::Other(format!(
                "unsupported map key type {key_ty:?}"
            )))
        }
    };

    // Value read. Map values are never `bytes::Bytes` (see basic.proto
    // comment: map_rust_type_from_entry unconditionally uses Vec<u8>).
    let val_read = match val_ty {
        Type::TYPE_MESSAGE => quote! {
            {
                let mut __m = ::core::default::Default::default();
                __d.merge_message(&mut __m)?;
                __m
            }
        },
        Type::TYPE_ENUM => {
            let closed = is_closed_enum(&val_features);
            let enum_ty = enum_type_path(ctx, val_fd, current_package, nesting)?;
            let read = enum_read(closed, &enum_ty, &format_ident!("__d"));
            quote! { #read? }
        }
        Type::TYPE_STRING => quote! { __d.read_string()?.into_owned() },
        Type::TYPE_BYTES => quote! { __d.read_bytes()? },
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => quote! { __d.read_i32()? },
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => quote! { __d.read_i64()? },
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => quote! { __d.read_u32()? },
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => quote! { __d.read_u64()? },
        Type::TYPE_FLOAT => quote! { __d.read_f32()? },
        Type::TYPE_DOUBLE => quote! { __d.read_f64()? },
        Type::TYPE_BOOL => quote! { __d.read_bool()? },
        Type::TYPE_GROUP => {
            // Map values can't be groups (spec forces length-prefixed).
            return Err(CodeGenError::Other("map value is a group".into()));
        }
    };

    // read_repeated_into handles both `f: [{...}, {...}]` and `f: {...}`
    // forms. Each call merges one entry via `merge_map_entry` (closure-taking
    // counterpart to merge_message) and returns (K, V). Absent key or value
    // defaults — same as binary map decode. One small Vec alloc per
    // list-form map field; textproto usage doesn't care.
    Ok(quote! {
        #name_lit => {
            let mut __pairs: ::buffa::alloc::vec::Vec<_> = ::buffa::alloc::vec::Vec::new();
            dec.read_repeated_into(&mut __pairs, |__d| {
                let mut __k = ::core::option::Option::None;
                let mut __v = ::core::option::Option::None;
                __d.merge_map_entry(|__d| {
                    while let ::core::option::Option::Some(__n) = __d.read_field_name()? {
                        match __n {
                            "key" => __k = ::core::option::Option::Some(#key_read),
                            "value" => __v = ::core::option::Option::Some(#val_read),
                            _ => __d.skip_value()?,
                        }
                    }
                    ::core::result::Result::Ok(())
                })?;
                ::core::result::Result::Ok((
                    __k.unwrap_or_default(),
                    __v.unwrap_or_default(),
                ))
            })?;
            for (__k, __v) in __pairs {
                self.#ident.insert(__k, __v);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_field(proto_name: &str, type_name: &str) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(proto_name.into()),
            type_name: Some(type_name.into()),
            r#type: Some(Type::TYPE_GROUP),
            ..Default::default()
        }
    }

    // ── text_field_name ──────────────────────────────────────────────────────

    #[test]
    fn text_name_non_group_is_field_name() {
        let f = FieldDescriptorProto {
            name: Some("child".into()),
            type_name: Some(".pkg.Child".into()),
            r#type: Some(Type::TYPE_MESSAGE),
            ..Default::default()
        };
        let (enc, pat) = text_field_name("child", &f, Type::TYPE_MESSAGE);
        assert_eq!(enc, "child");
        assert_eq!(pat.to_string(), "\"child\"");
    }

    #[test]
    fn text_name_proto2_group_uses_type_name() {
        // `optional group Data = N { ... }` → field name "data",
        // type_name ".pkg.Parent.Data".
        let f = group_field("data", ".pkg.Parent.Data");
        let (enc, pat) = text_field_name("data", &f, Type::TYPE_GROUP);
        assert_eq!(enc, "Data");
        assert_eq!(pat.to_string(), "\"Data\" | \"data\"");
    }

    #[test]
    fn text_name_multi_word_group() {
        // `optional group MultiWordGroupField = N` → field name is the
        // full lowercase, not snake_case.
        let f = group_field("multiwordgroupfield", ".pkg.MultiWordGroupField");
        let (enc, pat) = text_field_name("multiwordgroupfield", &f, Type::TYPE_GROUP);
        assert_eq!(enc, "MultiWordGroupField");
        assert_eq!(
            pat.to_string(),
            "\"MultiWordGroupField\" | \"multiwordgroupfield\""
        );
    }

    #[test]
    fn text_name_editions_delimited_unrelated_name_keeps_field_name() {
        // Editions `SomeMsg foo = 1 [features.message_encoding = DELIMITED]`.
        // effective_type rewrites to TYPE_GROUP, but lowercase("SomeMsg")
        // = "somemsg" ≠ "foo" so it's NOT group-like.
        let f = group_field("foo", ".pkg.SomeMsg");
        let (enc, pat) = text_field_name("foo", &f, Type::TYPE_GROUP);
        assert_eq!(enc, "foo");
        assert_eq!(pat.to_string(), "\"foo\"");
    }

    #[test]
    fn text_name_editions_delimited_matching_name_uses_type_name() {
        // Editions `Data data = 1 [DELIMITED]` — mirrors proto2 group shape
        // (as in the editions-proto2 conformance golden).
        let f = group_field("data", ".pkg.Parent.Data");
        let (enc, _) = text_field_name("data", &f, Type::TYPE_GROUP);
        assert_eq!(enc, "Data");
    }

    #[test]
    fn text_name_all_lowercase_type_no_duplicate_pattern() {
        // Pathological editions case: `message data {} data data = 1 [DELIMITED]`.
        // simple == proto_name → fall through, no `"data" | "data"` duplicate.
        let f = group_field("data", ".pkg.data");
        let (enc, pat) = text_field_name("data", &f, Type::TYPE_GROUP);
        assert_eq!(enc, "data");
        assert_eq!(pat.to_string(), "\"data\"");
    }
}

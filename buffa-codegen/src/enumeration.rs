//! Enum type code generation.

use std::collections::HashMap;

use crate::generated::descriptor::EnumDescriptorProto;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::CodeGenError;

/// Generate custom `Serialize` and `Deserialize` impls for a proto enum.
///
/// - Serialize: emits the proto name string via `Enumeration::proto_name`.
/// - Deserialize: accepts a string (via `from_proto_name`), an integer (via
///   `from_i32`), or null (→ `Default::default()`). Unknown values produce
///   a hard error — lenient handling happens at the field-level serde helpers.
fn generate_enum_serde(name_ident: &Ident) -> TokenStream {
    quote! {
        impl ::serde::Serialize for #name_ident {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> ::core::result::Result<S::Ok, S::Error> {
                s.serialize_str(::buffa::Enumeration::proto_name(self))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for #name_ident {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> ::core::result::Result<Self, D::Error> {
                struct _V;
                impl ::serde::de::Visitor<'_> for _V {
                    type Value = #name_ident;

                    fn expecting(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        f.write_str(concat!("a string, integer, or null for ", stringify!(#name_ident)))
                    }

                    fn visit_str<E: ::serde::de::Error>(self, v: &str) -> ::core::result::Result<#name_ident, E> {
                        <#name_ident as ::buffa::Enumeration>::from_proto_name(v).ok_or_else(|| {
                            ::serde::de::Error::unknown_variant(v, &[])
                        })
                    }

                    fn visit_i64<E: ::serde::de::Error>(self, v: i64) -> ::core::result::Result<#name_ident, E> {
                        let v32 = i32::try_from(v).map_err(|_| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("enum value {} out of i32 range", v)
                            )
                        })?;
                        <#name_ident as ::buffa::Enumeration>::from_i32(v32).ok_or_else(|| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("unknown enum value {}", v32)
                            )
                        })
                    }

                    fn visit_u64<E: ::serde::de::Error>(self, v: u64) -> ::core::result::Result<#name_ident, E> {
                        let v32 = i32::try_from(v).map_err(|_| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("enum value {} out of i32 range", v)
                            )
                        })?;
                        <#name_ident as ::buffa::Enumeration>::from_i32(v32).ok_or_else(|| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("unknown enum value {}", v32)
                            )
                        })
                    }

                    fn visit_unit<E: ::serde::de::Error>(self) -> ::core::result::Result<#name_ident, E> {
                        ::core::result::Result::Ok(::core::default::Default::default())
                    }
                }
                d.deserialize_any(_V)
            }
        }

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
}

/// Generate Rust code for a protobuf enum type.
///
/// `rust_name` is the Rust identifier to use.  For top-level enums this is
/// the proto enum name; for nested enums it is the parent-prefixed flat name
/// (e.g. `TestAllTypesProto3NestedEnum`) matching the type-map convention.
pub fn generate_enum(
    ctx: &CodeGenContext,
    enum_desc: &EnumDescriptorProto,
    rust_name: &str,
    proto_fqn: &str,
    features: &ResolvedFeatures,
    _resolver: &crate::imports::ImportResolver,
) -> Result<TokenStream, CodeGenError> {
    let name_ident = format_ident!("{}", rust_name);

    // Track which discriminant values have been seen to identify aliases.
    // Proto spec: the first value with a given number is the primary; subsequent
    // values with the same number (requires allow_alias = true in enum options)
    // are aliases.  Rust #[repr(i32)] enums cannot have duplicate discriminants,
    // so aliases are emitted as `pub const` items instead of enum variants.
    let mut seen: HashMap<i32, &str> = HashMap::new();
    let mut variants = Vec::new();
    let mut alias_consts = Vec::new();
    let mut from_i32_arms = Vec::new();
    let mut from_proto_name_arms: Vec<TokenStream> = Vec::new();
    let mut proto_name_arms = Vec::new();
    // Static slice for `Enumeration::values()`. Aliases are skipped — the
    // slice mirrors the *primary* declaration order, matching what
    // `from_i32` resolves to (so `MyEnum::values()[i].to_i32() ==
    // from_i32(...).unwrap().to_i32()` for unique values).
    let mut value_idents: Vec<Ident> = Vec::new();
    // Track the best candidate for Default: prefer value == 0 (proto3 default),
    // fall back to the first primary variant.
    let mut zero_variant: Option<Ident> = None;
    let mut first_variant: Option<Ident> = None;

    for v in &enum_desc.value {
        let value_name = v
            .name
            .as_deref()
            .ok_or(CodeGenError::MissingField("enum_value.name"))?;
        let number = v
            .number
            .ok_or(CodeGenError::MissingField("enum_value.number"))?;
        let variant_ident = crate::message::make_field_ident(value_name);
        let value_fqn = format!("{}.{}", proto_fqn, value_name);
        let variant_doc = crate::comments::doc_attrs(ctx.comment(&value_fqn));

        if let Some(&primary_name) = seen.get(&number) {
            let primary_ident = crate::message::make_field_ident(primary_name);
            alias_consts.push(quote! {
                #variant_doc
                #[allow(non_upper_case_globals)]
                pub const #variant_ident: Self = Self::#primary_ident;
            });
            // Accept alias names in from_proto_name for JSON deserialization.
            from_proto_name_arms.push(quote! {
                #value_name => ::core::option::Option::Some(Self::#primary_ident)
            });
        } else {
            seen.insert(number, value_name);
            if first_variant.is_none() {
                first_variant = Some(variant_ident.clone());
            }
            if number == 0 && zero_variant.is_none() {
                zero_variant = Some(variant_ident.clone());
            }
            variants.push(quote! { #variant_doc #variant_ident = #number });
            from_i32_arms.push(quote! {
                #number => ::core::option::Option::Some(Self::#variant_ident)
            });
            from_proto_name_arms.push(quote! {
                #value_name => ::core::option::Option::Some(Self::#variant_ident)
            });
            proto_name_arms.push(quote! {
                Self::#variant_ident => #value_name
            });
            value_idents.push(variant_ident);
        }
    }

    let alias_block = if alias_consts.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #name_ident {
                #(#alias_consts)*
            }
        }
    };

    // Proto3 (and editions): the first enum value must be 0 per spec, so
    // prefer the zero-valued variant for Default, falling back to first.
    // Proto2: the first declared value is the default regardless of its
    // number, so use first_variant unconditionally.
    let default_variant = if features.enum_type == crate::features::EnumType::Closed {
        first_variant
    } else {
        zero_variant.or(first_variant)
    };
    let default_block = match default_variant {
        Some(v) => quote! {
            impl ::core::default::Default for #name_ident {
                fn default() -> Self {
                    Self::#v
                }
            }
        },
        None => quote! {},
    };

    let serde_impls = if ctx.config.generate_json {
        generate_enum_serde(&name_ident)
    } else {
        quote! {}
    };
    let arbitrary_derive = if ctx.config.generate_arbitrary {
        quote! { #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))] }
    } else {
        quote! {}
    };

    let enum_doc = crate::comments::doc_attrs(ctx.comment(proto_fqn));
    let custom_type_attrs = crate::context::CodeGenContext::matching_attributes(
        &ctx.config.type_attributes,
        proto_fqn,
    )?;
    let custom_enum_attrs = crate::context::CodeGenContext::matching_attributes(
        &ctx.config.enum_attributes,
        proto_fqn,
    )?;

    Ok(quote! {
        #enum_doc
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #arbitrary_derive
        #custom_type_attrs
        #custom_enum_attrs
        #[repr(i32)]
        pub enum #name_ident {
            #(#variants,)*
        }

        #alias_block

        #default_block

        #serde_impls

        impl ::buffa::Enumeration for #name_ident {
            fn from_i32(value: i32) -> ::core::option::Option<Self> {
                match value {
                    #(#from_i32_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn to_i32(&self) -> i32 {
                *self as i32
            }

            fn proto_name(&self) -> &'static str {
                match self {
                    #(#proto_name_arms,)*
                }
            }

            fn from_proto_name(name: &str) -> ::core::option::Option<Self> {
                match name {
                    #(#from_proto_name_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn values() -> &'static [Self] {
                &[#(Self::#value_idents),*]
            }
        }
    })
}

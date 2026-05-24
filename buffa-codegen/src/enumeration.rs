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
                                ::buffa::alloc::format!("enum value {v} out of i32 range")
                            )
                        })?;
                        <#name_ident as ::buffa::Enumeration>::from_i32(v32).ok_or_else(|| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("unknown enum value {v32}")
                            )
                        })
                    }

                    fn visit_u64<E: ::serde::de::Error>(self, v: u64) -> ::core::result::Result<#name_ident, E> {
                        let v32 = i32::try_from(v).map_err(|_| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("enum value {v} out of i32 range")
                            )
                        })?;
                        <#name_ident as ::buffa::Enumeration>::from_i32(v32).ok_or_else(|| {
                            ::serde::de::Error::custom(
                                ::buffa::alloc::format!("unknown enum value {v32}")
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
    // Per-value records for idiomatic CamelCase alias generation, collected only
    // when the feature is enabled. Each entry is
    // `(proto_value_name, alias_target, own_ident_string)` where
    // `own_ident_string` is the value's existing variant/alias identifier (which
    // a CamelCase alias must not duplicate) and `alias_target` is the variant a
    // generated `const` would point at.
    let mut value_records: Vec<(String, Ident, String)> = Vec::new();

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
        let variant_doc =
            crate::comments::doc_attrs_resolved(ctx.comment(&value_fqn), proto_fqn, &ctx.type_map);

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
            if ctx.config.idiomatic_enum_aliases {
                value_records.push((
                    value_name.to_string(),
                    primary_ident,
                    variant_ident.to_string(),
                ));
            }
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
            if ctx.config.idiomatic_enum_aliases {
                value_records.push((
                    value_name.to_string(),
                    variant_ident.clone(),
                    variant_ident.to_string(),
                ));
            }
            value_idents.push(variant_ident);
        }
    }

    // Idiomatic CamelCase aliases (feature-gated; `value_records` is empty when
    // disabled). Returns the extra `const` items to emit and, when aliases are
    // suppressed by a conflict, a doc note to append to the enum.
    let enum_simple_name = enum_desc.name.as_deref().unwrap_or(rust_name);
    let (idiomatic_consts, idiomatic_doc_note) =
        idiomatic_aliases(ctx, rust_name, enum_simple_name, value_records);

    let alias_block = if alias_consts.is_empty() && idiomatic_consts.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #name_ident {
                #(#alias_consts)*
                #(#idiomatic_consts)*
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
        // `generate_enum_serde` returns multiple sibling items
        // (`impl Serialize`, `impl Deserialize`, `impl ProtoElemJson`). A
        // bare outer `#[cfg]` would attach only to the first; wrapping
        // them in a `#[cfg(...)] const _: () = { ... };` block lets one
        // outer cfg cover the lot — the anonymous const is itself a single
        // item, and trait impls inside it register globally on the enum
        // exactly as they would at module scope.
        crate::feature_gates::cfg_const_block(
            generate_enum_serde(&name_ident),
            ctx.config.feature_gates().json,
        )
    } else {
        quote! {}
    };
    let arbitrary_derive = if ctx.config.generate_arbitrary {
        quote! { #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))] }
    } else {
        quote! {}
    };

    let enum_doc = {
        let base =
            crate::comments::doc_attrs_resolved(ctx.comment(proto_fqn), proto_fqn, &ctx.type_map);
        quote! { #base #idiomatic_doc_note }
    };
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

/// Compute idiomatic `UpperCamelCase` alias `const`s for an enum.
///
/// `records` holds one entry per proto value (empty when the feature is
/// disabled): `(proto_value_name, alias_target, own_ident_string)`, where
/// `own_ident_string` is the value's existing variant/alias identifier (which a
/// CamelCase alias must not duplicate). The proto names stay the definitive
/// variants; this only adds aliases.
///
/// Returns the `const` items to emit and a doc-note token stream (empty unless
/// aliases were suppressed). The rule is all-or-nothing per enum: if any two
/// values would collide after conversion — or a value would yield an invalid
/// identifier — no aliases are emitted, a [`CodeGenWarning`](crate::CodeGenWarning)
/// is recorded on `ctx`, and the returned doc note explains the suppression. This
/// guarantees a match is never forced to mix `SHOUTY_SNAKE_CASE` and idiomatic
/// names.
fn idiomatic_aliases(
    ctx: &CodeGenContext,
    rust_name: &str,
    enum_simple_name: &str,
    records: Vec<(String, Ident, String)>,
) -> (Vec<TokenStream>, TokenStream) {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::fmt::Write;

    if records.is_empty() {
        return (Vec::new(), quote! {});
    }

    let is_valid = |c: &str| !c.is_empty() && !c.starts_with(|ch: char| ch.is_ascii_digit());

    let prefix = format!("{}_", crate::idents::to_shouty_snake_case(enum_simple_name));

    // Strip the enum-name prefix only if *every* value carries it and stays a
    // valid identifier afterwards; otherwise keep full names. Deciding this for
    // the whole enum (not per value) keeps the result from mixing stripped and
    // unstripped names.
    let strip = records.iter().all(|(name, ..)| {
        name.strip_prefix(&prefix)
            .is_some_and(|base| is_valid(&crate::idents::to_upper_camel_case(base)))
    });

    let camel = |name: &str| {
        let base = if strip {
            name.strip_prefix(&prefix).unwrap_or(name)
        } else {
            name
        };
        crate::idents::to_upper_camel_case(base)
    };

    // Identifiers already occupying the enum's type namespace (variants + proto
    // `allow_alias` consts), guaranteed unique by protobuf, owned so `records`
    // can be consumed below.
    let existing: HashSet<String> = records.iter().map(|(_, _, own)| own.clone()).collect();

    // Group proto values by the escaped CamelCase identifier they would claim.
    // A value also pulls in the owner of an existing variant/alias its CamelCase
    // form lands on — that is the silent variant↔const shadow case.
    let mut buckets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut invalid: BTreeSet<String> = BTreeSet::new();
    {
        let owner: HashMap<&str, &str> = records
            .iter()
            .map(|(name, _, own)| (own.as_str(), name.as_str()))
            .collect();
        for (name, _, _) in &records {
            let candidate = camel(name);
            if !is_valid(&candidate) {
                // Defensive: unreachable under the current strip logic (the
                // stripped path is taken only when every value stays valid, and
                // the unstripped path uses raw proto names, which are never empty
                // or digit-leading). Kept so a future converter that can emit an
                // invalid identifier still suppresses rather than emits bad code.
                invalid.insert(name.clone());
                continue;
            }
            let escaped = crate::idents::make_field_ident(&candidate).to_string();
            if let Some(&existing_owner) = owner.get(escaped.as_str()) {
                buckets
                    .entry(escaped.clone())
                    .or_default()
                    .insert(existing_owner.to_string());
            }
            buckets.entry(escaped).or_default().insert(name.clone());
        }
    }

    let conflicts: Vec<(&String, &BTreeSet<String>)> = buckets
        .iter()
        .filter(|(_, claimants)| claimants.len() > 1)
        .collect();

    if conflicts.is_empty() && invalid.is_empty() {
        // Clean: emit an alias for every value whose CamelCase form differs from
        // its own variant/alias identifier (skipping the redundant ones, which
        // are the only way an emitted name can already be in `existing`).
        let consts = records
            .into_iter()
            .filter_map(|(name, target, _own)| {
                let escaped = crate::idents::make_field_ident(&camel(&name));
                if existing.contains(&escaped.to_string()) {
                    return None;
                }
                // A short doc instead of duplicating the variant's proto comment:
                // links the reader to the canonical variant and warns that
                // `Debug` prints the variant name, not this alias.
                let alias_doc = format!(
                    "Idiomatic alias for [`Self::{target}`]; `Debug` prints the variant name."
                );
                Some(quote! {
                    #[doc = #alias_doc]
                    #[allow(non_upper_case_globals)]
                    pub const #escaped: Self = Self::#target;
                })
            })
            .collect();
        return (consts, quote! {});
    }

    // Suppressed: record a structured warning and an enum doc note describing
    // every clash, so the reason is visible both at build time and in docs.
    let conflict_data: Vec<crate::AliasConflict> = conflicts
        .iter()
        .map(|(camel_ident, claimants)| crate::AliasConflict {
            camel_target: (*camel_ident).clone(),
            proto_values: claimants.iter().cloned().collect(),
        })
        .collect();
    let invalid_data: Vec<String> = invalid.into_iter().collect();

    // Build the doc note by borrowing, then hand both lists to `warn` by move.
    let mut note = String::from(
        "Idiomatic CamelCase aliases are not generated for this enum: two or more proto values \
         collide after conversion (or would be invalid identifiers). Use the `SHOUTY_SNAKE_CASE` \
         variants directly. Collisions:\n",
    );
    for conflict in &conflict_data {
        let joined = conflict
            .proto_values
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(note, "- {joined} → `{}`", conflict.camel_target);
    }
    for name in &invalid_data {
        let _ = writeln!(note, "- `{name}` produces an invalid identifier");
    }

    ctx.warn(crate::CodeGenWarning::IdiomaticAliasesSuppressed {
        enum_name: rust_name.to_string(),
        conflicts: conflict_data,
        invalid: invalid_data,
    });

    (Vec::new(), quote! { #[doc = #note] })
}

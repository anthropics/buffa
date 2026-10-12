//! Optional impls for the trait families outside buffa's binary codec.
//!
//! A family is selected with a bare `#[buffa(<name>)]` key and its impl is
//! emitted unconditionally. The crate docs show how a consumer makes it
//! conditional with `cfg_attr`. Reflection impls are written by hand.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{parse_quote, GenericParam, Lifetime, LifetimeParam, WherePredicate};

use crate::remote_field::RemoteField;

/// Which optional impls a newtype asked for.
#[derive(Default)]
pub struct Flags {
    /// The span of the `arbitrary` key, when the newtype carries
    /// `#[buffa(arbitrary)]` — see [`arbitrary`].
    pub arbitrary: Option<Span>,
    /// The span of the `serde` key, when the newtype carries
    /// `#[buffa(serde)]` — see [`serde`].
    pub serde: Option<Span>,
}

/// The JSON form the `serde` impls give a newtype, chosen per family.
#[derive(Clone, Copy)]
pub enum SerdeForm {
    /// A JSON string, for the string family.
    String,
    /// A base64 JSON string, for the bytes family.
    Bytes,
    /// The wrapped field's own serde representation, for the list, box and
    /// map families.
    Wrapped,
}

/// Emits `impl serde::Serialize` and `impl serde::Deserialize` for the
/// newtype, or nothing when `#[buffa(serde)]` is absent.
///
/// The string and bytes forms deserialize from a string only and reject
/// `null`. The crate docs' section on the key gives the reason.
///
/// The wrapped form bounds each impl on the field type's own serde impl. So
/// the binary codec can use the newtype when that bound does not hold.
pub fn serde(remote: &RemoteField, form: SerdeForm) -> TokenStream {
    let Some(span) = remote.flags.serde else {
        return quote! {};
    };
    let RemoteField {
        ident,
        generics,
        field_ty,
        accessor,
        ..
    } = remote;
    let krate = Ident::new("serde", span);
    let lifetime: Lifetime = parse_quote!('__buffa_de);
    let mut ser_generics = generics.clone();
    let mut de_generics = generics.clone();
    de_generics.params.insert(
        0,
        GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())),
    );

    let (serialize, deserialize) = match form {
        SerdeForm::String => (
            quote! {
                ::#krate::Serializer::serialize_str(
                    __buffa_serializer,
                    <Self as ::core::convert::AsRef<str>>::as_ref(self),
                )
            },
            string_only_deserialize(
                &krate,
                &lifetime,
                &quote! { ::buffa::ProtoString },
                "a string",
                // An owned string moves into the newtype without a copy.
                &quote! {
                    fn visit_str<__BuffaError: ::#krate::de::Error>(
                        self,
                        __buffa_value: &str,
                    ) -> ::core::result::Result<__BuffaValue, __BuffaError> {
                        ::core::result::Result::Ok(
                            <__BuffaValue as ::buffa::ProtoString>::copy_from_str(__buffa_value),
                        )
                    }
                    fn visit_string<__BuffaError: ::#krate::de::Error>(
                        self,
                        __buffa_value: ::buffa::alloc::string::String,
                    ) -> ::core::result::Result<__BuffaValue, __BuffaError> {
                        ::core::result::Result::Ok(<__BuffaValue as ::core::convert::From<
                            ::buffa::alloc::string::String,
                        >>::from(__buffa_value))
                    }
                },
            ),
        ),
        SerdeForm::Bytes => (
            quote! { ::buffa::json_helpers::bytes::serialize(self, __buffa_serializer) },
            string_only_deserialize(
                &krate,
                &lifetime,
                &quote! { ::core::convert::From<::buffa::alloc::vec::Vec<u8>> },
                "a base64-encoded string",
                // `bytes::deserialize` owns the base64 alphabets and padding
                // rules that proto3 JSON accepts. It reads `null` as empty, so
                // it is handed the string and never the caller's deserializer.
                &quote! {
                    fn visit_str<__BuffaError: ::#krate::de::Error>(
                        self,
                        __buffa_value: &str,
                    ) -> ::core::result::Result<__BuffaValue, __BuffaError> {
                        ::buffa::json_helpers::bytes::deserialize(
                            <&str as ::#krate::de::IntoDeserializer<'_, __BuffaError>>
                                ::into_deserializer(__buffa_value),
                        )
                    }
                },
            ),
        ),
        SerdeForm::Wrapped => {
            ser_generics
                .make_where_clause()
                .predicates
                .push(parse_quote! { #field_ty: ::#krate::Serialize });
            de_generics
                .make_where_clause()
                .predicates
                .push(parse_quote! { #field_ty: ::#krate::Deserialize<#lifetime> });
            let construct = remote.construct(quote! {
                <#field_ty as ::#krate::Deserialize<#lifetime>>::deserialize(__buffa_deserializer)?
            });
            (
                quote! { ::#krate::Serialize::serialize(&#accessor, __buffa_serializer) },
                quote! { ::core::result::Result::Ok(#construct) },
            )
        }
    };
    let (ser_impl, ty_generics, ser_where) = ser_generics.split_for_impl();
    let (de_impl, _, de_where) = de_generics.split_for_impl();
    quote! {
        impl #ser_impl ::#krate::Serialize for #ident #ty_generics #ser_where {
            fn serialize<__BuffaSerializer: ::#krate::Serializer>(
                &self,
                __buffa_serializer: __BuffaSerializer,
            ) -> ::core::result::Result<__BuffaSerializer::Ok, __BuffaSerializer::Error> {
                #serialize
            }
        }
        impl #de_impl ::#krate::Deserialize<#lifetime> for #ident #ty_generics #de_where {
            fn deserialize<__BuffaDeserializer: ::#krate::Deserializer<#lifetime>>(
                __buffa_deserializer: __BuffaDeserializer,
            ) -> ::core::result::Result<Self, __BuffaDeserializer::Error> {
                #deserialize
            }
        }
    }
}

/// The body of a `deserialize` that accepts a string and rejects every other
/// input, `null` included.
///
/// `visit_methods` are the `Visitor` methods that build a `__BuffaValue` from
/// the string, and `bound` is what they need of `__BuffaValue`. The visitor
/// is generic over the value it builds, because an item in a function body
/// cannot name the newtype's generic parameters.
fn string_only_deserialize(
    krate: &Ident,
    lifetime: &Lifetime,
    bound: &TokenStream,
    expecting: &str,
    visit_methods: &TokenStream,
) -> TokenStream {
    quote! {
        struct __BuffaVisitor<__BuffaValue>(::core::marker::PhantomData<__BuffaValue>);
        impl<#lifetime, __BuffaValue: #bound> ::#krate::de::Visitor<#lifetime>
            for __BuffaVisitor<__BuffaValue>
        {
            type Value = __BuffaValue;
            fn expecting(
                &self,
                __buffa_formatter: &mut ::core::fmt::Formatter<'_>,
            ) -> ::core::fmt::Result {
                __buffa_formatter.write_str(#expecting)
            }
            #visit_methods
        }
        // `deserialize_string` hands over text of any length, owned or
        // borrowed. `deserialize_str` can refuse long text: ciborium's reads
        // only what fits its scratch buffer. A visitor that has `visit_str`
        // alone still receives an owned string, because serde's default
        // `visit_string` forwards to it.
        ::#krate::Deserializer::deserialize_string(
            __buffa_deserializer,
            __BuffaVisitor::<Self>(::core::marker::PhantomData),
        )
    }
}

/// Whether the impl overrides `arbitrary_take_rest`, chosen per family so
/// the newtype consumes input as the representation it replaces does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TakeRest {
    /// Forward to the seed's. `String`, `Vec<u8>`, `Vec<T>` and `Vec<(K, V)>`
    /// override `arbitrary_take_rest` to run to the end of the input, where
    /// the trait default stops short.
    Seed,
    /// Leave the trait default, for the box family. `Box<T>` does not
    /// override `arbitrary_take_rest`, so forwarding to the pointee's would
    /// consume more input than `Box<T>` does for every pointee that
    /// overrides it, which is every derived message whose last field is a
    /// `String`, `Vec` or map.
    TraitDefault,
}

/// Emits `impl arbitrary::Arbitrary` for the newtype, or nothing when
/// `#[buffa(arbitrary)]` is absent.
///
/// The impl builds `seed`, the canonical owned type for the family, and
/// converts it with `build`, an expression over the `__buffa_seed` binding.
/// The crate docs give the reason for going through the canonical type.
///
/// `extra_predicates` join `seed: Arbitrary<'_>` and the struct's own `where`
/// clause. A family whose `build` calls an impl the user writes
/// (`MapStorage`'s `FromIterator`) names its bound there, so the impl applies
/// exactly where that one does.
///
/// `size_hint` stays at the trait default `(0, None)` in every family. That
/// is what `String` and the `Vec` seeds report. `Box<T>` reports the
/// pointee's hint under a recursion guard, which from `arbitrary` 1.4 runs
/// through `try_size_hint` and propagates `MaxRecursionReached`. The
/// expansion compiles against every 1.x, so it cannot implement
/// `try_size_hint`, and a plain forward to the pointee's `size_hint` swallows
/// that error at each box and walks a recursive message's other fields again
/// at every level.
pub fn arbitrary(
    remote: &RemoteField,
    seed: &TokenStream,
    build: &TokenStream,
    take_rest: TakeRest,
    extra_predicates: &[WherePredicate],
) -> TokenStream {
    let Some(key_span) = remote.flags.arbitrary else {
        return quote! {};
    };
    let RemoteField {
        ident, generics, ..
    } = remote;

    // The crate name carries the key's span, so a missing `arbitrary`
    // dependency is reported at the key and not at the derive.
    let krate = Ident::new("arbitrary", key_span);

    // A fresh lifetime for `Arbitrary<'a>`, inserted ahead of the struct's own
    // parameters (lifetimes must precede types) and named so it cannot collide
    // with one the newtype declares.
    let lifetime: Lifetime = parse_quote!('__buffa_arb);
    let mut arb_generics = generics.clone();
    arb_generics.params.insert(
        0,
        GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())),
    );
    {
        let predicates = &mut arb_generics.make_where_clause().predicates;
        // Bounding the seed rather than the element types covers both: for a
        // generic seed (`Vec<T>`, `Vec<(K, V)>`) it implies the element bounds,
        // and for a concrete one (`String`) it is satisfied outright.
        predicates.push(parse_quote! { #seed: ::#krate::Arbitrary<#lifetime> });
        predicates.extend(extra_predicates.iter().cloned());
    }
    let (impl_generics, _, arb_where_clause) = arb_generics.split_for_impl();
    let (_, ty_generics, _) = generics.split_for_impl();

    let take_rest_fn = match take_rest {
        TakeRest::Seed => quote! {
            #[inline]
            fn arbitrary_take_rest(
                u: ::#krate::Unstructured<#lifetime>,
            ) -> ::#krate::Result<Self> {
                let __buffa_seed: #seed = ::#krate::Arbitrary::arbitrary_take_rest(u)?;
                ::core::result::Result::Ok(#build)
            }
        },
        TakeRest::TraitDefault => quote! {},
    };

    quote! {
        impl #impl_generics ::#krate::Arbitrary<#lifetime> for #ident #ty_generics
        #arb_where_clause
        {
            #[inline]
            fn arbitrary(
                u: &mut ::#krate::Unstructured<#lifetime>,
            ) -> ::#krate::Result<Self> {
                let __buffa_seed: #seed = ::#krate::Arbitrary::arbitrary(u)?;
                ::core::result::Result::Ok(#build)
            }

            #take_rest_fn
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    #[test]
    fn serde_opt_in_is_per_type_and_repeatable() {
        let plain: syn::DeriveInput = parse_quote! {
            #[buffa(remote = Remote)]
            struct S(Remote);
        };
        assert!(!crate::string::derive(plain)
            .unwrap()
            .to_string()
            .contains("serde"));
        let keyed: syn::DeriveInput = parse_quote! {
            #[buffa(remote = Remote, serde)]
            #[buffa(serde)]
            struct S(Remote);
        };
        let output = crate::string::derive(keyed).unwrap().to_string();
        assert_eq!(output.matches(":: serde :: Serialize for").count(), 1);
        assert!(!output.contains("cfg"));
    }

    #[test]
    fn serde_rejects_a_value() {
        let input = parse_quote! {
            #[buffa(remote = Remote, serde = true)]
            struct S(Remote);
        };
        assert!(crate::string::derive(input)
            .unwrap_err()
            .to_string()
            .contains("`serde` takes no value; write it as a bare key"));
    }

    /// Expands every derive with the `serde` key. Returns `(family, expansion)`.
    fn serde_expansions() -> Vec<(&'static str, String)> {
        macro_rules! keyed {
            ($name:literal, $derive:path, { $($decl:tt)* }) => {{
                let input: syn::DeriveInput = parse_quote! {
                    #[buffa(remote = Remote, serde)]
                    $($decl)*
                };
                ($name, $derive(input).expect("keyed expansion").to_string())
            }};
        }
        vec![
            keyed!("string", crate::string::derive, {
                struct S(Remote);
            }),
            keyed!("bytes", crate::bytes::derive, {
                struct B(Remote);
            }),
            keyed!("list", crate::list::derive, {
                struct L<T>(Remote);
            }),
            keyed!("box", crate::box_ptr::derive, {
                struct P<T>(Remote);
            }),
            keyed!("map", crate::map::derive, {
                struct M<K, V>(Remote);
            }),
        ]
    }

    /// Every name a serde expansion introduces starts with `__buffa` or
    /// `__Buffa`: the lifetime, the generic parameters, the bindings, and the
    /// visitor type of the string and bytes forms.
    #[test]
    fn serde_impls_use_the_reserved_names() {
        for (name, keyed) in serde_expansions() {
            let mut wants = vec![
                ":: serde :: Deserialize < '__buffa_de > for",
                "fn serialize < __BuffaSerializer : :: serde :: Serializer >",
                "__buffa_serializer : __BuffaSerializer",
                "fn deserialize < __BuffaDeserializer : :: serde :: Deserializer < '__buffa_de >>",
                "__buffa_deserializer : __BuffaDeserializer",
            ];
            if matches!(name, "string" | "bytes") {
                wants.extend([
                    "struct __BuffaVisitor < __BuffaValue >",
                    "fn expecting (& self , __buffa_formatter :",
                    "fn visit_str < __BuffaError : :: serde :: de :: Error > (self , __buffa_value : & str ,)",
                ]);
            }
            for want in wants {
                assert!(keyed.contains(want), "{name}: expected `{want}`:\n{keyed}");
            }
        }
    }

    /// The string impls call `serde` and the `ProtoString` surface only, so a
    /// crate can use the key on a string newtype without `buffa/json`. The
    /// bytes impls take base64 from `buffa::json_helpers`.
    #[test]
    fn only_the_bytes_form_uses_json_helpers() {
        for (name, keyed) in serde_expansions() {
            assert_eq!(
                keyed.contains("json_helpers"),
                name == "bytes",
                "{name}:\n{keyed}"
            );
        }
    }

    /// A `#![no_std]` crate can use the key: the expansions name `core`,
    /// `buffa::alloc` and `serde`, and never `std`.
    #[test]
    fn serde_expansions_do_not_name_std() {
        for (name, keyed) in serde_expansions() {
            assert!(
                !keyed.contains("std ::"),
                "{name}: expansion names `std`:\n{keyed}"
            );
        }
    }

    /// The string and bytes impls build the value in a visitor that has only
    /// string methods, so `null` reaches serde's default `invalid type` error.
    /// The visitor is driven by `deserialize_string` in both forms. The
    /// container impls call the wrapped type's `Deserialize`.
    #[test]
    fn string_and_bytes_deserialize_through_a_string_only_visitor() {
        for (name, keyed) in serde_expansions() {
            let wants_visitor = matches!(name, "string" | "bytes");
            for visitor_part in [
                "de :: Visitor < '__buffa_de > for __BuffaVisitor",
                ":: serde :: Deserializer :: deserialize_string (__buffa_deserializer ,",
            ] {
                assert_eq!(
                    keyed.contains(visitor_part),
                    wants_visitor,
                    "{name}: `{visitor_part}`:\n{keyed}"
                );
            }
            assert!(
                !keyed.contains("deserialize_str ("),
                "{name}: expansion calls `deserialize_str`:\n{keyed}"
            );
            for absent in [
                "fn visit_unit",
                "fn visit_none",
                "proto_string :: deserialize",
            ] {
                assert!(
                    !keyed.contains(absent),
                    "{name}: expansion contains `{absent}`:\n{keyed}"
                );
            }
        }
    }

    /// Expands every derive over the same newtype shapes the integration tests
    /// use. Returns `(family, without_key, with_key)` per derive.
    fn expansions() -> Vec<(&'static str, String, String)> {
        macro_rules! pair {
            ($name:literal, $derive:path, { $($decl:tt)* }) => {{
                let plain: syn::DeriveInput = parse_quote! {
                    #[buffa(remote = Remote)]
                    $($decl)*
                };
                let keyed: syn::DeriveInput = parse_quote! {
                    #[buffa(remote = Remote, arbitrary)]
                    $($decl)*
                };
                (
                    $name,
                    $derive(plain).expect("plain expansion").to_string(),
                    $derive(keyed).expect("keyed expansion").to_string(),
                )
            }};
        }
        vec![
            pair!("string", crate::string::derive, {
                struct S(Remote);
            }),
            pair!("bytes", crate::bytes::derive, {
                struct B(Remote);
            }),
            pair!("list", crate::list::derive, {
                struct L<T>(Remote);
            }),
            pair!("box", crate::box_ptr::derive, {
                struct P<T>(Remote);
            }),
            pair!("map", crate::map::derive, {
                struct M<K, V>(Remote);
            }),
        ]
    }

    /// Without the key, an expansion does not mention `Arbitrary`, so a
    /// newtype that did not ask for the impl does not depend on the
    /// `arbitrary` crate.
    #[test]
    fn key_absent_emits_no_arbitrary() {
        for (name, plain, _) in expansions() {
            assert!(
                !plain.contains("Arbitrary"),
                "{name}: expansion without the key mentions Arbitrary:\n{plain}"
            );
        }
    }

    /// The expansion contains the impl and no `cfg` attribute, which is the
    /// crate docs' contract.
    #[test]
    fn key_present_emits_an_unconditional_impl() {
        for (name, _, keyed) in expansions() {
            assert!(
                keyed.contains(":: arbitrary :: Arbitrary < '__buffa_arb > for"),
                "{name}: no Arbitrary impl emitted:\n{keyed}"
            );
            assert!(
                !keyed.contains("cfg"),
                "{name}: the expansion carries a cfg:\n{keyed}"
            );
        }
    }

    /// The seed is each family's canonical owned type, and the expansion
    /// does not name the remote type's `Arbitrary`.
    #[test]
    fn seed_is_the_canonical_type_not_the_remote() {
        let expected = [
            (
                "string",
                "__buffa_seed : :: buffa :: alloc :: string :: String",
            ),
            (
                "bytes",
                "__buffa_seed : :: buffa :: alloc :: vec :: Vec < u8 >",
            ),
            (
                "list",
                "__buffa_seed : :: buffa :: alloc :: vec :: Vec < T >",
            ),
            ("box", "__buffa_seed : T"),
            (
                "map",
                "__buffa_seed : :: buffa :: alloc :: vec :: Vec < (K , V) >",
            ),
        ];
        for (name, _, keyed) in expansions() {
            let want = expected
                .iter()
                .find(|(n, _)| *n == name)
                .expect("every family has an expected seed")
                .1;
            assert!(
                keyed.contains(want),
                "{name}: expected seed `{want}`:\n{keyed}"
            );
            assert!(
                !keyed.contains("Remote as :: arbitrary"),
                "{name}: the impl uses the remote type's own Arbitrary:\n{keyed}"
            );
        }
    }

    /// See [`super::TakeRest`]: only the box family keeps the trait default.
    /// No family overrides `size_hint`.
    #[test]
    fn only_the_box_family_keeps_the_default_take_rest() {
        for (name, _, keyed) in expansions() {
            assert_eq!(
                keyed.contains("fn arbitrary_take_rest"),
                name != "box",
                "{name}: wrong `arbitrary_take_rest` decision:\n{keyed}"
            );
            assert!(
                !keyed.contains("fn size_hint"),
                "{name}: the expansion overrides size_hint:\n{keyed}"
            );
        }
    }

    /// The map impl builds through the `FromIterator` the user writes, so its
    /// bound has to be on the impl: a map whose `FromIterator` is narrower
    /// than the struct (`CustomMap<K, V>` with `impl<K: Ord, V>`) otherwise
    /// fails to compile.
    #[test]
    fn map_impl_is_bounded_on_from_iterator() {
        let (_, _, keyed) = expansions()
            .into_iter()
            .find(|(n, _, _)| *n == "map")
            .expect("map family expands");
        assert!(
            keyed.contains("Self : :: core :: iter :: FromIterator < (K , V) >"),
            "map: FromIterator bound missing from the Arbitrary impl:\n{keyed}"
        );
    }

    #[test]
    fn arbitrary_key_rejects_a_value() {
        let input: syn::DeriveInput = parse_quote! {
            #[buffa(remote = Remote, arbitrary = Something)]
            struct S(Remote);
        };
        let err = crate::string::derive(input).expect_err("a valued key is an error");
        assert!(
            err.to_string().contains("takes no value"),
            "unexpected error: {err}"
        );
    }

    /// Two `cfg_attr`s that both hold reach the derive as two keys.
    #[test]
    fn a_repeated_arbitrary_key_emits_one_impl() {
        let input: syn::DeriveInput = parse_quote! {
            #[buffa(remote = Remote, arbitrary)]
            #[buffa(arbitrary)]
            struct S(Remote);
        };
        let keyed = crate::string::derive(input)
            .expect("a repeated key is accepted")
            .to_string();
        assert_eq!(
            keyed
                .matches(":: arbitrary :: Arbitrary < '__buffa_arb > for")
                .count(),
            1,
            "{keyed}"
        );
    }

    #[test]
    fn unsupported_key_error_lists_the_arbitrary_key() {
        let input: syn::DeriveInput = parse_quote! {
            #[buffa(remote = Remote, nonsense = Something)]
            struct S(Remote);
        };
        let err = crate::string::derive(input).expect_err("an unknown key is an error");
        assert!(
            err.to_string().contains("`arbitrary`"),
            "unexpected error: {err}"
        );
    }
}

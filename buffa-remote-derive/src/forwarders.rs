//! Optional impls for the trait families outside buffa's binary codec.
//!
//! A family is selected with a bare `#[buffa(<name>)]` key and its impl is
//! emitted unconditionally. The crate docs show how a consumer makes it
//! conditional with `cfg_attr`. `arbitrary` is the only family; serde and
//! `ReflectList`/`ReflectMap` impls are written by hand.

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

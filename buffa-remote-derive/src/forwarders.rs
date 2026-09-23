//! Optional forwarders for the trait families outside buffa's binary codec,
//! emitted on request rather than always.
//!
//! A family is opted into with a bare `#[buffa(<name>)]` key next to `remote`,
//! and the impl it emits is `#[cfg(feature = "<name>")]`-gated on the
//! *consuming* crate's feature of that name — see the crate docs for why the
//! gate is mandatory and its name fixed. Only `arbitrary` exists so far; the
//! serde and `ReflectList`/`ReflectMap` families are still hand-written.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_quote, GenericParam, Lifetime, LifetimeParam, WherePredicate};

use crate::remote_field::RemoteField;

/// The bare `#[buffa(...)]` key that turns the [`arbitrary`] forwarder on.
pub const ARBITRARY: &str = "arbitrary";

/// Which optional forwarders a newtype opted into.
#[derive(Default)]
pub struct Flags {
    /// `#[buffa(arbitrary)]` — see [`arbitrary`].
    pub arbitrary: bool,
}

/// Whether the emitted impl overrides `arbitrary_take_rest`, chosen per family
/// so the newtype consumes the same bytes as the representation it replaces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TakeRest {
    /// Forward to the seed's. Correct when the seed *is* the default
    /// representation and overrides `arbitrary_take_rest` — `String`,
    /// `Vec<u8>`, `Vec<T>`, `Vec<(K, V)>` all do, so leaving the trait default
    /// in place would stop short of the buffer's end where the default
    /// representation runs to it.
    Seed,
    /// Leave the trait default (`Self::arbitrary`). Correct when the default
    /// representation does not override `arbitrary_take_rest`: `arbitrary`'s
    /// `Box<T>` doesn't, so forwarding to the *pointee's* would consume more
    /// bytes than `Box<T>` for every pointee that does override it — which is
    /// every derived message whose last field is a `String`, `Vec` or map.
    TraitDefault,
}

/// Emits `impl arbitrary::Arbitrary` for the newtype, or nothing when
/// `#[buffa(arbitrary)]` is absent.
///
/// The impl materializes `seed` — the canonical owned type buffa's own
/// `__private::arbitrary_proto_*` builders materialize for this family — and
/// converts it with `build`, an expression over the `__buffa_seed` binding.
/// Going through the canonical type is what makes the forwarder usable at all
/// (`ecow::EcoString` and friends have no `Arbitrary` impl to forward to) and
/// what keeps byte consumption identical to the default representation the
/// newtype replaces.
///
/// `extra_predicates` join `seed: Arbitrary<'_>` and the struct's own `where`
/// clause. A family whose `build` calls an impl the *user* writes
/// (`MapStorage`'s `FromIterator`) names its bound there, so the forwarder
/// applies exactly where that impl does rather than failing to compile.
///
/// `take_rest` says whether `arbitrary_take_rest` is overridden; see
/// [`TakeRest`] for why it is per-family rather than always on.
///
/// `size_hint` forwards to the seed's instead of the trait's `(0, None)`
/// default, mirroring `arbitrary`'s own `Box<str>`-to-`String` forwarder.
/// `try_size_hint` is deliberately not emitted: it arrived in `arbitrary` 1.4
/// and this code has to compile against any 1.x a consumer resolved. Nothing
/// is lost — the recursion guard that matters for a `ProtoBox` around a
/// recursive message lives in *that message's* derived `size_hint`.
pub fn arbitrary(
    remote: &RemoteField,
    seed: &TokenStream,
    build: &TokenStream,
    take_rest: TakeRest,
    extra_predicates: &[WherePredicate],
) -> TokenStream {
    if !remote.flags.arbitrary {
        return quote! {};
    }
    let RemoteField {
        ident, generics, ..
    } = remote;

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
        predicates.push(parse_quote! { #seed: ::arbitrary::Arbitrary<#lifetime> });
        predicates.extend(extra_predicates.iter().cloned());
    }
    let (impl_generics, _, arb_where_clause) = arb_generics.split_for_impl();
    let (_, ty_generics, _) = generics.split_for_impl();

    let take_rest_fn = match take_rest {
        TakeRest::Seed => quote! {
            #[inline]
            fn arbitrary_take_rest(
                u: ::arbitrary::Unstructured<#lifetime>,
            ) -> ::arbitrary::Result<Self> {
                let __buffa_seed: #seed = ::arbitrary::Arbitrary::arbitrary_take_rest(u)?;
                ::core::result::Result::Ok(#build)
            }
        },
        TakeRest::TraitDefault => quote! {},
    };

    quote! {
        #[cfg(feature = "arbitrary")]
        impl #impl_generics ::arbitrary::Arbitrary<#lifetime> for #ident #ty_generics
        #arb_where_clause
        {
            #[inline]
            fn arbitrary(
                u: &mut ::arbitrary::Unstructured<#lifetime>,
            ) -> ::arbitrary::Result<Self> {
                let __buffa_seed: #seed = ::arbitrary::Arbitrary::arbitrary(u)?;
                ::core::result::Result::Ok(#build)
            }

            #take_rest_fn

            #[inline]
            fn size_hint(depth: usize) -> (usize, ::core::option::Option<usize>) {
                <#seed as ::arbitrary::Arbitrary<#lifetime>>::size_hint(depth)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    /// Expands every derive over the same newtype shapes the integration tests
    /// use, with and without the flag. Returns `(without_flag, with_flag)`.
    fn expansions() -> Vec<(&'static str, String, String)> {
        macro_rules! pair {
            ($name:literal, $derive:path, { $($decl:tt)* }) => {{
                let plain: syn::DeriveInput = parse_quote! {
                    #[buffa(remote = Remote)]
                    $($decl)*
                };
                let flagged: syn::DeriveInput = parse_quote! {
                    #[buffa(remote = Remote, arbitrary)]
                    $($decl)*
                };
                (
                    $name,
                    $derive(plain).expect("plain expansion").to_string(),
                    $derive(flagged).expect("flagged expansion").to_string(),
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

    /// Without the flag, no derive may mention `Arbitrary` at all — the
    /// forwarder is opt-in, and a newtype that did not ask for it must not
    /// acquire a dependency on the `arbitrary` crate.
    #[test]
    fn flag_absent_emits_no_arbitrary() {
        for (name, plain, _) in expansions() {
            assert!(
                !plain.contains("Arbitrary"),
                "{name}: unflagged expansion mentions Arbitrary:\n{plain}"
            );
        }
    }

    /// With the flag, the impl must be there *and* be `cfg`-gated. An ungated
    /// impl compiles for the derive's author and breaks every consumer whose
    /// `arbitrary` feature is off, so the gate is the part worth asserting.
    #[test]
    fn flag_present_emits_cfg_gated_impl() {
        for (name, _, flagged) in expansions() {
            let gate = "# [cfg (feature = \"arbitrary\")] impl";
            assert!(
                flagged.contains(gate),
                "{name}: Arbitrary impl is not gated on `feature = \"arbitrary\"`:\n{flagged}"
            );
            assert!(
                flagged.contains(":: arbitrary :: Arbitrary < '__buffa_arb > for"),
                "{name}: no Arbitrary impl emitted:\n{flagged}"
            );
            // One gate, one impl: the `cfg` count must match the `Arbitrary`
            // impl count, so a second forwarder can never slip in ungated.
            assert_eq!(
                flagged.matches("# [cfg (feature = \"arbitrary\")]").count(),
                1,
                "{name}: expected exactly one gated item:\n{flagged}"
            );
        }
    }

    /// Each family must materialize the canonical type buffa's own
    /// `arbitrary_proto_*` builders use, not the remote type: that is what
    /// keeps byte consumption equal to the default representation and what
    /// frees the remote type from needing its own `Arbitrary`.
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
        for (name, _, flagged) in expansions() {
            let want = expected
                .iter()
                .find(|(n, _)| *n == name)
                .expect("every family has an expected seed")
                .1;
            assert!(
                flagged.contains(want),
                "{name}: expected seed `{want}`:\n{flagged}"
            );
            assert!(
                !flagged.contains("Remote as :: arbitrary"),
                "{name}: forwarder leans on the remote type's own Arbitrary:\n{flagged}"
            );
        }
    }

    /// `arbitrary_take_rest` is overridden exactly where the representation
    /// the newtype replaces overrides it. `String` and the `Vec`s do, so those
    /// four forward to the seed; `arbitrary`'s `Box<T>` does not, so the box
    /// family keeps the trait default and consumes what `Box<T>` consumes.
    #[test]
    fn take_rest_is_overridden_per_family() {
        for (name, _, flagged) in expansions() {
            assert_eq!(
                flagged.contains("fn arbitrary_take_rest"),
                name != "box",
                "{name}: wrong `arbitrary_take_rest` decision:\n{flagged}"
            );
        }
    }

    /// The map forwarder builds through the `FromIterator` the *user* writes,
    /// so its bound has to be on the impl — a map whose `FromIterator` is
    /// narrower than the struct (`CustomMap<K, V>` with `impl<K: Ord, V>`)
    /// otherwise fails to compile.
    #[test]
    fn map_forwarder_bounds_from_iterator() {
        let (_, _, flagged) = expansions()
            .into_iter()
            .find(|(n, _, _)| *n == "map")
            .expect("map family expands");
        assert!(
            flagged.contains("Self : :: core :: iter :: FromIterator < (K , V) >"),
            "map: FromIterator bound missing from the Arbitrary impl:\n{flagged}"
        );
    }

    #[test]
    fn arbitrary_flag_rejects_a_value() {
        let input: syn::DeriveInput = parse_quote! {
            #[buffa(remote = Remote, arbitrary = Something)]
            struct S(Remote);
        };
        let err = crate::string::derive(input).expect_err("a valued flag is an error");
        assert!(
            err.to_string().contains("takes no value"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn unsupported_key_error_lists_the_flag() {
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

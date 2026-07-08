use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_quote, DeriveInput};

use crate::remote_field::{self, RemoteField};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let remote = remote_field::parse(&input)?;
    let RemoteField {
        ident,
        generics,
        field_ty,
        accessor,
        ..
    } = &remote;

    let element_ty = remote_field::single_type_param(generics)?;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let from_iter = remote_field::qualified_call(
        field_ty,
        quote! { ::core::iter::FromIterator<#element_ty> },
        "from_iter",
    );
    let from_vec = remote_field::qualified_call(
        field_ty,
        quote! { ::core::convert::From<::buffa::alloc::vec::Vec<#element_ty>> },
        "from",
    );
    // Fully qualified, not `#accessor.as_ref()` — see the matching comment in
    // string.rs for why plain method-call syntax is ambiguous here.
    let as_slice = remote_field::qualified_call(
        field_ty,
        quote! { ::core::convert::AsRef<[#element_ty]> },
        "as_ref",
    );
    let extend = remote_field::qualified_call(
        field_ty,
        quote! { ::core::iter::Extend<#element_ty> },
        "extend",
    );

    let ctor_from_iter = remote.construct(quote! { #from_iter(iter) });
    let ctor_from_vec = remote.construct(quote! { #from_vec(v) });

    // The `ProtoList` impl needs bounds beyond the struct's own (the element
    // bounds, `Extend`, `Default`), so it can't reuse `#where_clause` like
    // the impls below do. Augment a copy of the generics instead of
    // hand-writing the `where` block, so the struct's own predicates are
    // preserved — dropping them fails well-formedness for a newtype declared
    // with a `where` clause.
    let mut list_generics = (*generics).clone();
    {
        let predicates = &mut list_generics.make_where_clause().predicates;
        predicates.push(parse_quote! {
            #element_ty: ::core::clone::Clone
                + ::core::cmp::PartialEq
                + ::core::fmt::Debug
                + ::core::marker::Send
                + ::core::marker::Sync
        });
        predicates.push(parse_quote! { #field_ty: ::core::iter::Extend<#element_ty> });
        predicates.push(parse_quote! { Self: ::core::default::Default });
    }
    let list_where_clause = &list_generics.where_clause;

    Ok(quote! {
        impl #impl_generics ::core::ops::Deref for #ident #ty_generics #where_clause {
            type Target = [#element_ty];
            #[inline]
            fn deref(&self) -> &[#element_ty] {
                #as_slice(&#accessor)
            }
        }

        impl #impl_generics ::core::iter::FromIterator<#element_ty> for #ident #ty_generics #where_clause {
            #[inline]
            fn from_iter<__BuffaIter: ::core::iter::IntoIterator<Item = #element_ty>>(
                iter: __BuffaIter,
            ) -> Self {
                #ctor_from_iter
            }
        }

        impl #impl_generics ::core::convert::From<::buffa::alloc::vec::Vec<#element_ty>> for #ident #ty_generics #where_clause {
            #[inline]
            fn from(v: ::buffa::alloc::vec::Vec<#element_ty>) -> Self {
                #ctor_from_vec
            }
        }

        impl #impl_generics ::buffa::ProtoList<#element_ty> for #ident #ty_generics
        #list_where_clause
        {
            #[inline]
            fn push(&mut self, value: #element_ty) {
                #extend(&mut #accessor, ::core::iter::once(value));
            }

            // Reinitializes via `Default` rather than forwarding to a native
            // `clear` (no such method is assumed to exist on the remote
            // type), so the existing allocation is dropped rather than
            // retained. `ProtoList`'s contract only asks for capacity
            // retention "where the underlying type allows" — see the crate
            // docs if that matters for your workload.
            #[inline]
            fn clear(&mut self) {
                *self = ::core::default::Default::default();
            }
        }
    })
}

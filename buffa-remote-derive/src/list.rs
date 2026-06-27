use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

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

    let ctor_from_iter = remote.construct(quote! { #from_iter(iter) });
    let ctor_from_vec = remote.construct(quote! { #from_vec(v) });

    Ok(quote! {
        impl #impl_generics ::core::ops::Deref for #ident #ty_generics #where_clause {
            type Target = [#element_ty];
            #[inline]
            fn deref(&self) -> &[#element_ty] {
                #accessor.as_ref()
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
        where
            #element_ty: ::core::clone::Clone
                + ::core::cmp::PartialEq
                + ::core::fmt::Debug
                + ::core::marker::Send
                + ::core::marker::Sync,
            #field_ty: ::core::iter::Extend<#element_ty>,
            Self: ::core::default::Default,
        {
            #[inline]
            fn push(&mut self, value: #element_ty) {
                #accessor.extend(::core::iter::once(value));
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

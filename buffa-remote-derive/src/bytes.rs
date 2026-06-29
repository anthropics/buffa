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
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let from_vec = remote_field::qualified_call(
        field_ty,
        quote! { ::core::convert::From<::buffa::alloc::vec::Vec<u8>> },
        "from",
    );
    // Fully qualified, not `#accessor.as_ref()` — see the matching comment in
    // string.rs for why plain method-call syntax is ambiguous here.
    let as_bytes =
        remote_field::qualified_call(field_ty, quote! { ::core::convert::AsRef<[u8]> }, "as_ref");

    let ctor_from_vec = remote.construct(quote! { #from_vec(v) });
    let ctor_from_wire = remote.construct(quote! { #from_vec(payload.as_slice().to_vec()) });

    Ok(quote! {
        impl #impl_generics ::core::ops::Deref for #ident #ty_generics #where_clause {
            type Target = [u8];
            #[inline]
            fn deref(&self) -> &[u8] {
                #as_bytes(&#accessor)
            }
        }

        impl #impl_generics ::core::convert::AsRef<[u8]> for #ident #ty_generics #where_clause {
            #[inline]
            fn as_ref(&self) -> &[u8] {
                #as_bytes(&#accessor)
            }
        }

        impl #impl_generics ::core::convert::From<::buffa::alloc::vec::Vec<u8>> for #ident #ty_generics #where_clause {
            #[inline]
            fn from(v: ::buffa::alloc::vec::Vec<u8>) -> Self {
                #ctor_from_vec
            }
        }

        impl #impl_generics ::buffa::ProtoBytes for #ident #ty_generics #where_clause {
            #[inline]
            fn from_wire(
                payload: ::buffa::WirePayload<'_>,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                ::core::result::Result::Ok(#ctor_from_wire)
            }
        }
    })
}

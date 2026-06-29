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

    let from_string = remote_field::qualified_call(
        field_ty,
        quote! { ::core::convert::From<::buffa::alloc::string::String> },
        "from",
    );
    let from_str =
        remote_field::qualified_call(field_ty, quote! { ::core::convert::From<&str> }, "from");
    // Fully qualified, not `#accessor.as_ref()` — a remote type implementing
    // more than one `AsRef<_>` (e.g. both `AsRef<str>` and `AsRef<[u8]>`)
    // makes plain method-call syntax ambiguous, since method resolution
    // doesn't use the caller's expected return type to disambiguate.
    let as_str =
        remote_field::qualified_call(field_ty, quote! { ::core::convert::AsRef<str> }, "as_ref");

    let ctor_from_string = remote.construct(quote! { #from_string(s) });
    let ctor_from_str = remote.construct(quote! { #from_str(s) });
    let ctor_from_wire = remote.construct(quote! { #from_str(s) });

    Ok(quote! {
        impl #impl_generics ::core::ops::Deref for #ident #ty_generics #where_clause {
            type Target = str;
            #[inline]
            fn deref(&self) -> &str {
                #as_str(&#accessor)
            }
        }

        impl #impl_generics ::core::convert::AsRef<str> for #ident #ty_generics #where_clause {
            #[inline]
            fn as_ref(&self) -> &str {
                #as_str(&#accessor)
            }
        }

        impl #impl_generics ::core::convert::From<::buffa::alloc::string::String> for #ident #ty_generics #where_clause {
            #[inline]
            fn from(s: ::buffa::alloc::string::String) -> Self {
                #ctor_from_string
            }
        }

        impl #impl_generics ::core::convert::From<&str> for #ident #ty_generics #where_clause {
            #[inline]
            fn from(s: &str) -> Self {
                #ctor_from_str
            }
        }

        impl #impl_generics ::buffa::ProtoString for #ident #ty_generics #where_clause {
            #[inline]
            fn from_wire(
                payload: ::buffa::WirePayload<'_>,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                payload.to_str().map(|s| #ctor_from_wire)
            }
        }
    })
}

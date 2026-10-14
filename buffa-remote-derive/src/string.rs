use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::forwarders;
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

    let ctor_from_string = remote.construct(quote! { #from_string(__buffa_string) });
    let ctor_from_str = remote.construct(quote! { #from_str(__buffa_str) });

    let arbitrary_impl = forwarders::arbitrary(
        &remote,
        &quote! { ::buffa::alloc::string::String },
        &remote.construct(quote! { #from_string(__buffa_seed) }),
        forwarders::TakeRest::Seed,
        &[],
    );

    let serde_impl = forwarders::serde(&remote, forwarders::SerdeForm::String);

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
            fn from(__buffa_string: ::buffa::alloc::string::String) -> Self {
                #ctor_from_string
            }
        }

        impl #impl_generics ::core::convert::From<&str> for #ident #ty_generics #where_clause {
            #[inline]
            fn from(__buffa_str: &str) -> Self {
                #ctor_from_str
            }
        }

        impl #impl_generics ::buffa::ProtoString for #ident #ty_generics #where_clause {
            #[inline]
            fn copy_from_str(__buffa_str: &str) -> Self {
                #ctor_from_str
            }

            #[inline]
            fn from_wire(
                __buffa_payload: ::buffa::WirePayload<'_>,
            ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
                __buffa_payload.to_str().map(|__buffa_str| #ctor_from_str)
            }
        }

        #arbitrary_impl
        #serde_impl
    })
}

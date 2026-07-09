use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::remote_field::{self, RemoteField};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let (remote, overrides) = remote_field::parse_with_overrides(&input, &["new", "into_inner"])?;
    let RemoteField {
        ident,
        generics,
        field_ty,
        accessor,
        ..
    } = &remote;

    let element_ty = remote_field::single_type_param(generics)?;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Defaults assume the remote pointer follows the `Type::new(value) ->
    // Self` / `Type::into_inner(self) -> T` convention (e.g.
    // `smallbox::SmallBox`); override with `#[buffa(remote = ..., new = path,
    // into_inner = path)]` for a remote type that names them differently —
    // notably plain `std::boxed::Box`, whose `into_inner` is nightly-only
    // (`box_into_inner`), so wrapping `Box<T>` with this derive needs an
    // `into_inner` override (or just use buffa's built-in `Box<T>` impl,
    // which never needs this derive).
    let new_call = remote_field::overridable_call(&overrides, "new", field_ty, "new");
    let into_inner_call =
        remote_field::overridable_call(&overrides, "into_inner", field_ty, "into_inner");

    let ctor_new = remote.construct(quote! { #new_call(value) });

    Ok(quote! {
        impl #impl_generics ::core::ops::Deref for #ident #ty_generics #where_clause {
            type Target = #element_ty;
            #[inline]
            fn deref(&self) -> &#element_ty {
                &#accessor
            }
        }

        impl #impl_generics ::core::ops::DerefMut for #ident #ty_generics #where_clause {
            #[inline]
            fn deref_mut(&mut self) -> &mut #element_ty {
                &mut #accessor
            }
        }

        impl #impl_generics ::buffa::ProtoBox<#element_ty> for #ident #ty_generics #where_clause {
            #[inline]
            fn new(value: #element_ty) -> Self {
                #ctor_new
            }

            #[inline]
            fn into_inner(self) -> #element_ty {
                #into_inner_call(#accessor)
            }
        }
    })
}

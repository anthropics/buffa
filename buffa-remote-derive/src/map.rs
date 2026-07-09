use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::remote_field::{self, RemoteField};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let (remote, overrides) =
        remote_field::parse_with_overrides(&input, &["len", "insert", "clear", "iter"])?;
    let RemoteField {
        ident,
        generics,
        field_ty,
        accessor,
        ..
    } = &remote;

    let (key_ty, value_ty) = remote_field::two_type_params(generics)?;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Defaults assume the near-universal map naming convention (`HashMap`,
    // `BTreeMap`, `indexmap::IndexMap`, `dashmap::DashMap` all use these
    // names); override with `#[buffa(remote = ..., insert = path, ...)]` for
    // a remote map that names them differently.
    let len_call = remote_field::overridable_call(&overrides, "len", field_ty, "len");
    let insert_call = remote_field::overridable_call(&overrides, "insert", field_ty, "insert");
    let clear_call = remote_field::overridable_call(&overrides, "clear", field_ty, "clear");
    let iter_call = remote_field::overridable_call(&overrides, "iter", field_ty, "iter");

    Ok(quote! {
        impl #impl_generics ::buffa::MapStorage for #ident #ty_generics #where_clause {
            type Key = #key_ty;
            type Value = #value_ty;

            #[inline]
            fn storage_len(&self) -> usize {
                #len_call(&#accessor)
            }

            #[inline]
            fn storage_insert(&mut self, key: #key_ty, value: #value_ty) {
                #insert_call(&mut #accessor, key, value);
            }

            #[inline]
            fn storage_clear(&mut self) {
                #clear_call(&mut #accessor);
            }

            #[inline]
            fn storage_iter<'a>(
                &'a self,
            ) -> impl ::core::iter::Iterator<Item = (&'a #key_ty, &'a #value_ty)>
            where
                #key_ty: 'a,
                #value_ty: 'a,
            {
                #iter_call(&#accessor)
            }
        }
    })
}

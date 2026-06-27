use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields};

/// The single field a remote-derive newtype wraps, plus the struct's name,
/// generics, and the `remote` path it claims to wrap.
pub struct RemoteField {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    pub remote_ty: syn::Type,
    /// `self.0` for a tuple struct, `self.field_name` for a named-field one.
    pub accessor: TokenStream,
    /// `Some(name)` for a named-field struct, `None` for a tuple struct —
    /// used to build a `Self { name: value }` vs. `Self(value)` constructor.
    pub field_name: Option<syn::Ident>,
}

/// Extracts the single field from a tuple or named-field struct, and the
/// `#[buffa(remote = ...)]` attribute naming the wrapped foreign type.
///
/// The generated code always operates on the field's actual type, never on
/// the type written in the attribute — comparing the two would require
/// resolving `use` imports and module paths to decide whether two *spellings*
/// name the same type, which isn't possible from within a derive macro. The
/// attribute is therefore documentation, not codegen input: it must be
/// present (so the newtype's purpose is legible without reading the field
/// declaration) and must parse as a type (catching outright typos), but its
/// content is not checked against the field.
///
/// Requires the struct to have exactly one field (newtype shape).
pub fn parse(input: &DeriveInput) -> syn::Result<RemoteField> {
    let (field_ty, accessor, field_name) = single_field(input)?;
    require_remote_attr(input)?;

    Ok(RemoteField {
        ident: input.ident.clone(),
        generics: input.generics.clone(),
        remote_ty: field_ty,
        accessor,
        field_name,
    })
}

fn single_field(input: &DeriveInput) -> syn::Result<(syn::Type, TokenStream, Option<syn::Ident>)> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "this derive only applies to a single-field newtype struct",
        ));
    };
    match &data.fields {
        Fields::Named(f) if f.named.len() == 1 => {
            let field = &f.named[0];
            let name = field.ident.as_ref().expect("named field has an ident");
            Ok((field.ty.clone(), quote! { self.#name }, Some(name.clone())))
        }
        Fields::Unnamed(f) if f.unnamed.len() == 1 => {
            let field = &f.unnamed[0];
            Ok((field.ty.clone(), quote! { self.0 }, None))
        }
        Fields::Named(f) => Err(syn::Error::new(
            input.span(),
            format!(
                "this derive requires exactly one field wrapping the remote type, found {}",
                f.named.len()
            ),
        )),
        Fields::Unnamed(f) => Err(syn::Error::new(
            input.span(),
            format!(
                "this derive requires exactly one field wrapping the remote type, found {}",
                f.unnamed.len()
            ),
        )),
        Fields::Unit => Err(syn::Error::new(
            input.span(),
            "this derive requires exactly one field wrapping the remote type",
        )),
    }
}

/// Validates that `#[buffa(remote = ...)]` is present and its value parses as
/// a type (catching typos), without using the parsed type for codegen — see
/// [`parse`] for why.
fn require_remote_attr(input: &DeriveInput) -> syn::Result<()> {
    for attr in &input.attrs {
        if !attr.path().is_ident("buffa") {
            continue;
        }
        let mut found = false;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("remote") {
                let _: syn::Type = meta.value()?.parse()?;
                found = true;
                Ok(())
            } else {
                Err(meta.error("unsupported `buffa` attribute key, expected `remote`"))
            }
        })?;
        if found {
            return Ok(());
        }
    }
    Err(syn::Error::new(
        input.span(),
        "missing `#[buffa(remote = ...)]` naming the foreign type this newtype wraps",
    ))
}

/// Renders a `<Remote as Trait>::method` fully-qualified call path, for
/// disambiguating which impl a generated body invokes.
pub fn qualified_call(remote_ty: &syn::Type, trait_path: TokenStream, method: &str) -> TokenStream {
    let method = syn::Ident::new(method, proc_macro2::Span::call_site());
    quote! { <#remote_ty as #trait_path>::#method }
}

impl RemoteField {
    /// Builds `Self(value)` or `Self { field_name: value }`, matching whichever
    /// shape the wrapped struct uses.
    pub fn construct(&self, value: TokenStream) -> TokenStream {
        match &self.field_name {
            Some(name) => quote! { Self { #name: #value } },
            None => quote! { Self(#value) },
        }
    }
}

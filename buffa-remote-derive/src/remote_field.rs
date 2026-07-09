use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, GenericParam};

/// The single field a remote-derive newtype wraps, plus the struct's name and
/// generics.
pub struct RemoteField {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    /// The wrapped field's actual type — not the type written in the
    /// `#[buffa(remote = ...)]` attribute, which is documentation only and
    /// never read for codegen (see [`parse`]).
    pub field_ty: syn::Type,
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
    parse_with_overrides(input, &[]).map(|(remote, _)| remote)
}

/// Like [`parse`], but also collects any of `allowed_overrides` present in
/// `#[buffa(remote = ..., key = path, ...)]` as `syn::Path`s — used by derives
/// (`ProtoBox`, `MapStorage`) whose reference implementations call **inherent**
/// methods on the remote type (e.g. `into_inner()`, `insert()`) rather than
/// trait methods, so the method path can't be synthesized generically and
/// instead defaults to the common naming convention with an escape hatch to
/// override it.
pub fn parse_with_overrides(
    input: &DeriveInput,
    allowed_overrides: &[&str],
) -> syn::Result<(RemoteField, HashMap<String, syn::Path>)> {
    let overrides = parse_overrides(input, allowed_overrides)?;
    let (field_ty, accessor, field_name) = single_field(input)?;

    Ok((
        RemoteField {
            ident: input.ident.clone(),
            generics: input.generics.clone(),
            field_ty,
            accessor,
            field_name,
        },
        overrides,
    ))
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
/// a type (catching typos, without using the parsed type for codegen — see
/// [`parse`] for why), and collects any of `allowed_overrides` present
/// alongside it (e.g. `#[buffa(remote = ..., into_inner = MyType::unwrap)]`).
fn parse_overrides(
    input: &DeriveInput,
    allowed_overrides: &[&str],
) -> syn::Result<HashMap<String, syn::Path>> {
    let mut overrides = HashMap::new();
    let mut has_remote = false;
    for attr in &input.attrs {
        if !attr.path().is_ident("buffa") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("remote") {
                let _: syn::Type = meta.value()?.parse()?;
                has_remote = true;
                Ok(())
            } else if let Some(key) = allowed_overrides
                .iter()
                .find(|key| meta.path.is_ident(*key))
            {
                let path: syn::Path = meta.value()?.parse()?;
                overrides.insert((*key).to_string(), path);
                Ok(())
            } else {
                Err(meta.error(format!(
                    "unsupported `buffa` attribute key, expected `remote`{}",
                    allowed_overrides
                        .iter()
                        .map(|k| format!(" or `{k}`"))
                        .collect::<String>()
                )))
            }
        })?;
    }
    if !has_remote {
        return Err(syn::Error::new(
            input.span(),
            "missing `#[buffa(remote = ...)]` naming the foreign type this newtype wraps",
        ));
    }
    Ok(overrides)
}

/// Requires the struct to have exactly one type parameter and returns it —
/// used by derives whose element type is the struct's sole generic parameter
/// (`ProtoList<T>`'s element, `ProtoBox<T>`'s pointee). A struct with more
/// than one type parameter (e.g. a custom hasher parameter) is out of scope;
/// hand-write the impl in that case. Lifetime and const generics don't count
/// toward the limit.
pub fn single_type_param(generics: &syn::Generics) -> syn::Result<syn::Ident> {
    match type_params(generics).as_slice() {
        [single] => Ok(single.clone()),
        _ => Err(syn::Error::new_spanned(
            &generics.params,
            "this derive requires exactly one type parameter",
        )),
    }
}

/// Like [`single_type_param`], but for derives keyed on two type parameters
/// in declaration order (`MapStorage`'s `Key`, `Value`).
pub fn two_type_params(generics: &syn::Generics) -> syn::Result<(syn::Ident, syn::Ident)> {
    match type_params(generics).as_slice() {
        [key, value] => Ok((key.clone(), value.clone())),
        _ => Err(syn::Error::new_spanned(
            &generics.params,
            "this derive requires exactly two type parameters, the map's key and value types, \
             in that order",
        )),
    }
}

fn type_params(generics: &syn::Generics) -> Vec<syn::Ident> {
    generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(t.ident.clone()),
            _ => None,
        })
        .collect()
}

/// Renders a `<Remote as Trait>::method` fully-qualified call path, for
/// disambiguating which impl a generated body invokes.
pub fn qualified_call(field_ty: &syn::Type, trait_path: TokenStream, method: &str) -> TokenStream {
    let method = syn::Ident::new(method, proc_macro2::Span::call_site());
    quote! { <#field_ty as #trait_path>::#method }
}

/// Resolves an overridable inherent-method call path: the user's
/// `#[buffa(remote = ..., key = path)]` override if present, otherwise
/// `<Remote>::default_method`. Not built through `syn::Path` — a leading
/// `<Type>::` qualified-self segment isn't valid plain-`Path` syntax, only
/// valid as a qualified-path *expression*, so the default has to be assembled
/// as a `TokenStream` directly rather than round-tripped through `syn::Path`
/// like the override is.
pub fn overridable_call(
    overrides: &HashMap<String, syn::Path>,
    key: &str,
    field_ty: &syn::Type,
    default_method: &str,
) -> TokenStream {
    match overrides.get(key) {
        Some(path) => quote! { #path },
        None => {
            let method = syn::Ident::new(default_method, proc_macro2::Span::call_site());
            quote! { <#field_ty>::#method }
        }
    }
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

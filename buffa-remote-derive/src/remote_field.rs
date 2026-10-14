use std::collections::hash_map::Entry;
use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, GenericParam};

use crate::forwarders::Flags;

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
    /// The optional impls asked for with bare `#[buffa(..)]` keys (see
    /// [`crate::forwarders`]). Every derive honors them, so they are parsed
    /// here rather than per-derive.
    pub flags: Flags,
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
/// whose generated impl needs a caller-supplied method path. Two modes exist:
/// replacing a conventional inherent-method default (`ProtoBox`'s
/// `new`/`into_inner`, `MapStorage`'s `len`/`insert`/`clear`/`iter`, resolved
/// through [`overridable_call`]), and enabling an optional hook with no
/// default at all (`ProtoBytes`'s `as_shared`, where an absent key means the
/// method is not generated and the trait default applies).
pub fn parse_with_overrides(
    input: &DeriveInput,
    allowed_overrides: &[&str],
) -> syn::Result<(RemoteField, HashMap<String, syn::Path>)> {
    let (overrides, flags) = parse_attrs(input, allowed_overrides)?;
    let (field_ty, accessor, field_name) = single_field(input)?;

    Ok((
        RemoteField {
            ident: input.ident.clone(),
            generics: input.generics.clone(),
            field_ty,
            accessor,
            field_name,
            flags,
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
/// [`parse`] for why), and collects the other keys present alongside it: any of
/// `allowed_overrides` as a `key = path` pair (e.g.
/// `#[buffa(remote = ..., into_inner = MyType::unwrap)]`), and the bare
/// `arbitrary` and `serde` keys every derive accepts, in the same attribute
/// or another.
fn parse_attrs(
    input: &DeriveInput,
    allowed_overrides: &[&str],
) -> syn::Result<(HashMap<String, syn::Path>, Flags)> {
    let mut overrides = HashMap::new();
    let mut flags = Flags::default();
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
            } else if meta.path.is_ident("serde") {
                if meta.input.peek(syn::Token![=]) {
                    return Err(meta.error(
                        "`serde` takes no value; write it as a bare key, \
                         `#[buffa(remote = ..., serde)]`, or as \
                         `#[cfg_attr(<condition>, buffa(serde))]` to make the impls \
                         conditional",
                    ));
                }
                // A repeat is accepted: two `cfg_attr`s that both hold look
                // the same here as a key written twice.
                flags.serde.get_or_insert(meta.path.span());
                Ok(())
            } else if meta.path.is_ident("arbitrary") {
                if meta.input.peek(syn::Token![=]) {
                    return Err(meta.error(
                        "`arbitrary` takes no value; write it as a bare key, \
                         `#[buffa(remote = ..., arbitrary)]`, or as \
                         `#[cfg_attr(<condition>, buffa(arbitrary))]` to make the impl \
                         conditional",
                    ));
                }
                // A repeat is accepted: two `cfg_attr`s that both hold look
                // the same here as a key written twice.
                flags.arbitrary.get_or_insert(meta.path.span());
                Ok(())
            } else if let Some(key) = allowed_overrides
                .iter()
                .find(|key| meta.path.is_ident(*key))
            {
                let path: syn::Path = meta.value()?.parse()?;
                match overrides.entry((*key).to_string()) {
                    Entry::Vacant(slot) => {
                        slot.insert(path);
                        Ok(())
                    }
                    // A repeat of the same path is accepted: two `cfg_attr`s
                    // that both hold look the same here as a key written
                    // twice. Two different paths have no such reading.
                    Entry::Occupied(first) if same_tokens(first.get(), &path) => Ok(()),
                    Entry::Occupied(first) => {
                        let mut error = meta.error(format!(
                            "conflicting `{key}` override: `{}` was already given",
                            path_text(first.get())
                        ));
                        error.combine(syn::Error::new_spanned(
                            first.get(),
                            format!("`{key}` first given here"),
                        ));
                        Err(error)
                    }
                }
            } else {
                Err(meta.error(format!(
                    "unsupported `buffa` attribute key, expected `remote`, `arbitrary` or `serde`{}",
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
    Ok((overrides, flags))
}

/// Whether two paths are written with the same tokens. `a::b` and `::a::b`,
/// or two spellings that resolve to one item, compare unequal: a derive
/// cannot resolve paths.
fn same_tokens(a: &syn::Path, b: &syn::Path) -> bool {
    a.to_token_stream().to_string() == b.to_token_stream().to_string()
}

/// A compact rendering of `path`: `TokenStream`'s `Display` output without
/// the spaces it puts between tokens, except a space that separates two words
/// (`dyn Any`).
fn path_text(path: &syn::Path) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut text = String::new();
    let mut after_space = false;
    for c in path.to_token_stream().to_string().chars() {
        if c == ' ' {
            after_space = true;
            continue;
        }
        if after_space && is_word(c) && text.chars().next_back().is_some_and(is_word) {
            text.push(' ');
        }
        after_space = false;
        text.push(c);
    }
    text
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicting_override_in_one_attribute_is_rejected() {
        let input: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>, new = Holder::wrap, new = Holder::new)]
            struct Wrapped<T>(Holder<T>);
        };

        let error = parse_with_overrides(&input, &["new"])
            .err()
            .expect("a second, different path should be rejected");
        assert_eq!(
            error.to_string(),
            "conflicting `new` override: `Holder::wrap` was already given"
        );
    }

    #[test]
    fn conflicting_override_across_attributes_is_rejected() {
        let input: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>, new = Holder::wrap)]
            #[buffa(new = Holder::new)]
            struct Wrapped<T>(Holder<T>);
        };

        let error = parse_with_overrides(&input, &["new"])
            .err()
            .expect("a second, different path should be rejected");
        assert_eq!(
            error.to_string(),
            "conflicting `new` override: `Holder::wrap` was already given"
        );
    }

    #[test]
    fn identical_override_repeat_is_accepted() {
        // What two `cfg_attr`s that both hold expand to, and the same key
        // written twice in one attribute.
        let across: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>)]
            #[buffa(new = Holder::wrap)]
            #[buffa(new = Holder::wrap)]
            struct Wrapped<T>(Holder<T>);
        };
        let within: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>, new = Holder::wrap, new = Holder::wrap)]
            struct Wrapped<T>(Holder<T>);
        };

        for input in [across, within] {
            let (_, overrides) = parse_with_overrides(&input, &["new"])
                .unwrap_or_else(|e| panic!("identical repeat should be accepted: {e}"));
            assert_eq!(overrides.len(), 1);
            assert_eq!(path_text(&overrides["new"]), "Holder::wrap");
        }
    }

    #[test]
    fn conflicting_override_reports_both_sites() {
        let input: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>, new = Holder::wrap)]
            #[buffa(new = Holder::new)]
            struct Wrapped<T>(Holder<T>);
        };

        let error = parse_with_overrides(&input, &["new"])
            .err()
            .expect("a second, different path should be rejected");
        let messages: Vec<String> = error.into_iter().map(|e| e.to_string()).collect();
        assert_eq!(
            messages,
            [
                "conflicting `new` override: `Holder::wrap` was already given",
                "`new` first given here",
            ]
        );
    }

    #[test]
    fn path_text_keeps_the_space_between_two_words() {
        let path: syn::Path = syn::parse_quote!(Holder::<dyn Any, &'a mut [u8; 4]>::wrap);
        assert_eq!(path_text(&path), "Holder::<dyn Any,&'a mut[u8;4]>::wrap");
    }

    #[test]
    fn override_repeat_with_a_different_spelling_is_rejected() {
        // The comparison is by tokens: a derive cannot tell that two
        // spellings resolve to one function.
        let input: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>, new = Holder::wrap, new = self::Holder::wrap)]
            struct Wrapped<T>(Holder<T>);
        };

        assert!(parse_with_overrides(&input, &["new"]).is_err());
    }

    #[test]
    fn distinct_overrides_across_attributes_are_accepted() {
        let input: DeriveInput = syn::parse_quote! {
            #[buffa(remote = Holder<T>, new = Holder::wrap)]
            #[buffa(into_inner = Holder::unwrap)]
            struct Wrapped<T>(Holder<T>);
        };

        assert!(parse_with_overrides(&input, &["new", "into_inner"]).is_ok());
    }
}

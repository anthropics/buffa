//! Unit tests for the `repeated_type` custom-collection template handling
//! (`parse_custom_list_path` + the `RepeatedRepr` knob), and for the scalar
//! custom-type path parser (`parse_custom_type_path`) the same rules share.

use crate::{parse_custom_list_path, parse_custom_type_path, CodeGenError, RepeatedRepr};
use quote::quote;

#[test]
fn substitutes_element_into_template() {
    let got = parse_custom_list_path("::my_crate::SmallList<*>", &quote! { u32 }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::my_crate::SmallList<u32> }.to_string()
    );
}

#[test]
fn substitutes_into_array_wrapped_generic() {
    // The element sits inside an array inside the generic — the case the scalar
    // `*_custom` path-string mechanism could not express.
    let got = parse_custom_list_path("::smallvec::SmallVec<[*; 4]>", &quote! { i64 }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::smallvec::SmallVec<[i64; 4]> }.to_string()
    );
}

#[test]
fn substitutes_nested_generic_element() {
    let elem = quote! { ::buffa::EnumValue<my::Color> };
    let got = parse_custom_list_path("::my_crate::List<*>", &elem).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::my_crate::List<::buffa::EnumValue<my::Color> > }.to_string()
    );
}

#[test]
fn missing_placeholder_is_a_distinct_error() {
    // A complete path (valid Rust type, but no `*`) — the most likely mistake
    // for someone used to the scalar `string_type_custom` knobs.
    let err = parse_custom_list_path("::smallvec::SmallVec<u32>", &quote! { u32 }).unwrap_err();
    assert!(matches!(err, CodeGenError::MissingListPlaceholder(_)));
}

#[test]
fn unparseable_substitution_is_invalid_type_path() {
    let err = parse_custom_list_path("List<*", &quote! { u32 }).unwrap_err();
    assert!(matches!(err, CodeGenError::InvalidTypePath(_)));
}

#[test]
fn non_path_collection_is_invalid_type_path() {
    // Each parses as a Rust type once `*` is substituted, but none is a type
    // path a crate could implement `ProtoList` for.
    for template in [
        "[*; 4]",
        "(*,)",
        "&'static [*]",
        "impl Iterator<Item = *>",
        "([*; 4])",
    ] {
        let err = parse_custom_list_path(template, &quote! { u32 }).unwrap_err();
        assert!(
            matches!(&err, CodeGenError::InvalidTypePath(msg) if msg.contains(template)),
            "{template}: {err:?}"
        );
    }
}

#[test]
fn custom_type_path_accepts_paths_including_projections_and_parentheses() {
    for path in [
        "crate::SecretBytes",
        "::smol_str::SmolStr",
        "Local",
        "crate::Wrap<u8>",
        "<crate::A as crate::B>::C",
        "(crate::SecretBytes)",
        "((<crate::A as crate::B>::C))",
    ] {
        let got = parse_custom_type_path(path).unwrap();
        let expected: syn::Type = syn::parse_str(path).unwrap();
        assert_eq!(got.to_string(), quote! { #expected }.to_string());
    }
}

#[test]
fn custom_type_path_rejects_types_that_cannot_be_a_field_representation() {
    for path in [
        "_",
        "()",
        "[u8; 4]",
        "&'static [u8]",
        "impl AsRef<[u8]>",
        "dyn AsRef<[u8]>",
        "fn() -> u8",
        "(())",
        "([u8; 4])",
        "crate::",
    ] {
        let err = parse_custom_type_path(path).unwrap_err();
        assert!(
            matches!(&err, CodeGenError::InvalidTypePath(msg) if msg.contains(path)),
            "{path}: {err:?}"
        );
    }
}

#[test]
fn custom_list_path_accepts_projections_and_parentheses() {
    for template in [
        "<crate::A as crate::B>::List<*>",
        "<* as crate::B>::List",
        "(crate::List<*>)",
        "((<crate::A as crate::B>::List<*>))",
    ] {
        let got = parse_custom_list_path(template, &quote! { u32 }).unwrap();
        let expected: syn::Type = syn::parse_str(&template.replace('*', "u32")).unwrap();
        assert_eq!(got.to_string(), quote! { #expected }.to_string());
    }
}

#[test]
fn vec_repr_is_default_custom_is_not() {
    assert!(RepeatedRepr::default().is_default());
    assert!(RepeatedRepr::Vec.is_default());
    assert!(!RepeatedRepr::Custom("::x::L<*>".to_string()).is_default());
}

//! Unit tests for the `repeated_type` custom-collection template handling
//! (`parse_custom_list_path` + the `RepeatedRepr` knob).

use crate::{parse_custom_list_path, CodeGenError, RepeatedRepr};
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
fn vec_repr_is_default_custom_is_not() {
    assert!(RepeatedRepr::default().is_default());
    assert!(RepeatedRepr::Vec.is_default());
    assert!(!RepeatedRepr::Custom("::x::L<*>".to_string()).is_default());
}

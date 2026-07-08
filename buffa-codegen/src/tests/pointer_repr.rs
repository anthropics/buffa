//! Unit tests for the `box_type` pluggable-pointer template handling
//! (`parse_wildcard_type_path` + `PointerRepr`).

use crate::{parse_wildcard_type_path, CodeGenError, PointerRepr};
use quote::quote;

#[test]
fn substitutes_message_type_into_template() {
    let got = parse_wildcard_type_path("::my_crate::SmallBox<*>", &quote! { my::Msg }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::my_crate::SmallBox<my::Msg> }.to_string()
    );
}

#[test]
fn substitutes_into_multi_param_generic() {
    // The message type sits among extra pointer params — the case a complete
    // (non-templated) path could not express.
    let got = parse_wildcard_type_path(
        "::smallbox::SmallBox<*, ::smallbox::space::S4>",
        &quote! { Msg },
    )
    .unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::smallbox::SmallBox<Msg, ::smallbox::space::S4> }.to_string()
    );
}

#[test]
fn missing_placeholder_is_a_distinct_error() {
    let err = parse_wildcard_type_path("::my_crate::SmallBox", &quote! { Msg }).unwrap_err();
    assert!(matches!(err, CodeGenError::MissingWildcard(_)));
}

#[test]
fn unparseable_substitution_is_invalid_type_path() {
    let err = parse_wildcard_type_path("Box<*", &quote! { Msg }).unwrap_err();
    assert!(matches!(err, CodeGenError::InvalidTypePath(_)));
}

#[test]
fn box_type_path_is_message_field_without_pointer_param() {
    let mf = quote! { ::buffa::MessageField };
    let got = PointerRepr::Box.type_path(&mf, &quote! { Inner }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::MessageField<Inner> }.to_string()
    );
}

#[test]
fn custom_type_path_threads_pointer_param() {
    let mf = quote! { ::buffa::MessageField };
    let repr = PointerRepr::Custom("::my::SBox<*>".to_string());
    let got = repr.type_path(&mf, &quote! { Inner }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::MessageField<Inner, ::my::SBox<Inner> > }.to_string()
    );
}

#[test]
fn inline_type_path_uses_buffa_inline() {
    let mf = quote! { ::buffa::MessageField };
    let got = PointerRepr::Inline
        .type_path(&mf, &quote! { Inner })
        .unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::MessageField<Inner, ::buffa::Inline<Inner>> }.to_string()
    );
}

#[test]
fn inline_some_path_carries_pointer() {
    let got = PointerRepr::Inline.some_path(&quote! { Inner }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::MessageField::<Inner, ::buffa::Inline<Inner>> }.to_string()
    );
}

#[test]
fn inline_pointer_new_is_tuple_constructor() {
    let got = PointerRepr::Inline
        .pointer_new(&quote! { Inner }, &quote! { v })
        .unwrap();
    assert_eq!(got.to_string(), quote! { ::buffa::Inline(v) }.to_string());
}

#[test]
fn custom_some_path_carries_pointer() {
    let repr = PointerRepr::Custom("::my::SBox<*>".to_string());
    let got = repr.some_path(&quote! { Inner }).unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::MessageField::<Inner, ::my::SBox<Inner> > }.to_string()
    );
}

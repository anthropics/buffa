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

/// Lazy `to_owned_message` builds a repeated message field in a `Vec`. A
/// custom collection must take that `Vec` through `From`; the default `Vec`
/// must not, or the generated code would trip `clippy::useless_conversion`.
#[test]
fn lazy_to_owned_converts_vec_only_for_custom_collection() {
    use super::{joined, make_field, proto3_file};
    use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
    use crate::generated::descriptor::DescriptorProto;
    use crate::{generate, CodeGenConfig};

    let mut file = proto3_file("lazy_rep.proto");
    file.package = Some("lr".to_string());
    let mut items = make_field("items", 1, Label::LABEL_REPEATED, Type::TYPE_MESSAGE);
    items.type_name = Some(".lr.Inner".to_string());
    file.message_type.push(DescriptorProto {
        name: Some("Outer".to_string()),
        field: vec![items],
        ..Default::default()
    });
    file.message_type.push(DescriptorProto {
        name: Some("Inner".to_string()),
        field: vec![make_field("id", 1, Label::LABEL_OPTIONAL, Type::TYPE_INT32)],
        ..Default::default()
    });
    let render = |repeated_fields| {
        let config = CodeGenConfig {
            lazy_views: true,
            repeated_fields,
            ..Default::default()
        };
        joined(&generate(&[file.clone()], &["lazy_rep.proto".to_string()], &config).unwrap())
    };

    let conversion = "::core::convert::From::from(__out)";
    let default = render(Vec::new());
    assert!(default.contains("pub struct OuterLazyView"), "{default}");
    assert!(!default.contains(conversion), "{default}");
    let custom = render(vec![(
        ".".to_string(),
        RepeatedRepr::Custom("crate::L<*>".to_string()),
    )]);
    assert!(custom.contains(conversion), "{custom}");
}

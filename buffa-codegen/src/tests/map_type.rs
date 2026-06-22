//! Unit tests for the `map_type` custom-collection handling (`MapRepr` knob).
//!
//! Unlike the `repeated` knob (a `*`-templated path), a map type is always
//! `path<K, V>` with both parameters positional, so `MapRepr::type_path` just
//! wraps the resolved key/value tokens.

use crate::context::CodeGenContext;
use crate::generated::descriptor::FileDescriptorProto;
use crate::imports::ImportResolver;
use crate::{CodeGenConfig, MapRepr};
use quote::quote;

/// An empty descriptor set is enough: the BTreeMap / Custom branches never
/// consult the context, and the HashMap branch only needs the package-root
/// import registry (which the empty context still provides).
fn empty_ctx_config() -> (Vec<FileDescriptorProto>, CodeGenConfig) {
    (Vec::new(), CodeGenConfig::default())
}

#[test]
fn hashmap_is_default_others_are_not() {
    assert!(MapRepr::default().is_default());
    assert!(MapRepr::HashMap.is_default());
    assert!(!MapRepr::BTreeMap.is_default());
    assert!(!MapRepr::Custom("::x::M".to_string()).is_default());
}

#[test]
fn hashmap_type_path_uses_private_hashmap() {
    let (files, config) = empty_ctx_config();
    let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
    let resolver = ImportResolver::new();
    let got = MapRepr::HashMap
        .type_path(&quote! { String }, &quote! { i32 }, &resolver, &ctx, 0)
        .unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::__private::HashMap<String, i32> }.to_string()
    );
}

#[test]
fn btreemap_type_path_is_alloc_btreemap() {
    let (files, config) = empty_ctx_config();
    let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
    let resolver = ImportResolver::new();
    let got = MapRepr::BTreeMap
        .type_path(&quote! { String }, &quote! { i32 }, &resolver, &ctx, 0)
        .unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::buffa::alloc::collections::BTreeMap<String, i32> }.to_string()
    );
}

#[test]
fn custom_type_path_wraps_key_and_value_positionally() {
    let (files, config) = empty_ctx_config();
    let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
    let resolver = ImportResolver::new();
    let got = MapRepr::Custom("::my_crate::OrderedMap".to_string())
        .type_path(&quote! { u64 }, &quote! { MyMsg }, &resolver, &ctx, 0)
        .unwrap();
    assert_eq!(
        got.to_string(),
        quote! { ::my_crate::OrderedMap<u64, MyMsg> }.to_string()
    );
}

#[test]
fn custom_unparseable_path_is_invalid_type_path() {
    let (files, config) = empty_ctx_config();
    let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
    let resolver = ImportResolver::new();
    let err = MapRepr::Custom("not a path!".to_string())
        .type_path(&quote! { u64 }, &quote! { i32 }, &resolver, &ctx, 0)
        .unwrap_err();
    assert!(matches!(err, crate::CodeGenError::InvalidTypePath(_)));
}

#[test]
fn custom_path_with_own_generics_is_rejected_clearly() {
    // A path that already carries `<...>` would expand to `Foo<Bar><K,V>`.
    let err = crate::parse_custom_map_path("::my::Foo<Bar>").unwrap_err();
    assert!(matches!(err, crate::CodeGenError::InvalidTypePath(_)));
}

#[test]
fn custom_path_with_wildcard_is_rejected() {
    // Maps don't use the `*` placeholder the box/repeated knobs do.
    let err = crate::parse_custom_map_path("::my::Foo<*>").unwrap_err();
    assert!(matches!(err, crate::CodeGenError::InvalidTypePath(_)));
}

#[test]
fn custom_bare_path_is_accepted() {
    let got = crate::parse_custom_map_path("::my::OrderedMap").unwrap();
    assert_eq!(got.to_string(), quote! { ::my::OrderedMap }.to_string());
}

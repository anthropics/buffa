//! Helpers for emitting `#[cfg(feature = "...")]` / `#[cfg_attr(...)]`
//! wrappers around generated impls.
//!
//! Wired through [`CodeGenConfig::gate_impls_on_crate_features`]. When that
//! flag is off (the default), every helper is a no-op so the conditional
//! call-sites in `message.rs`/`enumeration.rs`/`oneof.rs`/etc. produce the
//! exact same tokens as before — which is what most consumers want: they
//! decide at build-script time whether to generate JSON, and the resulting
//! code carries a hard dependency on the runtime support.
//!
//! When the flag is on, the json/views/text impls are wrapped in `#[cfg]`
//! so the consuming crate can feature-gate them. That lets `buffa-descriptor`
//! and `buffa-types` ship every impl while keeping the codegen toolchain
//! lean (it deps on them with `default-features = false`).
//!
//! [`CodeGenConfig::gate_impls_on_crate_features`]: crate::CodeGenConfig::gate_impls_on_crate_features

use proc_macro2::TokenStream;
use quote::quote;

use crate::CodeGenConfig;

/// Crate feature names the gated impls are conditioned on.
///
/// Fixed for v1; the consuming crate must define matching features in its
/// `Cargo.toml`. Customisable names can be added later as a separate config
/// field if a concrete need arises.
pub(crate) const JSON_FEATURE: &str = "json";
pub(crate) const VIEWS_FEATURE: &str = "views";
pub(crate) const TEXT_FEATURE: &str = "text";

/// Resolved feature-gate names for the current codegen run, computed once
/// from [`CodeGenConfig`] and threaded through codegen call-sites.
///
/// Each field is `Some("name")` when the corresponding impl kind is both
/// enabled (`generate_*` is true) and gated
/// (`gate_impls_on_crate_features` is true), and `None` otherwise. Pass the
/// field to [`cfg_block`] / [`cfg_attr`] to wrap a token stream — they're
/// no-ops on `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FeatureGates {
    pub(crate) json: Option<&'static str>,
    pub(crate) views: Option<&'static str>,
    pub(crate) text: Option<&'static str>,
}

impl FeatureGates {
    /// Compute the active gates for a config.
    pub(crate) fn for_config(config: &CodeGenConfig) -> Self {
        if !config.gate_impls_on_crate_features {
            return Self::default();
        }
        Self {
            json: config.generate_json.then_some(JSON_FEATURE),
            views: config.generate_views.then_some(VIEWS_FEATURE),
            text: config.generate_text.then_some(TEXT_FEATURE),
        }
    }

    /// `Some("json")`, `Some("text")`, or — when both are active — the
    /// composite gate for items that exist iff *either* json or text is on
    /// (e.g. `register_types`, whose body registers both kinds of entry).
    ///
    /// Returns `None` when neither is gated. The caller should pass this to
    /// [`cfg_block_any`] to handle the two-feature case.
    pub(crate) fn json_or_text(&self) -> Vec<&'static str> {
        let mut v = Vec::with_capacity(2);
        if let Some(f) = self.json {
            v.push(f);
        }
        if let Some(f) = self.text {
            v.push(f);
        }
        v
    }
}

/// Wrap `tokens` in `#[cfg(feature = "<gate>")]` when `gate` is `Some`.
///
/// Use for **a single item or statement**: an `impl` block, a struct/enum
/// definition, a `pub use` re-export, a `pub mod` declaration, a `const`
/// item, or one statement inside a fn body. A `#[cfg]` outer attribute
/// attaches only to the **next** item — if `tokens` contains multiple
/// siblings, only the first is gated and the rest leak ungated, which is a
/// silent correctness bug. Use [`cfg_const_block`] for sibling impls, or
/// wrap each individually.
///
/// Debug builds assert `tokens` parses as a single `syn::Item` or
/// `syn::Stmt` to catch multi-item misuse early.
pub(crate) fn cfg_block(tokens: TokenStream, gate: Option<&str>) -> TokenStream {
    match gate {
        Some(feature) if !tokens.is_empty() => {
            debug_assert!(
                syn::parse2::<syn::Item>(tokens.clone()).is_ok()
                    || syn::parse2::<syn::Stmt>(tokens.clone()).is_ok(),
                "cfg_block applied to a token stream that is not a single item/statement; \
                 trailing siblings would leak ungated. Use cfg_const_block. tokens: {tokens}"
            );
            quote! {
                #[cfg(feature = #feature)]
                #tokens
            }
        }
        _ => tokens,
    }
}

/// Wrap `tokens` in `#[cfg(any(feature = "a", feature = "b", ...))]`.
///
/// Use for an item that should exist iff *at least one* of a set of gated
/// modes is enabled — e.g. `register_types`, which registers both JSON and
/// text entries and is useful when either is on. No-op for an empty set;
/// degenerates to a single `#[cfg(feature = "a")]` for a one-element set
/// (functionally identical to `cfg(any(feature = "a"))`, just less noise).
pub(crate) fn cfg_block_any(tokens: TokenStream, gates: &[&str]) -> TokenStream {
    match gates {
        [] => tokens,
        [single] => cfg_block(tokens, Some(single)),
        many if !tokens.is_empty() => {
            let preds = many.iter().map(|f| quote! { feature = #f });
            quote! {
                #[cfg(any(#(#preds),*))]
                #tokens
            }
        }
        _ => tokens,
    }
}

/// Wrap a token stream of multiple **sibling items** in a single
/// `#[cfg(feature = "<gate>")]` by enclosing them in an anonymous
/// `const _: () = { ... };` block.
///
/// A bare `#[cfg(...)]` outer attribute attaches only to the next item.
/// Wrapping in `const _: () = { ... }` lets one `#[cfg]` cover the lot —
/// the anonymous const is an item itself, and `impl` blocks inside it
/// register on the global type they target exactly as they would at
/// module scope. No-op for `None`.
pub(crate) fn cfg_const_block(tokens: TokenStream, gate: Option<&str>) -> TokenStream {
    match gate {
        Some(feature) if !tokens.is_empty() => quote! {
            #[cfg(feature = #feature)]
            const _: () = {
                #tokens
            };
        },
        _ => tokens,
    }
}

/// Wrap `attr_body` in `#[cfg_attr(feature = "<gate>", <attr_body>)]` when
/// `gate` is `Some`, or `#[<attr_body>]` when `None`.
///
/// Use for derives and helper attributes that must only apply when the
/// feature is on — e.g. `derive(::serde::Serialize, ::serde::Deserialize)`,
/// `serde(default)`, `serde(rename = "...")`. Without the gate, a
/// `#[serde(...)]` field attribute on a struct that doesn't
/// `#[derive(Serialize)]` (because the derive itself was gated off) is a
/// hard compile error — `serde` is a derive helper attribute and isn't in
/// scope without the derive.
///
/// Returns an empty stream for an empty `attr_body` so call-sites can build
/// up attribute lists with conditional pieces without spurious `#[]`.
pub(crate) fn cfg_attr(attr_body: TokenStream, gate: Option<&str>) -> TokenStream {
    if attr_body.is_empty() {
        return TokenStream::new();
    }
    match gate {
        Some(feature) => quote! { #[cfg_attr(feature = #feature, #attr_body)] },
        None => quote! { #[#attr_body] },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gated_config() -> CodeGenConfig {
        CodeGenConfig {
            generate_json: true,
            generate_views: true,
            generate_text: true,
            gate_impls_on_crate_features: true,
            ..CodeGenConfig::default()
        }
    }

    #[test]
    fn for_config_off_by_default() {
        let config = CodeGenConfig {
            generate_json: true,
            generate_views: true,
            generate_text: true,
            ..CodeGenConfig::default()
        };
        assert_eq!(FeatureGates::for_config(&config), FeatureGates::default());
    }

    #[test]
    fn for_config_gates_only_enabled_kinds() {
        // `generate_text` off → `text` gate is `None` even with
        // `gate_impls_on_crate_features` on. The flag controls *how* an
        // impl is emitted, not *whether*.
        let config = CodeGenConfig {
            generate_json: true,
            generate_views: false,
            generate_text: false,
            gate_impls_on_crate_features: true,
            ..CodeGenConfig::default()
        };
        let gates = FeatureGates::for_config(&config);
        assert_eq!(gates.json, Some(JSON_FEATURE));
        assert_eq!(gates.views, None);
        assert_eq!(gates.text, None);
    }

    #[test]
    fn for_config_all_gated() {
        let gates = FeatureGates::for_config(&gated_config());
        assert_eq!(gates.json, Some(JSON_FEATURE));
        assert_eq!(gates.views, Some(VIEWS_FEATURE));
        assert_eq!(gates.text, Some(TEXT_FEATURE));
        assert_eq!(gates.json_or_text(), vec![JSON_FEATURE, TEXT_FEATURE]);
    }

    #[test]
    fn json_or_text_subsets() {
        let none = FeatureGates::default();
        assert!(none.json_or_text().is_empty());
        let json_only = FeatureGates {
            json: Some(JSON_FEATURE),
            ..Default::default()
        };
        assert_eq!(json_only.json_or_text(), vec![JSON_FEATURE]);
        let text_only = FeatureGates {
            text: Some(TEXT_FEATURE),
            ..Default::default()
        };
        assert_eq!(text_only.json_or_text(), vec![TEXT_FEATURE]);
    }

    #[test]
    fn cfg_block_any_dispatches_by_arity() {
        let inner = quote! { pub fn f() {} };
        // Empty set → passthrough.
        assert_eq!(
            cfg_block_any(inner.clone(), &[]).to_string(),
            inner.to_string()
        );
        // One element → plain `cfg(feature = "...")`.
        assert_eq!(
            cfg_block_any(inner.clone(), &["json"]).to_string(),
            quote! { #[cfg(feature = "json")] pub fn f() {} }.to_string()
        );
        // Two elements → `cfg(any(...))`.
        assert_eq!(
            cfg_block_any(inner.clone(), &["json", "text"]).to_string(),
            quote! { #[cfg(any(feature = "json", feature = "text"))] pub fn f() {} }.to_string()
        );
        assert!(cfg_block_any(TokenStream::new(), &["json", "text"]).is_empty());
    }

    #[test]
    #[should_panic(expected = "cfg_block applied to a token stream that is not a single item")]
    #[cfg(debug_assertions)]
    fn cfg_block_rejects_multiple_siblings() {
        // Two sibling items → would silently leave the second ungated. The
        // debug_assert catches this misuse early.
        cfg_block(quote! { struct A; struct B; }, Some("json"));
    }

    #[test]
    fn cfg_block_wraps_when_gated() {
        let inner = quote! { impl Foo for Bar {} };
        let wrapped = cfg_block(inner.clone(), Some("json"));
        assert_eq!(
            wrapped.to_string(),
            quote! { #[cfg(feature = "json")] impl Foo for Bar {} }.to_string()
        );
        // No gate → passthrough.
        assert_eq!(
            cfg_block(inner.clone(), None).to_string(),
            inner.to_string()
        );
        // Empty input → empty output, no dangling `#[cfg]`.
        assert!(cfg_block(TokenStream::new(), Some("json")).is_empty());
    }

    #[test]
    fn cfg_const_block_wraps_siblings() {
        let inner = quote! { impl A for X {} impl B for X {} };
        let wrapped = cfg_const_block(inner.clone(), Some("json"));
        assert_eq!(
            wrapped.to_string(),
            quote! {
                #[cfg(feature = "json")]
                const _: () = { impl A for X {} impl B for X {} };
            }
            .to_string()
        );
        assert_eq!(
            cfg_const_block(inner.clone(), None).to_string(),
            inner.to_string()
        );
        assert!(cfg_const_block(TokenStream::new(), Some("json")).is_empty());
    }

    #[test]
    fn cfg_attr_wraps_when_gated() {
        let body = quote! { derive(::serde::Serialize) };
        assert_eq!(
            cfg_attr(body.clone(), Some("json")).to_string(),
            quote! { #[cfg_attr(feature = "json", derive(::serde::Serialize))] }.to_string()
        );
        assert_eq!(
            cfg_attr(body.clone(), None).to_string(),
            quote! { #[derive(::serde::Serialize)] }.to_string()
        );
        assert!(cfg_attr(TokenStream::new(), Some("json")).is_empty());
        assert!(cfg_attr(TokenStream::new(), None).is_empty());
    }
}

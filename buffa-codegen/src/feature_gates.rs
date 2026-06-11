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

/// Default crate feature names the gated impls are conditioned on.
pub(crate) const JSON_FEATURE: &str = "json";
pub(crate) const VIEWS_FEATURE: &str = "views";
pub(crate) const TEXT_FEATURE: &str = "text";
pub(crate) const REFLECT_FEATURE: &str = "reflect";

/// Crate feature names used by the gated impls, customizable per impl kind.
///
/// Used by [`CodeGenConfig::feature_gate_names`]. The defaults are `"json"`,
/// `"views"`, `"text"`, and `"reflect"`; the consuming crate must define
/// matching features in its `Cargo.toml`. Override a name when the consuming
/// crate already uses a different feature name for the same concern (e.g. a
/// crate whose JSON support is gated behind a `serde` feature).
///
/// Names are emitted verbatim into `#[cfg(feature = "...")]` /
/// `#[cfg_attr(feature = "...", ...)]` attributes; they must be valid Cargo
/// feature names. **An empty, misspelled, or undeclared name fails open**:
/// the emitted `#[cfg]` is permanently false in the consuming crate, so the
/// gated impls silently compile away. To catch the cases that are
/// detectable at generation time, [`generate`](crate::generate) returns an
/// error when an *active* gate name is empty or not a valid Cargo feature
/// name; an undeclared (but valid) name can only be diagnosed in the
/// consuming crate, via the `unexpected_cfgs` lint on Rust ≥ 1.80.
///
/// The struct is `#[non_exhaustive]`; construct it by mutating
/// [`FeatureGateNames::default()`] (or use the `buffa_build::Config`
/// setters).
///
/// [`CodeGenConfig::feature_gate_names`]: crate::CodeGenConfig::feature_gate_names
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FeatureGateNames {
    /// Feature gating the serde JSON impls (default `"json"`).
    pub json: String,
    /// Feature gating the view types and impls (default `"views"`).
    pub views: String,
    /// Feature gating the textproto impls (default `"text"`).
    pub text: String,
    /// Feature gating the reflection impls (default `"reflect"`).
    pub reflect: String,
}

impl FeatureGateNames {
    /// Whether `name` is a valid Cargo feature name: starts with an ASCII
    /// alphanumeric or `_`, with the remainder drawn from alphanumerics,
    /// `_`, `-`, `+`, and `.`.
    ///
    /// This is the rule [`generate`](crate::generate) enforces on every
    /// *active* gate name. It is public so toolchains layered on
    /// buffa-codegen (e.g. service generators with their own feature-gate
    /// knobs) can validate user-supplied names against the same rule
    /// instead of re-deriving it.
    #[must_use]
    pub fn is_valid_name(name: &str) -> bool {
        is_valid_feature_name(name)
    }
}

impl Default for FeatureGateNames {
    fn default() -> Self {
        Self {
            json: JSON_FEATURE.to_string(),
            views: VIEWS_FEATURE.to_string(),
            text: TEXT_FEATURE.to_string(),
            reflect: REFLECT_FEATURE.to_string(),
        }
    }
}

/// Whether `name` is a valid Cargo feature name: starts with an ASCII
/// alphanumeric or `_`, with the remainder drawn from Cargo's feature
/// alphabet (alphanumerics, `_`, `-`, `+`, `.`).
///
/// Enforced at the [`generate`](crate::generate) entry point for every
/// *active* gate name — the failure mode of an invalid name is silent
/// (`#[cfg(feature = "")]` is permanently false, so the gated impls just
/// disappear), so it must be a hard error in every build profile.
pub(crate) fn is_valid_feature_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '.'))
}

/// Resolved feature-gate names for the current codegen run, computed once
/// from [`CodeGenConfig`] and threaded through codegen call-sites.
///
/// Each field is `Some("name")` when the corresponding impl kind is both
/// enabled (`generate_*` is true) and gated
/// (`gate_impls_on_crate_features` is true), and `None` otherwise. The names
/// borrow from the config's [`FeatureGateNames`]. Pass the field to
/// [`cfg_block`] / [`cfg_attr`] to wrap a token stream — they're no-ops on
/// `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FeatureGates<'a> {
    pub(crate) json: Option<&'a str>,
    pub(crate) views: Option<&'a str>,
    pub(crate) text: Option<&'a str>,
    pub(crate) reflect: Option<&'a str>,
}

impl<'a> FeatureGates<'a> {
    /// Compute the active gates for a config.
    ///
    /// `gate_impls_on_crate_features` gates json/views/text/reflect together.
    /// When it is off, `gate_reflect_on_crate_feature` turns on reflect-only
    /// gating — for crates (notably `buffa-types`) that ship views/text
    /// unconditionally but want the `buffa-descriptor`-dependent reflection
    /// surface to be opt-in.
    pub(crate) fn for_config(config: &'a CodeGenConfig) -> Self {
        let gate_all = config.gate_impls_on_crate_features;
        let gate_reflect = gate_all || config.gate_reflect_on_crate_feature;
        let names = &config.feature_gate_names;
        Self {
            json: (gate_all && config.generate_json).then_some(names.json.as_str()),
            views: (gate_all && config.generate_views).then_some(names.views.as_str()),
            text: (gate_all && config.generate_text).then_some(names.text.as_str()),
            reflect: (gate_reflect && config.generate_reflection).then_some(names.reflect.as_str()),
        }
    }

    /// Check every *active* gate name against [`is_valid_feature_name`],
    /// returning the first offender as `Err((kind, name))`.
    ///
    /// Called from [`generate`](crate::generate) so an invalid name is a
    /// hard error in every build profile — the failure mode otherwise is
    /// silent (the emitted `#[cfg]` is permanently false and the gated
    /// impls compile away). Inactive names are not checked: they are inert
    /// and never reach the output.
    pub(crate) fn validate(&self) -> Result<(), (&'static str, &'a str)> {
        [
            ("json", self.json),
            ("views", self.views),
            ("text", self.text),
            ("reflect", self.reflect),
        ]
        .into_iter()
        .filter_map(|(kind, name)| Some((kind, name?)))
        .try_for_each(|(kind, name)| {
            if is_valid_feature_name(name) {
                Ok(())
            } else {
                Err((kind, name))
            }
        })
    }

    /// `Some("json")`, `Some("text")`, or — when both are active — the
    /// composite gate for items that exist iff *either* json or text is on
    /// (e.g. `register_types`, whose body registers both kinds of entry).
    ///
    /// Returns `None` when neither is gated. When both kinds are gated on
    /// the *same* custom name, the duplicate collapses to a single entry so
    /// the emitted attribute is a plain `#[cfg(feature = "...")]` rather
    /// than `#[cfg(any(feature = "x", feature = "x"))]`. The caller should
    /// pass this to [`cfg_block_any`] to handle the two-feature case.
    pub(crate) fn json_or_text(&self) -> Vec<&'a str> {
        let mut v = Vec::with_capacity(2);
        if let Some(f) = self.json {
            v.push(f);
        }
        if let Some(f) = self.text {
            if self.json != Some(f) {
                v.push(f);
            }
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
        let config = gated_config();
        let gates = FeatureGates::for_config(&config);
        assert_eq!(gates.json, Some(JSON_FEATURE));
        assert_eq!(gates.views, Some(VIEWS_FEATURE));
        assert_eq!(gates.text, Some(TEXT_FEATURE));
        assert_eq!(gates.json_or_text(), vec![JSON_FEATURE, TEXT_FEATURE]);
    }

    #[test]
    fn for_config_reflect_only_gate() {
        // `gate_reflect_on_crate_feature` gates reflect without gating
        // json/views/text — the `buffa-types` shape.
        let config = CodeGenConfig {
            generate_json: true,
            generate_views: true,
            generate_text: true,
            generate_reflection: true,
            gate_reflect_on_crate_feature: true,
            ..CodeGenConfig::default()
        };
        let gates = FeatureGates::for_config(&config);
        assert_eq!(gates.json, None);
        assert_eq!(gates.views, None);
        assert_eq!(gates.text, None);
        assert_eq!(gates.reflect, Some(REFLECT_FEATURE));
    }

    #[test]
    fn for_config_reflect_gate_requires_generate_reflection() {
        // The gate flag is inert unless reflection is actually generated.
        let config = CodeGenConfig {
            generate_reflection: false,
            gate_reflect_on_crate_feature: true,
            ..CodeGenConfig::default()
        };
        assert_eq!(FeatureGates::for_config(&config).reflect, None);
    }

    #[test]
    fn for_config_umbrella_gate_includes_reflect() {
        // `gate_impls_on_crate_features` also gates reflect when reflection is on.
        let config = CodeGenConfig {
            generate_reflection: true,
            gate_impls_on_crate_features: true,
            ..CodeGenConfig::default()
        };
        assert_eq!(
            FeatureGates::for_config(&config).reflect,
            Some(REFLECT_FEATURE)
        );
    }

    #[test]
    fn for_config_custom_names() {
        let config = CodeGenConfig {
            feature_gate_names: FeatureGateNames {
                json: "serde".to_string(),
                views: "zero-copy".to_string(),
                text: "textproto".to_string(),
                reflect: "reflection".to_string(),
            },
            generate_reflection: true,
            ..gated_config()
        };
        let gates = FeatureGates::for_config(&config);
        assert_eq!(gates.json, Some("serde"));
        assert_eq!(gates.views, Some("zero-copy"));
        assert_eq!(gates.text, Some("textproto"));
        assert_eq!(gates.reflect, Some("reflection"));
        assert_eq!(gates.json_or_text(), vec!["serde", "textproto"]);
    }

    #[test]
    fn custom_names_inert_without_gating() {
        // Renaming a gate without enabling gating changes nothing — the
        // names only matter once `gate_impls_on_crate_features` (or the
        // reflect-only flag) turns the gates on.
        let config = CodeGenConfig {
            generate_json: true,
            feature_gate_names: FeatureGateNames {
                json: "serde".to_string(),
                ..FeatureGateNames::default()
            },
            ..CodeGenConfig::default()
        };
        assert_eq!(FeatureGates::for_config(&config), FeatureGates::default());
    }

    #[test]
    fn json_or_text_dedups_shared_name() {
        // Gating both kinds behind one feature must emit a single
        // `#[cfg(feature = "serde")]`, not `#[cfg(any(.., ..))]` with a
        // duplicated predicate.
        let shared = FeatureGates {
            json: Some("serde"),
            text: Some("serde"),
            ..Default::default()
        };
        assert_eq!(shared.json_or_text(), vec!["serde"]);
    }

    #[test]
    fn public_validator_matches_internal_rule() {
        // The public surface for layered toolchains must agree with what
        // `generate` enforces.
        for name in ["json", "zero-copy", "", "-leading", "with space"] {
            assert_eq!(
                FeatureGateNames::is_valid_name(name),
                is_valid_feature_name(name)
            );
        }
    }

    #[test]
    fn feature_name_validity() {
        assert!(is_valid_feature_name("json"));
        assert!(is_valid_feature_name("zero-copy"));
        assert!(is_valid_feature_name("a_b.c+d2"));
        assert!(is_valid_feature_name("_private"));
        assert!(!is_valid_feature_name(""));
        assert!(!is_valid_feature_name("with space"));
        assert!(!is_valid_feature_name("quo\"te"));
        // Cargo requires the first character to be alphanumeric or `_`.
        assert!(!is_valid_feature_name("-leading"));
        assert!(!is_valid_feature_name(".leading"));
        assert!(!is_valid_feature_name("+leading"));
    }

    #[test]
    fn validate_reports_first_invalid_active_name() {
        let config = CodeGenConfig {
            feature_gate_names: FeatureGateNames {
                views: String::new(),
                ..FeatureGateNames::default()
            },
            ..gated_config()
        };
        assert_eq!(
            FeatureGates::for_config(&config).validate(),
            Err(("views", ""))
        );
    }

    #[test]
    fn validate_ignores_inactive_invalid_names() {
        // An invalid name on a kind that isn't gated never reaches the
        // output, so it must not fail validation.
        let config = CodeGenConfig {
            feature_gate_names: FeatureGateNames {
                reflect: "not valid".to_string(),
                ..FeatureGateNames::default()
            },
            ..gated_config() // generate_reflection is off in gated_config
        };
        let gates = FeatureGates::for_config(&config);
        assert_eq!(gates.reflect, None);
        assert_eq!(gates.validate(), Ok(()));
    }

    #[test]
    fn default_names_match_constants() {
        let names = FeatureGateNames::default();
        assert_eq!(names.json, JSON_FEATURE);
        assert_eq!(names.views, VIEWS_FEATURE);
        assert_eq!(names.text, TEXT_FEATURE);
        assert_eq!(names.reflect, REFLECT_FEATURE);
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

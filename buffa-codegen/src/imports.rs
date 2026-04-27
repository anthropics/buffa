//! Import management for generated code.
//!
//! Prelude types like `Option` are always emitted as fully-qualified paths
//! (`::core::option::Option<T>`) to prevent shadowing by proto-defined types
//! of the same name. This is necessary because the stitcher combines all
//! files from one package into a single module scope via `include!`, so a
//! `message Option` in *any* sibling file would shadow the prelude.
//!
//! `alloc` types (`String`, `Vec`, `Box`) are always emitted as
//! `::buffa::alloc::*` paths because they are not in the `no_std` prelude,
//! consistent with the `HashMap` approach via `::buffa::__private::HashMap`.
//! Buffa runtime types are always emitted as absolute paths since generated
//! files may be combined via `include!`.

use proc_macro2::TokenStream;
use quote::quote;

/// Single source of truth for type-path emission in generated code.
///
/// All prelude types are unconditionally emitted as fully-qualified paths
/// (e.g. `::core::option::Option`) to avoid shadowing by user-defined proto
/// types. This is simpler and more robust than trying to detect collisions:
/// the stitcher's `include!`-based module merging makes it impossible to
/// know at per-file generation time which names will be in scope.
///
/// Stateless — kept as a struct (rather than free functions) so call sites
/// uniformly take `&ImportResolver` and any future per-scope state can be
/// added without re-threading parameters.
pub(crate) struct ImportResolver;

impl ImportResolver {
    pub fn new() -> Self {
        Self
    }

    // ── Prelude type tokens ─────────────────────────────────────────────

    pub fn option(&self) -> TokenStream {
        quote! { ::core::option::Option }
    }

    // ── Alloc types (always absolute, no_std-safe via ::buffa::alloc) ───

    pub fn string(&self) -> TokenStream {
        quote! { ::buffa::alloc::string::String }
    }

    pub fn vec(&self) -> TokenStream {
        quote! { ::buffa::alloc::vec::Vec }
    }

    // ── Buffa runtime types (always absolute, include!-safe) ────────────

    pub fn message_field(&self) -> TokenStream {
        quote! { ::buffa::MessageField }
    }

    pub fn enum_value(&self) -> TokenStream {
        quote! { ::buffa::EnumValue }
    }

    pub fn hashmap(&self) -> TokenStream {
        quote! { ::buffa::__private::HashMap }
    }
}

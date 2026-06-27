//! Shared code generation logic for buffa.
//!
//! This crate takes protobuf descriptors (`google.protobuf.FileDescriptorProto`,
//! decoded from binary `FileDescriptorSet` data) and emits Rust source code
//! that uses the `buffa` runtime.
//!
//! It is used by:
//! - `protoc-gen-buffa` (protoc plugin)
//! - `buffa-build` (build.rs integration)
//!
//! # Architecture
//!
//! The code generator is intentionally decoupled from how descriptors are
//! obtained. It receives fully-resolved `FileDescriptorProto`s and produces
//! Rust source strings. This means:
//!
//! - It doesn't parse `.proto` files.
//! - It doesn't invoke `protoc`.
//! - It doesn't do import resolution or name linking.
//!
//! All of that is handled upstream (by protoc, buf, or a future parser).

pub(crate) mod comments;
pub mod context;
pub(crate) mod defaults;
pub(crate) mod enumeration;
pub(crate) mod extension;
pub(crate) mod feature_gates;
pub use feature_gates::FeatureGateNames;
pub(crate) mod features;
#[doc(hidden)]
pub use buffa_descriptor::generated;
pub mod idents;
pub(crate) mod impl_message;
pub(crate) mod impl_text;
pub(crate) mod imports;
pub(crate) mod lazy_view;
pub(crate) mod message;
pub(crate) mod oneof;
pub(crate) mod owned_view;
pub(crate) mod reflect;
pub(crate) mod reflect_owned;
pub(crate) mod reflect_view;
pub(crate) mod view;

use crate::generated::descriptor::FileDescriptorProto;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Lints suppressed on generated code at module boundaries.
///
/// Consumed by [`generate_module_tree`], the per-package `.mod.rs`
/// stitcher, and `buffa-build`'s `_include.rs` writer. One list keeps
/// them in sync.
pub const ALLOW_LINTS: &[&str] = &[
    "non_camel_case_types",
    "dead_code",
    "unused_imports",
    // Cross-proto refs within the same package are emitted through the
    // canonical `super::super::__buffa::view::…` path even though the
    // target lives in the same generated module — using the bare name
    // would resolve, but the canonical path is stable when a sibling
    // proto defines a same-named natural-path re-export.
    "unused_qualifications",
    "clippy::derivable_impls",
    "clippy::match_single_binding",
    "clippy::uninlined_format_args",
    "clippy::doc_lazy_continuation",
    // A user `message View { message Inner }` produces
    // `__buffa::view::view::InnerView`; harmless but trips this lint.
    "clippy::module_inception",
];

/// Render [`ALLOW_LINTS`] as a `#[allow(…)]` attribute token stream.
pub fn allow_lints_attr() -> TokenStream {
    let lints: Vec<TokenStream> = ALLOW_LINTS
        .iter()
        .map(|l| syn::parse_str(l).expect("lint name parses as path"))
        .collect();
    quote! { #[allow( #(#lints),* )] }
}

/// One generated output file.
///
/// Each `.proto` produces up to five **content files** (`<stem>.rs`,
/// `<stem>.__view.rs`, `<stem>.__oneof.rs`, `<stem>.__view_oneof.rs`,
/// `<stem>.__ext.rs`) and each proto package produces one
/// `<dotted.pkg>.mod.rs` **stitcher** that `include!`s the content files
/// and authors the `pub mod __buffa { … }` ancillary tree.
/// Ancillary kinds with no content for that input file (e.g. a message
/// with no oneofs and no extensions) are omitted, and the stitcher's
/// `include!` set is filtered to match. The `__buffa` wrapper (and each
/// `view` / `oneof` / `ext` submodule inside it) is itself omitted when
/// it would be empty, so packages with only owned messages emit no
/// `__buffa` block at all.
/// See `DESIGN.md` → "Generated code layout".
///
/// Consumers normally only need to wire up the
/// [`GeneratedFileKind::PackageMod`] entries (one per package); the
/// per-proto content kinds are reached transitively via `include!` from
/// the stitcher. Write all files to disk; build a module tree from only
/// the `PackageMod` ones.
///
/// With [`CodeGenConfig::file_per_package`] set, the per-proto content
/// kinds are not emitted at all — the single `<dotted.pkg>.rs` (still
/// kind `PackageMod`) inlines what the stitcher would `include!`.
#[derive(Debug)]
pub struct GeneratedFile {
    /// The output file path (e.g., `"my.pkg.foo.rs"` or `"my.pkg.mod.rs"`).
    pub name: String,
    /// The proto package this file belongs to.
    pub package: String,
    /// What this file contains. Build integrations only need to wire up
    /// [`GeneratedFileKind::PackageMod`] files; everything else is reached
    /// via `include!` from there.
    pub kind: GeneratedFileKind,
    /// The generated Rust source code.
    pub content: String,
}

/// Kind of [`GeneratedFile`].
///
/// [`generate`] produces up to five per-proto content kinds — one each
/// of [`Owned`](Self::Owned), [`View`](Self::View), [`Oneof`](Self::Oneof),
/// [`ViewOneof`](Self::ViewOneof), and [`Ext`](Self::Ext) per input
/// `.proto` file — plus one [`PackageMod`](Self::PackageMod) stitcher per
/// package. Kinds with no content for the input (a proto with no oneofs
/// emits no [`Oneof`](Self::Oneof) / [`ViewOneof`](Self::ViewOneof);
/// no extensions, no [`Ext`](Self::Ext); etc.) are omitted. Build
/// integrations only need to wire up `PackageMod` entries; the per-proto
/// content kinds are reached via `include!` from the stitcher and need
/// only be written to disk alongside it. Under
/// [`CodeGenConfig::file_per_package`] only `PackageMod` is emitted.
///
/// [`Companion`](Self::Companion) is the one kind *not* produced by
/// [`generate`]: downstream code generators construct `Companion` files
/// themselves and merge them into buffa's output via
/// [`apply_companions`].
///
/// This enum is `#[non_exhaustive]` — match with a wildcard arm so new
/// kinds can be added without a major version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneratedFileKind {
    /// Owned message structs and enums (`<stem>.rs`).
    Owned,
    /// View structs (`<stem>.__view.rs`).
    View,
    /// Lazy view structs (`<stem>.__lazy_view.rs`).
    LazyView,
    /// Owned oneof enums (`<stem>.__oneof.rs`).
    Oneof,
    /// View oneof enums (`<stem>.__view_oneof.rs`).
    ViewOneof,
    /// File-level proto-extension consts (`<stem>.__ext.rs`) — the
    /// `pub const` `ExtensionDescriptor` items generated from `extend`
    /// blocks. Not to be confused with [`Companion`](Self::Companion),
    /// which is unrelated downstream-supplied content.
    Ext,
    /// Per-package stitcher (`<dotted.pkg>.mod.rs`). The only file build
    /// systems need to wire up directly.
    PackageMod,
    /// Extra per-proto content from a downstream code generator (service
    /// stubs, extra trait impls, etc.) that travels with buffa's output.
    ///
    /// Not produced by [`generate`]. Construct these in your own generator
    /// and pass them to [`apply_companions`], which appends an `include!`
    /// for each one at file scope in the matching package's
    /// [`PackageMod`](Self::PackageMod) — after buffa's own output, at
    /// package root alongside the owned message types (**not** under the
    /// `__buffa::` sentinel module). Items declared `pub` in a companion
    /// file are visible at `crate::<pkg>::*`.
    ///
    /// Not to be confused with [`Ext`](Self::Ext), which is the buffa-
    /// generated file holding protobuf `extend` consts.
    Companion,
}

/// Parse a custom owned-type path string (e.g. `"::smol_str::SmolStr"`) into a
/// token stream, validating it as a Rust type so a malformed path surfaces as a
/// codegen error rather than unparseable generated output.
pub(crate) fn parse_custom_type_path(path: &str) -> Result<proc_macro2::TokenStream, CodeGenError> {
    let ty: syn::Type =
        syn::parse_str(path).map_err(|_| CodeGenError::InvalidTypePath(path.to_string()))?;
    Ok(quote::quote! { #ty })
}

/// Parse a custom **map** container path, which is applied as `path<K, V>`.
///
/// The path must therefore be a bare type path with no `<...>` parameters of its
/// own (and, unlike the box/repeated knobs, no `*` placeholder — a map's key and
/// value are appended positionally). Reject anything else with a message that
/// names the convention, rather than letting `Foo<Bar><K, V>` surface as an
/// opaque whole-file parse error later.
pub(crate) fn parse_custom_map_path(path: &str) -> Result<proc_macro2::TokenStream, CodeGenError> {
    let ty: syn::Type = syn::parse_str(path).map_err(|_| {
        CodeGenError::InvalidTypePath(format!(
            "{path} (map custom path takes no `<K, V>` parameters and no `*` placeholder)"
        ))
    })?;
    let syn::Type::Path(tp) = &ty else {
        return Err(CodeGenError::InvalidTypePath(format!(
            "{path} (map custom path must be a plain type path)"
        )));
    };
    if tp
        .path
        .segments
        .iter()
        .any(|s| !matches!(s.arguments, syn::PathArguments::None))
    {
        return Err(CodeGenError::InvalidTypePath(format!(
            "{path} (map custom path must not include `<K, V>`; the key and value are appended automatically)"
        )));
    }
    Ok(quote::quote! { #ty })
}

/// Build a custom wrapper type from a `*`-templated path and a resolved inner
/// type, validating the result as a Rust type.
///
/// `*` cannot be a parsed placeholder (it is not valid in Rust type position),
/// so substitution is textual — every `*` in `template` is replaced by `inner`'s
/// token text before the whole string is parsed. Used by the pluggable pointer
/// knob, where the wrapped type sits inside extra generic parameters (e.g.
/// `"smallbox::SmallBox<*, S4>"`). The template must contain at least one `*`.
pub(crate) fn parse_wildcard_type_path(
    template: &str,
    inner: &proc_macro2::TokenStream,
) -> Result<proc_macro2::TokenStream, CodeGenError> {
    if !template.contains('*') {
        return Err(CodeGenError::MissingWildcard(template.to_string()));
    }
    let substituted = template.replace('*', &inner.to_string());
    let ty: syn::Type = syn::parse_str(&substituted)
        .map_err(|_| CodeGenError::InvalidTypePath(format!("{template} (as {substituted})")))?;
    Ok(quote::quote! { #ty })
}

/// Build a custom collection type from a `*`-templated path and the resolved
/// element type, validating the result as a Rust type.
///
/// `*` cannot be a parsed placeholder (it is not valid in Rust type position),
/// so substitution is textual — every `*` in `template` is replaced by the
/// element's token text before the whole string is parsed. The template must
/// contain at least one `*`, otherwise the element type would have nowhere to
/// go and the field would silently drop its element type.
pub(crate) fn parse_custom_list_path(
    template: &str,
    elem: &proc_macro2::TokenStream,
) -> Result<proc_macro2::TokenStream, CodeGenError> {
    if !template.contains('*') {
        return Err(CodeGenError::MissingListPlaceholder(template.to_string()));
    }
    let substituted = template.replace('*', &elem.to_string());
    let ty: syn::Type = syn::parse_str(&substituted)
        .map_err(|_| CodeGenError::InvalidTypePath(template.to_string()))?;
    Ok(quote::quote! { #ty })
}

/// The Rust type a proto `string` field maps to in generated owned structs.
///
/// The default is [`String`](StringRepr::String).
/// [`Custom`](StringRepr::Custom) substitutes any type named by its
/// fully-qualified Rust path — for example `::smol_str::SmolStr`,
/// `::ecow::EcoString`, or `::compact_str::CompactString` for read-mostly
/// schemas — that satisfies the `buffa::ProtoString` bound. The downstream crate
/// must itself depend on the crate providing that type (buffa does not re-export
/// it).
///
/// Select a representation through `buffa_build`'s `string_type` /
/// `string_type_custom` builder methods. The wire format is identical regardless
/// of representation — only the in-memory owned type changes; view types keep
/// borrowing `&str`, and `map<_, string>` / `map<string, _>` keys and values
/// always stay `String`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StringRepr {
    /// `::buffa::alloc::string::String` — growable and mutable (the default).
    #[default]
    String,
    /// A custom type named by its fully-qualified Rust path (e.g.
    /// `"::smol_str::SmolStr"`). Must satisfy `buffa::ProtoString` and be
    /// provided by a crate the downstream depends on.
    ///
    /// # Limitations
    ///
    /// - A *foreign* custom type used as a `repeated` element fails to compile
    ///   (the emitted `ReflectElement` impl violates the orphan rule). Wrap it
    ///   in a crate-local newtype for that case; singular / optional / oneof /
    ///   map uses work with a foreign type directly.
    /// - A path that does not parse as a Rust type surfaces as
    ///   [`CodeGenError::InvalidTypePath`] at generation (`.compile()`) time.
    /// - The per-element impls are deduplicated within a single generation, but
    ///   the *same* crate-local type used as a `repeated` element across two
    ///   separate `compile()` invocations in one crate emits the impl twice (a
    ///   duplicate-impl `E0119`). Generate from a single `compile()`, or use
    ///   distinct element types.
    Custom(String),
}

impl StringRepr {
    /// The owned Rust type path emitted for a `string` field with this
    /// representation.
    ///
    /// `ctx` and `nesting` route the default `String` through the package-root
    /// import registry (`idiomatic_imports`); a custom path is parsed and
    /// emitted fully qualified.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::InvalidTypePath`] if a custom path does not parse
    /// as a Rust type.
    pub(crate) fn type_path(
        &self,
        resolver: &imports::ImportResolver,
        ctx: &context::CodeGenContext,
        nesting: usize,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        match self {
            StringRepr::String => Ok(resolver.string_at(ctx, nesting)),
            StringRepr::Custom(path) => parse_custom_type_path(path),
        }
    }

    /// Whether this is the default `String` representation, which keeps the
    /// `String`-specialized fast paths (in-place `merge_string`, `clear()`,
    /// native `Arbitrary`) instead of the generic `ProtoString` ones.
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, StringRepr::String)
    }
}

/// The Rust type a proto `bytes` field maps to in generated owned structs.
///
/// The default is [`Vec`](BytesRepr::Vec) (`Vec<u8>`). [`Bytes`](BytesRepr::Bytes)
/// uses `bytes::Bytes`, which decodes zero-copy from a
/// `Bytes`-backed buffer. [`Custom`](BytesRepr::Custom) substitutes any type
/// named by its fully-qualified Rust path that satisfies the `buffa::ProtoBytes`
/// bound; the downstream crate must itself depend on the providing crate.
///
/// Select a representation through `buffa_build`'s `bytes_type` /
/// `bytes_type_custom` builder methods (or the legacy `use_bytes_type`, which
/// selects [`Bytes`](BytesRepr::Bytes)). The wire format is identical regardless
/// of representation; view types keep borrowing `&[u8]`, and `map` bytes values
/// follow the same rules as the string path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BytesRepr {
    /// `::buffa::alloc::vec::Vec<u8>` — growable and mutable (the default).
    #[default]
    Vec,
    /// `::buffa::bytes::Bytes` — reference-counted, immutable, decodes zero-copy
    /// from a `Bytes`-backed buffer.
    Bytes,
    /// A custom type named by its fully-qualified Rust path. Must satisfy
    /// `buffa::ProtoBytes` and be provided by a crate the downstream depends on.
    ///
    /// # Limitations
    ///
    /// - A *foreign* custom type used as a `repeated` element fails to compile
    ///   (the emitted `ReflectElement` / `ProtoElemJson` impls violate the
    ///   orphan rule). Wrap it in a crate-local newtype for that case; singular
    ///   / optional / oneof uses work with a foreign type directly.
    /// - A `Custom` rule does **not** apply to `map<K, bytes>` values — they
    ///   stay `Vec<u8>`. Only the built-in [`Bytes`](BytesRepr::Bytes) applies
    ///   to map values.
    /// - A path that does not parse as a Rust type surfaces as
    ///   [`CodeGenError::InvalidTypePath`] at generation (`.compile()`) time.
    /// - The per-element impls are deduplicated within a single generation, but
    ///   the *same* crate-local type used as a `repeated` element across two
    ///   separate `compile()` invocations in one crate emits the impl twice (a
    ///   duplicate-impl `E0119`). Generate from a single `compile()`, or use
    ///   distinct element types.
    Custom(String),
}

impl BytesRepr {
    /// The owned Rust type path emitted for a `bytes` field with this
    /// representation.
    ///
    /// `ctx` and `nesting` route the default `Vec<u8>` through the package-root
    /// import registry; `Bytes` and a custom path are emitted fully qualified.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::InvalidTypePath`] if a custom path does not parse
    /// as a Rust type.
    pub(crate) fn type_path(
        &self,
        resolver: &imports::ImportResolver,
        ctx: &context::CodeGenContext,
        nesting: usize,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            BytesRepr::Vec => {
                let vec = resolver.vec_at(ctx, nesting);
                Ok(quote! { #vec<u8> })
            }
            BytesRepr::Bytes => Ok(quote! { ::buffa::bytes::Bytes }),
            BytesRepr::Custom(path) => parse_custom_type_path(path),
        }
    }

    /// Whether this is the default `Vec<u8>` representation, which keeps the
    /// `Vec`-specialized fast paths (in-place `merge_bytes`, `clear()`, native
    /// `Arbitrary`) instead of the generic `ProtoBytes` ones.
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, BytesRepr::Vec)
    }
}

/// The owned Rust collection a proto `map<K, V>` field maps to in generated
/// owned structs.
///
/// The default is [`HashMap`](MapRepr::HashMap) (`std::collections::HashMap`, or
/// `hashbrown::HashMap` under `no_std`). [`BTreeMap`](MapRepr::BTreeMap) selects
/// the buffa-provided `alloc::collections::BTreeMap` for deterministic iteration
/// order with no extra dependency or consumer code.
/// [`Custom`](MapRepr::Custom) substitutes any map that satisfies the
/// `buffa::map_codec::MapStorage` bound — for example a crate-local newtype
/// wrapping `indexmap::IndexMap`.
///
/// Unlike the `repeated` knob (which wraps the element type and needs a `*`
/// placeholder template), a map type is always `path<K, V>` with both
/// parameters positional and buffa-resolved, so a custom path is a plain type
/// path (e.g. `"::my_crate::OrderedMap"`) with no placeholder.
///
/// Select a representation through `buffa_build`'s `map_type` /
/// `map_type_custom` builder methods. The wire format is identical regardless of
/// the collection; only the in-memory owned type changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MapRepr {
    /// `::buffa::__private::HashMap<K, V>` — the default. Generated output is
    /// byte-identical to a build without the knob.
    #[default]
    HashMap,
    /// `::buffa::alloc::collections::BTreeMap<K, V>` — buffa-provided, no extra
    /// dependency, deterministic key order (so encoded bytes are stable across
    /// runs). The key type must be `Ord`, which every proto map key type
    /// (integers, bool, string) satisfies.
    BTreeMap,
    /// A custom map named by a fully-qualified Rust type path (e.g.
    /// `"::my_crate::OrderedMap"`). The named type must satisfy
    /// `buffa::map_codec::MapStorage` and be a **crate-local newtype** (a foreign
    /// map cannot implement the buffa-owned reflection / serde traits).
    ///
    /// # Limitations
    ///
    /// - The path is a plain type path applied as `path<K, V>` — it must **not**
    ///   include the `<K, V>` parameters or a `*` placeholder. A path that does
    ///   not parse as a Rust type surfaces as [`CodeGenError::InvalidTypePath`]
    ///   at generation (`.compile()`) time.
    /// - The newtype must implement `buffa::map_codec::MapStorage` plus the
    ///   derive / `FromIterator` / `ReflectMap` / serde / `arbitrary` bounds
    ///   listed on that trait's docs (the canonical list). JSON and `arbitrary`
    ///   now work for every proto map key/value type regardless of the container.
    ///   The buffa-provided [`BTreeMap`](MapRepr::BTreeMap) already satisfies every
    ///   bound, so prefer it unless you need a specific foreign map.
    Custom(String),
}

impl MapRepr {
    /// The owned Rust map type emitted for a `map<K, V>` field with this
    /// representation, given the already-resolved key and value type tokens.
    ///
    /// `ctx` and `nesting` route the default `HashMap` through the package-root
    /// import registry; `BTreeMap` and a custom path are emitted fully
    /// qualified.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::InvalidTypePath`] if a custom path does not parse
    /// as a Rust type.
    pub(crate) fn type_path(
        &self,
        key: &proc_macro2::TokenStream,
        value: &proc_macro2::TokenStream,
        resolver: &imports::ImportResolver,
        ctx: &context::CodeGenContext,
        nesting: usize,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            MapRepr::HashMap => {
                let hm = resolver.hashmap_at(ctx, nesting);
                Ok(quote! { #hm<#key, #value> })
            }
            MapRepr::BTreeMap => Ok(quote! { ::buffa::alloc::collections::BTreeMap<#key, #value> }),
            MapRepr::Custom(path) => {
                let ty = parse_custom_map_path(path)?;
                Ok(quote! { #ty<#key, #value> })
            }
        }
    }

    /// Whether this is the default `HashMap` representation, whose generated
    /// output is byte-identical to a build without the knob.
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, MapRepr::HashMap)
    }
}

/// The owned smart pointer a singular message field's `buffa::MessageField`
/// wraps in generated owned structs.
///
/// The default is [`Box`](PointerRepr::Box). [`Custom`](PointerRepr::Custom)
/// substitutes any pointer that satisfies the `buffa::ProtoBox<T>` bound — for
/// example a `smallbox`-style pointer that stores small messages inline.
/// Because the pointer *wraps* the message type, its path is a **template**
/// containing a `*` placeholder for the message type (e.g.
/// `"::smallbox::SmallBox<*, ::smallbox::space::S4>"` or
/// `"::my_crate::SmallBox<*>"`).
///
/// Because `buffa::ProtoBox` is buffa-owned, a *foreign* pointer cannot
/// implement it directly (orphan rule) — the template must name a crate-local
/// newtype, mirroring the `ProtoString` newtype expectation.
///
/// Select a representation through `buffa_build`'s `box_type_custom` builder
/// method. The wire format is identical regardless of the pointer; view types
/// are unaffected. Applies to singular message fields and **boxed** oneof
/// message/group variants (a variant opted into inline storage via
/// `unboxed_oneof_fields` takes precedence and gets no pointer). Repeated
/// message fields use a collection, not a pointer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PointerRepr {
    /// `::buffa::alloc::boxed::Box<T>` (inside `MessageField<T>`). The opt-out
    /// from the `Inline` default for large or rarely-set submessages, via
    /// `box_type_in(PointerRepr::Box, paths)` (or `box_type(PointerRepr::Box)`
    /// to restore the pre-0.9 global default).
    Box,
    /// `::buffa::Inline<T>` — store the message directly in the parent struct,
    /// no heap allocation. `MessageField<T, Inline<T>>` is laid out as
    /// `Option<T>`. The default.
    ///
    /// Recursion-aware: a singular field that would form an infinite-size cycle
    /// (directly, mutually, or via an
    /// [`unbox_oneof`](CodeGenConfig::unboxed_oneof_fields)-inlined oneof
    /// variant) is silently kept on `Box`, so the default is always sized. An
    /// *exact-path* `Inline` rule that names a recursive field is rejected at
    /// codegen time.
    #[default]
    Inline,
    /// A custom pointer named by a Rust type-path **template** with a `*`
    /// placeholder for the message type. Must satisfy `buffa::ProtoBox<T>` and
    /// be a crate-local newtype.
    ///
    /// # Limitations
    ///
    /// - The template must contain at least one `*`; a template that omits it
    ///   surfaces as [`CodeGenError::MissingWildcard`], and one whose
    ///   substitution does not parse as [`CodeGenError::InvalidTypePath`], at
    ///   generation (`.compile()`) time.
    /// - `Rc` / `Arc` and other shared/COW pointers are unusable: the decoder
    ///   merges in place (needs `DerefMut`), so only an exclusively-owned
    ///   pointer (heap `Box`, inline `SmallBox`) can implement `ProtoBox`.
    /// - An inline pointer inflates the parent struct per field, so select it
    ///   per field/prefix, never as a blanket default.
    /// - On a **boxed oneof variant** under the `arbitrary` feature, the custom
    ///   pointer must implement `arbitrary::Arbitrary` (the oneof enum derives it
    ///   and stores the pointer directly in the variant). The singular-field path
    ///   needs no such impl — `MessageField` constructs the pointer itself.
    Custom(String),
}

impl PointerRepr {
    /// The owned `MessageField<...>` type emitted for a singular message field
    /// with this representation, given the resolved inner message type tokens
    /// and the `MessageField` path from the resolver.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::MissingWildcard`] if a custom template omits `*`,
    /// or [`CodeGenError::InvalidTypePath`] if it does not parse once the message
    /// type is substituted.
    pub(crate) fn type_path(
        &self,
        message_field: &proc_macro2::TokenStream,
        inner: &proc_macro2::TokenStream,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            PointerRepr::Box => Ok(quote! { #message_field<#inner> }),
            PointerRepr::Inline => Ok(quote! { #message_field<#inner, ::buffa::Inline<#inner>> }),
            PointerRepr::Custom(template) => {
                let ptr = parse_wildcard_type_path(template, inner)?;
                Ok(quote! { #message_field<#inner, #ptr> })
            }
        }
    }

    /// The fully-qualified `::buffa::MessageField::<...>` path for a
    /// `::some(value)` construction of a singular message field with this
    /// representation: `<inner>` for `Box` (the pointer param defaults), or
    /// `<inner, ptr>` for a custom pointer. The view→owned conversion uses this
    /// so the constructed `MessageField` matches the field's declared type.
    ///
    /// # Errors
    ///
    /// As [`type_path`](Self::type_path) for a custom template.
    pub(crate) fn some_path(
        &self,
        inner: &proc_macro2::TokenStream,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            PointerRepr::Box => Ok(quote! { ::buffa::MessageField::<#inner> }),
            PointerRepr::Inline => {
                Ok(quote! { ::buffa::MessageField::<#inner, ::buffa::Inline<#inner>> })
            }
            PointerRepr::Custom(template) => {
                let ptr = parse_wildcard_type_path(template, inner)?;
                Ok(quote! { ::buffa::MessageField::<#inner, #ptr> })
            }
        }
    }

    /// The bare pointer type wrapping `inner` for a **boxed oneof variant**
    /// (`Box<inner>` by default, or the custom pointer). Unlike
    /// [`type_path`](Self::type_path) this is the pointer alone, not wrapped in
    /// `MessageField`, because a oneof enum stores the pointer directly in the
    /// variant.
    ///
    /// # Errors
    ///
    /// As [`type_path`](Self::type_path) for a custom template.
    pub(crate) fn pointer_type(
        &self,
        inner: &proc_macro2::TokenStream,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            PointerRepr::Box => Ok(quote! { ::buffa::alloc::boxed::Box<#inner> }),
            PointerRepr::Inline => Ok(quote! { ::buffa::Inline<#inner> }),
            PointerRepr::Custom(template) => parse_wildcard_type_path(template, inner),
        }
    }

    /// Construct the pointer from a value expression for a boxed oneof variant:
    /// `Box::new(value)` (byte-identical default) or the fully-qualified
    /// `<Ptr as ProtoBox<inner>>::new(value)` for a custom pointer (so an
    /// inherent `new` on the pointer can't shadow the trait method).
    ///
    /// # Errors
    ///
    /// As [`type_path`](Self::type_path) for a custom template.
    pub(crate) fn pointer_new(
        &self,
        inner: &proc_macro2::TokenStream,
        value: &proc_macro2::TokenStream,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            PointerRepr::Box => Ok(quote! { ::buffa::alloc::boxed::Box::new(#value) }),
            PointerRepr::Inline => Ok(quote! { ::buffa::Inline(#value) }),
            PointerRepr::Custom(template) => {
                let ptr = parse_wildcard_type_path(template, inner)?;
                Ok(quote! { <#ptr as ::buffa::ProtoBox<#inner>>::new(#value) })
            }
        }
    }
}

/// The owned Rust collection a proto `repeated` field maps to in generated
/// owned structs.
///
/// The default is [`Vec`](RepeatedRepr::Vec) (`Vec<T>`).
/// [`Custom`](RepeatedRepr::Custom) substitutes any collection that satisfies
/// the `buffa::ProtoList<T>` bound — for example a crate-local newtype wrapping
/// a `SmallVec`-backed inline collection. Unlike the scalar `string`/`bytes`
/// knobs the custom collection *wraps* the element type, so its path is a
/// **template** containing a `*` placeholder where the element type is
/// substituted (e.g. `"::my_crate::SmallList<*>"`).
///
/// Because `buffa::ProtoList` is buffa-owned, a *foreign* collection cannot
/// implement it directly (orphan rule) — the template must always name a
/// crate-local newtype, mirroring the `ProtoString` newtype expectation.
///
/// Select a representation through `buffa_build`'s `repeated_type_custom`
/// builder method. The wire format is identical regardless of the collection;
/// view types keep borrowing `&[T]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RepeatedRepr {
    /// `::buffa::alloc::vec::Vec<T>` — the default. Keeps the `Vec`-specialized
    /// fast paths (in-place `push`/`reserve`/`clear`, native `Arbitrary`)
    /// instead of the generic `ProtoList` ones, so generated output for the
    /// default is byte-identical to a build without the knob.
    #[default]
    Vec,
    /// A custom collection named by a Rust type-path **template** with a `*`
    /// placeholder for the element type (e.g. `"::my_crate::SmallList<*>"`). The
    /// named type must satisfy `buffa::ProtoList<T>` and be a **crate-local
    /// newtype** (a foreign collection cannot implement the buffa-owned
    /// `ProtoList`).
    ///
    /// # Limitations
    ///
    /// - The template must contain at least one `*`; the element type is
    ///   substituted for every `*` before the result is parsed as a Rust type.
    ///   A template that omits `*` surfaces as
    ///   [`CodeGenError::MissingListPlaceholder`], and one whose substitution
    ///   does not parse as [`CodeGenError::InvalidTypePath`], at generation
    ///   (`.compile()`) time.
    /// - A custom collection always needs a crate-local newtype — this is not
    ///   limited to the reflection path. The generated decode and clear code
    ///   require `Field: ProtoList`, so even a binary-only build cannot use a
    ///   foreign collection directly.
    /// - Under reflection / vtable the newtype must implement
    ///   `buffa_descriptor`'s `ReflectList` (a `Vec`-backed newtype can delegate
    ///   to the inner `Vec<T>: ReflectList`). Under JSON it must implement
    ///   `serde::Serialize` / `Deserialize`; under the `arbitrary` feature,
    ///   `arbitrary::Arbitrary` (derivable on a newtype).
    /// - A `repeated <self-type>` field becomes `Collection<Self>`, so the
    ///   collection must be heap-backed; an inline collection (`SmallVec<[Self;
    ///   N]>`) would be infinitely sized and fail to compile.
    Custom(String),
}

impl RepeatedRepr {
    /// The owned Rust collection type emitted for a `repeated` field with this
    /// representation, given the already-resolved element type tokens.
    ///
    /// `ctx` and `nesting` route the default `Vec` through the package-root
    /// import registry; a custom template has its `*` placeholders replaced by
    /// `elem` and the result is parsed and emitted fully qualified.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::MissingListPlaceholder`] if a custom template
    /// omits `*`, or [`CodeGenError::InvalidTypePath`] if it does not parse as a
    /// Rust type once the element is substituted.
    pub(crate) fn type_path(
        &self,
        elem: &proc_macro2::TokenStream,
        resolver: &imports::ImportResolver,
        ctx: &context::CodeGenContext,
        nesting: usize,
    ) -> Result<proc_macro2::TokenStream, CodeGenError> {
        use quote::quote;
        match self {
            RepeatedRepr::Vec => {
                let vec = resolver.vec_at(ctx, nesting);
                Ok(quote! { #vec<#elem> })
            }
            RepeatedRepr::Custom(template) => parse_custom_list_path(template, elem),
        }
    }

    /// Whether this is the default `Vec` representation, which keeps the
    /// `Vec`-specialized fast paths instead of the generic `ProtoList` ones.
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, RepeatedRepr::Vec)
    }
}

/// How much reflection support generated types get.
///
/// Selected through `buffa_build`'s `reflect_mode` builder method (or the
/// `protoc-gen-buffa` `reflect_mode=` option). All modes need the consuming
/// crate to depend on `buffa-descriptor` with its `reflect` feature and on
/// `std`; the call site is `foo.reflect().get(fd)` regardless of mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ReflectMode {
    /// No reflection impls.
    #[default]
    Off,
    /// `Reflectable::reflect()` round-trips the message through a
    /// `DynamicMessage` (encode → decode → boxed handle). Smaller generated
    /// code; pays an allocation and a re-encode per `reflect()` call.
    Bridge,
    /// `impl ReflectMessage` directly on the owned and view types, and
    /// `Reflectable::reflect()` borrows `self` with no round-trip. Larger
    /// generated code; near-free reflective access. Does not require view
    /// generation — with views off, only the owned impls are emitted.
    VTable,
}

impl ReflectMode {
    /// Apply this mode to a [`CodeGenConfig`] (sets `generate_reflection` /
    /// `generate_reflection_vtable`). Used by the `buffa-build` and
    /// `protoc-gen-buffa` front-ends.
    pub fn apply(self, config: &mut CodeGenConfig) {
        let (reflection, vtable) = match self {
            ReflectMode::Off => (false, false),
            ReflectMode::Bridge => (true, false),
            ReflectMode::VTable => (true, true),
        };
        config.generate_reflection = reflection;
        config.generate_reflection_vtable = vtable;
    }
}

/// Configuration for code generation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CodeGenConfig {
    /// Whether to generate borrowed view types (`MyMessageView<'a>`) in
    /// addition to owned types.
    pub generate_views: bool,
    /// Whether to additionally generate the lazy view family
    /// (`MyMessageLazyView<'a>`) alongside the eager views (default: false).
    ///
    /// Lazy views implement `buffa::LazyMessageView`: `decode_lazy` performs
    /// a single non-recursive scan, recording singular/repeated message
    /// fields as undecoded byte ranges (`LazyMessageFieldView` /
    /// `LazyRepeatedView`) that decode on access — reading a few fields of
    /// many sub-messages no longer allocates or recurses into untouched
    /// sub-trees. The eager `MyMessageView` family is unchanged (output is
    /// byte-identical with or without this flag), so eager and lazy views
    /// coexist and generic `MessageView` consumers never silently inherit
    /// deferred validation.
    ///
    /// Semantics of the lazy family:
    ///
    /// - **Eager carve-outs**: groups / editions `DELIMITED` fields (no
    ///   length prefix to defer), oneof message variants, and map message
    ///   values use the eager view types.
    /// - **Merge preserved**: a singular message field split across wire
    ///   occurrences is recorded as fragments and merged on access.
    /// - **Budgets flow**: the recursion depth and unknown-field allowance
    ///   remaining at each deferred field are recorded and replayed per
    ///   access (a per-subtree approximation of the shared allowance).
    /// - **Deferred validation**: malformed deferred bytes error on access,
    ///   from the fallible `to_owned_message`, and as a serde error from the
    ///   view `Serialize` impl. `ViewEncode` replays recorded fragments
    ///   **without validating them**.
    /// - No `ReflectMessage`, `OwnedView`, or text-format surface — use the
    ///   eager family for those.
    ///
    /// Requires [`generate_views`](Self::generate_views) (the lazy family
    /// reuses the eager view-oneof enums and eager sub-view types); with
    /// views disabled the flag is ignored with a warning.
    pub lazy_views: bool,
    /// Whether to preserve unknown fields (default: true).
    pub preserve_unknown_fields: bool,
    /// Whether to derive `serde::Serialize` / `serde::Deserialize` on
    /// generated message structs and enum types, and emit `#[serde(with = "...")]`
    /// attributes for proto3 JSON's special scalar encodings (int64 as quoted
    /// string, bytes as base64, etc.).
    ///
    /// When this is `true`, the downstream crate must depend on `serde` and
    /// must enable the `buffa/json` feature for the runtime helpers.
    ///
    /// Oneof fields use `#[serde(flatten)]` with custom `Serialize` /
    /// `Deserialize` impls so that each variant appears as a top-level
    /// JSON field (proto3 JSON inline oneof encoding).
    pub generate_json: bool,
    /// Whether to emit `#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]`
    /// on generated message structs and enum types.
    ///
    /// When this is `true`, the downstream crate must add `arbitrary` as an
    /// optional dependency and enable the `buffa/arbitrary` feature. The
    /// downstream crate's Cargo feature that gates `arbitrary` must be named
    /// exactly `"arbitrary"` — the generated `cfg_attr` uses that literal
    /// string and cannot be customized. This applies to both the struct-level
    /// `derive(Arbitrary)` and the per-field `#[arbitrary(with = ...)]`
    /// attributes emitted for `bytes_fields`-typed fields.
    ///
    /// For `bytes_fields`-typed fields, codegen emits `#[arbitrary(with = ...)]`
    /// using helpers in `::buffa::__private` since `bytes::Bytes` has no
    /// `Arbitrary` impl. Singular, optional, and repeated bytes fields are all
    /// covered. Map values are always `Vec<u8>` regardless of `bytes_fields`
    /// and require no special handling.
    pub generate_arbitrary: bool,
    /// External type path mappings.
    ///
    /// Each entry maps either a fully-qualified protobuf package prefix
    /// (e.g., `".my.common"`) to a Rust module path (e.g.,
    /// `"::common_protos"`), or a single type FQN (e.g.,
    /// `".my.common.Shared"`) to a full Rust type path (e.g.,
    /// `"::shared_types::Shared"`). Matched types reference the extern Rust
    /// path instead of being generated, allowing shared proto packages to be
    /// compiled once in a dedicated crate and referenced from others. An
    /// exact type-FQN entry wins over a covering package prefix; otherwise
    /// the longest matching prefix wins.
    ///
    /// Well-known types (`google.protobuf.*`) are automatically mapped to
    /// `::buffa_types::google::protobuf::*` without needing an explicit
    /// entry here. To override with a custom implementation, add an
    /// `extern_path` for `.google.protobuf` pointing to your crate.
    pub extern_paths: Vec<(String, String)>,
    /// Ordered (proto-path-prefix, [`BytesRepr`]) rules selecting the Rust type
    /// for `bytes` fields. Later rules win, so a broad rule (e.g. `"."` →
    /// `Bytes`) can be refined by a more specific one. Fields matching no rule
    /// use `Vec<u8>`. The path is matched with the same proto-segment-aware
    /// prefix logic as [`string_fields`](Self::string_fields).
    pub bytes_fields: Vec<(String, BytesRepr)>,
    /// Ordered (proto-path-prefix, [`StringRepr`]) rules selecting the Rust type
    /// for `string` fields. Later rules win, so a broad rule (e.g. `"."` →
    /// `SmolStr`) can be refined by a more specific one
    /// (`".my.pkg.Msg.field"` → `CompactString`). Fields matching no rule use
    /// `String`. The path is matched with the same proto-segment-aware prefix
    /// logic as [`bytes_fields`](Self::bytes_fields).
    ///
    /// Applies to singular, optional, and repeated `string` fields and oneof
    /// `string` variants. Map keys and values always stay `String`, mirroring
    /// the bytes path (where map values always stay `Vec<u8>`).
    pub string_fields: Vec<(String, StringRepr)>,
    /// Ordered (proto-path-prefix, [`MapRepr`]) rules selecting the owned Rust
    /// map collection for `map` fields. Later rules win, with the same
    /// proto-segment-aware prefix matching as [`bytes_fields`](Self::bytes_fields)
    /// (`"."` matches every field). Fields matching no rule use `HashMap<K, V>`.
    ///
    /// Independent of the element/value representation: a `map` field's key and
    /// value types are chosen by the usual scalar/string/bytes/message rules,
    /// and this knob only changes the surrounding collection.
    pub map_fields: Vec<(String, MapRepr)>,
    /// Ordered (proto-path-prefix, [`PointerRepr`]) rules selecting the owned
    /// smart pointer for singular message fields (the pointer inside
    /// `MessageField<T>`). Later rules win, same proto-segment-aware prefix
    /// matching as [`bytes_fields`](Self::bytes_fields). Fields matching no rule
    /// use `Box<T>`.
    ///
    /// Applies to singular (and proto2 optional/required) message fields only —
    /// not repeated message fields (a collection) or oneof message variants.
    pub pointer_fields: Vec<(String, PointerRepr)>,
    /// Ordered (proto-path-prefix, [`RepeatedRepr`]) rules selecting the owned
    /// Rust collection for `repeated` fields. Later rules win, with the same
    /// proto-segment-aware prefix matching as [`bytes_fields`](Self::bytes_fields)
    /// (`"."` matches every field). Fields matching no rule use `Vec<T>`.
    ///
    /// Applies only to `repeated` fields (not `map`, whose collection stays
    /// the configured map type). The element type is chosen by the usual
    /// scalar/string/bytes/message rules and substituted into the collection
    /// template.
    pub repeated_fields: Vec<(String, RepeatedRepr)>,
    /// Fully-qualified proto paths whose message-typed oneof variants should
    /// **not** be wrapped in `Box<T>`. By default every message/group oneof
    /// variant is boxed (so recursive types compile); entries here opt matching
    /// variants out, storing the message inline in the enum.
    ///
    /// Each entry is a proto path prefix matched with the same
    /// proto-segment-aware logic as [`bytes_fields`](Self::bytes_fields)
    /// (`"."` matches every variant). Recursive variants cannot be stored
    /// inline (the type would be unsized): an entry naming one *exactly* is
    /// rejected at codegen time, while a broader prefix entry silently keeps
    /// recursive variants boxed and inlines the rest.
    pub unboxed_oneof_fields: Vec<String>,
    /// Honor `features.utf8_validation = NONE` by emitting `Vec<u8>` / `&[u8]`
    /// for such string fields instead of `String` / `&str`.
    ///
    /// When `false` (the default), buffa emits `String` for all string fields
    /// and **validates UTF-8 on decode** — stricter than proto2 requires, but
    /// ergonomic and safe.
    ///
    /// When `true`, string fields with `utf8_validation = NONE` (all proto2
    /// strings by default, and editions fields that opt into `NONE`) become
    /// `Vec<u8>` / `&[u8]`. Decode skips validation; the caller decides at the
    /// call site whether to `std::str::from_utf8` (checked) or
    /// `from_utf8_unchecked` (trusted-input fast path). This is the only
    /// sound Rust mapping when strings may actually contain non-UTF-8 bytes.
    ///
    /// **This is a breaking change for proto2** — enable only for new code or
    /// when profiling identifies UTF-8 validation as a bottleneck.
    pub strict_utf8_mapping: bool,
    /// Permit `option message_set_wire_format = true` on input messages.
    ///
    /// MessageSet is a legacy Google-internal wire format that wraps each
    /// extension in a group structure instead of using regular field tags.
    /// When `false` (the default), encountering such a message is a codegen
    /// error — the flag exists to make MessageSet use explicit, since the
    /// format is obsolete outside of interop with very old Google protos.
    pub allow_message_set: bool,
    /// Whether to emit `impl buffa::text::TextFormat` on generated message
    /// structs for textproto (human-readable text format) encoding/decoding.
    ///
    /// When this is `true`, the downstream crate must enable the `buffa/text`
    /// feature for the runtime encoder/decoder.
    pub generate_text: bool,
    /// Whether the per-package `.mod.rs` stitcher emits
    /// `__buffa::register_types(&mut TypeRegistry)`.
    ///
    /// Default `true`. The fn aggregates `Any` type entries and extension
    /// entries for every message in the package. Set to `false` for
    /// crates that don't use extensions/`Any`, or that hand-roll
    /// registration (e.g. `buffa-types`' `register_wkt_types`, which
    /// knows the JSON-Any `is_wkt` special-casing the generic fn does
    /// not). The per-message `__*_JSON_ANY` / `__*_TEXT_ANY` consts are
    /// still emitted; only the aggregating fn is suppressed.
    pub emit_register_fn: bool,
    /// Emit one `<dotted.package>.rs` per proto package instead of the
    /// per-proto-file content set plus `<pkg>.mod.rs` stitcher.
    ///
    /// The single file inlines what the stitcher would otherwise `include!`,
    /// producing the same `__buffa::{view,oneof,ext,...}` module structure.
    /// Intended for Buf Schema Registry generated SDKs, whose `lib.rs`
    /// synthesis builds the module tree from `<dotted.package>.rs` filenames.
    ///
    /// Under `strategy: directory` this only sees one directory's files per
    /// invocation, so the input module must be `PACKAGE_DIRECTORY_MATCH`-clean
    /// (one package per directory) for the output to be complete. BSR-hosted
    /// modules satisfy this by lint default. If a package spans multiple
    /// directories, separate invocations each emit their own `<pkg>.rs` and
    /// the last write wins — silent partial output, not a codegen error.
    pub file_per_package: bool,
    /// Custom attributes to inject on generated types (messages, enums, and
    /// oneof enums — the latter matched on the oneof's own path,
    /// `.my.pkg.MyMessage.my_oneof`).
    ///
    /// Each entry is `(proto_path, attribute)`. The `proto_path` is matched
    /// as a prefix against the fully-qualified proto name: `"."` applies to
    /// all types, `".my.pkg"` to types in that package, `".my.pkg.MyMessage"`
    /// to a specific type. The `attribute` is a raw Rust attribute string
    /// (e.g., `"#[derive(serde::Serialize)]"`).
    pub type_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated struct fields.
    ///
    /// Each entry is `(proto_path, attribute)`. The `proto_path` is matched
    /// as a prefix against the fully-qualified field path (e.g.,
    /// `".my.pkg.MyMessage.my_field"`). `"."` applies to all fields.
    pub field_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated message structs only (not enums).
    ///
    /// Same path-matching semantics as `type_attributes`, but only applied to
    /// message structs, not enum types. Useful for struct-only attributes like
    /// `#[serde(default)]`.
    pub message_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated enum types only (not messages).
    ///
    /// Same path-matching semantics as `type_attributes`, but only applied to
    /// enum types. Useful for enum-only attributes like
    /// `#[derive(strum::EnumIter)]` when the user does not want to apply the
    /// same attribute to every message in the matched scope.
    pub enum_attributes: Vec<(String, String)>,
    /// Custom attributes to inject on generated oneof enums only (not messages,
    /// not regular enums).
    ///
    /// Same path-matching semantics as `type_attributes`, matched against the
    /// oneof's fully-qualified path (`.pkg.Message.oneof_name`). Useful when a
    /// oneof needs a different attribute set than the surrounding types — e.g.
    /// keeping `#[derive(serde::Serialize)]` on messages and oneofs while a
    /// separate `enum_attributes` entry puts a different serde derive on the
    /// regular enums.
    pub oneof_attributes: Vec<(String, String)>,
    /// Wrap generated `impl`s in `#[cfg(feature = "...")]` instead of
    /// emitting them unconditionally.
    ///
    /// When `true`, the impls controlled by [`generate_json`],
    /// [`generate_views`], and [`generate_text`] are emitted wrapped in
    /// `#[cfg(feature = "json" | "views" | "text")]` (or
    /// `#[cfg_attr(feature = ..., ...)]` for derives and field attributes)
    /// rather than unconditionally. The consuming crate must define matching
    /// Cargo features that enable the corresponding runtime support, e.g.:
    ///
    /// ```toml
    /// [features]
    /// json  = ["buffa/json", "dep:serde", "dep:serde_json"]
    /// views = []
    /// text  = ["buffa/text"]
    /// ```
    ///
    /// The [`generate_*`] flags still control *whether* an impl kind is
    /// emitted at all — this flag only controls whether it is `cfg`-gated.
    /// `generate_arbitrary` is always `cfg_attr`-gated on
    /// `feature = "arbitrary"` regardless of this flag, because `arbitrary`
    /// is an optional dependency by design.
    ///
    /// When [`generate_reflection`](Self::generate_reflection) is also on, the
    /// reflection impls are gated on `feature = "reflect"` alongside
    /// json/views/text. To gate *only* reflection without gating json/views/text,
    /// use [`gate_reflect_on_crate_feature`](Self::gate_reflect_on_crate_feature)
    /// instead.
    ///
    /// This is the mechanism that lets `buffa-descriptor` and `buffa-types`
    /// ship every impl while keeping the codegen toolchain
    /// (`buffa-codegen`/`buffa-build`/`protoc-gen-buffa`) lean: those crates
    /// depend on `buffa-descriptor` with `default-features = false` and so
    /// don't pull `serde`/`serde_json`/`base64`. Most consumers don't need
    /// this — they decide at build-script time whether to generate JSON, and
    /// if they say yes, they want `impl Serialize` to just exist.
    ///
    /// [`generate_json`]: Self::generate_json
    /// [`generate_views`]: Self::generate_views
    /// [`generate_text`]: Self::generate_text
    /// [`generate_*`]: Self::generate_json
    pub gate_impls_on_crate_features: bool,
    /// Generate `with_*` builder-style setter methods for explicit-presence fields.
    ///
    /// Each explicit-presence scalar, bytes, or enum field gets a
    /// `pub fn with_<name>(mut self, value: T) -> Self` method that wraps the
    /// value in `Some` and returns `self`, enabling chained construction:
    ///
    /// ```ignore
    /// let req = MyRequest::default()
    ///     .with_name("alice")
    ///     .with_timeout_ms(30_000);
    /// ```
    ///
    /// **Fields that receive a setter:** proto3 `optional`, proto2 `optional`,
    /// and editions fields with `field_presence = EXPLICIT`.
    ///
    /// **Fields that do not receive a setter:** message fields
    /// (`MessageField<T>`), repeated fields, map fields, oneof variant fields,
    /// proto2 `required` fields, and any implicit-presence field.
    ///
    /// There is no `clear_<name>` companion — to clear a field, assign `None`
    /// directly: `msg.name = None;`.
    ///
    /// Defaults to `true`.
    pub generate_with_setters: bool,
    /// Generate `impl Reflectable` for owned message types (bridge mode).
    ///
    /// When enabled, each generated message gets an
    /// `impl ::buffa_descriptor::reflect::Reflectable` whose `reflect()`
    /// round-trips through `DynamicMessage` (encode → decode → reflective
    /// handle), and the package's `__buffa::reflect` submodule embeds the
    /// `FileDescriptorSet` bytes plus a lazily-built `DescriptorPool`.
    ///
    /// **Runtime requirements** — the consuming crate must depend on:
    /// - `buffa-descriptor` with the `reflect` feature.
    /// - `std` (the lazy pool accessor uses `std::sync::OnceLock`).
    ///
    /// When [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features)
    /// is on, the impls are wrapped in `#[cfg(feature = "reflect")]` so the
    /// consuming crate can opt out per build.
    ///
    /// **Performance** — `reflect()` is one full encode/decode round-trip
    /// plus a heap allocation. The first call also pays a one-time pool
    /// build cost (linking the embedded `FileDescriptorSet`). For zero-copy
    /// reflective access over view types without the round-trip, additionally
    /// enable [`generate_reflection_vtable`](Self::generate_reflection_vtable).
    ///
    /// **Binary size** — each package embeds its own copy of the full
    /// `FileDescriptorSet` (transitive closure). For a multi-package
    /// codegen run this duplicates the FDS bytes per package. Acceptable
    /// for the bridge prototype; deduplication via a crate-root module is
    /// a planned follow-up.
    ///
    /// Defaults to `false`.
    pub generate_reflection: bool,
    /// Emit vtable-mode reflection: `impl ReflectMessage` / `impl
    /// ReflectElement` on the owned message structs and (when views are
    /// generated) the view types, and switch the owned
    /// `Reflectable::reflect()` body to borrow `self`
    /// (`ReflectCow::Borrowed(self)`) instead of the bridge round-trip.
    ///
    /// Reflective access then reads struct fields in place — no encode/decode
    /// round-trip and no per-field allocation — for both a decoded view and an
    /// in-memory owned message.
    ///
    /// Requires [`generate_reflection`](Self::generate_reflection) (the impls
    /// resolve against the same embedded `DescriptorPool`) but not
    /// [`generate_views`](Self::generate_views) — with views off, only the
    /// owned impls are emitted. Set via [`ReflectMode::VTable`]
    /// — front-ends expose it as `buffa_build::Config::reflect_mode` /
    /// `protoc-gen-buffa`'s `reflect_mode=vtable`.
    ///
    /// Defaults to `false`.
    pub generate_reflection_vtable: bool,
    /// Gate the reflection impls behind a `reflect` crate feature, *without*
    /// gating json/views/text (unlike
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features),
    /// which gates them all together).
    ///
    /// Used by crates that ship view/text impls unconditionally but want the
    /// reflection surface — which pulls a `buffa-descriptor` dependency and
    /// `std` — to be opt-in. `buffa-types` is the motivating case: its WKT
    /// views are always available, but `impl ReflectMessage` for them is gated
    /// behind `buffa-types`'s `reflect` feature.
    ///
    /// When [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features)
    /// is already on, reflection is gated regardless and this flag is ignored.
    ///
    /// A low-level knob for crates whose generated code is a public interface
    /// (`buffa-types`, the conformance harness). Set directly by `gen_wkt_types`
    /// and exposed through `buffa_build::Config::gate_reflect_on_crate_feature`
    /// (currently `#[doc(hidden)]`, paired with the experimental vtable flag).
    ///
    /// Defaults to `false`.
    pub gate_reflect_on_crate_feature: bool,
    /// Emit idiomatic `UpperCamelCase` constant aliases alongside each enum
    /// variant.
    ///
    /// Protobuf style names enum values in `SHOUTY_SNAKE_CASE`, conventionally
    /// prefixed with the enum name (`RULE_LEVEL_HIGH`). Those names remain the
    /// definitive Rust variants — they are guaranteed unique and valid by
    /// protobuf, and existing references (including `Debug` output) are
    /// unchanged. When this is enabled, codegen additionally emits associated
    /// `const`s with the prefix stripped and the name converted to
    /// `UpperCamelCase` (`RULE_LEVEL_HIGH` → `High`), so downstream code can
    /// write `RuleLevel::High`.
    ///
    /// The conversion is lossy, so two values can collide (`FOO_BAR` and
    /// `FOO__BAR` both map to `FooBar`). The rule is all-or-nothing per enum:
    /// if any two values would collide after conversion, or a value would yield
    /// an invalid identifier, **no** aliases are emitted for that enum (a
    /// [`CodeGenWarning`] and an enum doc note explain why). This keeps every
    /// match either fully `SHOUTY_SNAKE_CASE` or fully idiomatic, never a forced
    /// mix.
    ///
    /// The aliases are associated `const`s, which work in pattern position too:
    /// a `match` written entirely against aliases is still exhaustiveness-checked
    /// (the "non-exhaustive" error names the underlying `SHOUTY_SNAKE_CASE`
    /// variant, since that is the canonical name).
    ///
    /// Defaults to `true`: the aliases are purely additive (the proto names
    /// remain the variants, and `Debug` is unchanged), so enabling by default is
    /// backward-compatible, and the all-or-nothing rule guarantees correctness on
    /// any enum.
    pub idiomatic_enum_aliases: bool,
    /// Emit `use`-backed short type names at the package root instead of
    /// fully-qualified paths, so generated code reads like hand-written
    /// Rust (`pub at: MessageField<Timestamp>` instead of
    /// `pub at: ::buffa::MessageField<::buffa_types::google::protobuf::Timestamp>`).
    ///
    /// Requires [`file_per_package`](Self::file_per_package): only there is
    /// the package-root scope a single-writer file whose complete name set
    /// is known at generation time. In the multi-file layout the stitcher
    /// `include!`-merges every proto's content files into the shared root
    /// scope, where emitted `use` directives could collide across files —
    /// [`generate`] returns an error for that combination rather than
    /// silently ignoring the flag.
    ///
    /// Off by default; default output is byte-for-byte unchanged. Short
    /// names are always backed by an explicit `use` (never glob reliance),
    /// are refused when they would collide with the package's own items or
    /// names referenced bare by sibling emissions, and fall back to
    /// parent-module qualification and then the fully-qualified path. The
    /// short-name *assignment* (use block and per-path choices) is computed
    /// from a collection pre-pass and is stable under `.proto` file
    /// reordering; item order within the file still follows input order,
    /// so whole-file output is not reorder-invariant. The pre-pass
    /// generates the package twice, roughly doubling codegen time for it.
    ///
    /// Scope: only package-root *type declarations* (struct fields, oneof
    /// `Option` wrappers) are shortened. Impl bodies, nested-message
    /// modules, and `__buffa` internals keep fully-qualified paths — the
    /// readability payoff lands where consumers look (struct definitions
    /// and rustdoc), not in the codec internals.
    ///
    /// **Experimental** means: the generated-output shape may change
    /// between releases (requiring regeneration of checked-in code), and
    /// the option itself may be renamed or removed outside semver
    /// guarantees.
    pub idiomatic_imports: bool,
    /// Crate feature names used by the `#[cfg(feature = "...")]` gates that
    /// [`gate_impls_on_crate_features`](Self::gate_impls_on_crate_features)
    /// and
    /// [`gate_reflect_on_crate_feature`](Self::gate_reflect_on_crate_feature)
    /// emit.
    ///
    /// Defaults to `"json"` / `"views"` / `"text"` / `"reflect"`. Override a
    /// name when the consuming crate gates the same concern behind a
    /// different feature name (e.g. its JSON support behind a `serde`
    /// feature). Inert unless one of the gating flags is on.
    pub feature_gate_names: FeatureGateNames,
    /// Prefix prepended to every locally-generated Rust type name.
    ///
    /// With prefix `"Rpc"`, `message User {}` generates `struct RpcUser`,
    /// its view becomes `RpcUserView` / `RpcUserOwnedView`, and every
    /// cross-reference (fields, oneof variants, maps, extensions) uses the
    /// prefixed name. Useful in multi-protocol systems where generated
    /// types from different domains would otherwise collide with each
    /// other or with a canonical hand-written model.
    ///
    /// The prefix applies to **message structs and enum types** (top-level
    /// and nested, plus their derived view/owned-view types). It does not
    /// apply to:
    ///
    /// - module names (`message Outer` still nests under `pub mod outer` —
    ///   modules are namespaced by the package tree and never collide with
    ///   type names),
    /// - oneof enums (structurally namespaced under `__buffa::oneof::`,
    ///   named after the oneof declaration, not the message),
    /// - types mapped away via [`extern_paths`](Self::extern_paths) or the
    ///   automatic well-known-type mapping (their names are owned by the
    ///   external crate),
    /// - wire-format and JSON output (proto names, `TYPE_URL`s, and JSON
    ///   field names are unaffected — this is a pure Rust-identifier
    ///   rename).
    ///
    /// When another codegen run references these prefixed types via its own
    /// [`extern_paths`](Self::extern_paths) mapping, the mapped Rust path
    /// must spell out the prefixed name (e.g. `::crate_a::RpcUser`) — the
    /// proto name carries no prefix, so the mapping is not derived
    /// automatically. Prefix-induced name collisions (e.g. `message RpcUser`
    /// alongside `message User` with prefix `Rpc`) are not detected here;
    /// they surface as ordinary duplicate-definition errors when the
    /// generated code is compiled.
    ///
    /// Must be PascalCase (`[A-Z][A-Za-z0-9]*`) — an ASCII uppercase letter
    /// followed by ASCII letters and digits — so the prefixed names stay
    /// conventionally cased; generation fails with
    /// [`CodeGenError::InvalidTypeNamePrefix`] otherwise. Defaults to `""`
    /// (no prefix).
    pub type_name_prefix: String,
}

impl Default for CodeGenConfig {
    fn default() -> Self {
        Self {
            generate_views: true,
            lazy_views: false,
            preserve_unknown_fields: true,
            generate_json: false,
            generate_arbitrary: false,
            extern_paths: Vec::new(),
            bytes_fields: Vec::new(),
            string_fields: Vec::new(),
            map_fields: Vec::new(),
            pointer_fields: Vec::new(),
            repeated_fields: Vec::new(),
            unboxed_oneof_fields: Vec::new(),
            strict_utf8_mapping: false,
            allow_message_set: false,
            generate_text: false,
            emit_register_fn: true,
            file_per_package: false,
            type_attributes: Vec::new(),
            field_attributes: Vec::new(),
            message_attributes: Vec::new(),
            enum_attributes: Vec::new(),
            oneof_attributes: Vec::new(),
            gate_impls_on_crate_features: false,
            generate_with_setters: true,
            generate_reflection: false,
            generate_reflection_vtable: false,
            gate_reflect_on_crate_feature: false,
            idiomatic_enum_aliases: true,
            idiomatic_imports: false,
            feature_gate_names: FeatureGateNames::default(),
            type_name_prefix: String::new(),
        }
    }
}

impl CodeGenConfig {
    /// Active [`feature_gates::FeatureGates`] for this config.
    ///
    /// Recomputed on each call (cheap — three boolean ANDs); call once at
    /// the top of a generation function and thread through, or call inline
    /// at each use site, whichever reads better.
    pub(crate) fn feature_gates(&self) -> feature_gates::FeatureGates<'_> {
        feature_gates::FeatureGates::for_config(self)
    }

    /// Apply [`type_name_prefix`](Self::type_name_prefix) to a locally
    /// generated type's proto simple name, yielding the Rust identifier to
    /// declare (and register in the type map).
    pub(crate) fn prefixed_type_name(&self, proto_name: &str) -> String {
        format!("{}{proto_name}", self.type_name_prefix)
    }

    /// Validate [`type_name_prefix`](Self::type_name_prefix): empty (no
    /// prefix) or PascalCase (`[A-Z][A-Za-z0-9]*`), so `{prefix}{TypeName}`
    /// is always a valid, conventionally-cased identifier that does not
    /// trip `non_camel_case_types` in consumer crates.
    pub(crate) fn validate_type_name_prefix(&self) -> Result<(), CodeGenError> {
        let prefix = &self.type_name_prefix;
        let valid = prefix.is_empty()
            || (prefix.starts_with(|c: char| c.is_ascii_uppercase())
                && prefix.chars().all(|c| c.is_ascii_alphanumeric()));
        if valid {
            Ok(())
        } else {
            Err(CodeGenError::InvalidTypeNamePrefix {
                prefix: prefix.clone(),
            })
        }
    }
}

/// Compute the effective extern path list by starting with user-provided
/// mappings and adding the default WKT mapping if appropriate.
///
/// The default mapping `".google.protobuf" → "::buffa_types::google::protobuf"`
/// is added unless:
/// - The user already provided an extern_path covering `.google.protobuf`
/// - Any of the files being generated are in the `google.protobuf` package
///   (i.e., we're building `buffa-types` itself)
pub(crate) fn effective_extern_paths(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Vec<(String, String)> {
    let mut paths = config.extern_paths.clone();

    // Only an EXACT .google.protobuf mapping suppresses auto-injection.
    // A sub-package mapping like .google.protobuf.compiler does NOT cover
    // WKTs like Timestamp — resolve_extern_prefix's longest-prefix matching
    // lets both coexist, so we still inject the parent mapping.
    let has_wkt_mapping = paths.iter().any(|(proto, _)| proto == ".google.protobuf");

    if !has_wkt_mapping {
        // Check if we're generating google.protobuf files ourselves
        // (e.g., building buffa-types). If so, don't auto-map.
        let generating_wkts = file_descriptors
            .iter()
            .filter(|fd| {
                fd.name
                    .as_deref()
                    .is_some_and(|n| files_to_generate.iter().any(|f| f == n))
            })
            .any(|fd| fd.package.as_deref() == Some("google.protobuf"));

        if !generating_wkts {
            paths.push((
                ".google.protobuf".to_string(),
                "::buffa_types::google::protobuf".to_string(),
            ));
        }
    }

    paths
}

/// Compute the effective file-level extern path list.
///
/// File-level mappings route a specific `.proto` file to a Rust module root,
/// taking priority over the package-level mappings from
/// [`effective_extern_paths`]. They exist to resolve a structural problem:
/// `descriptor.proto` is in the same `google.protobuf` package as the
/// JSON-mappable WKTs (`Timestamp`, `Any`, …), but its types live in
/// `buffa-descriptor`, not `buffa-types`. A single package-keyed
/// `.google.protobuf` extern_path can route the package to one crate or the
/// other; it can't split it. The file-level mapping splits it.
///
/// Auto-injected mappings (when not suppressed):
///
/// | Proto file | Rust module |
/// |---|---|
/// | `google/protobuf/descriptor.proto` | `::buffa_descriptor::generated::descriptor` |
/// | `google/protobuf/compiler/plugin.proto` | `::buffa_descriptor::generated::compiler` |
///
/// Suppression conditions, evaluated **per file**:
///
/// - **A user-provided `extern_path` covers the file's package.** That
///   override has covered the file's types since the package mapping was
///   introduced; auto-injecting a higher-priority file-level mapping would
///   silently redirect them away from the user's crate. Matching is via
///   the same longest-prefix logic the package resolver uses, so both an
///   exact `.google.protobuf` mapping and a sub-package
///   `.google.protobuf.compiler` mapping suppress the entries they cover —
///   `.google.protobuf` suppresses both, `.google.protobuf.compiler`
///   suppresses only `plugin.proto`.
/// - **The proto file itself is in `files_to_generate`.** When building
///   `buffa-descriptor` (or any local copy of `descriptor.proto`), its types
///   must resolve to the local module, not externally.
///
/// Currently internal-only — there is no `CodeGenConfig` field for
/// user-provided *file-level* mappings. The user-facing `extern_path` API is
/// keyed by proto package *or* type FQN (per-type overrides, issue #111);
/// per-file overrides may be added later as a public feature if a concrete
/// need arises.
pub(crate) fn effective_file_extern_paths(
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Vec<(String, String)> {
    // (proto file path, proto package, Rust module root). The package is
    // recorded alongside the file so the user-override suppression check
    // is per-file: a `.google.protobuf.compiler` extern_path covers only
    // `plugin.proto`, while `.google.protobuf` covers both.
    const DESCRIPTOR_FILES: [(&str, &str, &str); 2] = [
        (
            "google/protobuf/descriptor.proto",
            "google.protobuf",
            "::buffa_descriptor::generated::descriptor",
        ),
        (
            "google/protobuf/compiler/plugin.proto",
            "google.protobuf.compiler",
            "::buffa_descriptor::generated::compiler",
        ),
    ];

    DESCRIPTOR_FILES
        .into_iter()
        .filter(|(proto_file, package, _)| {
            // Yield to a user package-level extern_path that already covers
            // this file's package: anyone who wrote
            // `extern_path(".google.protobuf", "::my_crate")` (or a
            // sub-package mapping) today routes these types to their crate;
            // the auto-injected file-level mapping must not silently
            // outrank it.
            if context::resolve_extern_prefix(package, &config.extern_paths).is_some() {
                return false;
            }
            // Don't externalize a file we're generating locally.
            !files_to_generate.iter().any(|f| f == proto_file)
        })
        .map(|(proto_file, _, rust_module)| (proto_file.to_string(), rust_module.to_string()))
        .collect()
}

/// One CamelCase collision: a target identifier and the proto value names that
/// would all convert onto it.
///
/// Part of [`CodeGenWarning::IdiomaticAliasesSuppressed`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AliasConflict {
    /// The `UpperCamelCase` identifier the colliding values map to.
    pub camel_target: String,
    /// The proto value names that convert onto `camel_target` (includes a
    /// literal variant name when an alias would shadow it).
    pub proto_values: Vec<String>,
}

/// A non-fatal diagnostic produced during code generation.
///
/// Returned by [`generate_with_diagnostics`]. Render the human-readable form via
/// the [`Display`](core::fmt::Display) impl (e.g. `cargo:warning={warning}`), or
/// match on the variant for programmatic handling. The enum and its variants are
/// `#[non_exhaustive]` so new diagnostic kinds and fields can be added without a
/// breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodeGenWarning {
    /// Idiomatic CamelCase aliases were suppressed for an enum because two or
    /// more proto values collide after conversion, or a value would convert to
    /// an invalid identifier. The enum's `SHOUTY_SNAKE_CASE` variants are
    /// unaffected.
    #[non_exhaustive]
    IdiomaticAliasesSuppressed {
        /// The Rust name of the affected enum.
        enum_name: String,
        /// Each collision, by target identifier. Empty if the only problem was
        /// invalid identifiers.
        conflicts: Vec<AliasConflict>,
        /// Proto values that would convert to an invalid Rust identifier.
        invalid: Vec<String>,
    },
    /// A field or oneof accessor on a generated `FooOwnedView` wrapper was
    /// suppressed because the proto name collides with one of the wrapper's
    /// reserved method names (`decode`, `view`, `bytes`, …). The field stays
    /// fully accessible through `view()` on the wrapper (or
    /// `OwnedView::reborrow`).
    #[non_exhaustive]
    OwnedViewAccessorSuppressed {
        /// The Rust name of the wrapper type (e.g. `FooOwnedView`).
        wrapper_name: String,
        /// The proto field or oneof name whose accessor was suppressed.
        field_name: String,
    },
    /// `lazy_views` was requested with `generate_views` disabled; the lazy
    /// family reuses the eager view-oneof enums and eager sub-view types, so
    /// no lazy views were generated. Emitted once per generation run.
    #[non_exhaustive]
    LazyViewsRequireViews,
}

impl core::fmt::Display for CodeGenWarning {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IdiomaticAliasesSuppressed {
                enum_name,
                conflicts,
                invalid,
            } => {
                // Name the cause accurately: a collision, an invalid identifier,
                // or both.
                let cause = match (conflicts.is_empty(), invalid.is_empty()) {
                    (false, true) => "naming conflict",
                    (true, false) => "invalid identifier",
                    _ => "naming conflict / invalid identifier",
                };
                write!(
                    f,
                    "enum `{enum_name}`: idiomatic CamelCase aliases suppressed ({cause})"
                )?;
                let mut parts: Vec<String> = conflicts
                    .iter()
                    .map(|c| format!("{} → {}", c.proto_values.join(", "), c.camel_target))
                    .collect();
                parts.extend(invalid.iter().map(|n| format!("{n} → invalid identifier")));
                if !parts.is_empty() {
                    write!(f, ": {}", parts.join("; "))?;
                }
                Ok(())
            }
            Self::OwnedViewAccessorSuppressed {
                wrapper_name,
                field_name,
            } => {
                write!(
                    f,
                    "`{wrapper_name}`: accessor for field `{field_name}` suppressed \
                     (collides with a reserved wrapper method); use `.view().{field_name}` instead"
                )
            }
            Self::LazyViewsRequireViews => {
                write!(
                    f,
                    "lazy_views requires generate_views (the lazy family reuses the \
                     eager view-oneof enums and sub-view types); no lazy views were \
                     generated — enable generate_views (buffa-build: \
                     `.generate_views(true)`, the default; plugin: `views=true`)"
                )
            }
        }
    }
}

/// Generate Rust source files from a set of file descriptors.
///
/// `files_to_generate` is the set of file names that were explicitly requested
/// (matching `CodeGeneratorRequest.file_to_generate`). Descriptors for
/// dependencies may be present in `file_descriptors` but won't produce output
/// files unless they appear in `files_to_generate`.
///
/// Each `.proto` emits up to five content files (kinds with no content
/// are omitted); each distinct package emits one `<pkg>.mod.rs`
/// stitcher. Packages are processed in sorted order for deterministic
/// output.
///
/// # Diagnostics
///
/// Non-fatal diagnostics produced during generation (e.g. an enum whose
/// idiomatic CamelCase aliases were suppressed by a naming conflict) are
/// **discarded** here. Use [`generate_with_diagnostics`] to receive them and
/// surface them as build warnings.
pub fn generate(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Result<Vec<GeneratedFile>, CodeGenError> {
    Ok(generate_with_diagnostics(file_descriptors, files_to_generate, config)?.0)
}

/// Like [`generate`], but also returns the non-fatal [`CodeGenWarning`]s
/// collected during generation (e.g. enums whose idiomatic CamelCase aliases
/// were suppressed by a naming conflict).
///
/// Surface each warning via its [`Display`](core::fmt::Display) impl — e.g. as a
/// `cargo:warning=...` from a `build.rs`, or on stderr from a standalone
/// generator — or match on it for programmatic handling. [`generate`] discards
/// them, so existing callers are unaffected.
///
/// Warnings are returned only on success. On error, any warnings already
/// collected are dropped along with the partial output — the [`CodeGenError`]
/// is the actionable signal.
///
/// # Errors
///
/// Returns [`CodeGenError::FileNotFound`] if a name in `files_to_generate` has
/// no matching descriptor, [`CodeGenError::InvalidTypeNamePrefix`] if
/// [`CodeGenConfig::type_name_prefix`] is not empty or PascalCase,
/// [`CodeGenError::Other`] if `generate_reflection_vtable`
/// is set without `generate_reflection` or if an active feature-gate name in
/// [`CodeGenConfig::feature_gate_names`] is not a valid Cargo feature name,
/// and other [`CodeGenError`] variants for malformed descriptors (e.g. a
/// missing required field) encountered while generating.
/// Whether a custom `repeated` element type holds proto `string` or `bytes` —
/// selects `ValueRef::String`/`ValueRef::Bytes` and the JSON delegate module.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CustomElemKind {
    String,
    Bytes,
}

/// The custom owned types collected generation-wide that need a codegen-emitted
/// reflection / JSON impl, split by the trait each needs.
#[derive(Default)]
struct CustomElements {
    /// Types needing `ReflectElement` (+ `ProtoElemJson` for bytes): custom
    /// `repeated` elements, custom `map` *values* (`string` or `bytes`).
    elements: std::collections::BTreeMap<String, CustomElemKind>,
    /// Custom `string` types used as a `map` *key*: need `ReflectMapKey` (vtable
    /// reflection only — the bridge path keys maps by the borrowed `&str` view).
    map_keys: std::collections::BTreeSet<String>,
}

/// Collect the distinct custom owned types that need a codegen-emitted element
/// impl (`ReflectElement` / `ProtoElemJson`), keyed by Rust type path, across
/// the whole request. These are custom `string`/`bytes` types used as the
/// element of a `repeated` field, and custom `bytes` types used as a
/// `map<K, bytes>` value — both reflect via the element trait and (for bytes)
/// serialize JSON via `proto_map`/`proto_seq`. Singular / optional / oneof
/// custom fields reach JSON and reflection without an element-trait impl, and
/// `string`/`Vec<u8>`/`Bytes` map values are covered by the built-in impls.
fn collect_custom_elements(
    ctx: &context::CodeGenContext,
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
) -> CustomElements {
    use crate::generated::descriptor::field_descriptor_proto::{Label, Type};

    fn walk(
        ctx: &context::CodeGenContext,
        messages: &[crate::generated::descriptor::DescriptorProto],
        scope: &str,
        parent_features: &crate::features::ResolvedFeatures,
        out: &mut CustomElements,
    ) {
        for msg in messages {
            let name = msg.name.as_deref().unwrap_or("");
            let fqn = if scope.is_empty() {
                name.to_string()
            } else {
                format!("{scope}.{name}")
            };
            let msg_features = crate::features::resolve_child(
                parent_features,
                crate::features::message_features(msg),
            );
            for field in &msg.field {
                if field.label.unwrap_or_default() != Label::LABEL_REPEATED {
                    continue;
                }
                let field_name = field.name.as_deref().unwrap_or("");
                let field_fqn = format!(".{fqn}.{field_name}");

                // `map` slots: a custom value type needs the element impls
                // (reflected via ReflectMap → ReflectElement, JSON via
                // proto_map → ProtoElemJson for bytes), and a custom `string`
                // key needs ReflectMapKey. All keyed on the outer map field
                // path (the same `string_type` rule covers both slots), with the
                // `map<bytes, bytes>` value carve-out.
                if let Some(entry) = crate::message::find_map_entry(msg, field) {
                    let key_ty = crate::message::map_entry_key_type(ctx, entry, &msg_features);
                    let val_ty = crate::message::map_entry_value_type(ctx, entry, &msg_features);
                    if let crate::BytesRepr::Custom(path) =
                        crate::impl_message::map_value_bytes_repr(
                            ctx, key_ty, val_ty, &fqn, field_name,
                        )
                    {
                        out.elements.entry(path).or_insert(CustomElemKind::Bytes);
                    }
                    if let crate::StringRepr::Custom(path) = ctx.string_repr(&field_fqn) {
                        if key_ty == Some(Type::TYPE_STRING) {
                            out.map_keys.insert(path.clone());
                        }
                        if val_ty == Some(Type::TYPE_STRING) {
                            out.elements.entry(path).or_insert(CustomElemKind::String);
                        }
                    }
                    continue;
                }

                let field_features = crate::features::resolve_field(ctx, field, &msg_features);
                let ty = crate::impl_message::effective_type(ctx, field, &field_features);
                match ty {
                    Type::TYPE_STRING => {
                        if let crate::StringRepr::Custom(path) = ctx.string_repr(&field_fqn) {
                            out.elements.entry(path).or_insert(CustomElemKind::String);
                        }
                    }
                    Type::TYPE_BYTES => {
                        if let crate::BytesRepr::Custom(path) = ctx.bytes_repr(&field_fqn) {
                            out.elements.entry(path).or_insert(CustomElemKind::Bytes);
                        }
                    }
                    _ => {}
                }
            }
            walk(ctx, &msg.nested_type, &fqn, &msg_features, out);
        }
    }

    let mut out = CustomElements::default();
    for file_name in files_to_generate {
        let Some(file) = file_descriptors
            .iter()
            .find(|f| f.name.as_deref() == Some(file_name.as_str()))
        else {
            continue;
        };
        let pkg = file.package.as_deref().unwrap_or("");
        let file_features = crate::features::for_file(file);
        walk(ctx, &file.message_type, pkg, &file_features, &mut out);
    }
    out
}

/// Render the deduped `ProtoElemJson` / `ReflectElement` impls for the collected
/// custom element types (repeated elements and `map<K, bytes>` values). Each
/// impl is feature-gated so a non-JSON /
/// non-reflect build never references an absent trait. These compile only when
/// the custom type is local to the generating crate (the orphan rule); that is
/// the documented limitation of a custom `repeated` element under JSON or vtable
/// reflection.
fn render_custom_elem_impls(
    ctx: &context::CodeGenContext,
    elems: &CustomElements,
) -> Result<TokenStream, CodeGenError> {
    let json_gate = ctx.config.feature_gates().json;
    let reflect_gate = ctx.config.feature_gates().reflect;
    let mut out = TokenStream::new();
    for (path, kind) in &elems.elements {
        let ty = parse_custom_type_path(path)?;
        // `ProtoElemJson` is only needed for the `bytes` element path (proto3
        // JSON base64). A repeated `string` element serializes through the
        // native `Vec<T>` serde derive, and custom `string` map keys/values go
        // through serde too (the derive / `string_key_map` / `proto_str_key_map`
        // paths), so a String-kind `ProtoElemJson` impl would be dead code.
        if ctx.config.generate_json && *kind == CustomElemKind::Bytes {
            out.extend(feature_gates::cfg_block(
                quote! {
                    impl ::buffa::json_helpers::ProtoElemJson for #ty {
                        fn serialize_proto_json<S: ::serde::Serializer>(
                            v: &Self,
                            s: S,
                        ) -> ::core::result::Result<S::Ok, S::Error> {
                            ::buffa::json_helpers::bytes::serialize(
                                ::core::convert::AsRef::<[u8]>::as_ref(v),
                                s,
                            )
                        }
                        fn deserialize_proto_json<'de, D: ::serde::Deserializer<'de>>(
                            d: D,
                        ) -> ::core::result::Result<Self, D::Error> {
                            ::buffa::json_helpers::bytes::deserialize(d)
                        }
                    }
                },
                json_gate,
            ));
        }
        if ctx.config.generate_reflection_vtable {
            let value_ref = match kind {
                CustomElemKind::String => quote! {
                    ::buffa_descriptor::reflect::ValueRef::String(
                        ::core::convert::AsRef::<str>::as_ref(self),
                    )
                },
                CustomElemKind::Bytes => quote! {
                    ::buffa_descriptor::reflect::ValueRef::Bytes(
                        ::core::convert::AsRef::<[u8]>::as_ref(self),
                    )
                },
            };
            out.extend(feature_gates::cfg_block(
                quote! {
                    impl ::buffa_descriptor::reflect::ReflectElement for #ty {
                        fn as_value_ref(&self) -> ::buffa_descriptor::reflect::ValueRef<'_> {
                            #value_ref
                        }
                    }
                },
                reflect_gate,
            ));
        }
    }
    // A custom `string` type used as a `map` key needs `ReflectMapKey` for
    // vtable reflection (the bridge path keys maps by the borrowed `&str` view,
    // which already implements it). Like the element impls above, this compiles
    // only when the type is local to the generating crate (the orphan rule).
    if ctx.config.generate_reflection_vtable {
        for path in &elems.map_keys {
            let ty = parse_custom_type_path(path)?;
            out.extend(feature_gates::cfg_block(
                quote! {
                    impl ::buffa_descriptor::reflect::ReflectMapKey for #ty {
                        fn as_map_key_ref(&self) -> ::buffa_descriptor::reflect::MapKeyRef<'_> {
                            ::buffa_descriptor::reflect::MapKeyRef::String(
                                ::core::convert::AsRef::<str>::as_ref(self),
                            )
                        }
                    }
                },
                reflect_gate,
            ));
        }
    }
    Ok(out)
}

pub fn generate_with_diagnostics(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
    config: &CodeGenConfig,
) -> Result<(Vec<GeneratedFile>, Vec<CodeGenWarning>), CodeGenError> {
    // Vtable reflection resolves against the per-package descriptor pool, which
    // is emitted by bridge-mode reflection — so it requires `generate_reflection`.
    // It does NOT require views: the owned `impl ReflectMessage` is self-contained,
    // so with views off, vtable mode still emits owned-message reflection (the
    // view impls are simply skipped along with the views).
    if config.generate_reflection_vtable && !config.generate_reflection {
        return Err(CodeGenError::Other(
            "generate_reflection_vtable requires generate_reflection to be enabled \
             (it provides the descriptor pool the reflect impls resolve against)"
                .into(),
        ));
    }

    // Idiomatic imports place `use` directives in the package-root scope,
    // which is only single-writer (collision-free by construction) when the
    // whole package is one generated file.
    if config.idiomatic_imports && !config.file_per_package {
        return Err(CodeGenError::Other(
            "idiomatic_imports requires file_per_package to be enabled (the multi-file \
             layout include!-merges every proto's content into the shared package root, \
             where emitted `use` directives could collide across files)"
                .into(),
        ));
    }

    // Active feature-gate names are emitted verbatim into
    // `#[cfg(feature = "...")]`; an invalid name fails open (the cfg is
    // permanently false and the gated impls silently compile away), so it
    // must be a hard error here rather than a debug assertion — build
    // scripts and protoc plugins typically run as release builds.
    if let Err((kind, name)) = config.feature_gates().validate() {
        return Err(CodeGenError::Other(format!(
            "invalid {kind} feature-gate name {name:?}: a Cargo feature name starts \
             with an ASCII alphanumeric or '_' and contains only alphanumerics, \
             '_', '-', '+', or '.'; an invalid name would leave the emitted \
             #[cfg(feature = ...)] permanently false, silently compiling the \
             gated impls away"
        )));
    }

    config.validate_type_name_prefix()?;

    let ctx = context::CodeGenContext::for_generate(file_descriptors, files_to_generate, config);

    // Lazy views need the eager view machinery; warn once per run.
    if config.lazy_views && !config.generate_views {
        ctx.warn(CodeGenWarning::LazyViewsRequireViews);
    }

    // Group requested files by package. BTreeMap → deterministic output order.
    let mut by_package: std::collections::BTreeMap<String, Vec<&FileDescriptorProto>> =
        std::collections::BTreeMap::new();
    for file_name in files_to_generate {
        let file_desc = file_descriptors
            .iter()
            .find(|f| f.name.as_deref() == Some(file_name.as_str()))
            .ok_or_else(|| CodeGenError::FileNotFound(file_name.clone()))?;
        let pkg = file_desc.package.as_deref().unwrap_or("").to_string();
        by_package.entry(pkg).or_default().push(file_desc);
    }

    // Reflection: serialize the FileDescriptorSet once, regardless of how
    // many packages are in the request. Each package embeds its own copy of
    // the bytes (binary-size dedup is a follow-up), but the build-time
    // re-encoding cost shouldn't scale with the package count.
    let fds_bytes = if config.generate_reflection {
        reflect::encode_fds_once(file_descriptors)
    } else {
        Vec::new()
    };

    // Custom owned types used as elements of a `repeated` field need a
    // `ProtoElemJson` (JSON) and/or `ReflectElement` (vtable) impl, which buffa
    // cannot provide for a foreign type (orphan rule). Collect them once across
    // the whole request, render the impls, and hand them to the first package so
    // they are emitted exactly once (a per-package emit would collide, E0119).
    let custom_elems = collect_custom_elements(&ctx, file_descriptors, files_to_generate);
    let custom_elem_impls = render_custom_elem_impls(&ctx, &custom_elems)?;

    let empty_impls = TokenStream::new();
    let mut output = Vec::new();
    let mut custom_emitted = false;
    for (package, files) in by_package {
        let impls = if custom_emitted {
            &empty_impls
        } else {
            custom_emitted = true;
            &custom_elem_impls
        };
        generate_package(&ctx, &package, &files, &fds_bytes, impls, &mut output)?;
    }

    Ok((output, ctx.take_warnings()))
}

/// Generate a module tree that assembles per-package `.mod.rs` files into
/// nested `pub mod` blocks matching the protobuf package hierarchy.
///
/// Each entry is a `(mod_file_name, package)` pair where `package` is the
/// dot-separated protobuf package name (e.g., `"google.api"`) and
/// `mod_file_name` is the corresponding `<pkg>.mod.rs` (only
/// [`GeneratedFileKind::PackageMod`] outputs need wiring; per-proto
/// content files are reached via `include!` from the stitcher).
///
/// `include_mode` controls how `include!` paths are emitted.
///
/// `emit_inner_allow` adds a `#![allow(...)]` inner attribute at the top —
/// valid when the output is used directly as a module file (`mod.rs`),
/// invalid when consumed via `include!`.
pub fn generate_module_tree<F: AsRef<str>, P: AsRef<str>>(
    entries: &[(F, P)],
    include_mode: IncludeMode<'_>,
    emit_inner_allow: bool,
) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;

    use crate::idents::escape_mod_ident;

    #[derive(Default)]
    struct ModNode {
        files: Vec<String>,
        children: BTreeMap<String, Self>,
    }

    let mut root = ModNode::default();

    for (file_name, package) in entries {
        let package = package.as_ref();
        let pkg_parts: Vec<&str> = if package.is_empty() {
            vec![]
        } else {
            package.split('.').collect()
        };

        let mut node = &mut root;
        for seg in &pkg_parts {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node.files.push(file_name.as_ref().to_string());
    }

    let lints = ALLOW_LINTS.join(", ");
    let mut out = String::new();
    let _ = writeln!(out, "// @generated by buffa-codegen. DO NOT EDIT.");
    if emit_inner_allow {
        let _ = writeln!(out, "#![allow({lints})]");
    }
    let _ = writeln!(out);

    fn emit(out: &mut String, node: &ModNode, depth: usize, mode: IncludeMode<'_>, lints: &str) {
        let indent = "    ".repeat(depth);

        for file in &node.files {
            match mode {
                IncludeMode::Relative(prefix) => {
                    let _ = writeln!(out, r#"{indent}include!("{prefix}{file}");"#);
                }
                IncludeMode::OutDir => {
                    let _ = writeln!(
                        out,
                        r#"{indent}include!(concat!(env!("OUT_DIR"), "/{file}"));"#
                    );
                }
            }
        }

        for (name, child) in &node.children {
            let escaped = escape_mod_ident(name);
            let _ = writeln!(out, "{indent}#[allow({lints})]");
            let _ = writeln!(out, "{indent}pub mod {escaped} {{");
            let _ = writeln!(out, "{indent}    use super::*;");
            emit(out, child, depth + 1, mode, lints);
            let _ = writeln!(out, "{indent}}}");
        }
    }

    emit(&mut out, &root, 0, include_mode, &lints);
    out
}

/// How [`generate_module_tree`] emits `include!` paths.
#[derive(Debug, Clone, Copy)]
pub enum IncludeMode<'a> {
    /// `include!("<prefix><file>")` — relative to the including file.
    /// Prefix is typically `""` or `"gen/"`.
    Relative(&'a str),
    /// `include!(concat!(env!("OUT_DIR"), "/<file>"))` — for build.rs output.
    OutDir,
}

/// Validate one input descriptor before generating code for it.
///
/// Checks, in one walk of the message tree:
///
/// - **Reserved field names**: no field starts with `__buffa_` (would clash
///   with generated `__buffa_unknown_fields` / `__buffa_cached_size`).
/// - **Module-name conflicts**: no two sibling messages snake_case to the
///   same module name (e.g. `HTTPRequest` vs `HttpRequest`).
/// - **Reserved sentinel**: no package segment, message-module name, or
///   file-level enum name equals [`SENTINEL_MOD`](context::SENTINEL_MOD).
///   Ancillary types live under `pkg::__buffa::…`; a proto element
///   emitting an item named `__buffa` at package root would produce
///   E0428 against `pub mod __buffa`. This is the only name buffa
///   reserves in user namespace.
fn validate_file(file: &FileDescriptorProto) -> Result<(), CodeGenError> {
    use std::collections::HashMap;

    let sentinel = context::SENTINEL_MOD;
    let package = file.package.as_deref().unwrap_or("");
    if package.split('.').any(|seg| seg == sentinel) {
        return Err(CodeGenError::ReservedModuleName {
            name: sentinel.to_string(),
            location: format!("package '{package}'"),
        });
    }
    // File-level enums emit `pub enum <name>` at package root with the
    // proto name preserved verbatim (no PascalCase normalization), so a
    // proto `enum __buffa` would land beside `pub mod __buffa`. Nested
    // enums live inside their owner message's module and cannot collide
    // with the package-root sentinel, so only file-level is checked.
    for enum_type in &file.enum_type {
        let name = enum_type.name.as_deref().unwrap_or("");
        if name == sentinel {
            return Err(CodeGenError::ReservedModuleName {
                name: sentinel.to_string(),
                location: format!("enum '{package}.{name}'"),
            });
        }
    }

    fn walk(
        messages: &[crate::generated::descriptor::DescriptorProto],
        scope: &str,
        sentinel: &str,
    ) -> Result<(), CodeGenError> {
        // snake_case module name → original proto name (for conflict diag).
        let mut seen: HashMap<String, &str> = HashMap::new();

        for msg in messages {
            let name = msg.name.as_deref().unwrap_or("");
            let fqn = if scope.is_empty() {
                name.to_string()
            } else {
                format!("{scope}.{name}")
            };

            for field in &msg.field {
                if let Some(fname) = &field.name {
                    if fname.starts_with("__buffa_") {
                        return Err(CodeGenError::ReservedFieldName {
                            message_name: fqn,
                            field_name: fname.clone(),
                        });
                    }
                }
            }

            let module_name = crate::oneof::to_snake_case(name);
            if module_name == sentinel {
                return Err(CodeGenError::ReservedModuleName {
                    name: sentinel.to_string(),
                    location: format!("message '{fqn}'"),
                });
            }
            if let Some(existing) = seen.get(&module_name) {
                return Err(CodeGenError::ModuleNameConflict {
                    scope: scope.to_string(),
                    name_a: existing.to_string(),
                    name_b: name.to_string(),
                    module_name,
                });
            }
            seen.insert(module_name, name);

            walk(&msg.nested_type, &fqn, sentinel)?;
        }
        Ok(())
    }

    walk(&file.message_type, package, sentinel)
}

/// Per-proto content streams plus the file stem, ready to be formatted.
struct ProtoContent {
    stem: String,
    owned: TokenStream,
    view: TokenStream,
    lazy_view: TokenStream,
    oneof: TokenStream,
    view_oneof: TokenStream,
    ext: TokenStream,
    /// Candidate `pub use` re-exports targeting the package root (top-level
    /// view structs, file-level extension consts). Filtered against the
    /// package-wide root namespace in [`generate_package_mod`] — the package
    /// can span multiple `.proto` files, so collisions are only knowable at
    /// the stitcher level.
    root_reexports: Vec<message::ReexportCandidate>,
}

/// Generate the per-`.proto` content token streams for one input file.
/// Each ancillary kind that has no content yields an empty stream and
/// is dropped at the file-emission stage.
fn generate_proto_content(
    ctx: &context::CodeGenContext,
    current_package: &str,
    file: &FileDescriptorProto,
    reg: &mut message::RegistryPaths,
) -> Result<ProtoContent, CodeGenError> {
    use crate::idents::make_field_ident;
    use crate::message::MessageOutput;

    validate_file(file)?;

    let resolver = imports::ImportResolver::new();
    let features = crate::features::for_file(file);

    let mut owned = TokenStream::new();
    let mut view = TokenStream::new();
    let mut lazy_view = TokenStream::new();
    let mut oneof = TokenStream::new();
    let mut view_oneof = TokenStream::new();
    let mut ext = TokenStream::new();
    let mut root_reexports: Vec<message::ReexportCandidate> = Vec::new();
    let sentinel = make_field_ident(context::SENTINEL_MOD);

    for enum_type in &file.enum_type {
        let enum_proto_name = enum_type.name.as_deref().unwrap_or("");
        let enum_rust_name = ctx.config.prefixed_type_name(enum_proto_name);
        let enum_fqn = if current_package.is_empty() {
            enum_proto_name.to_string()
        } else {
            format!("{}.{}", current_package, enum_proto_name)
        };
        owned.extend(enumeration::generate_enum(
            ctx,
            enum_type,
            &enum_rust_name,
            &enum_fqn,
            &features,
            &resolver,
        )?);
    }

    for message_type in &file.message_type {
        let top_level_name = message_type.name.as_deref().unwrap_or("");
        let rust_name = ctx.config.prefixed_type_name(top_level_name);
        let proto_fqn = if current_package.is_empty() {
            top_level_name.to_string()
        } else {
            format!("{}.{}", current_package, top_level_name)
        };
        let MessageOutput {
            owned_top,
            owned_mod,
            oneof_tree: msg_oneof,
            view_tree: msg_view,
            lazy_view_tree: msg_lazy_view,
            view_oneof_tree: msg_view_oneof,
            reg: msg_reg,
        } = message::generate_message(
            ctx,
            message_type,
            current_package,
            &rust_name,
            &proto_fqn,
            &features,
            &resolver,
        )?;
        owned.extend(owned_top);
        let mod_name = ctx.nested_module_name(current_package, top_level_name);
        let mod_ident = make_field_ident(&mod_name);
        // When the nested-types module was deconflicted from a sub-package
        // (issue #135), document why the name carries a trailing `_`.
        let mod_doc = if mod_name == crate::oneof::to_snake_case(top_level_name) {
            quote! {}
        } else {
            let doc = format!(
                "Nested items of `{top_level_name}`. The module name carries a \
                 trailing `_` to avoid a collision with another module in this \
                 scope (a sub-package or sibling message of the same name). See \
                 buffa#135."
            );
            quote! { #[doc = #doc] }
        };
        for p in msg_reg.json_ext {
            reg.json_ext.push(quote! { #mod_ident :: #p });
        }
        for p in msg_reg.text_ext {
            reg.text_ext.push(quote! { #mod_ident :: #p });
        }
        reg.json_any.extend(msg_reg.json_any);
        reg.text_any.extend(msg_reg.text_any);

        if !owned_mod.is_empty() {
            owned.extend(quote! {
                #mod_doc
                pub mod #mod_ident {
                    #[allow(unused_imports)]
                    use super::*;
                    #owned_mod
                }
            });
        }
        oneof.extend(msg_oneof);
        view.extend(msg_view);
        lazy_view.extend(msg_lazy_view);
        view_oneof.extend(msg_view_oneof);

        // Top-level message view → re-export at package root. The leading
        // `self::` is load-bearing: when consumers nest packages with
        // `pub mod a { use super::*; pub mod a_b { use super::*; … } }`
        // (`buffa-build`'s `_include.rs` does this), a parent package's
        // `__buffa` is in scope via the glob, and Rust's import-resolution
        // pass treats a glob-imported name as ambiguous against a
        // **macro-expanded** local one (the `pub mod __buffa` block arrives
        // via `include!()`), even though a non-macro local definition would
        // shadow the glob — see rustc E0659. `self::` resolves it
        // deterministically. `#[doc(inline)]` makes rustdoc render the type's
        // full page at the natural path instead of a "Re-export of …" stub.
        if ctx.config.generate_views {
            let view_ident = format_ident!("{rust_name}View");
            root_reexports.push(message::ReexportCandidate {
                name: view_ident.to_string(),
                tokens: feature_gates::cfg_block(
                    quote! {
                        #[doc(inline)]
                        pub use self :: #sentinel :: view :: #view_ident;
                    },
                    ctx.config.feature_gates().views,
                ),
            });
            // The owned-view wrapper gets the same natural-path treatment as
            // the view struct, so `pkg::FooOwnedView` works out of the box.
            let owned_view_ident = format_ident!("{rust_name}OwnedView");
            root_reexports.push(message::ReexportCandidate {
                name: owned_view_ident.to_string(),
                tokens: feature_gates::cfg_block(
                    quote! {
                        #[doc(inline)]
                        pub use self :: #sentinel :: view :: #owned_view_ident;
                    },
                    ctx.config.feature_gates().views,
                ),
            });
            if ctx.config.lazy_views {
                let lazy_ident = format_ident!("{rust_name}LazyView");
                root_reexports.push(message::ReexportCandidate {
                    name: lazy_ident.to_string(),
                    tokens: feature_gates::cfg_block(
                        quote! {
                            #[doc(inline)]
                            pub use self :: #sentinel :: lazy_view :: #lazy_ident;
                        },
                        ctx.config.feature_gates().views,
                    ),
                });
            }
        }
    }

    // File-level `extend` declarations → `__buffa::ext::` (depth 2).
    let (file_ext_tokens, file_ext_json, file_ext_text) = extension::generate_extensions(
        ctx,
        &file.extension,
        current_package,
        2,
        &features,
        current_package,
    )?;
    ext.extend(file_ext_tokens);
    for id in file_ext_json {
        reg.json_ext.push(quote! { #sentinel :: ext :: #id });
    }
    for id in file_ext_text {
        reg.text_ext.push(quote! { #sentinel :: ext :: #id });
    }
    // File-level extension consts → re-export at package root. `self::` and
    // `#[doc(inline)]` for the same reasons as the view re-exports above.
    for ext_field in &file.extension {
        let const_ident = extension::extension_const_ident(ext_field.name.as_deref().unwrap_or(""));
        root_reexports.push(message::ReexportCandidate {
            name: const_ident.to_string(),
            tokens: quote! {
                #[doc(inline)]
                pub use self :: #sentinel :: ext :: #const_ident;
            },
        });
    }

    Ok(ProtoContent {
        stem: proto_path_to_stem(file.name.as_deref().unwrap_or("")),
        owned,
        view,
        lazy_view,
        oneof,
        view_oneof,
        ext,
        root_reexports,
    })
}

/// Per-section token streams for one package, ready for the stitcher.
///
/// In per-file mode each section holds `include!("<stem>...rs")` calls; in
/// `file_per_package` mode each holds the actual generated items.
#[derive(Default)]
struct PackageSections {
    owned: Vec<TokenStream>,
    view: Vec<TokenStream>,
    lazy_view: Vec<TokenStream>,
    oneof: Vec<TokenStream>,
    view_oneof: Vec<TokenStream>,
    ext: Vec<TokenStream>,
}

impl PackageSections {
    /// Append one proto file's generated items in-line.
    ///
    /// Empty streams are skipped so each section's emptiness reflects
    /// "the package has no content of this kind" — symmetric with the
    /// per-file branch that filters at file-emission time.
    fn push_inline(&mut self, pc: ProtoContent) {
        let push_if_nonempty = |dst: &mut Vec<TokenStream>, ts: TokenStream| {
            if !ts.is_empty() {
                dst.push(ts);
            }
        };
        push_if_nonempty(&mut self.owned, pc.owned);
        push_if_nonempty(&mut self.view, pc.view);
        push_if_nonempty(&mut self.lazy_view, pc.lazy_view);
        push_if_nonempty(&mut self.oneof, pc.oneof);
        push_if_nonempty(&mut self.view_oneof, pc.view_oneof);
        push_if_nonempty(&mut self.ext, pc.ext);
    }
}

/// Generate all output files for one proto package: up to five content
/// files per `.proto` (empty ancillary kinds are skipped) plus one
/// `<pkg>.mod.rs` stitcher, or a single `<pkg>.rs` when
/// [`CodeGenConfig::file_per_package`] is set.
fn generate_package(
    ctx: &context::CodeGenContext,
    current_package: &str,
    files: &[&FileDescriptorProto],
    fds_bytes: &[u8],
    // Deduped `ProtoElemJson` / `ReflectElement` impls for custom repeated
    // element types, collected generation-wide and emitted into exactly one
    // package's `__buffa` module (empty for every package but the first).
    custom_elem_impls: &TokenStream,
    out: &mut Vec<GeneratedFile>,
) -> Result<(), CodeGenError> {
    // Registry paths are package-root-relative; `register_types` lives at
    // `__buffa::register_types` (one level deep), so each path gets a
    // single `super::` prefix when emitted into the fn body.
    let mut reg = message::RegistryPaths::default();
    let mut root_reexports: Vec<message::ReexportCandidate> = Vec::new();

    // Idiomatic imports: dry-run the package's generation once with the
    // registry collecting, so the set of package-root path references is
    // known — by construction, exactly the set the real pass will emit —
    // then assign short names and generate for real with the registry
    // resolving. Generation is deterministic, so the two passes see the
    // same references; assignment sorts the collected set, so the result
    // is also stable under `.proto` file reordering. The dry run's other
    // outputs (tokens, registry paths, re-export candidates, warnings) are
    // discarded; only the candidate *names* feed the occupied set, since a
    // surviving re-export occupies a root name a `use` must not claim.
    if ctx.config.idiomatic_imports && ctx.config.file_per_package {
        ctx.imports_begin_collecting();
        let warn_mark = ctx.warnings_len();
        let mut scratch_reg = message::RegistryPaths::default();
        let mut occupied = root_occupied_names(ctx, files);
        for file in files {
            let pc = generate_proto_content(ctx, current_package, file, &mut scratch_reg)?;
            occupied.extend(pc.root_reexports.into_iter().map(|c| c.name));
        }
        ctx.truncate_warnings(warn_mark);
        occupied.insert("register_types".to_string());
        // The reflection pool accessor is re-exported at the package root
        // directly by `generate_package_mod` (not via a ReexportCandidate),
        // so the dry run doesn't capture it — reserve it explicitly.
        if ctx.config.generate_reflection {
            occupied.insert("descriptor_pool".to_string());
        }
        let collected = ctx.imports_take_collected();
        ctx.imports_set_resolving(imports::RootImports::assign(&collected, &occupied));
    }

    let sections = if ctx.config.file_per_package {
        let mut sections = PackageSections::default();
        for file in files {
            let mut pc = generate_proto_content(ctx, current_package, file, &mut reg)?;
            root_reexports.append(&mut pc.root_reexports);
            sections.push_inline(pc);
        }
        sections
    } else {
        let mut sections = PackageSections::default();
        for file in files {
            let mut pc = generate_proto_content(ctx, current_package, file, &mut reg)?;
            root_reexports.append(&mut pc.root_reexports);
            let source = file.name.as_deref().unwrap_or("");
            let stem = pc.stem;

            // Empty ancillary token streams are skipped — neither the
            // content file nor the stitcher's `include!` is emitted.
            let emit = |suffix: &str,
                        kind: GeneratedFileKind,
                        tokens: TokenStream,
                        section: &mut Vec<TokenStream>,
                        out: &mut Vec<GeneratedFile>|
             -> Result<(), CodeGenError> {
                if tokens.is_empty() {
                    return Ok(());
                }
                let name = format!("{stem}{suffix}.rs");
                section.push(quote! { include!(#name); });
                out.push(GeneratedFile {
                    name,
                    package: current_package.to_string(),
                    kind,
                    content: format_tokens(tokens, source)?,
                });
                Ok(())
            };
            emit(
                "",
                GeneratedFileKind::Owned,
                pc.owned,
                &mut sections.owned,
                out,
            )?;
            emit(
                ".__view",
                GeneratedFileKind::View,
                pc.view,
                &mut sections.view,
                out,
            )?;
            emit(
                ".__lazy_view",
                GeneratedFileKind::LazyView,
                pc.lazy_view,
                &mut sections.lazy_view,
                out,
            )?;
            emit(
                ".__oneof",
                GeneratedFileKind::Oneof,
                pc.oneof,
                &mut sections.oneof,
                out,
            )?;
            emit(
                ".__view_oneof",
                GeneratedFileKind::ViewOneof,
                pc.view_oneof,
                &mut sections.view_oneof,
                out,
            )?;
            emit(
                ".__ext",
                GeneratedFileKind::Ext,
                pc.ext,
                &mut sections.ext,
                out,
            )?;
        }
        sections
    };

    let reexport_block = surviving_root_reexports(ctx, files, &reg, root_reexports);

    out.push(GeneratedFile {
        name: if ctx.config.file_per_package {
            package_to_filename(current_package)
        } else {
            package_to_mod_filename(current_package)
        },
        package: current_package.to_string(),
        kind: GeneratedFileKind::PackageMod,
        content: generate_package_mod(
            ctx,
            &sections,
            &reg,
            &reexport_block,
            fds_bytes,
            custom_elem_impls,
        )?,
    });

    // Drop the import registry so its bindings can't leak into the next
    // package's generation.
    ctx.imports_reset();

    Ok(())
}

/// Names occupied at a package's root by real items: top-level messages,
/// enums, message nested-types modules (deconflicted name, #135), and the
/// `__buffa` sentinel itself.
///
/// The package root is shared across every `.proto` file in the package, so
/// the set is built from *all* of them. File-level extension consts live in
/// `__buffa::ext::`, not at the root, so they are re-export *candidates*
/// (added by `generate_proto_content`) rather than occupants. Used both to
/// filter root re-exports and as the base reserved set for
/// `idiomatic_imports` short-name assignment.
fn root_occupied_names(
    ctx: &context::CodeGenContext,
    files: &[&FileDescriptorProto],
) -> std::collections::BTreeSet<String> {
    let mut occupied = std::collections::BTreeSet::new();
    occupied.insert(context::SENTINEL_MOD.to_string());
    for file in files {
        let package = file.package.as_deref().unwrap_or("");
        for m in &file.message_type {
            let name = m.name.as_deref().unwrap_or("");
            // The declared struct name carries the configured prefix; the
            // module name stays proto-derived.
            occupied.insert(ctx.config.prefixed_type_name(name));
            // The actual module name (deconflicted from sub-packages, #135).
            occupied.insert(ctx.nested_module_name(package, name));
        }
        for e in &file.enum_type {
            occupied.insert(
                ctx.config
                    .prefixed_type_name(e.name.as_deref().unwrap_or("")),
            );
        }
    }
    occupied
}

/// Filter the candidate package-root re-exports against the package's
/// existing root namespace and against each other, returning the surviving
/// `pub use` lines.
///
/// The package root is shared across every `.proto` file in the package, so
/// the occupied-name set must be built from *all* of them — a top-level
/// message named `FooView` declared in `a.proto` would shadow `Foo`'s view
/// re-export from `b.proto`.
fn surviving_root_reexports(
    ctx: &context::CodeGenContext,
    files: &[&FileDescriptorProto],
    reg: &message::RegistryPaths,
    mut candidates: Vec<message::ReexportCandidate>,
) -> TokenStream {
    use crate::idents::make_field_ident;

    let occupied = root_occupied_names(ctx, files);

    // `register_types`, when emitted, lives at `__buffa::register_types`.
    // `self::` and `#[doc(inline)]` for the same reasons as the view
    // re-exports above. Same `any(json, text)` gate as the fn itself.
    if ctx.config.emit_register_fn && !reg.is_empty() {
        let sentinel = make_field_ident(context::SENTINEL_MOD);
        let json_or_text = ctx.config.feature_gates().json_or_text();
        candidates.push(message::ReexportCandidate {
            name: "register_types".to_string(),
            tokens: feature_gates::cfg_block_any(
                quote! {
                    #[doc(inline)]
                    pub use self :: #sentinel :: register_types;
                },
                &json_or_text,
            ),
        });
    }

    message::emit_surviving_reexports(candidates, &occupied)
}

/// Render the per-package stitcher: owned items at root plus the
/// `__buffa::{view,oneof,ext,...}` module wrappers, followed by the
/// surviving package-root `pub use` re-exports.
fn generate_package_mod(
    ctx: &context::CodeGenContext,
    sections: &PackageSections,
    reg: &message::RegistryPaths,
    root_reexports: &TokenStream,
    fds_bytes: &[u8],
    custom_elem_impls: &TokenStream,
) -> Result<String, CodeGenError> {
    use crate::idents::make_field_ident;

    let owned = &sections.owned;
    let view = &sections.view;
    let lazy_view = &sections.lazy_view;
    let view_oneof = &sections.view_oneof;
    let oneof = &sections.oneof;
    let ext = &sections.ext;

    // Each ancillary module is emitted only when its section has
    // content. The natural-path re-exports outside `__buffa` target
    // these modules — they are emitted only when their target items
    // exist, so the conditions align and re-exports never reference
    // a missing module.
    let view_oneof_mod = if !view_oneof.is_empty() {
        quote! {
            pub mod oneof {
                #[allow(unused_imports)]
                use super::*;
                #(#view_oneof)*
            }
        }
    } else {
        TokenStream::new()
    };

    // `view_oneof` is only populated for messages that have oneofs, and
    // every message also contributes to `view`, so `!view.is_empty()` is
    // sufficient — `view_oneof` non-empty implies `view` non-empty.
    debug_assert!(view_oneof.is_empty() || !view.is_empty());
    let view_mod = if ctx.config.generate_views && !view.is_empty() {
        feature_gates::cfg_block(
            quote! {
                pub mod view {
                    #[allow(unused_imports)]
                    use super::*;
                    #(#view)*
                    #view_oneof_mod
                }
            },
            ctx.config.feature_gates().views,
        )
    } else {
        TokenStream::new()
    };

    // `lazy_view` is only populated when `view` is (the lazy family is
    // generated per-message alongside the eager view).
    debug_assert!(lazy_view.is_empty() || !view.is_empty());
    let lazy_view_mod = if !lazy_view.is_empty() {
        feature_gates::cfg_block(
            quote! {
                pub mod lazy_view {
                    #[allow(unused_imports)]
                    use super::*;
                    #(#lazy_view)*
                }
            },
            ctx.config.feature_gates().views,
        )
    } else {
        TokenStream::new()
    };

    let oneof_mod = if !oneof.is_empty() {
        quote! {
            pub mod oneof {
                #[allow(unused_imports)]
                use super::*;
                #(#oneof)*
            }
        }
    } else {
        TokenStream::new()
    };

    let ext_mod = if !ext.is_empty() {
        quote! {
            pub mod ext {
                #[allow(unused_imports)]
                use super::*;
                #(#ext)*
            }
        }
    } else {
        TokenStream::new()
    };

    let register_fn = if ctx.config.emit_register_fn && !reg.is_empty() {
        let gates = ctx.config.feature_gates();
        // When the gated consts (`__*_JSON_ANY` / `__*_TEXT_ANY`) are
        // `#[cfg(feature = "...")]`, each registration statement that
        // references them gets the same gate. `#[cfg]` on a statement is
        // allowed; the call disappears with the const.
        let json_regs = reg
            .json_any
            .iter()
            .map(|p| {
                feature_gates::cfg_block(quote! { reg.register_json_any(super::#p); }, gates.json)
            })
            .chain(reg.json_ext.iter().map(|p| {
                feature_gates::cfg_block(quote! { reg.register_json_ext(super::#p); }, gates.json)
            }));
        let text_regs = reg
            .text_any
            .iter()
            .map(|p| {
                feature_gates::cfg_block(quote! { reg.register_text_any(super::#p); }, gates.text)
            })
            .chain(reg.text_ext.iter().map(|p| {
                feature_gates::cfg_block(quote! { reg.register_text_ext(super::#p); }, gates.text)
            }));
        // When gating, a feature subset may leave one bucket of statements
        // cfg'd out while the other survives — `reg` is still used. But if
        // `register_types` itself is gated on `any(json, text)` (below),
        // the only reachable bodies have at least one statement, so `reg`
        // can't be unused. Keep `#[allow(unused_variables)]` defensively
        // anyway: it's harmless, and the alternative — proving the
        // invariant holds across future statement-shape changes — is
        // brittle.
        let allow_unused = if ctx.config.gate_impls_on_crate_features {
            quote! { #[allow(unused_variables)] }
        } else {
            quote! {}
        };
        // The fn is useless without at least one of the gated modes that
        // populate it — and `::buffa::type_registry::TypeRegistry` may
        // become feature-gated in the runtime in a future release. Gate the
        // fn on `any(...)` of whichever modes are active so it disappears
        // alongside the last entry.
        feature_gates::cfg_block_any(
            quote! {
                /// Register this package's `Any` type entries and extension entries.
                #allow_unused
                pub fn register_types(reg: &mut ::buffa::type_registry::TypeRegistry) {
                    #(#json_regs)*
                    #(#text_regs)*
                }
            },
            &gates.json_or_text(),
        )
    } else {
        TokenStream::new()
    };

    // Reflection: embed the FileDescriptorSet bytes and a lazy pool
    // accessor so per-message `Reflectable` impls have a descriptor pool to
    // resolve against. Lives inside `__buffa` so the impls can reach it via
    // a relative `__buffa::reflect::descriptor_pool()` path. A package-root
    // `pub use` re-exports `descriptor_pool` so consumers don't have to
    // route through the reserved `__buffa` sentinel.
    let (reflect_mod, reflect_reexport) = if ctx.config.generate_reflection {
        let gate = ctx.config.feature_gates().reflect;
        (
            feature_gates::cfg_block(reflect::reflect_pool_module(fds_bytes), gate),
            feature_gates::cfg_block(reflect::pool_accessor_reexport(&quote! { __buffa }), gate),
        )
    } else {
        (TokenStream::new(), TokenStream::new())
    };

    let sentinel = make_field_ident(context::SENTINEL_MOD);
    // The whole `pub mod __buffa { ... }` wrapper is itself omitted
    // when none of its inner modules or `register_types` exist.
    let buffa_mod = if view_mod.is_empty()
        && lazy_view_mod.is_empty()
        && oneof_mod.is_empty()
        && ext_mod.is_empty()
        && register_fn.is_empty()
        && reflect_mod.is_empty()
        && custom_elem_impls.is_empty()
    {
        TokenStream::new()
    } else {
        let allow = allow_lints_attr();
        quote! {
            #allow
            pub mod #sentinel {
                #[allow(unused_imports)]
                use super::*;
                #view_mod
                #lazy_view_mod
                #oneof_mod
                #ext_mod
                #register_fn
                #reflect_mod
                #custom_elem_impls
            }
        }
    };

    // Idiomatic imports: the `use` block backing the package-root short
    // names (empty unless the registry is in its resolution phase). Only
    // ever non-empty in file_per_package mode, where this output is the
    // whole single-writer package file.
    //
    // Load-bearing lint coupling: impl bodies still write fully-qualified
    // paths (e.g. `::buffa::MessageField<…>`) for types this block also
    // imports — exactly what `unused_qualifications` flags. That lint is
    // suppressed by the `ALLOW_LINTS` attr the module-tree wrapper carries,
    // so generated files must keep their `#[allow]` wrapper when consumed.
    let use_block = ctx.imports_use_block();

    let tokens = quote! {
        #use_block
        #(#owned)*
        #buffa_mod
        #reflect_reexport
        #root_reexports
    };

    format_tokens(tokens, "")
}

/// Format a token stream into a generated-file string with the standard
/// header comment.
fn format_tokens(tokens: TokenStream, source: &str) -> Result<String, CodeGenError> {
    let syntax_tree =
        syn::parse2::<syn::File>(tokens).map_err(|e| CodeGenError::InvalidSyntax(e.to_string()))?;
    let formatted = prettyplease::unparse(&syntax_tree);
    let source_line = if source.is_empty() {
        String::new()
    } else {
        format!("// source: {source}\n")
    };
    Ok(format!(
        "// @generated by buffa-codegen. DO NOT EDIT.\n{source_line}\n{formatted}"
    ))
}

/// Convert a proto package name to its `.mod.rs` stitcher filename.
///
/// e.g., `"google.protobuf"` → `"google.protobuf.mod.rs"`. The unnamed
/// package uses the [`SENTINEL_MOD`](context::SENTINEL_MOD) name as its
/// filename stem — `package __buffa;` is already rejected by
/// `validate_file`, so the unnamed-package stitcher cannot
/// collide with any real package's.
pub fn package_to_mod_filename(package: &str) -> String {
    if package.is_empty() {
        format!("{}.mod.rs", context::SENTINEL_MOD)
    } else {
        format!("{package}.mod.rs")
    }
}

/// Convert a proto package name to its [`file_per_package`] output filename.
///
/// e.g., `"google.protobuf"` → `"google.protobuf.rs"`. The unnamed
/// package uses [`SENTINEL_MOD`](context::SENTINEL_MOD) — same
/// collision-avoidance as [`package_to_mod_filename`].
///
/// [`file_per_package`]: CodeGenConfig::file_per_package
pub fn package_to_filename(package: &str) -> String {
    if package.is_empty() {
        format!("{}.rs", context::SENTINEL_MOD)
    } else {
        format!("{package}.rs")
    }
}

/// Convert a `.proto` file path to its content-file stem.
///
/// e.g., `"google/protobuf/timestamp.proto"` → `"google.protobuf.timestamp"`.
/// Content files append `""`, `".__view"`, `".__oneof"`,
/// `".__view_oneof"`, or `".__ext"` plus `".rs"` — emitted only for
/// kinds with non-empty content.
pub fn proto_path_to_stem(proto_path: &str) -> String {
    let without_ext = proto_path.strip_suffix(".proto").unwrap_or(proto_path);
    without_ext.replace('/', ".")
}

/// Merge downstream [`Companion`](GeneratedFileKind::Companion) files into
/// the per-package stitcher produced by [`generate`].
///
/// For each companion file this function locates the
/// [`PackageMod`](GeneratedFileKind::PackageMod) entry in `files` with a
/// matching package and appends `include!("<name>");` at file scope after
/// buffa's own output — at package root, alongside the owned message types,
/// not under `__buffa::`. The companion files themselves are appended to
/// `files` so that build integrations can write everything to disk in one
/// pass.
///
/// **Call this once per build**; it does not deduplicate, so a second call
/// with the same companions emits a second `include!` for each, which fails
/// to compile downstream with a duplicate-definition error.
///
/// `name` must be a bare-sibling filename — the same convention buffa uses
/// for its own `include!` calls, so it resolves relative to the stitcher
/// without any `OUT_DIR` prefix. Names must not contain `"`, `\`, `/`, or
/// newlines (the function `debug_assert!`s this in debug builds), and must
/// not collide with any of buffa's own generated filenames for the same
/// package (`<stem>.rs`, `<stem>.__view.rs`, etc.) — pick an unused suffix
/// such as `<stem>.__myplugin.rs`.
///
/// Companion files with no matching `PackageMod` (e.g. for a package buffa
/// did not generate any output for) are still appended to `files` but no
/// `include!` is emitted; the caller is responsible for wiring them up. If
/// you don't expect orphans, check that every companion's `package` appears
/// in `files` as a `PackageMod` after calling.
pub fn apply_companions(files: &mut Vec<GeneratedFile>, companions: Vec<GeneratedFile>) {
    for comp in &companions {
        debug_assert!(
            !comp.name.contains(['"', '\\', '/', '\n']),
            "companion file name {:?} contains a character that would break \
             the generated include!() literal or its bare-sibling resolution",
            comp.name
        );
        if let Some(pkg_mod) = files
            .iter_mut()
            .find(|f| f.kind == GeneratedFileKind::PackageMod && f.package == comp.package)
        {
            pkg_mod
                .content
                .push_str(&format!("include!(\"{}\");\n", comp.name));
        }
    }
    files.extend(companions);
}

/// Code generation error.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CodeGenError {
    /// A required field was absent in a descriptor.
    ///
    /// The `&'static str` names the missing field for diagnostics.
    #[error("missing required descriptor field: {0}")]
    MissingField(&'static str),
    /// A resolved type path string could not be parsed as a Rust type.
    #[error("invalid Rust type path: '{0}'")]
    InvalidTypePath(String),
    /// A `box_type_custom` pointer template did not contain the `*` placeholder.
    ///
    /// The custom pointer wraps the message type, so the template must mark where
    /// it goes with `*`, e.g. `"::smallbox::SmallBox<*, smallbox::space::S4>"`.
    #[error("box_type template must contain a `*` placeholder for the message type: '{0}'")]
    MissingWildcard(String),
    /// A `repeated_type_custom` collection template did not contain the `*`
    /// element placeholder.
    ///
    /// Unlike the scalar `string_type_custom` / `bytes_type_custom` knobs (which
    /// take a complete type path), a collection template wraps the element type
    /// and must mark where it goes with `*`, e.g. `"::my_crate::SmallList<*>"`.
    #[error("repeated_type template must contain a `*` element placeholder: '{0}'")]
    MissingListPlaceholder(String),
    /// The accumulated `TokenStream` failed to parse as valid Rust syntax.
    #[error("generated code failed to parse as Rust: {0}")]
    InvalidSyntax(String),
    /// A requested file was not present in the descriptor set.
    #[error("file_to_generate '{0}' not found in descriptor set")]
    FileNotFound(String),
    /// Unexpected descriptor state (e.g. a map entry or oneof that cannot be
    /// resolved to a known descriptor field).
    #[error("codegen error: {0}")]
    Other(String),
    /// A proto field name uses the `__buffa_` reserved prefix, which would
    /// conflict with buffa's internal generated fields.
    #[error(
        "reserved field name '{field_name}' in message '{message_name}': \
             proto field names starting with '__buffa_' conflict with buffa's \
             internal fields"
    )]
    ReservedFieldName {
        message_name: String,
        field_name: String,
    },
    /// Two sibling messages produce the same Rust module name after
    /// snake_case conversion (e.g., `HTTPRequest` and `HttpRequest` both
    /// become `pub mod http_request`).
    #[error(
        "module name conflict in '{scope}': messages '{name_a}' and '{name_b}' \
         both produce module '{module_name}'"
    )]
    ModuleNameConflict {
        scope: String,
        name_a: String,
        name_b: String,
        module_name: String,
    },
    /// A proto package segment, message name, or file-level enum name
    /// would emit a Rust item matching the reserved sentinel `__buffa`.
    ///
    /// This is the only name buffa reserves in user namespace. Resolve by
    /// renaming the proto element.
    #[error(
        "reserved name '{name}' at {location}: this name is reserved for \
         buffa's generated ancillary types (views, oneof enums, \
         extensions). Rename the proto element."
    )]
    ReservedModuleName { name: String, location: String },
    /// The input contains a message with `option message_set_wire_format = true`
    /// but [`CodeGenConfig::allow_message_set`] was not set.
    #[error(
        "message '{message_name}' uses `option message_set_wire_format = true` \
         but CodeGenConfig::allow_message_set is false; MessageSet is a legacy \
         wire format — set allow_message_set(true) if this is intentional"
    )]
    MessageSetNotSupported { message_name: String },
    /// A custom attribute string configured via [`CodeGenConfig::type_attributes`],
    /// [`CodeGenConfig::field_attributes`], [`CodeGenConfig::message_attributes`],
    /// [`CodeGenConfig::enum_attributes`], or [`CodeGenConfig::oneof_attributes`]
    /// could not be parsed as a Rust attribute.
    #[error(
        "invalid custom attribute for path '{path}': '{attribute}' is not a valid \
         Rust attribute ({detail})"
    )]
    InvalidCustomAttribute {
        path: String,
        attribute: String,
        detail: String,
    },
    /// [`CodeGenConfig::type_name_prefix`] is not PascalCase
    /// (`[A-Z][A-Za-z0-9]*`), so prepending it to a type name would produce
    /// an invalid or unconventionally-cased Rust identifier.
    #[error(
        "invalid type_name_prefix '{prefix}': must be empty or PascalCase \
         (start with an ASCII uppercase letter, followed by ASCII letters \
         and digits only)"
    )]
    InvalidTypeNamePrefix { prefix: String },
}

#[cfg(test)]
mod tests;

//! Code generation for the owned message's `impl Reflectable` and the
//! per-package descriptor pool.
//!
//! Wired through [`CodeGenConfig::generate_reflection`]. Every generated owned
//! message gets an `impl ::buffa_descriptor::reflect::Reflectable`, plus a
//! per-package `__buffa::reflect` submodule embedding the `FileDescriptorSet`
//! bytes and a lazy [`DescriptorPool`](buffa_descriptor::DescriptorPool)
//! accessor that both modes resolve against.
//!
//! Two `reflect()` bodies are emitted, selected by mode:
//!
//! - **Bridge** ([`reflectable_impl`]) — round-trips through
//!   [`DynamicMessage`](buffa_descriptor::DynamicMessage) (encode → decode →
//!   boxed handle).
//! - **Vtable** ([`reflectable_impl_vtable`]) — returns
//!   `ReflectCow::Borrowed(self)`, with no round-trip. Requires the owned
//!   `impl ReflectMessage` emitted by [`reflect_owned`](crate::reflect_owned)
//!   (and the view impls by [`reflect_view`](crate::reflect_view)).
//!
//! The call-site contract is identical (`foo.reflect().get(fd)`), so flipping a
//! message between modes requires no diff in consumer code.
//!
//! ## Runtime requirements
//!
//! - `buffa-descriptor` with the `reflect` feature (and `json` if the
//!   consuming crate uses JSON).
//! - `std` — the lazy pool accessor uses [`std::sync::OnceLock`].
//!
//! When [`gate_impls_on_crate_features`](crate::CodeGenConfig::gate_impls_on_crate_features)
//! is on, the impls are wrapped in `#[cfg(feature = "reflect")]` so the
//! consuming crate can opt out.

use proc_macro2::TokenStream;
use quote::quote;

use crate::generated::descriptor::{FileDescriptorProto, FileDescriptorSet};

/// Generate `impl ::buffa_descriptor::reflect::Reflectable for #ty`.
///
/// The impl resolves the message index from the package's lazily-built
/// `DescriptorPool` (looked up by `Self::FULL_NAME`, which `MessageName`
/// already provides) and bridges through `DynamicMessage::from_message`.
///
/// `buffa_path` is the path to `__buffa` from the impl's location —
/// `__buffa` for top-of-package types, `super::__buffa` for nested types
/// that live in a sub-module.
pub(crate) fn reflectable_impl(ty: &TokenStream, buffa_path: &TokenStream) -> TokenStream {
    quote! {
        impl ::buffa_descriptor::reflect::Reflectable for #ty {
            /// Bridge-mode reflective handle: encodes `self` and decodes
            /// it into a [`DynamicMessage`](::buffa_descriptor::reflect::DynamicMessage)
            /// against the package's embedded descriptor pool.
            ///
            /// # Performance
            ///
            /// One full encode/decode round-trip plus a heap allocation per
            /// call. Hold onto the returned handle for repeated field reads
            /// rather than calling `reflect()` per field.
            ///
            /// # Panics
            ///
            /// Panics if the embedded `FileDescriptorSet` is malformed or
            /// `Self::FULL_NAME` is not registered. Both indicate codegen
            /// emitted inconsistent output, not consumer misuse — except
            /// when this type was re-exported from a different
            /// `buffa-build` invocation, whose pool is a different
            /// instance. Each `generate_reflection(true)` codegen run
            /// embeds its own pool; do not mix `reflect()` calls across
            /// independently-generated crates.
            fn reflect(&self) -> ::buffa_descriptor::reflect::ReflectCow<'_> {
                let pool = #buffa_path::reflect::descriptor_pool();
                let idx = pool
                    .message_index(<Self as ::buffa::MessageName>::FULL_NAME)
                    .unwrap_or_else(|| panic!(
                        "type {:?} not registered in this package's descriptor pool (cross-crate reflect()?)",
                        <Self as ::buffa::MessageName>::FULL_NAME,
                    ));
                ::buffa_descriptor::reflect::ReflectCow::Owned(
                    ::buffa::alloc::boxed::Box::new(
                        ::buffa_descriptor::reflect::DynamicMessage::from_message(
                            self,
                            ::buffa::alloc::sync::Arc::clone(pool),
                            idx,
                        ),
                    ),
                )
            }
        }
    }
}

/// Generate the bridge-mode `impl ReflectElement for #ty`.
///
/// `ReflectElement` is how repeated-field and map-value elements surface
/// through vtable reflection (`Vec<T>: ReflectList` requires
/// `T: ReflectElement`). Vtable mode emits its own zero-cost impl in
/// [`reflect_owned`](crate::reflect_owned); this bridge-mode counterpart
/// routes through [`Reflectable::reflect`], paying the encode/decode
/// round-trip per element — which is what lets a vtable-mode message in
/// another compilation hold `repeated` / `map` fields of this type and
/// degrade at the boundary instead of failing to compile.
pub(crate) fn reflect_element_impl_bridge(ty: &TokenStream) -> TokenStream {
    quote! {
        impl ::buffa_descriptor::reflect::ReflectElement for #ty {
            /// Bridge-mode element reflection: each call snapshots this
            /// element through [`Reflectable::reflect`]
            /// (one encode/decode round-trip plus an allocation).
            ///
            /// [`Reflectable::reflect`]: ::buffa_descriptor::reflect::Reflectable::reflect
            fn as_value_ref(&self) -> ::buffa_descriptor::reflect::ValueRef<'_> {
                ::buffa_descriptor::reflect::ValueRef::Message(
                    ::buffa_descriptor::reflect::Reflectable::reflect(self),
                )
            }
        }
    }
}

/// Generate the vtable-mode `impl Reflectable for #ty`, whose `reflect()`
/// borrows `self` directly as `ReflectCow::Borrowed(self)` — no encode/decode
/// round-trip. Requires `#ty: ReflectMessage` (the owned vtable impl emitted by
/// [`reflect_owned`](crate::reflect_owned)).
///
/// The body carries `#[inline]` so a vtable parent in *another crate*
/// reading this type through `Reflectable::reflect()` (the mixed-mode
/// routing) stays zero-cost without LTO.
pub(crate) fn reflectable_impl_vtable(ty: &TokenStream) -> TokenStream {
    quote! {
        impl ::buffa_descriptor::reflect::Reflectable for #ty {
            /// Vtable-mode reflective handle: borrows `self` directly. No
            /// encode/decode round-trip and no allocation — the reflective
            /// accessors read this message's fields in place.
            #[inline]
            fn reflect(&self) -> ::buffa_descriptor::reflect::ReflectCow<'_> {
                ::buffa_descriptor::reflect::ReflectCow::Borrowed(self)
            }
        }
    }
}

/// Serialize the full `FileDescriptorSet` once per codegen run.
///
/// `reflect_pool_module` is invoked once per package, so without caching
/// this re-encodes the FDS `O(packages)` times — wasteful build-time CPU
/// for googleapis-scale workloads with hundreds of packages. The cached
/// bytes are also shared between the byte-literal emission and any future
/// build-script-output deduplication.
///
/// `source_code_info` is stripped from every file before encoding: codegen
/// consumes it for doc comments, but the runtime `DescriptorPool` never
/// reads it, so embedding it would spend binary size on bytes nothing looks
/// at. Consumers that need source info at runtime should build their own
/// descriptor set with `protoc --include_source_info` / `buf build`.
///
/// The `to_vec` clone copies the source info only to drop it — a deliberate
/// trade: a field-by-field clone that skips it would silently omit any
/// future `FileDescriptorProto` field from the embedded set, and the cost
/// is transient build-time memory on comment-heavy runs.
pub(crate) fn encode_fds_once(file_descriptors: &[FileDescriptorProto]) -> Vec<u8> {
    use buffa::Message;
    let mut files = file_descriptors.to_vec();
    for file in &mut files {
        file.source_code_info = Default::default();
    }
    FileDescriptorSet {
        file: files,
        ..Default::default()
    }
    .encode_to_vec()
}

/// Generate the `__buffa::reflect` submodule: the embedded
/// `FILE_DESCRIPTOR_SET_BYTES` constant and the lazy `descriptor_pool()`
/// accessor that all `Reflectable` impls in this package call.
///
/// `fds_bytes` is the pre-serialized `FileDescriptorSet` for the **full**
/// codegen run (the transitive closure), encoded once via [`encode_fds_once`]
/// and shared across packages. Each package still embeds its own copy of the
/// bytes; per-package binary-size deduplication is a planned follow-up.
pub(crate) fn reflect_pool_module(fds_bytes: &[u8]) -> TokenStream {
    let byte_literals = fds_bytes.iter().map(|b| quote! { #b });
    quote! {
        /// Reflection support: embedded descriptor pool shared by this
        /// package's [`Reflectable`](::buffa_descriptor::reflect::Reflectable)
        /// and `ReflectMessage` impls (bridge and vtable mode alike).
        pub mod reflect {
            /// The serialized `FileDescriptorSet` for this codegen run,
            /// including transitive dependencies, with `source_code_info`
            /// stripped. Used to build the runtime
            /// [`DescriptorPool`](::buffa_descriptor::DescriptorPool);
            /// also suitable for shipping the schema over the wire.
            /// Re-exported at the package root — prefer that path over
            /// going through `__buffa`.
            pub const FILE_DESCRIPTOR_SET_BYTES: &[u8] = &[#(#byte_literals),*];

            /// The lazily-built descriptor pool for this package's
            /// `Reflectable` impls. Built from
            /// [`FILE_DESCRIPTOR_SET_BYTES`] on first access.
            ///
            /// # Panics
            ///
            /// Panics on first access if the embedded bytes are malformed —
            /// they're emitted by `buffa-codegen` from the same descriptors
            /// it generated this code from, so a panic indicates a codegen
            /// bug, not consumer input.
            pub fn descriptor_pool() -> &'static ::buffa::alloc::sync::Arc<::buffa_descriptor::DescriptorPool> {
                static POOL: ::std::sync::OnceLock<
                    ::buffa::alloc::sync::Arc<::buffa_descriptor::DescriptorPool>,
                > = ::std::sync::OnceLock::new();
                POOL.get_or_init(|| {
                    ::buffa::alloc::sync::Arc::new(
                        ::buffa_descriptor::DescriptorPool::decode(FILE_DESCRIPTOR_SET_BYTES)
                            .expect("embedded FileDescriptorSet is well-formed"),
                    )
                })
            }
        }
    }
}

/// Generate package-root re-exports so the reflect surface is reachable as
/// `pkg::descriptor_pool()` and `pkg::FILE_DESCRIPTOR_SET_BYTES` without
/// going through the `__buffa` sentinel.
///
/// `__buffa` is documented as a reserved sentinel module ("don't reference
/// this directly"); anything consumers are expected to touch needs a
/// discoverable home outside it.
///
/// Takes the feature gate directly (rather than being wrapped by the caller)
/// because [`cfg_block`](crate::feature_gates::cfg_block) attaches to a
/// single item — each of the two `use` items needs its own gate.
pub(crate) fn reflect_reexports(buffa_path: &TokenStream, gate: Option<&str>) -> TokenStream {
    // Gating happens inside this closure so a future third re-export
    // cannot be added without it — each emitted `use` is one item, which
    // is all `cfg_block` can gate.
    let reexport = |docs: &[&str], name: TokenStream| {
        crate::feature_gates::cfg_block(
            quote! {
                #(#[doc = #docs])*
                #[doc(inline)]
                pub use self::#buffa_path::reflect::#name;
            },
            gate,
        )
    };
    let pool = reexport(
        &[
            "The lazily-built descriptor pool for this package's",
            "`Reflectable` impls. Re-exported from `__buffa::reflect`.",
        ],
        quote! { descriptor_pool },
    );
    let fds_bytes = reexport(
        &[
            "The serialized `FileDescriptorSet` this package's descriptor",
            "pool is built from (`source_code_info` stripped).",
            "Re-exported from `__buffa::reflect`.",
        ],
        quote! { FILE_DESCRIPTOR_SET_BYTES },
    );
    quote! {
        #pool
        #fds_bytes
    }
}

const _: usize = {
    // Documentation breadcrumb: the byte literal embedding produces ~3 bytes
    // of source per descriptor byte (`123, ` for each). A 50KB FDS → ~150KB
    // of source, which prettyplease and rustc handle without issue. If a
    // consumer's FDS is large enough that this matters, the dedup follow-up
    // (hoist to a crate-root `include_bytes!` of a build-script output) is
    // the right fix.
    0
};

#[cfg(test)]
mod tests {
    use super::*;
    use quote::format_ident;

    #[test]
    fn reflectable_impl_emits_well_formed_tokens() {
        let ty = format_ident!("Person");
        let ty_ts = quote! { #ty };
        let buffa = quote! { __buffa };
        let tokens = reflectable_impl(&ty_ts, &buffa);
        // The output must parse as an `impl` item — codegen blind spots
        // hide behind quote!'s tolerance for un-parseable token soup.
        let parsed = syn::parse2::<syn::ItemImpl>(tokens.clone());
        assert!(parsed.is_ok(), "generated impl must parse: {tokens}");
    }

    #[test]
    fn reflect_pool_module_emits_well_formed_tokens() {
        let fd = FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("test".into()),
            ..Default::default()
        };
        let bytes = encode_fds_once(&[fd]);
        // The encoded FDS must round-trip back to a FileDescriptorSet —
        // this is the contract `descriptor_pool()` relies on at runtime.
        {
            use buffa::Message;
            let decoded =
                FileDescriptorSet::decode_from_slice(&bytes).expect("encoded FDS round-trips");
            assert_eq!(decoded.file.len(), 1);
            assert_eq!(decoded.file[0].name.as_deref(), Some("test.proto"));
        }
        let tokens = reflect_pool_module(&bytes);
        let parsed = syn::parse2::<syn::ItemMod>(tokens.clone());
        assert!(parsed.is_ok(), "generated module must parse: {tokens}");
        assert!(tokens.to_string().contains("FILE_DESCRIPTOR_SET_BYTES"));
    }

    #[test]
    fn reflect_reexports_emit_well_formed_tokens() {
        let buffa = quote! { __buffa };
        let tokens = reflect_reexports(&buffa, None);
        let parsed = syn::parse2::<syn::File>(tokens.clone());
        let file = parsed.expect("generated re-exports must parse");
        assert_eq!(file.items.len(), 2, "pool accessor + FDS bytes constant");
        assert!(
            file.items.iter().all(|i| matches!(i, syn::Item::Use(_))),
            "both items must be `use` re-exports"
        );
        let rendered = tokens.to_string();
        assert!(rendered.contains("descriptor_pool"));
        assert!(rendered.contains("FILE_DESCRIPTOR_SET_BYTES"));
    }

    #[test]
    fn reflect_reexports_gate_each_item() {
        // `cfg_block` attaches to a single item; both `use` items must carry
        // their own `#[cfg]` or the second leaks into non-reflect builds.
        let buffa = quote! { __buffa };
        let tokens = reflect_reexports(&buffa, Some("reflect"));
        let file = syn::parse2::<syn::File>(tokens).expect("gated re-exports must parse");
        assert_eq!(file.items.len(), 2);
        for item in &file.items {
            let syn::Item::Use(item_use) = item else {
                panic!("expected a use item");
            };
            assert!(
                item_use.attrs.iter().any(|a| a.path().is_ident("cfg")),
                "re-export missing its own #[cfg] gate"
            );
        }
    }

    #[test]
    fn encode_fds_once_strips_source_code_info() {
        use crate::generated::descriptor::SourceCodeInfo;
        let fd = FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("test".into()),
            source_code_info: SourceCodeInfo::default().into(),
            ..Default::default()
        };
        assert!(fd.source_code_info.is_set());
        let bytes = encode_fds_once(&[fd]);
        use buffa::Message;
        let decoded =
            FileDescriptorSet::decode_from_slice(&bytes).expect("encoded FDS round-trips");
        assert_eq!(decoded.file.len(), 1);
        assert!(
            !decoded.file[0].source_code_info.is_set(),
            "source_code_info must not survive into the embedded FDS"
        );
    }
}

//! Derive macros that implement buffa's pluggable owned-type traits for a
//! newtype wrapping a **foreign** ("remote") type.
//!
//! The owned Rust representation backing a proto `string`/`bytes`/`repeated`
//! field is pluggable (see [`buffa::ProtoString`], [`buffa::ProtoBytes`],
//! [`buffa::ProtoList`]). A custom representation implements one of those
//! traits. The friction is the orphan rule: a type from another crate (e.g.
//! `ecow::EcoString`) cannot implement a buffa-owned trait directly, so it
//! must be wrapped in a crate-local newtype with the trait impl — plus
//! `Deref`, `AsRef`, and the `From` conversions the trait requires —
//! hand-written on the wrapper. That boilerplate is mechanical and identical
//! in shape every time; these derives generate it from one annotation,
//! mirroring `serde`'s `remote` attribute pattern.
//!
//! ```rust
//! #[derive(Clone, PartialEq, Default, Debug, buffa_remote_derive::ProtoString)]
//! #[buffa(remote = ecow::EcoString)]
//! pub struct MyEcoString(pub ecow::EcoString);
//! ```
//!
//! expands the `Deref<Target = str>`, `AsRef<str>`, `From<String>`,
//! `From<&str>`, and `buffa::ProtoString` impls that would otherwise be
//! hand-written (compare to the worked example in `buffa-smolstr` or
//! `examples/custom-types`). The remote type must already satisfy
//! `ProtoString`'s non-buffa-owned supertraits (`Clone`, `PartialEq`,
//! `Default`, `Debug`, `Send`, `Sync`, `AsRef<str>`, `From<String>`,
//! `From<&str>`) — true of essentially every inline/shared-string crate, since
//! that's the API surface they're built to offer as a `String` substitute.
//! Derive those on the newtype yourself (they forward to the inner field
//! automatically via `#[derive(..)]`); this crate only generates the
//! buffa-specific pieces that the orphan rule blocks. If the remote type is
//! missing one of those supertraits, the compiler error names the missing
//! trait bound against the newtype's field — there is no need to expand the
//! macro to diagnose it.
//!
//! [`ProtoBytes`](macro@ProtoBytes) and [`ProtoList`](macro@ProtoList) follow
//! the same shape for `bytes` and `repeated` fields respectively. `ProtoList`
//! additionally requires the remote collection to implement `Extend<T>` (used
//! to implement `push`); its generated `clear` reinitializes the field via
//! `Default::default()`, which drops the existing allocation rather than
//! retaining capacity — acceptable per `ProtoList`'s contract ("retaining
//! capacity *where the underlying type allows*"), but worth knowing if a
//! decoder reuses long-lived buffers and capacity retention matters for that
//! workload. Hand-write `clear` to forward to the remote's own clearing
//! method instead, in that case.
//!
//! # Scope
//!
//! `ProtoBox` and `MapStorage` are deliberately not covered here — their
//! reference newtypes call **inherent** methods on the remote type (e.g.
//! `smallbox::SmallBox::into_inner()`, `indexmap::IndexMap::insert()`) rather
//! than trait methods, so a derive covering them needs a different,
//! attribute-driven design and ships separately.
//!
//! # Why a `remote` attribute that just repeats the field's type?
//!
//! It doesn't change what's generated — the macro always reads the wrapped
//! field's actual type, never the type written in the attribute — and its
//! content is not checked against the field (comparing two type *spellings*
//! for equality isn't possible from within a derive macro without resolving
//! `use` imports). It exists so the newtype's purpose is legible without
//! reading the field declaration, the same role `serde`'s `remote` attribute
//! plays. The value still has to parse as a type, so a typo is caught even
//! though its content isn't otherwise used.

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod bytes;
mod list;
mod remote_field;
mod string;

/// See the [crate-level docs](crate) for the full pattern. Generates
/// `Deref<Target = str>`, `AsRef<str>`, `From<String>`, `From<&str>`, and
/// `buffa::ProtoString` for a single-field newtype wrapping the type named by
/// `#[buffa(remote = ...)]`.
#[proc_macro_derive(ProtoString, attributes(buffa))]
pub fn derive_proto_string(input: TokenStream) -> TokenStream {
    expand(input, string::derive)
}

/// See the [crate-level docs](crate). Generates `Deref<Target = [u8]>`,
/// `AsRef<[u8]>`, `From<Vec<u8>>`, and `buffa::ProtoBytes` for a single-field
/// newtype wrapping the type named by `#[buffa(remote = ...)]`.
#[proc_macro_derive(ProtoBytes, attributes(buffa))]
pub fn derive_proto_bytes(input: TokenStream) -> TokenStream {
    expand(input, bytes::derive)
}

/// See the [crate-level docs](crate). Generates `Deref<Target = [T]>`,
/// `FromIterator<T>`, `From<Vec<T>>`, and `buffa::ProtoList<T>` for a
/// single-field, single-type-parameter newtype wrapping the type named by
/// `#[buffa(remote = ...)]`. Requires the remote type to implement
/// `Extend<T>`, and the newtype itself to implement `Default` by hand (not
/// `#[derive(Default)]`, which would wrongly force `T: Default`).
#[proc_macro_derive(ProtoList, attributes(buffa))]
pub fn derive_proto_list(input: TokenStream) -> TokenStream {
    expand(input, list::derive)
}

fn expand(
    input: TokenStream,
    f: impl FnOnce(DeriveInput) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match f(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

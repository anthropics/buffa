//! Derive macros that implement buffa's pluggable owned-type traits for a
//! newtype wrapping a **foreign** ("remote") type.
//!
//! The owned Rust representation backing a proto `string`/`bytes`/`repeated`
//! field is pluggable (see `buffa::ProtoString`, `buffa::ProtoBytes`,
//! `buffa::ProtoList`). A custom representation implements one of those
//! traits. The friction is the orphan rule: a type from another crate (e.g.
//! `ecow::EcoString`) cannot implement a buffa-owned trait directly, so it
//! must be wrapped in a crate-local newtype with the trait impl — plus
//! `Deref`, `AsRef`, and the `From` conversions the trait requires —
//! hand-written on the wrapper. That boilerplate is mechanical and identical
//! in shape every time; these derives generate it from one annotation,
//! mirroring `serde`'s `remote` attribute pattern.
//!
//! # Scope: the binary codec only
//!
//! **These derives cover the binary wire format only.** A pluggable
//! owned-type trait's own supertraits don't mention `serde::Serialize` /
//! `Deserialize`, `arbitrary::Arbitrary`, or `buffa_descriptor`'s
//! `ReflectList`/`ReflectMap` — those are pulled in separately, by whichever
//! optional feature needs them (`json`, `arbitrary`, `reflect`), and the
//! reference newtypes in `examples/custom-types/src/types/` add them as
//! ordinary extra `#[derive(..)]`s alongside the buffa-trait impl. This crate
//! does the same: it generates nothing serde-, `Arbitrary`-, or
//! reflection-related, so a newtype produced by one of these derives that's
//! used as a message field in a JSON-enabled, fuzzed, or reflection/vtable
//! build needs those impls added by hand, exactly as the hand-written
//! reference newtypes do. For serde, what "by hand" means depends on where
//! the storage type's own serde is actually consulted:
//!
//! - **Singular and oneof `string` fields, and `bytes` fields in every
//!   position**, are handled by buffa's own `#[serde(with = ...)]` modules
//!   (`proto_string`, base64 `bytes`), which use only the `AsRef`/`From`
//!   surface these derives already generate — the newtype needs no serde
//!   impls for them. Don't add `#[serde(transparent)]` to a bytes newtype
//!   in particular: it is dead weight in message context, and if the
//!   newtype is ever serialized standalone it produces a JSON array of
//!   numbers instead of base64. (Compare `SmallBytes` in
//!   `examples/custom-types`, which derives no serde and documents why.)
//! - **A string newtype that appears as a repeated element, an `optional`
//!   (explicit-presence) field, or a map value** serializes through its
//!   own serde, so it needs `#[derive(serde::Serialize,
//!   serde::Deserialize)]` — `#[serde(transparent)]` suffices when the
//!   remote type itself supports serde, since the newtype is a single-field
//!   wrapper. The full per-trait matrix, including list, map, and box
//!   container newtypes, is tabulated in `examples/custom-types/README.md`
//!   in the buffa repository.
//!
//! Skipping a *required* impl on a JSON/reflect/fuzz build surfaces as a
//! trait-bound error deep in *generated message code*
//! (`MyType: Serialize` is not satisfied), not at the derive site — there is
//! no diagnostic from this crate pointing back to it.
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
//! On the newtype itself, derive the derivable subset (`Clone`,
//! `PartialEq`, `Default`, `Debug`) yourself — `Send`/`Sync` are automatic
//! for a single-field wrapper, and for the generic list/map derives
//! implement `Default` by hand instead (see those macros' docs) — and this
//! crate generates the rest (`Deref`,
//! `AsRef`, the `From` conversions, and the trait impl). If the remote type is
//! missing one of those supertraits, the compiler error names the missing
//! trait bound against the newtype's field — there is no need to expand the
//! macro to diagnose it.
//!
//! [`ProtoBytes`](macro@ProtoBytes) and [`ProtoList`](macro@ProtoList) follow
//! the same shape for `bytes` and `repeated` fields respectively.
//! `ProtoBytes`'s generated `from_wire` always copies the payload via
//! `to_vec()` before handing it to the remote type's `From<Vec<u8>>` — there
//! is no generic way to ask an arbitrary remote type to take ownership of a
//! borrowed/`Bytes`-backed payload without copying, so this derive can't
//! reach the zero-copy decode path the built-in `bytes::Bytes` representation
//! gets. A hand-written `from_wire` doesn't escape the copy either:
//! `WirePayload::into_bytes` is zero-copy only for an owned multi-chunk
//! payload, and the common single-chunk source arrives borrowed and is
//! copied there too. When that copy matters, use the built-in `bytes::Bytes`
//! representation for the field rather than a custom type.
//! `ProtoList` additionally requires the
//! remote collection to implement `Extend<T>` (used
//! to implement `push`); its generated `clear` reinitializes the field via
//! `Default::default()`, which drops the existing allocation rather than
//! retaining capacity — acceptable per `ProtoList`'s contract ("retaining
//! capacity *where the underlying type allows*"), but worth knowing if a
//! decoder reuses long-lived buffers and capacity retention matters for that
//! workload. Hand-write `clear` to forward to the remote's own clearing
//! method instead, in that case.
//!
//! # `ProtoBox` and `MapStorage`: inherent methods, not trait methods
//!
//! [`ProtoBox`](macro@ProtoBox) and [`MapStorage`](macro@MapStorage) follow a
//! different shape from the three above. Their reference newtypes
//! (`smallbox::SmallBox::into_inner()`, `indexmap::IndexMap::insert()`) call
//! **inherent** methods on the remote type, not trait methods — `ProtoBox`'s
//! and `MapStorage`'s own supertraits (`Deref`/`DerefMut`; none, for
//! `MapStorage`) don't give a generic derive enough to call through to
//! `new`/`into_inner`/`insert`/`clear`/`iter`/`len` the way `From`/
//! `FromIterator`/`Extend` did for `ProtoString`/`ProtoBytes`/`ProtoList`.
//!
//! So these two derives default to the near-universal naming convention
//! (`Type::new`/`Type::into_inner` for pointers — `Rc`, `Arc`,
//! `smallbox::SmallBox` all use these names, though plain `std::boxed::Box`
//! does not, since its `into_inner` is nightly-only; `len`/`insert`/`clear`/
//! `iter` for maps — `HashMap`, `BTreeMap`, `indexmap::IndexMap`,
//! `dashmap::DashMap` all use these names), with an attribute escape hatch
//! when a remote type names them differently. If a default doesn't match —
//! the remote names its insert method `put`, say — the compiler reports
//! `no function or associated item named 'insert' found for struct '...'`;
//! that's the signal to add the matching override, named below.
//!
//! ```rust
//! #[derive(buffa_remote_derive::ProtoBox)]
//! #[buffa(remote = smallbox::SmallBox<T, smallbox::space::S4>)]
//! pub struct SmallBox<T>(pub smallbox::SmallBox<T, smallbox::space::S4>);
//! ```
//!
//! ```rust
//! #[derive(Clone, PartialEq, Debug, buffa_remote_derive::MapStorage)]
//! #[buffa(remote = indexmap::IndexMap<K, V>)]
//! pub struct MyIndexMap<K: core::hash::Hash + Eq, V>(pub indexmap::IndexMap<K, V>);
//!
//! impl<K: core::hash::Hash + Eq, V> Default for MyIndexMap<K, V> {
//!     fn default() -> Self {
//!         Self(indexmap::IndexMap::new())
//!     }
//! }
//! impl<K: core::hash::Hash + Eq, V> FromIterator<(K, V)> for MyIndexMap<K, V> {
//!     fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
//!         Self(indexmap::IndexMap::from_iter(iter))
//!     }
//! }
//! ```
//!
//! `Default` and `FromIterator<(Key, Value)>` are required by the message
//! codec that drives every `MapStorage` field, not by this derive — `cargo`
//! won't suggest them, since the derive itself compiles fine without them; the
//! failure shows up later, in generated message code, as `the trait bound
//! '...: Default' is not satisfied`. They aren't generated here for the same
//! reason [`ProtoList`](macro@ProtoList)'s `Default` isn't (see that macro's
//! docs): a derived impl would force `K: Default`/`V: Default`, which
//! `MapStorage` does not require.
//!
//! To override a default, name the method explicitly:
//! `#[buffa(remote = ..., into_inner = MyType::unwrap)]` for `ProtoBox`, or
//! any of `len`/`insert`/`clear`/`iter` for `MapStorage`. The override path is
//! called the same way the default is — as a free function taking the
//! receiver as its first argument (`Type::method(&self.0, ...)`) — so it
//! **must** accept the same receiver as the method it replaces: `new` takes
//! the value by ownership and returns `Self`; `into_inner` takes `self` by
//! ownership; `insert`/`clear` take `&mut self`; `len`/`iter` take `&self`.
//! `iter` additionally must yield `(&Key, &Value)` pairs, matching
//! `storage_iter`'s contract. A receiver or item-type mismatch is a type error
//! at the generated call site, not a special diagnostic from this macro.
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

mod box_ptr;
mod bytes;
mod list;
mod map;
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

/// See the [crate-level docs](crate). Generates `Deref<Target = T>`,
/// `DerefMut`, and `buffa::ProtoBox<T>` for a single-field,
/// single-type-parameter newtype wrapping the type named by
/// `#[buffa(remote = ...)]`. Calls the remote type's `new`/`into_inner`
/// methods by the conventional names unless overridden with
/// `#[buffa(remote = ..., new = path, into_inner = path)]`.
#[proc_macro_derive(ProtoBox, attributes(buffa))]
pub fn derive_proto_box(input: TokenStream) -> TokenStream {
    expand(input, box_ptr::derive)
}

/// See the [crate-level docs](crate). Generates `buffa::MapStorage` for a
/// single-field, two-type-parameter (`Key`, `Value`) newtype wrapping the
/// type named by `#[buffa(remote = ...)]`. Calls the remote map's
/// `len`/`insert`/`clear`/`iter` methods by their conventional names unless
/// overridden with `#[buffa(remote = ..., insert = path, ...)]`. The newtype
/// itself must implement `Default` and `FromIterator<(Key, Value)>` by hand —
/// see the crate docs' example.
#[proc_macro_derive(MapStorage, attributes(buffa))]
pub fn derive_map_storage(input: TokenStream) -> TokenStream {
    expand(input, map::derive)
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

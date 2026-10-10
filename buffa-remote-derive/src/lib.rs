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
//! the forwarding bounds (`Clone`, `PartialEq`,
//! `Default`, `Debug`, `Send`, `Sync`, `AsRef<str>`, `From<String>`,
//! `for<'a> From<&'a str>`). The derive uses the borrowed conversion for
//! `copy_from_str` as well, avoiding an intermediate `String`. This last bound
//! belongs to the derive, not to `ProtoString`: if the remote type's `From<&str>`
//! borrows, implement `ProtoString` by hand and provide an owning copy operation.
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
//! `ProtoBytes`'s generated `from_wire` always copies the payload with
//! `to_vec()` before handing it to the remote type's `From<Vec<u8>>`. A remote
//! type that wraps `bytes::Bytes` can decode without that copy, but not
//! through this derive: replace the derive with hand-written
//! `Deref<Target = [u8]>`, `AsRef<[u8]>`, `From<Vec<u8>>` and
//! `buffa::ProtoBytes` impls, and set
//! `buffa::ProtoBytes::PREFERS_OWNED_PAYLOAD` to `true`. That const's
//! documentation in `buffa` has an example.
//!
//! The encode side has the mirror-image limitation with an escape hatch: by
//! default the generated `ProtoBytes` impl inherits the trait's `as_shared`
//! default of `None`, so encoding into a segmented sink (`buffa::Rope`)
//! copies the payload instead of splicing it by reference count. A remote
//! type that stores (or can cheaply produce) a `bytes::Bytes` handle can name
//! the callable via `#[buffa(remote = ..., as_shared = path)]`; it is called
//! as a free function on the wrapped field — `path(&self.0)` or
//! `path(&self.field)` — and must have the shape `fn(&Remote) ->
//! Option<bytes::Bytes>`. A signature mismatch is a type error at the
//! generated call site, not a special diagnostic from this macro. The
//! returned handle must satisfy `buffa::ProtoBytes::as_shared`'s correctness
//! contract, and only a segmented sink ever calls it — test against a
//! `Rope` explicitly.
//!
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
//! # Optional serde and arbitrary impls
//!
//! The derives implement the binary storage traits. The bare
//! [`serde`](#the-serde-key) and [`arbitrary`](#the-arbitrary-key) keys add
//! `serde::Serialize` / `Deserialize` and `arbitrary::Arbitrary` impls to an
//! individual newtype. The derives do not generate `ReflectList`/`ReflectMap`
//! impls; write those by hand for a custom list or map.
//!
//! A newtype that lacks an impl the generated code calls fails to compile in
//! generated message code, and the error does not mention the derive.
//! [The `serde` key](#the-serde-key) and
//! [the `arbitrary` key](#the-arbitrary-key) list the fields that call each
//! impl.
//!
//! # The `serde` key
//!
//! A `generate_json(true)` build that fails in generated message code with
//! ``the trait bound `MyStr: serde::Serialize` is not satisfied`` needs serde
//! impls on the newtype. The bare `serde` key emits them:
//!
//! ```rust
//! #[derive(Clone, PartialEq, Default, Debug, buffa_remote_derive::ProtoString)]
//! #[buffa(remote = ecow::EcoString, serde)]
//! pub struct MyEcoString(pub ecow::EcoString);
//! ```
//!
//! Each of the five derives accepts the key. The JSON form of the
//! `serde::Serialize` and `serde::Deserialize` impls depends on the derive:
//!
//! - [`ProtoString`](macro@ProtoString): a string.
//! - [`ProtoBytes`](macro@ProtoBytes): a base64 string. Serializing writes
//!   the standard alphabet with padding. Deserializing accepts the standard
//!   and the URL-safe alphabet, each with or without padding.
//! - [`ProtoList`](macro@ProtoList), [`ProtoBox`](macro@ProtoBox) and
//!   [`MapStorage`](macro@MapStorage): the form of the remote type. The
//!   impls call the remote type's own `Serialize` and `Deserialize`.
//!
//! The string and bytes forms are the proto3 JSON forms, and the remote type
//! does not need serde support of its own. For a bytes newtype, use the key
//! and not `#[serde(transparent)]`, whose impls use the remote type's form:
//! for `Vec<u8>`, a JSON array of numbers.
//!
//! The `ProtoString` and `ProtoBytes` `Deserialize` impls reject JSON `null`;
//! `serde_json` reports `invalid type: null, expected a string` (for bytes,
//! `expected a base64-encoded string`). Proto3 JSON forbids `null` as a
//! `repeated` element or a map value. A generated message handles every
//! other `null` itself: the default for a whole field, and unset for a field
//! with explicit presence or a oneof variant. So `null` reaches the newtype's
//! impl only where proto3 JSON forbids it.
//!
//! The `ProtoString` and `ProtoBytes` impls write the same form in every
//! serde format, so a binary format such as bincode or postcard also stores a
//! `ProtoBytes` newtype as base64 text. Write the impls by hand for a compact
//! binary form.
//!
//! The list, box and map impls need the remote type to implement serde. For
//! a foreign type, that usually means enabling a `serde` feature of its
//! crate. Each impl is bounded on the remote type's impl, such as
//! `Vec<T>: Serialize` for a newtype over `Vec<T>`. So a newtype whose
//! remote type lacks one still works with the binary codec. The remote
//! type's impls decide the JSON, and they do not apply the proto3 JSON rules
//! for integers, floats or bytes. An `EnumValue` element reads and writes
//! proto3 JSON through its own impls, but a `null` element is read as
//! value 0.
//!
//! The impls are emitted unconditionally, so the crate that defines the
//! newtype must depend on `serde`, under that name, whenever the key is
//! present. A `ProtoBytes` newtype also needs the `json` feature of `buffa`,
//! which has the base64 codec. The other four derives need only `serde`. The
//! `ProtoString` impls need serde's `alloc` or `std` feature, which
//! `buffa/json` enables. If the dependency is optional, put the key in a
//! second `#[buffa(..)]` attribute behind the same condition:
//!
//! ```toml
//! [dependencies]
//! serde = { version = "1", optional = true }
//!
//! [features]
//! json = ["dep:serde", "buffa/json"]
//! ```
//!
//! ```rust
//! #[derive(Clone, PartialEq, Default, Debug, buffa_remote_derive::ProtoString)]
//! #[buffa(remote = ecow::EcoString)]
//! #[cfg_attr(feature = "json", buffa(serde))]
//! pub struct MyEcoString(pub ecow::EcoString);
//! ```
//!
//! Generated messages carry their serde impls unconditionally, or under a
//! crate feature when codegen's `gate_impls_on_crate_features` is on. The
//! feature is `json` unless `json_feature_name` renames it, and the key's
//! `cfg_attr` uses the same name. The key's `cfg_attr` condition must be true
//! in every build that compiles those impls. If the newtype is in another
//! crate, the messages' crate forwards the feature
//! (`json = [.., "my-types/json"]`). The proc-macro crate has no feature
//! that controls the emitted impls, so another dependency cannot enable them
//! through Cargo feature unification. Do not also derive serde on the same
//! type: that creates conflicting impls.
//!
//! Without the `serde` dependency the build fails to resolve the `serde`
//! crate, and the error points at the key. A `ProtoBytes` newtype with the
//! key in a build without `buffa/json` fails to resolve
//! `buffa::json_helpers`; enable the `json` feature of `buffa`. A list, box
//! or map newtype with the key implements `Serialize` and `Deserialize` only
//! when its remote type does. Otherwise the error appears where the newtype
//! is used and names the remote type, as in ``the trait bound
//! `SmallVec<[MyStr; 4]>: serde::Serialize` is not satisfied``: enable the
//! `serde` feature of that type's crate. `smallbox` has no such feature, so
//! the key on a `SmallBox` newtype emits impls that never apply.
//!
//! Not every newtype needs the key. Under `generate_json(true)`, generated
//! message code calls a newtype's own serde impls only for these fields, and
//! a `buffa::json_helpers` function that needs only the storage trait for
//! the rest:
//!
//! | Derive | Needs serde impls when used for |
//! | --- | --- |
//! | `ProtoString` | a `repeated` element, a field with explicit presence (proto3 or proto2 `optional`, and the edition 2023 default), a oneof variant, a map key or value. A singular field with implicit presence, or a proto2 `required` field, does not need them. |
//! | `ProtoList` | a `repeated` field of strings, bools, or messages other than the `google.protobuf` wrapper types |
//! | `MapStorage` | a string-keyed map whose values are strings, bools, or messages other than the wrapper types |
//! | `ProtoBytes`, `ProtoBox` | never; the impls are for your own use of the newtype outside a message |
//!
//! # The `arbitrary` key
//!
//! A bare `arbitrary` key makes any of the five derives also emit an
//! `arbitrary::Arbitrary` impl for the newtype:
//!
//! ```rust
//! #[derive(Clone, PartialEq, Default, Debug, buffa_remote_derive::ProtoString)]
//! #[buffa(remote = ecow::EcoString, arbitrary)]
//! pub struct MyEcoString(pub ecow::EcoString);
//! ```
//!
//! The impl is emitted unconditionally, so the crate that defines the
//! newtype must depend on `arbitrary` (any 1.x release, under that name)
//! whenever the key is present. Without the dependency the build fails to
//! resolve the `arbitrary` crate, and the error points at the key. If the
//! dependency is optional, as it is in a crate set up for
//! `generate_arbitrary`, put the key in a second `#[buffa(..)]` attribute
//! behind the same condition:
//!
//! ```toml
//! [dependencies]
//! arbitrary = { version = "1", optional = true }
//!
//! [features]
//! arbitrary = ["dep:arbitrary", "buffa/arbitrary"]
//! ```
//!
//! ```rust
//! #[derive(Clone, PartialEq, Default, Debug, buffa_remote_derive::ProtoString)]
//! #[buffa(remote = ecow::EcoString)]
//! #[cfg_attr(feature = "arbitrary", buffa(arbitrary))]
//! pub struct MyEcoString(pub ecow::EcoString);
//! ```
//!
//! Generated messages derive `Arbitrary` under `feature = "arbitrary"`, so
//! the condition has to hold whenever that feature is on in the crate that
//! holds the messages. If the newtype lives in another crate, the messages'
//! crate forwards the feature (`arbitrary = [.., "my-types/arbitrary"]`). A
//! newtype that lacks the impl in such a build fails in generated code: the
//! message's derive reports that the newtype does not implement `Arbitrary`,
//! and that error does not mention the key.
//!
//! The remote type does not need an `Arbitrary` impl (`ecow::EcoString` and
//! `flexstr::SharedStr` have none), and one it does have is not used. The
//! impl builds the canonical owned type first (`String`, `Vec<u8>`,
//! `Vec<T>`, `Vec<(Key, Value)>`, or the pointee `T`) and converts through
//! the `From`/`FromIterator`/`ProtoBox::new` surface the derive already
//! generates, as codegen does for a custom `string` or `bytes` field. Byte
//! consumption therefore matches the default representation the newtype
//! replaces, so a fuzz corpus carries across unchanged. For `MapStorage`,
//! the `FromIterator<(Key, Value)>` impl its map codec already requires
//! assembles the map, and the `Arbitrary` impl is bounded on it.
//!
//! Not every newtype needs the key. Under `generate_arbitrary`, codegen
//! attaches its own builder to custom `string` and `bytes` fields, and
//! `MessageField` builds a custom box pointer itself:
//!
//! - `ProtoString`: not needed for generated messages.
//! - `ProtoBytes`: not needed for generated messages.
//! - `ProtoList`: needed, except for a `repeated` field whose element is a
//!   custom string type or a non-default `bytes` type.
//! - `MapStorage`: needed, except for a map with a custom string key, a
//!   custom string value, or a non-default `bytes` value (`bytes::Bytes` or a
//!   custom one).
//! - `ProtoBox`: needed only when the pointer boxes a oneof variant.
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
//! (`ProtoBytes`'s `as_shared` key is a different kind of override — an
//! opt-in over a working trait default, not a renamed inherent method —
//! and is documented above.)
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
//! any of `len`/`insert`/`clear`/`iter` for `MapStorage`. (The keys are
//! exactly `remote`; `new` and `into_inner` for `ProtoBox`; `len`, `insert`,
//! `clear` and `iter` for `MapStorage`; `as_shared` for `ProtoBytes`; and the
//! bare `arbitrary` and `serde` keys on every derive.) The override path is
//! called the same way the default is — as a free function taking the
//! receiver as its first argument (`Type::method(&self.0, ...)`) — so it
//! **must** accept the same receiver as the method it replaces: `new` takes
//! the value by ownership and returns `Self`; `into_inner` takes `self` by
//! ownership; `insert`/`clear` take `&mut self`; `len`/`iter` take `&self`.
//! `iter` additionally must yield `(&Key, &Value)` pairs, matching
//! `storage_iter`'s contract. A receiver or item-type mismatch is a type error
//! at the generated call site, not a special diagnostic from this macro.
//!
//! # Reserved identifiers
//!
//! The generated impls declare lifetimes and type parameters of their own,
//! and each of those names starts with `__buffa` or `__Buffa` (for example
//! `'__buffa_iter` and `__BuffaIter`). Keep those two prefixes out of the
//! newtype's own lifetime and generic parameter names and out of the types and
//! override paths it names. A parameter such as `'a` or `T` cannot collide with
//! a generated one.
//!
//! The generated methods also bind the parameters `value`, `key` and `u`.
//! Write a `new` or `insert` override as a path with two or more segments,
//! such as `Type::method` or `self::helper`, because a bare `value`, `key` or
//! `u` resolves to the parameter.
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
mod forwarders;
mod list;
mod map;
mod remote_field;
mod string;

/// See the [crate-level docs](crate) for the full pattern. Generates
/// `Deref<Target = str>`, `AsRef<str>`, `From<String>`, `From<&str>`, and
/// `buffa::ProtoString` for a single-field newtype wrapping the type named by
/// `#[buffa(remote = ...)]`.
/// The generated `copy_from_str` forwards to the inner type's `From<&str>`.
/// The bare `arbitrary` key adds an `arbitrary::Arbitrary` impl, which needs
/// the `arbitrary` crate wherever the key is active; the crate docs' section
/// on the key shows how to make it conditional.
/// The bare `serde` key adds `serde::Serialize` and `serde::Deserialize`
/// impls. [The `serde` key](crate#the-serde-key) gives their JSON form, the
/// dependencies they need, and how to make them conditional.
#[proc_macro_derive(ProtoString, attributes(buffa))]
pub fn derive_proto_string(input: TokenStream) -> TokenStream {
    expand(input, string::derive)
}

/// See the [crate-level docs](crate). Generates `Deref<Target = [u8]>`,
/// `AsRef<[u8]>`, `From<Vec<u8>>`, and `buffa::ProtoBytes` for a single-field
/// newtype wrapping the type named by `#[buffa(remote = ...)]`. An optional
/// `as_shared = path` key generates the encode-side
/// `buffa::ProtoBytes::as_shared` override — see the crate docs for the
/// callable's contract.
/// The generated impl leaves `buffa::ProtoBytes::PREFERS_OWNED_PAYLOAD` at
/// `false` and copies each payload; the crate docs say what a type that
/// should share the input does instead.
/// The bare `arbitrary` key adds an `arbitrary::Arbitrary` impl, which needs
/// the `arbitrary` crate wherever the key is active; the crate docs' section
/// on the key shows how to make it conditional.
/// The bare `serde` key adds `serde::Serialize` and `serde::Deserialize`
/// impls. [The `serde` key](crate#the-serde-key) gives their JSON form, the
/// dependencies they need, and how to make them conditional.
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
/// The bare `arbitrary` key adds an `arbitrary::Arbitrary` impl, which needs
/// the `arbitrary` crate wherever the key is active; the crate docs' section
/// on the key shows how to make it conditional.
/// The bare `serde` key adds `serde::Serialize` and `serde::Deserialize`
/// impls. [The `serde` key](crate#the-serde-key) gives their JSON form, the
/// dependencies they need, and how to make them conditional.
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
/// The bare `arbitrary` key adds an `arbitrary::Arbitrary` impl, which needs
/// the `arbitrary` crate wherever the key is active; the crate docs' section
/// on the key shows how to make it conditional.
/// The bare `serde` key adds `serde::Serialize` and `serde::Deserialize`
/// impls. [The `serde` key](crate#the-serde-key) gives their JSON form, the
/// dependencies they need, and how to make them conditional.
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
/// The bare `arbitrary` key adds an `arbitrary::Arbitrary` impl, which needs
/// the `arbitrary` crate wherever the key is active; the crate docs' section
/// on the key shows how to make it conditional.
/// The bare `serde` key adds `serde::Serialize` and `serde::Deserialize`
/// impls. [The `serde` key](crate#the-serde-key) gives their JSON form, the
/// dependencies they need, and how to make them conditional.
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

//! A pure-Rust Protocol Buffers runtime with first-class **editions** support,
//! zero-copy views, and `no_std` compatibility.
//!
//! This is the runtime crate. For code generation, see `buffa-build` (for
//! `build.rs` integration) or `protoc-gen-buffa` (for use as a `protoc` plugin).
//!
//! # Quick start
//!
//! Generated message types implement [`Message`]. Encode and decode:
//!
//! ```no_run
//! # use buffa::__doctest_fixtures::Person;
//! use buffa::Message;
//!
//! # fn example(bytes: &[u8]) -> Result<(), buffa::DecodeError> {
//! # let person = Person::default();
//! # let mut existing = Person::default();
//! // Encode to a Vec<u8> or bytes::Bytes
//! let bytes: Vec<u8> = person.encode_to_vec();
//! let bytes: buffa::bytes::Bytes = person.encode_to_bytes();  // re-export of `bytes` crate
//!
//! // Decode from a byte slice
//! let decoded = Person::decode_from_slice(&bytes)?;
//!
//! // Merge into an existing message (proto3 last-wins / proto2 merge semantics)
//! existing.merge_from_slice(&bytes)?;
//! # Ok(())
//! # }
//! ```
//!
//! For untrusted input, use [`DecodeOptions`] to tighten limits:
//!
//! ```no_run
//! # use buffa::__doctest_fixtures::Person;
//! use buffa::DecodeOptions;
//!
//! # fn example(bytes: &[u8]) -> Result<(), buffa::DecodeError> {
//! let msg: Person = DecodeOptions::new()
//!     .with_recursion_limit(50)
//!     .with_max_message_size(1024 * 1024)     // 1 MiB
//!     .with_unknown_field_limit(10_000)        // unknown fields per decode
//!     .decode_from_slice(&bytes)?;
//! # Ok(())
//! # }
//! ```
//!
//! The trait-level convenience methods (`decode_from_slice`, `merge_from_slice`)
//! use a fixed recursion limit of [`RECURSION_LIMIT`] (100), a fixed
//! [`DEFAULT_UNKNOWN_FIELD_LIMIT`] (1,000,000) bounding how many unknown
//! fields the decoder will materialize, and no explicit size cap — a
//! `&[u8]` is already bounded by whatever allocated it. Use `DecodeOptions`
//! to tune these, e.g. to reject oversized inputs at the decode entry point
//! rather than at the allocator.
//!
//! # Zero-copy views
//!
//! Every owned message type `Foo` has a corresponding `FooView<'a>` that
//! borrows `string` and `bytes` fields directly from the input buffer — no
//! allocation on the read path. See the [`view`] module.
//!
//! ```no_run
//! # use buffa::__doctest_fixtures::PersonView;
//! # use buffa::view::MessageView;
//! # fn example(wire_bytes: &[u8]) -> Result<(), buffa::DecodeError> {
//! let req = PersonView::decode_view(&wire_bytes)?;
//! println!("name: {}", req.name);  // &'a str, zero-copy
//! # Ok(())
//! # }
//! ```
//!
//! # Feature flags
//!
//! | Flag | Default | Enables |
//! |------|:-------:|---------|
//! | `std` | ✓ | `std::io::Read` decoders, `std::collections::HashMap` for map fields, thread-local JSON parse options (the `json` module) |
//! | `json` |  | Proto3 JSON via `serde` (the `json_helpers` and `any_registry` modules) |
//! | `text` |  | Textproto (human-readable) encoding and decoding |
//! | `arbitrary` |  | `arbitrary::Arbitrary` impls for fuzzing |
//!
//! Generated `string` / `bytes` fields can use a custom owned type (any type
//! satisfying [`ProtoString`] / [`ProtoBytes`]) selected at code-generation
//! time through `buffa_build`'s `string_type` / `bytes_type` knobs. The chosen
//! crate is an ordinary dependency of the consuming crate — buffa does not
//! re-export it (see [`ProtoString`]).
//!
//! With `default-features = false` the crate is `#![no_std]` (requires
//! `alloc`). Proto3 JSON serialization still works without `std` via
//! `serde` + `serde_json` with their own `alloc` features.
//!
//! # Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Message`] | Core trait for encode / decode / merge |
//! | [`MessageName`] | Compile-time `FULL_NAME` const (`"pkg.Msg"`) for generic dispatch |
//! | [`DecodeOptions`] | Configurable recursion and size limits |
//! | [`MessageField<T>`](MessageField) | Optional sub-message with transparent `Deref` to default |
//! | [`EnumValue<E>`](EnumValue) | Open enum wrapper (`Known(E)` / `Unknown(i32)`) |
//! | [`UnknownFields`] | Unknown-field preservation for round-trip fidelity |
//! | [`Extension<C>`](Extension) | Typed extension descriptor (codegen-emitted `pub const`) |
//! | [`ExtensionSet`] | Get/set extensions via unknown-field storage |
//! | [`view::MessageView`] | Zero-copy borrowed view trait |
//! | [`view::OwnedView<V>`](view::OwnedView) | Self-contained `'static` view backed by `Bytes` |
//! | [`view::ViewReborrow`] | Expose real borrow lifetime from `OwnedView` via [`reborrow`](view::OwnedView::reborrow) |
//!
//! # `no_std`
//!
//! ```toml
//! buffa = { version = "0.5", default-features = false }
//! ```
//!
//! Generated code uses `::buffa::alloc::string::String` etc. rather than
//! relying on the prelude, so it compiles unchanged on bare-metal targets.
//! Map fields use `std::collections::HashMap` (`hashbrown::HashMap` under
//! `no_std`) with the [`foldhash`] hasher on both paths — see [`Map`] for
//! the rationale and the opt-out.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
// Crate-level examples above use the `__doctest_fixtures` stubs (Person,
// PersonView) and compile as `no_run`. Examples in module/item docs that
// reference types specific to a user's schema (Address, Inner, etc.)
// remain `ignore` — the fixture boilerplate would exceed the example.

// Re-exported for use in generated code so that downstream crates only need
// to depend on `buffa`, not on `alloc`, `bytes`, or `serde_json` directly.
#[doc(hidden)]
pub extern crate alloc;
#[doc(hidden)]
pub use ::bytes;
// Generated `Deserialize` impls for messages with extension ranges buffer
// `"[pkg.ext]"` JSON keys into a `serde_json::Value` before dispatching to
// `extension_registry::deserialize_extension_key`. Re-export so that path
// resolves without the consumer adding `serde_json` to its own `Cargo.toml`.
//
// `serde` is *not* re-exported: the `#[derive(::serde::Serialize)]` macro
// emits `extern crate serde as _serde;` by default, so the consumer crate
// must depend on `serde` directly. Routing it through a re-export would
// require stamping `#[serde(crate = "::buffa::serde")]` on every generated
// derive (and rewriting every other emitted `::serde::` path) — feasible,
// but not done. Keep `serde` in the documented consumer requirements.
#[cfg(feature = "json")]
#[doc(hidden)]
pub use ::serde_json;

/// Include the generated stitcher for a proto **package** from `OUT_DIR`.
///
/// Codegen emits one `<pkg>.mod.rs` per package which `include!`s the
/// per-proto content files and authors the `__buffa::{view, oneof, ext}`
/// ancillary tree, so a single macro call brings in everything.
///
/// `$pkg` is the **dotted proto package literal** exactly as it appears
/// in the `.proto`'s `package` declaration (e.g. `"google.protobuf"`,
/// not a Rust path or the `.proto` file path). For protos with no
/// `package` declaration, pass `"__buffa"` (the reserved sentinel; no
/// real package can use it).
///
/// ```ignore
/// pub mod my_pkg {
///     buffa::include_proto!("my.pkg");
/// }
/// ```
///
/// For checked-in generated code (no `OUT_DIR`), use
/// [`include_proto_relative!`].
#[macro_export]
macro_rules! include_proto {
    ($pkg:literal) => {
        include!(concat!(env!("OUT_DIR"), "/", $pkg, ".mod.rs"));
    };
}

/// Like [`include_proto!`] but takes a relative directory instead of
/// reading `OUT_DIR` — for crates that check generated code into the
/// source tree (e.g. `buffa-types`, `buffa-descriptor`).
///
/// `$pkg` is the dotted proto package literal exactly as in the
/// `.proto`'s `package` declaration; for the unnamed package pass
/// `"__buffa"`. `$dir` is relative to the calling source file.
///
/// ```ignore
/// pub mod protobuf {
///     buffa::include_proto_relative!("generated", "google.protobuf");
/// }
/// ```
#[macro_export]
macro_rules! include_proto_relative {
    ($dir:literal, $pkg:literal) => {
        include!(concat!($dir, "/", $pkg, ".mod.rs"));
    };
}

/// Test helper: a [`DecodeContext`] at `depth` with a fresh default-size
/// unknown-field allowance. Leaks the limit cell (tests only) so the
/// context can be passed around without scope gymnastics.
#[cfg(test)]
pub(crate) fn test_ctx(depth: u32) -> DecodeContext<'static> {
    let limit = alloc::boxed::Box::leak(alloc::boxed::Box::new(core::cell::Cell::new(
        DEFAULT_UNKNOWN_FIELD_LIMIT,
    )));
    DecodeContext::new(depth, limit)
}

#[cfg(feature = "json")]
pub mod any_registry;
pub mod editions;
pub mod encoding;
pub mod enumeration;
pub mod error;
pub mod extension;
#[cfg(feature = "json")]
pub mod extension_registry;
#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "json")]
pub mod json_helpers;
// Generated-code plumbing: codecs are named via turbofish by `write_to` /
// `merge_field` emission, never by hand-written code. Hidden so the codec
// ZSTs and their sealed traits don't surface as consumer API on docs.rs.
#[doc(hidden)]
pub mod map_codec;
pub mod message;
pub mod message_field;
pub mod message_set;
pub mod oneof;
mod size_cache;
#[cfg(feature = "text")]
pub mod text;
#[cfg(any(feature = "json", feature = "text"))]
pub mod type_registry;
pub mod types;
pub mod unknown_fields;
pub mod view;

// ── User-facing re-exports ─────────────────────────────────────────────────────
// Types that appear in user code: trait bounds, field types, return types,
// and things users explicitly construct or call methods on.

pub use enumeration::{EnumValue, Enumeration};
pub use error::{DecodeError, EncodeError};
pub use extension::{Extension, ExtensionCodec, ExtensionSet};
/// Re-exported so downstream code that needs to name the hasher type behind
/// [`Map`] (in an explicit signature, or to `with_capacity_and_hasher` without
/// inference) does so against buffa's `foldhash` version rather than a
/// separately-resolved one. This couples buffa's public-API semver to
/// `foldhash`'s major version. Most callers should not need this — `Map` and
/// `Default::default()` keep the hasher fully inferred.
pub use foldhash;
pub use map_codec::{Map, MapStorage};
pub use message::{
    DecodeContext, DecodeOptions, Message, MessageName, DEFAULT_UNKNOWN_FIELD_LIMIT,
    RECURSION_LIMIT,
};
pub use message_field::{DefaultInstance, MessageField, ProtoBox};
pub use oneof::Oneof;
pub use size_cache::SizeCache;
pub use types::{ProtoBytes, ProtoList, ProtoString, WirePayload};
pub use unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};

#[cfg(feature = "text")]
pub use text::TextFormat;
pub use view::{
    DefaultViewInstance, HasMessageView, LazyMessageFieldView, LazyMessageView, LazyRepeatedView,
    MapView, MessageFieldView, MessageView, OwnedView, RepeatedView, UnknownFieldsView, ViewEncode,
    ViewReborrow,
};

/// Private re-exports used exclusively by generated code.
///
/// Not part of the public API. These items may change or disappear in any
/// release. Do not use them directly.
#[doc(hidden)]
pub mod __private {
    /// Re-exported for use in generated `DefaultInstance` implementations.
    ///
    /// Generated code refers to this as `::buffa::__private::OnceBox<T>` so
    /// that downstream crates do not need a direct `once_cell` dependency.
    pub use once_cell::race::OnceBox;

    /// The concrete `map<K, V>` field type generated code uses by default.
    ///
    /// Generated code refers to this as `::buffa::__private::HashMap<K, V>`
    /// so that downstream crates do not need a direct dependency on the
    /// hasher or container crate.
    ///
    /// On `std` builds this is `std::collections::HashMap` parameterized with
    /// [`foldhash::fast::RandomState`] as the hasher; on `no_std` builds it is
    /// `hashbrown::HashMap` (which defaults to the same `foldhash` hasher).
    /// The table implementation is identical either way — `std::collections::HashMap`
    /// is itself a thin wrapper around hashbrown's SwissTable — so the only
    /// load-bearing choice here is the hasher.
    ///
    /// See [`Map`](crate::Map) for the hasher rationale and construction
    /// guidance.
    #[cfg(not(feature = "std"))]
    pub type HashMap<K, V> = hashbrown::HashMap<K, V>;
    #[cfg(feature = "std")]
    pub type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;

    // ── Type-agnostic `Arbitrary` builders for configurable owned types ──────
    //
    // These replace the per-type shims above (`arbitrary_ecow*`,
    // `arbitrary_bytes*`): codegen attaches them to any field whose
    // representation is non-default, selecting by *kind* (string vs bytes,
    // singular vs optional vs repeated) rather than by the concrete type. They
    // build the canonical `String` / `Vec<u8>` first and convert through the
    // `From` bound, so a substituted type needs no native `Arbitrary` impl and
    // codegen carries no knowledge of any specific type. Materializing the
    // canonical type first also keeps byte-consumption order identical to the
    // default-representation impl, which the parity tests assert.

    /// Build a [`ProtoString`](crate::ProtoString) from `Arbitrary` bytes.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_string<S: crate::ProtoString>(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<S> {
        let s: ::alloc::string::String = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(S::from(s))
    }

    /// Build an `Option<S>` for an explicit-presence `string` field.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_string_opt<S: crate::ProtoString>(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::core::option::Option<S>> {
        let opt: ::core::option::Option<::alloc::string::String> =
            ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(opt.map(S::from))
    }

    /// Build a `Vec<S>` for a repeated `string` field.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_string_vec<S: crate::ProtoString>(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::alloc::vec::Vec<S>> {
        let vv: ::alloc::vec::Vec<::alloc::string::String> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(vv.into_iter().map(S::from).collect())
    }

    /// Build a [`ProtoBytes`](crate::ProtoBytes) from `Arbitrary` bytes.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_bytes<B: crate::ProtoBytes>(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<B> {
        let v: ::alloc::vec::Vec<u8> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(B::from(v))
    }

    /// Build an `Option<B>` for an explicit-presence `bytes` field.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_bytes_opt<B: crate::ProtoBytes>(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::core::option::Option<B>> {
        let opt: ::core::option::Option<::alloc::vec::Vec<u8>> =
            ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(opt.map(B::from))
    }

    /// Build a `Vec<B>` for a repeated `bytes` field.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_bytes_vec<B: crate::ProtoBytes>(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::alloc::vec::Vec<B>> {
        let vv: ::alloc::vec::Vec<::alloc::vec::Vec<u8>> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(vv.into_iter().map(B::from).collect())
    }

    /// Build the owned map collection for a `map<K, bytes>` field with a
    /// non-default value representation. Generic over the container (any
    /// [`MapStorage`](crate::map_codec::MapStorage)) and the key type, so a
    /// `HashMap`, `BTreeMap`, or custom map field needs no per-container shim.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_proto_bytes_map<'a, C>(
        u: &mut ::arbitrary::Unstructured<'a>,
    ) -> ::arbitrary::Result<C>
    where
        C: crate::map_codec::MapStorage + Default,
        C::Key: ::arbitrary::Arbitrary<'a>,
        C::Value: crate::ProtoBytes,
    {
        let mut out = C::default();
        for entry in u.arbitrary_iter::<(C::Key, ::alloc::vec::Vec<u8>)>()? {
            let (k, v) = entry?;
            out.storage_insert(k, <C::Value as ::core::convert::From<_>>::from(v));
        }
        Ok(out)
    }
}

#[cfg(all(test, feature = "arbitrary"))]
mod arbitrary_tests {
    use super::__private::{
        arbitrary_proto_bytes, arbitrary_proto_bytes_opt, arbitrary_proto_bytes_vec,
    };
    use ::bytes::Bytes;
    use alloc::vec::Vec;
    use arbitrary::{Arbitrary, Unstructured};

    /// Non-zero seed so the helper has data to consume — an all-zero buffer
    /// deterministically produces empty vectors regardless of whether the
    /// helper's inner `Arbitrary` call is wired correctly. The exact lengths
    /// that arise depend on the `arbitrary` crate's internal byte-consumption
    /// strategy (and may change between versions), so the tests below assert
    /// equivalence against the underlying `Vec<u8>` impl from an identical
    /// `Unstructured` rather than a hard-coded non-empty length.
    const SEED: [u8; 128] = {
        let mut a = [0u8; 128];
        let mut i = 0;
        while i < 128 {
            a[i] = (i as u8).wrapping_mul(31).wrapping_add(7);
            i += 1;
        }
        a
    };

    // The generic `arbitrary_proto_bytes*` builders construct the canonical
    // `Vec<u8>` and convert via `From`, so for any `ProtoBytes` type they must
    // mirror the underlying `Vec<u8>` / `Option<Vec<u8>>` / `Vec<Vec<u8>>`
    // impls byte-for-byte. Exercised here against `bytes::Bytes`.
    #[test]
    fn arbitrary_proto_bytes_matches_vec_u8() {
        let b: Bytes = arbitrary_proto_bytes(&mut Unstructured::new(&SEED)).unwrap();
        let v: Vec<u8> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(b.as_ref(), v.as_slice());
        assert!(b.len() <= SEED.len());
    }

    #[test]
    fn arbitrary_proto_bytes_opt_matches_option_vec_u8() {
        let b: Option<Bytes> = arbitrary_proto_bytes_opt(&mut Unstructured::new(&SEED)).unwrap();
        let v: Option<Vec<u8>> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(b.is_some(), v.is_some());
        assert_eq!(
            b.as_ref().map(|x| x.to_vec()),
            v,
            "Option<Bytes> builder must mirror Option<Vec<u8>>"
        );
    }

    #[test]
    fn arbitrary_proto_bytes_vec_matches_vec_vec_u8() {
        let bs: Vec<Bytes> = arbitrary_proto_bytes_vec(&mut Unstructured::new(&SEED)).unwrap();
        let vs: Vec<Vec<u8>> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(bs.len(), vs.len());
        for (b, v) in bs.iter().zip(&vs) {
            assert_eq!(b.as_ref(), v.as_slice());
        }
    }

    // The generic `arbitrary_proto_string` builder is the identity for `String`;
    // confirm it consumes bytes identically to the native `String` impl.
    #[test]
    fn arbitrary_proto_string_matches_string() {
        use super::__private::arbitrary_proto_string;
        use alloc::string::String;

        let s: String = arbitrary_proto_string(&mut Unstructured::new(&SEED)).unwrap();
        let v: String = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(s, v);
    }
}

/// Minimal fixture types for compile-checking doc examples.
///
/// These stand in for generated message types (`Person`, `PersonView`, etc.)
/// so that crate-level `//!` examples can be `no_run` instead of `ignore`.
/// The module is `#[doc(hidden)]` — it appears in neither rendered docs
/// nor IDE autocomplete — and its contents are stripped by LTO in release
/// builds if unused. Not part of the public API; may change at any time.
///
/// README.md examples remain `ignore` because GitHub/crates.io Markdown
/// rendering would show hidden `#` lines.
#[doc(hidden)]
pub mod __doctest_fixtures {
    use crate::*;

    #[derive(Clone, Default, PartialEq)]
    pub struct Person {
        pub name: alloc::string::String,
        pub id: i32,
    }

    impl DefaultInstance for Person {
        fn default_instance() -> &'static Self {
            static INST: __private::OnceBox<Person> = __private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(Self::default()))
        }
    }

    impl Message for Person {
        fn compute_size(&self, _cache: &mut SizeCache) -> u32 {
            0
        }
        fn write_to(&self, _cache: &mut SizeCache, _buf: &mut impl bytes::BufMut) {}
        fn merge_field(
            &mut self,
            tag: crate::encoding::Tag,
            buf: &mut impl bytes::Buf,
            _ctx: DecodeContext<'_>,
        ) -> Result<(), DecodeError> {
            crate::encoding::skip_field(tag, buf)
        }
        fn clear(&mut self) {
            *self = Self::default();
        }
    }

    #[derive(Clone, Default)]
    pub struct PersonView<'a> {
        pub name: &'a str,
        pub id: i32,
    }

    impl<'a> view::MessageView<'a> for PersonView<'a> {
        type Owned = Person;
        fn decode_view(_buf: &'a [u8]) -> Result<Self, DecodeError> {
            // Stub: examples are `no_run`, so this never executes.
            Ok(PersonView::default())
        }
        fn merge_view_field(
            &mut self,
            _tag: encoding::Tag,
            cur: &'a [u8],
            _before_tag: &'a [u8],
            _ctx: DecodeContext<'_>,
        ) -> Result<&'a [u8], DecodeError> {
            // Stub for the doc-example view; never executes.
            Ok(cur)
        }
        fn to_owned_message(&self) -> Result<Person, DecodeError> {
            Ok(Person {
                name: self.name.into(),
                id: self.id,
            })
        }
    }

    impl view::ViewReborrow for PersonView<'static> {
        type Reborrowed<'b> = PersonView<'b>;
        fn reborrow<'b>(this: &'b Self) -> &'b Self::Reborrowed<'b> {
            this
        }
    }
}

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
//! | `smol_str` |  | Allow generated `string` fields to use `smol_str::SmolStr` (see [`ProtoString`]) |
//! | `ecow` |  | Allow generated `string` fields to use `ecow::EcoString` (see [`ProtoString`]) |
//! | `compact_str` |  | Allow generated `string` fields to use `compact_str::CompactString` (see [`ProtoString`]) |
//!
//! The three string-type flags compose with `json` and `arbitrary`: enabling,
//! for example, both `smol_str` and `json` turns on `smol_str/serde`, and
//! `smol_str` + `arbitrary` turns on `smol_str/arbitrary`. `ecow` has no native
//! `Arbitrary` impl, so `ecow` + `arbitrary` is served by an in-crate shim
//! instead. None of the three is selected by generated code until a build is
//! configured to use it (see [`ProtoString`]).
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
//! Map fields use `hashbrown::HashMap` under `no_std`; with the `std` feature
//! enabled they use `std::collections::HashMap` for ecosystem interop.

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

// Configurable `string` field representations. Re-exported so that code
// generated with `buffa_build`'s `string_type` knob can name
// `::buffa::smol_str::SmolStr` (etc.) without the consumer crate declaring the
// dependency — the same arrangement as `bytes` above. Each is gated on the
// matching `buffa` feature.
#[cfg(feature = "compact_str")]
#[doc(hidden)]
pub use ::compact_str;
#[cfg(feature = "ecow")]
#[doc(hidden)]
pub use ::ecow;
#[cfg(feature = "smol_str")]
#[doc(hidden)]
pub use ::smol_str;

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
pub use message::{
    DecodeContext, DecodeOptions, Message, MessageName, DEFAULT_UNKNOWN_FIELD_LIMIT,
    RECURSION_LIMIT,
};
pub use message_field::{DefaultInstance, MessageField};
pub use oneof::Oneof;
pub use size_cache::SizeCache;
pub use types::ProtoString;
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

    /// Re-exported for use in generated `map<K, V>` field types.
    ///
    /// Generated code refers to this as `::buffa::__private::HashMap<K, V>`
    /// so that downstream crates do not need a direct `hashbrown` dependency.
    /// On `no_std` builds this is `hashbrown::HashMap`; on `std` builds it is
    /// `std::collections::HashMap` for full stdlib interoperability.
    ///
    /// Note: these are distinct concrete types even though `std::collections::HashMap`
    /// is backed by `hashbrown` internally.  Code that needs to interoperate
    /// across the `std`/`no_std` feature boundary should use the re-exported
    /// type rather than either concrete type directly.
    #[cfg(not(feature = "std"))]
    pub use hashbrown::HashMap;
    #[cfg(feature = "std")]
    pub use std::collections::HashMap;

    /// `arbitrary` helpers for `bytes::Bytes` fields generated with `bytes_fields`.
    ///
    /// `bytes::Bytes` has no `Arbitrary` impl. Generated code attaches
    /// `#[arbitrary(with = ::buffa::__private::arbitrary_bytes*)]` to
    /// `bytes_fields`-typed fields so the struct-level `#[derive(Arbitrary)]`
    /// can still be used. The three variants cover singular, optional, and
    /// repeated bytes fields respectively; oneof variant inner fields use
    /// `arbitrary_bytes` (the singular form).
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_bytes(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::bytes::Bytes> {
        let v: ::alloc::vec::Vec<u8> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(::bytes::Bytes::from(v))
    }

    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_bytes_opt(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::core::option::Option<::bytes::Bytes>> {
        let opt: ::core::option::Option<::alloc::vec::Vec<u8>> =
            ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(opt.map(::bytes::Bytes::from))
    }

    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_bytes_vec(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::alloc::vec::Vec<::bytes::Bytes>> {
        let vv: ::alloc::vec::Vec<::alloc::vec::Vec<u8>> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(vv.into_iter().map(::bytes::Bytes::from).collect())
    }

    /// `Arbitrary` shim for `map<K, bytes>` fields under
    /// `bytes_fields`, where the value type is `bytes::Bytes`.
    ///
    /// Generic over the key type so the codegen call site doesn't need a
    /// per-key-type shim; `K`'s own `Arbitrary` impl drives key generation.
    /// The intermediate `HashMap<K, Vec<u8>>` keeps byte-consumption order
    /// identical to the underlying impl.
    #[cfg(feature = "arbitrary")]
    pub fn arbitrary_bytes_map<'a, K>(
        u: &mut ::arbitrary::Unstructured<'a>,
    ) -> ::arbitrary::Result<HashMap<K, ::bytes::Bytes>>
    where
        K: ::arbitrary::Arbitrary<'a> + ::core::cmp::Eq + ::core::hash::Hash,
    {
        let m: HashMap<K, ::alloc::vec::Vec<u8>> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(m.into_iter()
            .map(|(k, v)| (k, ::bytes::Bytes::from(v)))
            .collect())
    }

    /// `arbitrary` helpers for `ecow::EcoString` string fields.
    ///
    /// Unlike `smol_str::SmolStr` and `compact_str::CompactString`, `EcoString`
    /// ships no `Arbitrary` impl, so codegen attaches
    /// `#[arbitrary(with = ::buffa::__private::arbitrary_ecow*)]` to
    /// `EcoString`-typed string fields (the same pattern used for
    /// `bytes::Bytes`). The three variants cover singular, optional, and
    /// repeated fields; oneof variant inner fields use `arbitrary_ecow`.
    #[cfg(all(feature = "ecow", feature = "arbitrary"))]
    pub fn arbitrary_ecow(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::ecow::EcoString> {
        let s: ::alloc::string::String = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(::ecow::EcoString::from(s))
    }

    #[cfg(all(feature = "ecow", feature = "arbitrary"))]
    pub fn arbitrary_ecow_opt(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::core::option::Option<::ecow::EcoString>> {
        let opt: ::core::option::Option<::alloc::string::String> =
            ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(opt.map(::ecow::EcoString::from))
    }

    #[cfg(all(feature = "ecow", feature = "arbitrary"))]
    pub fn arbitrary_ecow_vec(
        u: &mut ::arbitrary::Unstructured<'_>,
    ) -> ::arbitrary::Result<::alloc::vec::Vec<::ecow::EcoString>> {
        // Materializing a `Vec<String>` first (rather than building `EcoString`s
        // element-by-element) is deliberate: it makes the byte-consumption order
        // identical to the underlying `Vec<String>` impl, which is what the
        // parity test asserts. Do not "optimize" the intermediate `Vec` away.
        let vv: ::alloc::vec::Vec<::alloc::string::String> = ::arbitrary::Arbitrary::arbitrary(u)?;
        Ok(vv.into_iter().map(::ecow::EcoString::from).collect())
    }
}

#[cfg(all(test, feature = "arbitrary"))]
mod arbitrary_tests {
    use super::__private::{arbitrary_bytes, arbitrary_bytes_opt, arbitrary_bytes_vec};
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

    #[test]
    fn arbitrary_bytes_matches_vec_u8() {
        let b = arbitrary_bytes(&mut Unstructured::new(&SEED)).unwrap();
        let v: Vec<u8> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        // Must be a real `Bytes` — `slice(..)` is `Bytes`-specific.
        assert_eq!(b.slice(..).as_ref(), v.as_slice());
        assert!(b.len() <= SEED.len());
    }

    #[test]
    fn arbitrary_bytes_opt_matches_option_vec_u8() {
        let b = arbitrary_bytes_opt(&mut Unstructured::new(&SEED)).unwrap();
        let v: Option<Vec<u8>> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(b.is_some(), v.is_some());
        assert_eq!(
            b.as_ref().map(|x| x.slice(..).to_vec()),
            v,
            "Option<Bytes> shim must mirror Option<Vec<u8>>"
        );
    }

    #[test]
    fn arbitrary_bytes_vec_matches_vec_vec_u8() {
        let bs = arbitrary_bytes_vec(&mut Unstructured::new(&SEED)).unwrap();
        let vs: Vec<Vec<u8>> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(bs.len(), vs.len());
        for (b, v) in bs.iter().zip(&vs) {
            assert_eq!(b.slice(..).as_ref(), v.as_slice());
        }
    }

    // The `EcoString` arbitrary shims must mirror the underlying `String` /
    // `Option<String>` / `Vec<String>` impls, since `ecow` ships no native
    // `Arbitrary`. The other two configurable string types use their own
    // native impls and need no shim.
    #[cfg(feature = "ecow")]
    #[test]
    fn arbitrary_ecow_matches_string() {
        use super::__private::{arbitrary_ecow, arbitrary_ecow_opt, arbitrary_ecow_vec};
        use alloc::string::String;

        let s = arbitrary_ecow(&mut Unstructured::new(&SEED)).unwrap();
        let v: String = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(s.as_str(), v.as_str());

        let so = arbitrary_ecow_opt(&mut Unstructured::new(&SEED)).unwrap();
        let vo: Option<String> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(so.as_ref().map(|x| x.as_str()), vo.as_deref());

        let sv = arbitrary_ecow_vec(&mut Unstructured::new(&SEED)).unwrap();
        let vv: Vec<String> = Arbitrary::arbitrary(&mut Unstructured::new(&SEED)).unwrap();
        assert_eq!(sv.len(), vv.len());
        for (a, b) in sv.iter().zip(&vv) {
            assert_eq!(a.as_str(), b.as_str());
        }
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

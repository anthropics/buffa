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
//!     .with_max_message_size(1024 * 1024)  // 1 MiB
//!     .decode_from_slice(&bytes)?;
//! # Ok(())
//! # }
//! ```
//!
//! The trait-level convenience methods (`decode_from_slice`, `merge_from_slice`)
//! use a fixed recursion limit of [`RECURSION_LIMIT`] (100) and no explicit size
//! cap — a `&[u8]` is already bounded by whatever allocated it. Use `DecodeOptions`
//! when you want to reject oversized inputs at the decode entry point rather than
//! at the allocator.
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
//! With `default-features = false` the crate is `#![no_std]` (requires
//! `alloc`). Proto3 JSON serialization still works without `std` via
//! `serde` + `serde_json` with their own `alloc` features.
//!
//! # Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Message`] | Core trait for encode / decode / merge |
//! | [`DecodeOptions`] | Configurable recursion and size limits |
//! | [`MessageField<T>`](MessageField) | Optional sub-message with transparent `Deref` to default |
//! | [`EnumValue<E>`](EnumValue) | Open enum wrapper (`Known(E)` / `Unknown(i32)`) |
//! | [`UnknownFields`] | Unknown-field preservation for round-trip fidelity |
//! | [`Extension<C>`](Extension) | Typed extension descriptor (codegen-emitted `pub const`) |
//! | [`ExtensionSet`] | Get/set extensions via unknown-field storage |
//! | [`view::MessageView`] | Zero-copy borrowed view trait |
//! | [`view::OwnedView<V>`](view::OwnedView) | Self-contained `'static` view backed by `Bytes` |
//!
//! # `no_std`
//!
//! ```toml
//! buffa = { version = "0.3", default-features = false }
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
// to depend on `buffa`, not on `alloc` or `bytes` directly.
#[doc(hidden)]
pub extern crate alloc;
#[doc(hidden)]
pub use ::bytes;

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
pub use message::{DecodeOptions, Message, RECURSION_LIMIT};
pub use message_field::{DefaultInstance, MessageField};
pub use oneof::Oneof;
pub use size_cache::SizeCache;
pub use unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};

#[cfg(feature = "text")]
pub use text::TextFormat;
pub use view::{
    DefaultViewInstance, MapView, MessageFieldView, MessageView, OwnedView, RepeatedView,
    UnknownFieldsView, ViewEncode,
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
            _depth: u32,
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
        fn to_owned_message(&self) -> Person {
            Person {
                name: self.name.into(),
                id: self.id,
            }
        }
    }
}

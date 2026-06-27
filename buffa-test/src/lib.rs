// Wrap generated code in the package module so intra-file type references
// (e.g. `basic::Status`, `basic::Address`) resolve correctly.
//
// The clippy allows suppress lints that fire on generated code patterns:
// - derivable_impls: generated enum Default impls are explicit rather than derived
// - match_single_binding: empty messages generate a single-arm wildcard merge match
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod basic {
    buffa::include_proto!("basic");
}

/// `[debug_redact = true]` — generated Debug impls print a placeholder
/// instead of the annotated field's value.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod debug_redact {
    buffa::include_proto!("debug_redact");
}

/// `box_type` + a crate-local `CustomBox<T>` pointer for singular message
/// fields. `CustomBox<T>` is a thin `Box<T>`-backed `ProtoBox<T>` impl — the
/// point is to exercise the generic codegen path (`MessageField<T,
/// CustomBox<T>>`, decode via `get_or_insert_default`, view→owned via `some`),
/// independent of any external smallbox crate. A real consumer would back this
/// with e.g. `smallbox::SmallBox`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod box_type {
    /// A `Box`-backed pointer implementing `buffa::ProtoBox<T>`. No `Send`/`Sync`
    /// or `Default` bound is needed (`ProtoBox` requires neither).
    #[derive(Clone, PartialEq, Debug)]
    pub struct CustomBox<T>(pub ::buffa::alloc::boxed::Box<T>);

    impl<T> ::core::ops::Deref for CustomBox<T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }

    impl<T> ::core::ops::DerefMut for CustomBox<T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.0
        }
    }

    impl<T> ::buffa::ProtoBox<T> for CustomBox<T> {
        fn new(value: T) -> Self {
            CustomBox(::buffa::alloc::boxed::Box::new(value))
        }
        fn into_inner(self) -> T {
            *self.0
        }
    }

    buffa::include_proto!("box_type");
}

/// `PointerRepr::Inline` default: the built-in inline pointer
/// (`::buffa::Inline<T>`) for every non-recursive singular message field. The
/// `self_ref` field is recursive and stays on `Box`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod inline_field {
    buffa::include_proto!("inline_field");
}

/// `string_type` + vtable reflection with a crate-local newtype string used as
/// a `repeated` element. Because the type is local, codegen may emit the
/// `ReflectElement` and `ProtoElemJson` impls for it — the orphan rule forbids
/// those for a foreign type used in a repeated field.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod vtable_string_repr {
    /// `String`-backed newtype satisfying `buffa::ProtoString` (`Deref<str>` +
    /// `AsRef<str>` + `From<String>`/`From<&str>`). It derives `Serialize` /
    /// `Deserialize` because a `repeated string` JSON field serializes its
    /// elements through their native serde impls (singular fields use the
    /// `proto_string` with-module instead, which needs only `AsRef`/`From`).
    #[derive(Clone, PartialEq, Eq, Default, Debug, ::serde::Serialize, ::serde::Deserialize)]
    pub struct LocalStr(pub ::buffa::alloc::string::String);

    impl ::core::ops::Deref for LocalStr {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::AsRef<str> for LocalStr {
        fn as_ref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::From<::buffa::alloc::string::String> for LocalStr {
        fn from(s: ::buffa::alloc::string::String) -> Self {
            LocalStr(s)
        }
    }
    impl ::core::convert::From<&str> for LocalStr {
        fn from(s: &str) -> Self {
            LocalStr(::buffa::alloc::string::String::from(s))
        }
    }
    impl ::buffa::ProtoString for LocalStr {
        fn from_wire(
            payload: ::buffa::WirePayload<'_>,
        ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
            ::core::str::from_utf8(payload.as_slice())
                .map(|s| LocalStr(::buffa::alloc::string::String::from(s)))
                .map_err(|_| ::buffa::DecodeError::InvalidUtf8)
        }
    }

    buffa::include_proto!("vtable_string_repr");
}

/// `bytes_type` + vtable reflection with a crate-local newtype used as a
/// `repeated` element. Mirrors `vtable_string_repr` for the bytes side: the
/// codegen-emitted `ReflectElement` and `ProtoElemJson` (base64) impls for
/// `LocalBytes` compile only because the type is local to this crate.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod vtable_bytes_repr {
    /// `Vec<u8>`-backed newtype satisfying `buffa::ProtoBytes` (`Deref<[u8]>` +
    /// `AsRef<[u8]>` + `From<Vec<u8>>`). It needs no `serde` impl: singular bytes
    /// use the `bytes` JSON with-module and repeated bytes use the emitted
    /// `ProtoElemJson` base64 impl.
    #[derive(Clone, PartialEq, Eq, Default, Debug)]
    pub struct LocalBytes(pub ::buffa::alloc::vec::Vec<u8>);

    impl ::core::ops::Deref for LocalBytes {
        type Target = [u8];
        fn deref(&self) -> &[u8] {
            &self.0
        }
    }
    impl ::core::convert::AsRef<[u8]> for LocalBytes {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }
    impl ::core::convert::From<::buffa::alloc::vec::Vec<u8>> for LocalBytes {
        fn from(v: ::buffa::alloc::vec::Vec<u8>) -> Self {
            LocalBytes(v)
        }
    }
    impl ::buffa::ProtoBytes for LocalBytes {
        fn from_wire(
            payload: ::buffa::WirePayload<'_>,
        ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
            ::core::result::Result::Ok(LocalBytes(payload.as_slice().to_vec()))
        }
    }

    buffa::include_proto!("vtable_bytes_repr");
}

/// `repeated_type` + a crate-local `CustomList<T>` collection used for every
/// `repeated` field. `CustomList<T>` is a thin `Vec<T>`-backed `ProtoList<T>`
/// impl — the point is to exercise the generic codegen path (merge via
/// `ProtoList::push`/`reserve`, encode via the `Deref` slice, clear via
/// `ProtoList::clear`, view→owned via `FromIterator`), independent of any
/// external collection crate. A real consumer would back this with e.g.
/// `smallvec::SmallVec`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod repeated_type {
    /// A `Vec`-backed collection implementing `buffa::ProtoList<T>`. `Default`
    /// is hand-written (not derived) so it does not require `T: Default`, which
    /// the supertrait bound would otherwise force on every element type.
    #[derive(Clone, PartialEq, Debug)]
    pub struct CustomList<T>(pub ::buffa::alloc::vec::Vec<T>);

    impl<T> ::core::default::Default for CustomList<T> {
        fn default() -> Self {
            CustomList(::buffa::alloc::vec::Vec::new())
        }
    }

    impl<T> ::core::ops::Deref for CustomList<T> {
        type Target = [T];
        fn deref(&self) -> &[T] {
            &self.0
        }
    }

    impl<T> ::core::iter::FromIterator<T> for CustomList<T> {
        fn from_iter<I: ::core::iter::IntoIterator<Item = T>>(iter: I) -> Self {
            CustomList(::buffa::alloc::vec::Vec::from_iter(iter))
        }
    }

    impl<T> ::core::convert::From<::buffa::alloc::vec::Vec<T>> for CustomList<T> {
        fn from(v: ::buffa::alloc::vec::Vec<T>) -> Self {
            CustomList(v)
        }
    }

    impl<T> ::buffa::ProtoList<T> for CustomList<T>
    where
        T: ::core::clone::Clone
            + ::core::cmp::PartialEq
            + ::core::fmt::Debug
            + ::core::marker::Send
            + ::core::marker::Sync,
    {
        fn push(&mut self, value: T) {
            self.0.push(value);
        }
        fn clear(&mut self) {
            self.0.clear();
        }
        // Left as the advisory no-op (the default would do): exercises the
        // decode path tolerating a collection that ignores the capacity hint,
        // which is the recommended form for a bounded/inline collection.
        fn reserve(&mut self, _additional: usize) {}
    }

    buffa::include_proto!("repeated_type");
}

/// Crate-local `ProtoString` newtypes wrapping foreign small-string types, used
/// by the `string_types` fixture. They mirror `buffa_smolstr::SmolStr`: a thin
/// newtype with an inline, allocation-free `from_wire`. Direct use of the
/// foreign types is no longer possible (the blanket impl is gone), so a
/// downstream crate wraps them like this. None of them needs a native
/// `Arbitrary` impl — codegen's generic `arbitrary_proto_*` builder handles it.
pub mod reprs {
    /// Newtype over `ecow::EcoString`. `ecow` ships no native `Arbitrary`, so
    /// this fixture also exercises the generic arbitrary builder path.
    #[derive(Clone, PartialEq, Eq, Default, Debug)]
    pub struct EcoStr(pub ::ecow::EcoString);

    impl EcoStr {
        pub fn as_str(&self) -> &str {
            self.0.as_str()
        }
    }

    impl ::core::ops::Deref for EcoStr {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::AsRef<str> for EcoStr {
        fn as_ref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::From<::buffa::alloc::string::String> for EcoStr {
        fn from(s: ::buffa::alloc::string::String) -> Self {
            EcoStr(::ecow::EcoString::from(s))
        }
    }
    impl ::core::convert::From<&str> for EcoStr {
        fn from(s: &str) -> Self {
            EcoStr(::ecow::EcoString::from(s))
        }
    }
    impl ::buffa::ProtoString for EcoStr {
        fn from_wire(
            payload: ::buffa::WirePayload<'_>,
        ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
            ::core::str::from_utf8(payload.as_slice())
                .map(|s| EcoStr(::ecow::EcoString::from(s)))
                .map_err(|_| ::buffa::DecodeError::InvalidUtf8)
        }
    }

    /// Newtype over `compact_str::CompactString`.
    #[derive(Clone, PartialEq, Eq, Default, Debug)]
    pub struct CompactStr(pub ::compact_str::CompactString);

    impl CompactStr {
        pub fn as_str(&self) -> &str {
            self.0.as_str()
        }
    }

    impl ::core::ops::Deref for CompactStr {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::AsRef<str> for CompactStr {
        fn as_ref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::From<::buffa::alloc::string::String> for CompactStr {
        fn from(s: ::buffa::alloc::string::String) -> Self {
            CompactStr(::compact_str::CompactString::from(s))
        }
    }
    impl ::core::convert::From<&str> for CompactStr {
        fn from(s: &str) -> Self {
            CompactStr(::compact_str::CompactString::from(s))
        }
    }
    impl ::buffa::ProtoString for CompactStr {
        fn from_wire(
            payload: ::buffa::WirePayload<'_>,
        ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
            ::core::str::from_utf8(payload.as_slice())
                .map(|s| CompactStr(::compact_str::CompactString::from(s)))
                .map_err(|_| ::buffa::DecodeError::InvalidUtf8)
        }
    }
}

/// `generate_views(false)` + vtable reflection — owned-only vtable, no views.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod vtable_no_views {
    buffa::include_proto!("vtable_no_views");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto3sem {
    buffa::include_proto!("test.proto3sem");
}

/// `map_type` fixture: every `map` field uses the buffa-provided `BTreeMap<K, V>`
/// (selected with `.map_type(MapRepr::BTreeMap)`) instead of `HashMap`. No
/// consumer code is needed — `BTreeMap` already satisfies `MapStorage`,
/// `ReflectMap`, serde, and the derive bounds — so this module is just the
/// generated code, exercised by `tests/map_type.rs`.
pub mod map_type {
    buffa::include_proto!("map_type");
}

/// `string_map` fixture: a crate-local `MapStr` newtype (a `ProtoString` impl,
/// selected with `.string_type_custom(...)`) is used for every `string` map key
/// and value. `MapStr` is `Hash + Eq + Ord + serde`, so it satisfies the
/// `HashMap` key bound and every JSON dispatch path. The type is crate-local
/// because vtable reflection emits `impl ReflectMapKey` / `impl ReflectElement`
/// for it (a foreign type would be an orphan-rule error — exactly as for a
/// custom `repeated` element). The fields cover all six custom-string-key/value
/// JSON dispatch modules; exercised by `tests/string_map.rs`.
#[allow(clippy::derivable_impls, non_camel_case_types)]
pub mod string_map {
    /// `String`-backed newtype satisfying `buffa::ProtoString`, plus the
    /// `Hash + Eq + Ord` a map key needs and `Serialize`/`Deserialize` the JSON
    /// paths need.
    #[derive(
        Clone,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Hash,
        Default,
        Debug,
        ::serde::Serialize,
        ::serde::Deserialize,
    )]
    // A custom string used in a `map` under `generate_arbitrary` must impl
    // `Arbitrary` (unlike singular/repeated string fields, which get a generic
    // builder): the map arbitrary path has no per-key shim. Deriving it on the
    // newtype is the one-line requirement.
    #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
    pub struct MapStr(pub ::buffa::alloc::string::String);

    impl ::core::ops::Deref for MapStr {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::AsRef<str> for MapStr {
        fn as_ref(&self) -> &str {
            &self.0
        }
    }
    impl ::core::convert::From<::buffa::alloc::string::String> for MapStr {
        fn from(s: ::buffa::alloc::string::String) -> Self {
            MapStr(s)
        }
    }
    impl ::core::convert::From<&str> for MapStr {
        fn from(s: &str) -> Self {
            MapStr(::buffa::alloc::string::String::from(s))
        }
    }
    impl ::buffa::ProtoString for MapStr {
        fn from_wire(
            payload: ::buffa::WirePayload<'_>,
        ) -> ::core::result::Result<Self, ::buffa::DecodeError> {
            ::core::str::from_utf8(payload.as_slice())
                .map(|s| MapStr(::buffa::alloc::string::String::from(s)))
                .map_err(|_| ::buffa::DecodeError::InvalidUtf8)
        }
    }

    buffa::include_proto!("string_map");
}

/// `map_type_custom` fixture: a crate-local `CustomMap<K, V>` newtype used for
/// every `map` field (via `.map_type_custom(...)`). `CustomMap` is a thin
/// `BTreeMap`-backed `MapStorage` impl — the point is to exercise the
/// `MapRepr::Custom` codegen path and the consumer-provided trait surface
/// (`MapStorage`, a `ReflectMap` impl delegating to the inner map, and
/// `FromIterator` for the view→owned `.collect()`), independent of buffa's
/// built-in container impls. A real consumer would back this with a foreign map
/// such as `indexmap::IndexMap`; `BTreeMap` is used here so the derives
/// (`Clone` / `PartialEq` / `Debug`) need no extra key bounds.
#[allow(clippy::derivable_impls)]
pub mod map_type_custom {
    use ::buffa::alloc::collections::BTreeMap;
    use ::buffa_descriptor::reflect::{
        MapKey, MapKeyRef, ReflectElement, ReflectMap, ReflectMapKey, ValueRef,
    };

    /// A `BTreeMap`-backed map implementing `buffa::MapStorage`. `Default`
    /// is hand-written (not derived) so it does not require `K: Default` /
    /// `V: Default`.
    #[derive(Clone, PartialEq, Debug)]
    pub struct CustomMap<K, V>(pub BTreeMap<K, V>);

    impl<K, V> ::core::default::Default for CustomMap<K, V> {
        fn default() -> Self {
            CustomMap(BTreeMap::new())
        }
    }

    impl<K: ::core::cmp::Ord, V> ::core::iter::FromIterator<(K, V)> for CustomMap<K, V> {
        fn from_iter<I: ::core::iter::IntoIterator<Item = (K, V)>>(iter: I) -> Self {
            CustomMap(BTreeMap::from_iter(iter))
        }
    }

    impl<K: ::core::cmp::Ord, V> ::buffa::MapStorage for CustomMap<K, V> {
        type Key = K;
        type Value = V;
        fn storage_len(&self) -> usize {
            self.0.len()
        }
        fn storage_insert(&mut self, key: K, value: V) {
            self.0.insert(key, value);
        }
        fn storage_clear(&mut self) {
            self.0.clear();
        }
        fn storage_iter<'a>(&'a self) -> impl ::core::iter::Iterator<Item = (&'a K, &'a V)>
        where
            K: 'a,
            V: 'a,
        {
            self.0.iter()
        }
    }

    // The vtable reflect path needs `ReflectMap`; delegate every method to the
    // inner `BTreeMap`'s impl (the "Vec/BTreeMap/HashMap-backed newtype can
    // delegate" claim in the `MapStorage` docs, verified by compilation).
    impl<K: ReflectMapKey, V: ReflectElement> ReflectMap for CustomMap<K, V> {
        fn len(&self) -> usize {
            ReflectMap::len(&self.0)
        }
        fn get(&self, key: &MapKey) -> ::core::option::Option<ValueRef<'_>> {
            ReflectMap::get(&self.0, key)
        }
        fn get_str(&self, key: &str) -> ::core::option::Option<ValueRef<'_>> {
            ReflectMap::get_str(&self.0, key)
        }
        fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>)) {
            ReflectMap::for_each(&self.0, f)
        }
    }

    buffa::include_proto!("map_type_custom");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod keywords {
    buffa::include_proto!("test.keywords");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod nested {
    buffa::include_proto!("test.nested");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod wkt {
    buffa::include_proto!("test.wkt");
}

/// `lazy_views(true)` — the additive `FooLazyView` decode-on-access family.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod lazyviews {
    buffa::include_proto!("test.lazyviews");
}

/// `lazy_views(true)` + `preserve_unknown_fields(false)` — the lazy decode
/// loop without unknown-field capture, and an all-scalar lazy struct whose
/// lifetime is anchored by `PhantomData`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod lazyviewslean {
    buffa::include_proto!("test.lazyviewslean");
}

// unbox_oneof: `Envelope.body.small` is stored inline (opted out of Box),
// `large` stays boxed. Compiling this module exercises every boxing site for
// both shapes; runtime round-trips live in `tests/unbox_oneof.rs`.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod unbox_oneof {
    buffa::include_proto!("unboxoneof");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross {
    buffa::include_proto!("test.cross");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross_syntax {
    buffa::include_proto!("test.cross_syntax");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod cross_pertype {
    buffa::include_proto!("test.cross_pertype");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod collisions {
    buffa::include_proto!("test.collisions");
}

#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod prelude_shadow {
    buffa::include_proto!("test.prelude_shadow");
}

// Nested-package pair, wrapped exactly the way `buffa-build`'s
// `_include.rs` would. The chain of `use super::*;` glob imports makes the
// outer package's `__buffa` reachable from `inner`'s scope, which is the
// only consumer layout where a bare `pub use __buffa::…;` import path is
// E0659-ambiguous against the locally-`include!`d `__buffa`. The natural
// re-exports must use `self::__buffa::…` / `super::__buffa::…` to compile
// here — see gh#80. Compilation is the assertion (`tests/nestpkg.rs` adds a
// type-resolution sanity check).
#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod nestpkg {
    #[allow(unused_imports)]
    use super::*;
    buffa::include_proto!("test.nestpkg");
    pub mod inner {
        #[allow(unused_imports)]
        use super::*;
        buffa::include_proto!("test.nestpkg.inner");
    }
}

// Issue #135: message-nesting module vs sub-package module collision. The
// sub-package `modcollide.oof` is nested under `modcollide` as `pub mod oof`;
// `message Oof`'s nested-types module is deconflicted to `oof_`, so the two no
// longer redefine `mod oof`. Compiling this module is the regression guard.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod modcollide {
    buffa::include_proto!("modcollide");
    pub mod oof {
        buffa::include_proto!("modcollide.oof");
    }
}

// Issue #135, multi-message race: `Oof`/`Oof_` nested modules deconflict to
// `oof__`/`oof___` while sub-packages `oof`/`oof_` keep their names. Compiling
// this nested layout proves the four modules coexist.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod modrace {
    buffa::include_proto!("modrace");
    pub mod oof {
        buffa::include_proto!("modrace.oof");
    }
    pub mod oof_ {
        buffa::include_proto!("modrace.oof_");
    }
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod proto2 {
    buffa::include_proto!("test.proto2");
}

// Mixed-mode reflection fixtures: bridge-mode dependency, vtable-mode parent
// referencing it via extern_path. See tests/reflect_mixed_mode.rs.
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod mixed_reflect_dep {
    buffa::include_proto!("mixedref.dep");
}
#[allow(clippy::derivable_impls, clippy::match_single_binding)]
pub mod mixed_reflect_parent {
    buffa::include_proto!("mixedref.parent");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod json_types {
    buffa::include_proto!("test.json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod view_json {
    buffa::include_proto!("test.viewjson");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod view_json_p2 {
    buffa::include_proto!("test.viewjson.p2");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod p2json {
    buffa::include_proto!("test.p2json");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod utf8test {
    buffa::include_proto!("utf8test");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::wildcard_in_or_patterns,
    non_camel_case_types,
    dead_code
)]
pub mod edenumjson {
    buffa::include_proto!("test.edenumjson");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod edge {
    buffa::include_proto!("test.edge");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod custopts {
    buffa::include_proto!("buffa.test.options");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod extjson {
    buffa::include_proto!("buffa.test.extjson");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod groupext {
    buffa::include_proto!("buffa.test.groupext");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod msgset {
    buffa::include_proto!("buffa.test.messageset");
}

#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types
)]
pub mod with_setters {
    buffa::include_proto!("test.setters");
}

#[cfg(has_edition_2024)]
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod ed2024 {
    buffa::include_proto!("test.ed2024");
}

// Idiomatic imports (file_per_package): package-root references emitted as
// `use`-backed short names. Compiling this module IS the primary test — the
// `use` directives must resolve, every import must be referenced, and no
// short name may shadow what sibling emissions reference bare. The index
// file reproduces the test::idiomatic / test::idiomatic_other sibling
// nesting the generated `super::` chains assume.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod idiomatic {
    include!(concat!(env!("OUT_DIR"), "/idiomatic_variant/_include.rs"));
}

// Regression: use_bytes_type() previously produced uncompilable decode code.
// Compiling this module IS the test — if merge_bytes/decode_bytes mismatch
// the bytes::Bytes field type, the build fails.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod basic_bytes {
    include!(concat!(env!("OUT_DIR"), "/bytes_variant/basic.mod.rs"));
}

// type_name_prefix (#46): basic.proto compiled with `.type_name_prefix("Rpc")`
// — every generated type is `Rpc*` (RpcPerson, RpcStatus, RpcPersonView, ...)
// while module names and the wire format stay unchanged. Compilation plus the
// runtime checks in `tests/type_prefix.rs` are the assertion.
// `clippy::manual_map`: the lazy-view oneof `to_owned` conversion always
// emits the match form (unlike the eager view, which uses `.map()` for
// scalar-only groups); basic.proto's bytes/string `choice` oneof is the
// first lazy-compiled proto to hit it. Generator follow-up tracked
// separately.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::manual_map,
    dead_code
)]
pub mod basic_prefixed {
    include!(concat!(env!("OUT_DIR"), "/prefix_variant/basic.mod.rs"));
}

// type_name_prefix + nested messages: nested_deep.proto compiled with
// `.type_name_prefix("Rpc")` and lazy views — the nested view / owned-view /
// lazy-view re-exports must reference the prefixed type names. Compile-only;
// no runtime tests.
#[allow(clippy::derivable_impls, clippy::match_single_binding, dead_code)]
pub mod nested_prefixed {
    include!(concat!(
        env!("OUT_DIR"),
        "/prefix_nested_variant/test.nested.mod.rs"
    ));
}

// Carve-out (#76): utf8_validation.proto with a NONE-keyed `map<string, bytes>`,
// compiled with strict_utf8_mapping() + use_bytes_type(). The effective
// `map<bytes, bytes>` keeps `Vec<u8>` values; runtime checks live in
// `tests/bytes_type.rs`.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod utf8_bytes {
    include!(concat!(
        env!("OUT_DIR"),
        "/utf8_bytes_variant/utf8test.mod.rs"
    ));
}

// Regression #88: bytes_fields + generate_arbitrary(true). Compilation is the
// primary assertion — all four bytes field shapes (singular, optional,
// repeated, oneof variant) must compile with the arbitrary shims in place.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod basic_arbitrary_bytes {
    include!(concat!(env!("OUT_DIR"), "/arbitrary_bytes/basic.mod.rs"));
}

// Configurable string_type: SmolStr default + CompactString/EcoString
// overrides, generate_json + arbitrary. Compiling this module exercises every
// string code path against the real crates; runtime checks live in
// `tests/string_type.rs`.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod string_types {
    include!(concat!(
        env!("OUT_DIR"),
        "/string_variant/stringtypes.mod.rs"
    ));
}

// proto2 `[default = "..."]` + string_type. Compiling this verifies the
// generated Default impl and clear() build the literal via the configured
// repr's From<String>.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod string_proto2 {
    include!(concat!(
        env!("OUT_DIR"),
        "/string_proto2_variant/stringproto2.mod.rs"
    ));
}

// Views + preserve_unknown_fields=false: covers the else-branches in view
// codegen that omit the unknown-fields view field. Compilation IS the test.
#[allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    non_camel_case_types,
    dead_code
)]
pub mod basic_no_uf {
    include!(concat!(env!("OUT_DIR"), "/no_unknown_views/basic.mod.rs"));
}

// These tests intentionally use the field-assignment style
// (`let mut m = T::default(); m.f = v;`) because it mirrors how protobuf
// messages are constructed in other languages and is what the docs show.
// `3.14` is a test value, not an attempt at PI.
#[allow(
    clippy::field_reassign_with_default,
    clippy::approx_constant,
    clippy::unnecessary_to_owned,
    clippy::assertions_on_constants
)]
#[cfg(test)]
mod tests;

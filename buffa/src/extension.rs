//! Typed access to protobuf extensions stored in unknown fields.
//!
//! Custom options — `(buf.validate.field)`, `(google.api.http)`, etc. — are
//! declared with `extend google.protobuf.FieldOptions { ... }` and attached in
//! proto source as `[(my.option) = {...}]`. Editions removed general-purpose
//! message extensions but **custom options remain the sanctioned use of
//! `extend`**: `descriptor.proto` still declares `extensions 1000 to max;` on
//! every `*Options` message.
//!
//! On the wire, extension values are ordinary fields at their declared field
//! numbers. When an extendee is decoded without the extension in its schema,
//! those fields land in [`UnknownFields`] — already preserved. This module
//! provides a typed extraction/injection path on top of that storage.
//!
//! # Usage
//!
//! Codegen emits a `pub const` for each `extend` declaration:
//!
//! ```ignore
//! // Generated from: extend google.protobuf.FieldOptions { optional FieldRules field = 1159; }
//! pub const FIELD: buffa::Extension<buffa::extension::codecs::MessageCodec<FieldRules>>
//!     = buffa::Extension::new(1159, "google.protobuf.FieldOptions");
//! ```
//!
//! The extendee (any message with unknown-field preservation) implements
//! [`ExtensionSet`]:
//!
//! ```ignore
//! use buffa::ExtensionSet;
//! let rules: Option<FieldRules> = field.options.extension(&buf_validate::FIELD);
//! opts.set_extension(&buf_validate::FIELD, my_rules);
//! ```
//!
//! # Panics
//!
//! [`extension`], [`set_extension`], and [`clear_extension`] panic if the
//! descriptor's [`extendee`](Extension::extendee) does not match the extendee
//! message's [`PROTO_FQN`](ExtensionSet::PROTO_FQN). This catches bugs like
//! `field_options.extension(&MESSAGE_LEVEL_OPTION)` at the first call site
//! (matches protobuf-go, which panics, and protobuf-es, which throws).
//! [`has_extension`] returns `false` gracefully on mismatch — "is this
//! extension set here" when it can't extend here has a legitimate answer.
//!
//! # Defaults
//!
//! Proto2 `[default = ...]` values are surfaced by [`extension_or_default`],
//! which returns the declared default when the extension is absent. Presence
//! is still distinguishable via [`extension`] (returns `None`) or
//! [`has_extension`].
//!
//! # JSON
//!
//! Extension fields serialize as `"[pkg.ext]"` keys in proto3 JSON. This
//! requires a populated [`TypeRegistry`](crate::type_registry::TypeRegistry)
//! — see the [`type_registry`](crate::type_registry) module. Without a
//! registry, extension bytes stay in `__buffa_unknown_fields` and are silently
//! dropped from JSON output.
//!
//! # Presence semantics
//!
//! Extensions **always** have explicit field presence, regardless of proto3 or
//! editions `IMPLICIT` file defaults (see protocolbuffers/protobuf#8234). This
//! falls out naturally from unknown-field storage: `set_extension(&ext, 0)`
//! pushes a record with value 0, so `has_extension` returns true. Singular
//! `Output` is therefore `Option<T>`, not `T` with a sentinel.
//!
//! [`extension`]: ExtensionSet::extension
//! [`set_extension`]: ExtensionSet::set_extension
//! [`clear_extension`]: ExtensionSet::clear_extension
//! [`has_extension`]: ExtensionSet::has_extension
//! [`extension_or_default`]: ExtensionSet::extension_or_default

use core::marker::PhantomData;

use crate::unknown_fields::UnknownFields;

/// Typed extension descriptor.
///
/// Emitted by codegen as a `pub const` for each `extend` declaration. The
/// type parameter `C` is an [`ExtensionCodec`] — a zero-sized codec struct
/// that encodes the proto field type. Users don't name `C` directly; it flows
/// through inference from the `const` to the [`ExtensionSet`] method call.
///
/// For proto2 extensions declared with `[default = ...]`, codegen stores a
/// function pointer that lazily produces the default value. See
/// [`Extension::with_default`] and [`ExtensionSet::extension_or_default`].
#[derive(Debug)]
pub struct Extension<C: ExtensionCodec> {
    number: u32,
    /// Fully-qualified proto name of the extendee (e.g.
    /// `"google.protobuf.FieldOptions"`), no leading dot.
    ///
    /// Checked against [`ExtensionSet::PROTO_FQN`] on `extension()`,
    /// `set_extension()`, and `clear_extension()` — a mismatch panics.
    /// `has_extension()` stays graceful (returns `false`), matching
    /// protobuf-go and protobuf-es precedent.
    extendee: &'static str,
    default: Option<fn() -> C::Value>,
    _codec: PhantomData<fn() -> C>,
}

impl<C: ExtensionCodec> Extension<C> {
    /// Construct an extension descriptor for the given field number and
    /// extendee message.
    ///
    /// `const fn` so that generated `pub const` items can use it.
    ///
    /// `extendee` is the fully-qualified proto type name (no leading dot) of
    /// the message this extension extends — e.g. `"google.protobuf.FieldOptions"`.
    /// Passing an extension with a mismatched extendee to `extension()` /
    /// `set_extension()` / `clear_extension()` will panic.
    ///
    /// Field number `0` is invalid in protobuf. Codegen never emits it;
    /// a descriptor constructed with `0` will never match valid wire data.
    pub const fn new(number: u32, extendee: &'static str) -> Self {
        Self {
            number,
            extendee,
            default: None,
            _codec: PhantomData,
        }
    }

    /// Construct an extension descriptor with a proto2 `[default = ...]` value.
    ///
    /// The default is returned by [`ExtensionSet::extension_or_default`] when
    /// the extension is absent. [`ExtensionSet::extension`] continues to return
    /// `None` when absent — this is additive, not a semantic change.
    ///
    /// The function pointer is called lazily on each `extension_or_default`
    /// call. For `Copy` scalars codegen emits a `const fn`; for `String` and
    /// `bytes` a regular `fn` (allocates on each call — same cost as a
    /// hand-written `.unwrap_or_else(|| "x".into())`).
    pub const fn with_default(
        number: u32,
        extendee: &'static str,
        default: fn() -> C::Value,
    ) -> Self {
        Self {
            number,
            extendee,
            default: Some(default),
            _codec: PhantomData,
        }
    }

    /// The extension's field number on the extendee.
    pub const fn number(&self) -> u32 {
        self.number
    }

    /// The fully-qualified proto name of the extendee message.
    pub const fn extendee(&self) -> &'static str {
        self.extendee
    }
}

/// Asserts that `ext` actually extends `Self`. Called from `extension()`,
/// `set_extension()`, and `clear_extension()` (but NOT `has_extension()` —
/// asking "is this extension set here" has a legitimate answer of `false`
/// when it can't extend here, matching protobuf-go and protobuf-es).
///
/// `#[track_caller]` so the panic points at the user's call site, not here.
#[track_caller]
#[inline]
fn assert_extendee<C: ExtensionCodec>(ext: &Extension<C>, expected: &'static str) {
    assert_eq!(
        ext.extendee, expected,
        "extension at field {} extends `{}`, not `{}`",
        ext.number, ext.extendee, expected
    );
}

// Manual impls avoid a `C: Clone` / `C: Copy` bound that `#[derive]` would add.
// `Option<fn() -> T>` is `Copy` for any `T` (fn pointers are always `Copy`),
// so the struct remains `Copy` regardless of the codec's `Value` type.
impl<C: ExtensionCodec> Clone for Extension<C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C: ExtensionCodec> Copy for Extension<C> {}

/// One impl per proto field type. `Value` is what users pass to `set`;
/// `Output` is what `get` returns (`Option<Value>` for singular,
/// `Vec<Elem>` for repeated).
pub trait ExtensionCodec {
    /// The value type passed to [`ExtensionSet::set_extension`].
    type Value;
    /// The return type of [`ExtensionSet::extension`].
    type Output;

    /// Decode this extension's value from the extendee's unknown fields.
    fn decode(number: u32, fields: &UnknownFields) -> Self::Output;

    /// Encode `value` into the extendee's unknown fields.
    ///
    /// The caller is responsible for clearing any prior occurrences (see
    /// [`ExtensionSet::set_extension`]).
    fn encode(number: u32, value: Self::Value, fields: &mut UnknownFields);
}

/// Implemented by codegen on every message that preserves unknown fields.
///
/// The only required methods are the two `unknown_fields*` accessors; all
/// extension operations are defaulted on top of them. The generated impl is
/// therefore a two-line body.
///
/// Not object-safe (the default methods are generic over `C`). Extension
/// access is always statically dispatched from a `const` descriptor.
pub trait ExtensionSet {
    /// Fully-qualified proto type name of this message (no leading dot),
    /// e.g. `"google.protobuf.FieldOptions"`.
    ///
    /// Checked against [`Extension::extendee`] on every `extension()`,
    /// `set_extension()`, and `clear_extension()` call. A mismatch panics:
    /// passing an extension for the wrong message is a bug in the caller.
    const PROTO_FQN: &'static str;

    /// Immutable access to the extendee's unknown-field storage.
    fn unknown_fields(&self) -> &UnknownFields;
    /// Mutable access to the extendee's unknown-field storage.
    fn unknown_fields_mut(&mut self) -> &mut UnknownFields;

    /// Read an extension value.
    ///
    /// For singular extensions: `Option<T>` — `None` if absent or if the
    /// stored wire data is malformed for this codec. For repeated: `Vec<T>`.
    ///
    /// # Panics
    ///
    /// Panics if `ext.extendee()` does not match `Self::PROTO_FQN`.
    #[track_caller]
    fn extension<C: ExtensionCodec>(&self, ext: &Extension<C>) -> C::Output {
        assert_extendee(ext, Self::PROTO_FQN);
        C::decode(ext.number, self.unknown_fields())
    }

    /// Write an extension value, replacing any prior occurrences.
    ///
    /// # Panics
    ///
    /// Panics if `ext.extendee()` does not match `Self::PROTO_FQN`.
    #[track_caller]
    fn set_extension<C: ExtensionCodec>(&mut self, ext: &Extension<C>, value: C::Value) {
        assert_extendee(ext, Self::PROTO_FQN);
        self.unknown_fields_mut().retain(|f| f.number != ext.number);
        C::encode(ext.number, value, self.unknown_fields_mut());
    }

    /// Returns `true` if any record at the extension's field number is present.
    ///
    /// Does not check wire-type validity — this is a fast presence test, not
    /// a decode. Also does not check extendee identity: asking "is this
    /// extension set here" when it can't extend here has a legitimate answer
    /// (`false`). This matches protobuf-go's `HasExtension` and protobuf-es's
    /// `hasExtension`, both of which return `false` gracefully on mismatch.
    fn has_extension<C: ExtensionCodec>(&self, ext: &Extension<C>) -> bool {
        if ext.extendee != Self::PROTO_FQN {
            return false;
        }
        self.unknown_fields().iter().any(|f| f.number == ext.number)
    }

    /// Remove all records at the extension's field number.
    ///
    /// # Panics
    ///
    /// Panics if `ext.extendee()` does not match `Self::PROTO_FQN`.
    #[track_caller]
    fn clear_extension<C: ExtensionCodec>(&mut self, ext: &Extension<C>) {
        assert_extendee(ext, Self::PROTO_FQN);
        self.unknown_fields_mut().retain(|f| f.number != ext.number);
    }

    /// Read a singular extension value, returning the proto2 `[default = ...]`
    /// value if absent, or the type's `Default` if no proto default was declared.
    ///
    /// Only meaningful for singular codecs (`Output = Option<Value>`). For
    /// repeated codecs use [`extension`](Self::extension) — there is no such
    /// thing as a repeated default.
    ///
    /// Presence is still distinguishable via [`extension`](Self::extension)
    /// (returns `None`) or [`has_extension`](Self::has_extension).
    ///
    /// # Panics
    ///
    /// Panics if `ext.extendee()` does not match `Self::PROTO_FQN`
    /// (transitively, via the inner `extension()` call).
    #[must_use]
    #[track_caller]
    fn extension_or_default<C>(&self, ext: &Extension<C>) -> C::Value
    where
        C: ExtensionCodec<Output = Option<<C as ExtensionCodec>::Value>>,
        C::Value: Default,
    {
        self.extension(ext)
            .or_else(|| ext.default.map(|f| f()))
            .unwrap_or_default()
    }
}

/// Codec types — one per proto field type.
///
/// These live in a submodule so they don't clutter the crate root. Generated
/// consts mention the codec type (`Extension<codecs::MessageCodec<FieldRules>>`)
/// but users never type it out — they just `opts.extension(&FIELD)` and
/// inference handles the rest.
pub mod codecs {
    use core::marker::PhantomData;

    use alloc::string::String;
    use alloc::vec::Vec;
    use bytes::Buf;

    use crate::encoding::{decode_varint, encode_varint};
    use crate::message::Message;
    use crate::types::{
        zigzag_decode_i32, zigzag_decode_i64, zigzag_encode_i32, zigzag_encode_i64,
    };
    use crate::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};

    use super::ExtensionCodec;

    /// Codecs that can appear as elements of [`Repeated`] / [`PackedRepeated`].
    ///
    /// Separated from [`ExtensionCodec`] because the singular decode semantics
    /// differ by type: scalars use last-wins, but singular messages merge all
    /// records. This trait provides the per-record primitives; each codec
    /// assembles them into its own [`ExtensionCodec::decode`].
    pub trait SingularCodec {
        type Value;

        /// Decode one value from one unknown-field record's data.
        ///
        /// Returns `None` on wire-type mismatch (and, for string/message,
        /// on malformed payload).
        fn decode_one(data: &UnknownFieldData) -> Option<Self::Value>;

        /// Decode all values from a packed `LengthDelimited` payload.
        ///
        /// Non-packable types (string, bytes, message) **must leave `out`
        /// unmodified**: `decode_repeated` falls through to this method when
        /// [`decode_one`](Self::decode_one) returns `None` for a
        /// `LengthDelimited` record, relying on the no-op to skip malformed
        /// elements. A `LengthDelimited` record at their field number IS the
        /// single value — there is no packed form.
        fn decode_packed(bytes: &[u8], out: &mut Vec<Self::Value>);

        /// Encode one value as a single unknown-field record.
        fn encode_one(value: &Self::Value) -> UnknownFieldData;
    }

    /// Codecs whose elements can be concatenated into a packed wire form.
    ///
    /// Only varint-family and fixed-width scalars are packable. String, bytes,
    /// and message use `LengthDelimited` for a single value, so there's no
    /// distinct packed encoding.
    pub trait PackableCodec: SingularCodec {
        /// Write one value's wire payload (without tag) into `buf`.
        fn encode_packed(value: &Self::Value, buf: &mut Vec<u8>);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Varint-family scalar codecs
    // ─────────────────────────────────────────────────────────────────────

    macro_rules! varint_codec {
        ($name:ident, $ty:ty, |$d:ident| $decode:expr, |$e:ident| $encode:expr $(,)?) => {
            #[doc = concat!("Codec for the `", stringify!($name), "` proto scalar type.")]
            pub struct $name;

            impl ExtensionCodec for $name {
                type Value = $ty;
                type Output = Option<$ty>;
                fn decode(number: u32, fields: &UnknownFields) -> Option<$ty> {
                    fields
                        .iter()
                        .rev()
                        .filter(|f| f.number == number)
                        .find_map(|f| Self::decode_one(&f.data))
                }
                fn encode(number: u32, value: $ty, fields: &mut UnknownFields) {
                    fields.push(UnknownField {
                        number,
                        data: Self::encode_one(&value),
                    });
                }
            }

            impl SingularCodec for $name {
                type Value = $ty;
                fn decode_one(data: &UnknownFieldData) -> Option<$ty> {
                    match *data {
                        UnknownFieldData::Varint($d) => Some($decode),
                        _ => None,
                    }
                }
                fn decode_packed(bytes: &[u8], out: &mut Vec<$ty>) {
                    let mut buf = bytes;
                    while buf.has_remaining() {
                        match decode_varint(&mut buf) {
                            Ok($d) => out.push($decode),
                            Err(_) => return,
                        }
                    }
                }
                fn encode_one($e: &$ty) -> UnknownFieldData {
                    let $e = *$e;
                    UnknownFieldData::Varint($encode)
                }
            }

            impl PackableCodec for $name {
                fn encode_packed($e: &$ty, buf: &mut Vec<u8>) {
                    let $e = *$e;
                    encode_varint($encode, buf);
                }
            }
        };
    }

    // `int32` negative values are sign-extended to 64 bits before varint
    // encoding (matching protoc's behavior — a negative int32 is 10 bytes on
    // the wire). `sint32` uses zigzag instead.
    varint_codec!(Int32, i32, |v| v as i32, |v| v as i64 as u64);
    varint_codec!(Int64, i64, |v| v as i64, |v| v as u64);
    varint_codec!(Uint32, u32, |v| v as u32, |v| v as u64);
    varint_codec!(Uint64, u64, |v| v, |v| v);
    varint_codec!(
        Sint32,
        i32,
        |v| zigzag_decode_i32(v as u32),
        |v| zigzag_encode_i32(v) as u64
    );
    varint_codec!(
        Sint64,
        i64,
        |v| zigzag_decode_i64(v),
        |v| zigzag_encode_i64(v)
    );
    varint_codec!(Bool, bool, |v| v != 0, |v| v as u64);
    /// Codec for proto `enum` extension fields.
    ///
    /// `Value = i32`; cast via `EnumValue::from_i32` at the call site.
    /// A type-directed enum codec (emitting `Extension<MyEnum>`) is deferred.
    pub struct EnumI32;
    // EnumI32 is wire-identical to Int32 — delegate rather than re-expand.
    impl ExtensionCodec for EnumI32 {
        type Value = i32;
        type Output = Option<i32>;
        fn decode(number: u32, fields: &UnknownFields) -> Option<i32> {
            Int32::decode(number, fields)
        }
        fn encode(number: u32, value: i32, fields: &mut UnknownFields) {
            Int32::encode(number, value, fields)
        }
    }
    impl SingularCodec for EnumI32 {
        type Value = i32;
        fn decode_one(data: &UnknownFieldData) -> Option<i32> {
            Int32::decode_one(data)
        }
        fn decode_packed(bytes: &[u8], out: &mut Vec<i32>) {
            Int32::decode_packed(bytes, out)
        }
        fn encode_one(value: &i32) -> UnknownFieldData {
            Int32::encode_one(value)
        }
    }
    impl PackableCodec for EnumI32 {
        fn encode_packed(value: &i32, buf: &mut Vec<u8>) {
            Int32::encode_packed(value, buf)
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Fixed-width scalar codecs
    // ─────────────────────────────────────────────────────────────────────

    macro_rules! fixed32_codec {
        ($name:ident, $ty:ty, |$d:ident| $decode:expr, |$e:ident| $encode:expr $(,)?) => {
            #[doc = concat!("Codec for the `", stringify!($name), "` proto scalar type.")]
            pub struct $name;

            impl ExtensionCodec for $name {
                type Value = $ty;
                type Output = Option<$ty>;
                fn decode(number: u32, fields: &UnknownFields) -> Option<$ty> {
                    fields
                        .iter()
                        .rev()
                        .filter(|f| f.number == number)
                        .find_map(|f| Self::decode_one(&f.data))
                }
                fn encode(number: u32, value: $ty, fields: &mut UnknownFields) {
                    fields.push(UnknownField {
                        number,
                        data: Self::encode_one(&value),
                    });
                }
            }

            impl SingularCodec for $name {
                type Value = $ty;
                fn decode_one(data: &UnknownFieldData) -> Option<$ty> {
                    match *data {
                        UnknownFieldData::Fixed32($d) => Some($decode),
                        _ => None,
                    }
                }
                fn decode_packed(bytes: &[u8], out: &mut Vec<$ty>) {
                    let mut buf = bytes;
                    while buf.remaining() >= 4 {
                        let $d = buf.get_u32_le();
                        out.push($decode);
                    }
                }
                fn encode_one($e: &$ty) -> UnknownFieldData {
                    let $e = *$e;
                    UnknownFieldData::Fixed32($encode)
                }
            }

            impl PackableCodec for $name {
                fn encode_packed($e: &$ty, buf: &mut Vec<u8>) {
                    use bytes::BufMut;
                    let $e = *$e;
                    buf.put_u32_le($encode);
                }
            }
        };
    }

    macro_rules! fixed64_codec {
        ($name:ident, $ty:ty, |$d:ident| $decode:expr, |$e:ident| $encode:expr $(,)?) => {
            #[doc = concat!("Codec for the `", stringify!($name), "` proto scalar type.")]
            pub struct $name;

            impl ExtensionCodec for $name {
                type Value = $ty;
                type Output = Option<$ty>;
                fn decode(number: u32, fields: &UnknownFields) -> Option<$ty> {
                    fields
                        .iter()
                        .rev()
                        .filter(|f| f.number == number)
                        .find_map(|f| Self::decode_one(&f.data))
                }
                fn encode(number: u32, value: $ty, fields: &mut UnknownFields) {
                    fields.push(UnknownField {
                        number,
                        data: Self::encode_one(&value),
                    });
                }
            }

            impl SingularCodec for $name {
                type Value = $ty;
                fn decode_one(data: &UnknownFieldData) -> Option<$ty> {
                    match *data {
                        UnknownFieldData::Fixed64($d) => Some($decode),
                        _ => None,
                    }
                }
                fn decode_packed(bytes: &[u8], out: &mut Vec<$ty>) {
                    let mut buf = bytes;
                    while buf.remaining() >= 8 {
                        let $d = buf.get_u64_le();
                        out.push($decode);
                    }
                }
                fn encode_one($e: &$ty) -> UnknownFieldData {
                    let $e = *$e;
                    UnknownFieldData::Fixed64($encode)
                }
            }

            impl PackableCodec for $name {
                fn encode_packed($e: &$ty, buf: &mut Vec<u8>) {
                    use bytes::BufMut;
                    let $e = *$e;
                    buf.put_u64_le($encode);
                }
            }
        };
    }

    fixed32_codec!(Fixed32, u32, |v| v, |v| v);
    fixed32_codec!(Sfixed32, i32, |v| v as i32, |v| v as u32);
    fixed32_codec!(Float, f32, |v| f32::from_bits(v), |v| v.to_bits());
    fixed64_codec!(Fixed64, u64, |v| v, |v| v);
    fixed64_codec!(Sfixed64, i64, |v| v as i64, |v| v as u64);
    fixed64_codec!(Double, f64, |v| f64::from_bits(v), |v| v.to_bits());

    // ─────────────────────────────────────────────────────────────────────
    // String / bytes / message codecs (hand-written, not packable)
    // ─────────────────────────────────────────────────────────────────────

    /// Codec for the `string` proto type.
    pub struct StringCodec;

    impl ExtensionCodec for StringCodec {
        type Value = String;
        type Output = Option<String>;
        fn decode(number: u32, fields: &UnknownFields) -> Option<String> {
            fields
                .iter()
                .rev()
                .filter(|f| f.number == number)
                .find_map(|f| Self::decode_one(&f.data))
        }
        fn encode(number: u32, value: String, fields: &mut UnknownFields) {
            fields.push(UnknownField {
                number,
                data: UnknownFieldData::LengthDelimited(value.into_bytes()),
            });
        }
    }

    impl SingularCodec for StringCodec {
        type Value = String;
        fn decode_one(data: &UnknownFieldData) -> Option<String> {
            match data {
                UnknownFieldData::LengthDelimited(bytes) => String::from_utf8(bytes.clone()).ok(),
                _ => None,
            }
        }
        fn decode_packed(_bytes: &[u8], _out: &mut Vec<String>) {}
        fn encode_one(value: &String) -> UnknownFieldData {
            UnknownFieldData::LengthDelimited(value.clone().into_bytes())
        }
    }

    /// Codec for the `bytes` proto type.
    pub struct BytesCodec;

    impl ExtensionCodec for BytesCodec {
        type Value = Vec<u8>;
        type Output = Option<Vec<u8>>;
        fn decode(number: u32, fields: &UnknownFields) -> Option<Vec<u8>> {
            fields
                .iter()
                .rev()
                .filter(|f| f.number == number)
                .find_map(|f| Self::decode_one(&f.data))
        }
        fn encode(number: u32, value: Vec<u8>, fields: &mut UnknownFields) {
            fields.push(UnknownField {
                number,
                data: UnknownFieldData::LengthDelimited(value),
            });
        }
    }

    impl SingularCodec for BytesCodec {
        type Value = Vec<u8>;
        fn decode_one(data: &UnknownFieldData) -> Option<Vec<u8>> {
            match data {
                UnknownFieldData::LengthDelimited(bytes) => Some(bytes.clone()),
                _ => None,
            }
        }
        fn decode_packed(_bytes: &[u8], _out: &mut Vec<Vec<u8>>) {}
        fn encode_one(value: &Vec<u8>) -> UnknownFieldData {
            UnknownFieldData::LengthDelimited(value.clone())
        }
    }

    /// Codec for message-typed extension fields.
    ///
    /// Singular decode **merges all** `LengthDelimited` records at the field
    /// number (proto spec: split messages merge). Any malformed record aborts
    /// the merge and returns `None`.
    pub struct MessageCodec<M>(PhantomData<fn() -> M>);

    impl<M: Message + Default> ExtensionCodec for MessageCodec<M> {
        type Value = M;
        type Output = Option<M>;
        fn decode(number: u32, fields: &UnknownFields) -> Option<M> {
            let mut msg: Option<M> = None;
            for f in fields.iter().filter(|f| f.number == number) {
                if let UnknownFieldData::LengthDelimited(bytes) = &f.data {
                    let m = msg.get_or_insert_with(M::default);
                    if m.merge_from_slice(bytes).is_err() {
                        return None;
                    }
                }
            }
            msg
        }
        fn encode(number: u32, value: M, fields: &mut UnknownFields) {
            fields.push(UnknownField {
                number,
                data: UnknownFieldData::LengthDelimited(value.encode_to_vec()),
            });
        }
    }

    impl<M: Message + Default> SingularCodec for MessageCodec<M> {
        type Value = M;
        fn decode_one(data: &UnknownFieldData) -> Option<M> {
            match data {
                UnknownFieldData::LengthDelimited(bytes) => {
                    let mut m = M::default();
                    m.merge_from_slice(bytes).ok()?;
                    Some(m)
                }
                _ => None,
            }
        }
        fn decode_packed(_bytes: &[u8], _out: &mut Vec<M>) {}
        fn encode_one(value: &M) -> UnknownFieldData {
            UnknownFieldData::LengthDelimited(value.encode_to_vec())
        }
    }

    /// Codec for group-encoded message extension fields (proto2 `group` syntax
    /// or editions `features.message_encoding = DELIMITED`).
    ///
    /// The wire data is [`UnknownFieldData::Group`] — the inner fields are
    /// already parsed into an [`UnknownFields`] sub-tree by the group decoder.
    /// Decode re-serializes the inner fields to a temporary buffer then merges
    /// into `M`; encode does the reverse via [`UnknownFields::decode_from_slice`].
    ///
    /// The round-trip through bytes is correct but not optimal. The alternative
    /// (a `Message::write_to_unknown_fields()` trait method) would be invasive
    /// for a rare-in-practice path: group-encoded extensions don't appear in
    /// real custom options.
    ///
    /// Singular decode **merges all** `Group` records at the field number
    /// (same as [`MessageCodec`]). Any malformed record aborts and returns
    /// `None`.
    pub struct GroupCodec<M>(PhantomData<fn() -> M>);

    impl<M: Message + Default> ExtensionCodec for GroupCodec<M> {
        type Value = M;
        type Output = Option<M>;
        fn decode(number: u32, fields: &UnknownFields) -> Option<M> {
            let mut msg: Option<M> = None;
            for f in fields.iter().filter(|f| f.number == number) {
                if let UnknownFieldData::Group(inner) = &f.data {
                    let m = msg.get_or_insert_with(M::default);
                    let mut buf = Vec::with_capacity(inner.encoded_len());
                    inner.write_to(&mut buf);
                    if m.merge_from_slice(&buf).is_err() {
                        return None;
                    }
                }
            }
            msg
        }
        fn encode(number: u32, value: M, fields: &mut UnknownFields) {
            let bytes = value.encode_to_vec();
            // We just encoded `value` — re-decoding its bytes cannot fail
            // unless there's a bug in the Message encoder itself.
            let inner = UnknownFields::decode_from_slice(&bytes)
                .expect("BUG: re-decoding freshly-encoded message bytes failed");
            fields.push(UnknownField {
                number,
                data: UnknownFieldData::Group(inner),
            });
        }
    }

    impl<M: Message + Default> SingularCodec for GroupCodec<M> {
        type Value = M;
        fn decode_one(data: &UnknownFieldData) -> Option<M> {
            match data {
                UnknownFieldData::Group(inner) => {
                    let mut buf = Vec::with_capacity(inner.encoded_len());
                    inner.write_to(&mut buf);
                    let mut m = M::default();
                    m.merge_from_slice(&buf).ok()?;
                    Some(m)
                }
                _ => None,
            }
        }
        // Groups are never packed — leave `out` unmodified (see
        // `SingularCodec::decode_packed` contract).
        fn decode_packed(_bytes: &[u8], _out: &mut Vec<M>) {}
        fn encode_one(value: &M) -> UnknownFieldData {
            let bytes = value.encode_to_vec();
            // We just encoded `value` — re-decoding cannot fail.
            let inner = UnknownFields::decode_from_slice(&bytes)
                .expect("BUG: re-decoding freshly-encoded message bytes failed");
            UnknownFieldData::Group(inner)
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Repeated wrappers
    // ─────────────────────────────────────────────────────────────────────

    /// Codec for `repeated T` extension fields, unpacked wire encoding.
    ///
    /// Decode accepts both packed and unpacked wire forms (proto spec
    /// requires this). Encode emits one record per element — this is the
    /// proto2 default for extensions and what `*Options` extendees expect.
    pub struct Repeated<C>(PhantomData<fn() -> C>);

    impl<C: SingularCodec> ExtensionCodec for Repeated<C> {
        type Value = Vec<C::Value>;
        type Output = Vec<C::Value>;
        fn decode(number: u32, fields: &UnknownFields) -> Vec<C::Value> {
            decode_repeated::<C>(number, fields)
        }
        fn encode(number: u32, value: Vec<C::Value>, fields: &mut UnknownFields) {
            for v in &value {
                fields.push(UnknownField {
                    number,
                    data: C::encode_one(v),
                });
            }
        }
    }

    /// Codec for `repeated T` extension fields, packed wire encoding.
    ///
    /// Decode accepts both packed and unpacked wire forms (identical to
    /// [`Repeated`]). Encode emits a single `LengthDelimited` record
    /// containing concatenated wire values. Only valid for packable scalar
    /// element types (varint / fixed families).
    ///
    /// Codegen picks this over [`Repeated`] when `field.options.packed ==
    /// Some(true)` or the resolved edition feature `repeated_field_encoding
    /// == PACKED`.
    pub struct PackedRepeated<C>(PhantomData<fn() -> C>);

    impl<C: PackableCodec> ExtensionCodec for PackedRepeated<C> {
        type Value = Vec<C::Value>;
        type Output = Vec<C::Value>;
        fn decode(number: u32, fields: &UnknownFields) -> Vec<C::Value> {
            decode_repeated::<C>(number, fields)
        }
        fn encode(number: u32, value: Vec<C::Value>, fields: &mut UnknownFields) {
            if value.is_empty() {
                return;
            }
            let mut buf = Vec::new();
            for v in &value {
                C::encode_packed(v, &mut buf);
            }
            fields.push(UnknownField {
                number,
                data: UnknownFieldData::LengthDelimited(buf),
            });
        }
    }

    /// Shared repeated-decode logic: accepts both unpacked (one record per
    /// value) and packed (`LengthDelimited` of concatenated values).
    fn decode_repeated<C: SingularCodec>(number: u32, fields: &UnknownFields) -> Vec<C::Value> {
        let mut out = Vec::new();
        for f in fields.iter().filter(|f| f.number == number) {
            if let Some(v) = C::decode_one(&f.data) {
                out.push(v);
            } else if let UnknownFieldData::LengthDelimited(bytes) = &f.data {
                C::decode_packed(bytes, &mut out);
            }
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::codecs::*;
    use super::*;
    use crate::unknown_fields::{UnknownField, UnknownFieldData};
    use alloc::string::{String, ToString};
    use alloc::{vec, vec::Vec};

    // Fake extendee — manually implements ExtensionSet over an UnknownFields.
    // All test `Extension<_>` consts use `CARRIER` as their extendee so the
    // identity check passes.
    const CARRIER: &str = "test.Carrier";

    #[derive(Default)]
    struct Carrier {
        unknown: UnknownFields,
    }
    impl ExtensionSet for Carrier {
        const PROTO_FQN: &'static str = CARRIER;
        fn unknown_fields(&self) -> &UnknownFields {
            &self.unknown
        }
        fn unknown_fields_mut(&mut self) -> &mut UnknownFields {
            &mut self.unknown
        }
    }

    fn varint(number: u32, v: u64) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Varint(v),
        }
    }
    fn fixed32(number: u32, v: u32) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Fixed32(v),
        }
    }
    fn fixed64(number: u32, v: u64) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Fixed64(v),
        }
    }
    fn ld(number: u32, data: Vec<u8>) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::LengthDelimited(data),
        }
    }
    fn group(number: u32, inner: UnknownFields) -> UnknownField {
        UnknownField {
            number,
            data: UnknownFieldData::Group(inner),
        }
    }

    // ── Extension<C> basics ─────────────────────────────────────────────────

    #[test]
    fn extension_const_fn() {
        const EXT: Extension<Int32> = Extension::new(50001, CARRIER);
        assert_eq!(EXT.number(), 50001);
        assert_eq!(EXT.extendee(), CARRIER);
        let copy = EXT; // Copy
        assert_eq!(copy.number(), 50001);
    }

    #[test]
    fn extension_with_default_const_fn() {
        // Naming a non-const fn in a const context is fine; only *calling*
        // it would not be. `with_default` just stores the pointer.
        const fn seven() -> i32 {
            7
        }
        const E: Extension<Int32> = Extension::with_default(1, CARRIER, seven);
        assert_eq!(E.number(), 1);
        assert_eq!(E.extendee(), CARRIER);
        let copy = E; // still Copy with the fn-pointer field
        assert_eq!(copy.number(), 1);
    }

    // ── Extendee identity check ─────────────────────────────────────────────

    // An extension declared for a different message. Using it on `Carrier`
    // is a bug in the caller — get/set/clear panic, has returns false.
    const WRONG: Extension<Int32> = Extension::new(1, "other.Message");

    #[test]
    #[should_panic(expected = "extends `other.Message`, not `test.Carrier`")]
    fn extension_panics_on_extendee_mismatch() {
        let c = Carrier::default();
        let _ = c.extension(&WRONG);
    }

    #[test]
    #[should_panic(expected = "extends `other.Message`, not `test.Carrier`")]
    fn set_extension_panics_on_extendee_mismatch() {
        let mut c = Carrier::default();
        c.set_extension(&WRONG, 42);
    }

    #[test]
    #[should_panic(expected = "extends `other.Message`, not `test.Carrier`")]
    fn clear_extension_panics_on_extendee_mismatch() {
        let mut c = Carrier::default();
        c.clear_extension(&WRONG);
    }

    #[test]
    #[should_panic(expected = "extends `other.Message`, not `test.Carrier`")]
    fn extension_or_default_panics_on_extendee_mismatch() {
        // Transitively via the inner extension() call.
        let c = Carrier::default();
        let _ = c.extension_or_default(&WRONG);
    }

    #[test]
    fn has_extension_returns_false_on_extendee_mismatch() {
        // has_extension is graceful: matches protobuf-go's HasExtension
        // (extension.go:24-26) and protobuf-es's hasExtension
        // (extensions.ts:134), both of which return false rather than
        // panic/throw on mismatch.
        let mut c = Carrier::default();
        // Even with a record at the same field number.
        c.unknown.push(varint(1, 42));
        assert!(!c.has_extension(&WRONG));
        // Same number, matching extendee → true.
        const RIGHT: Extension<Int32> = Extension::new(1, CARRIER);
        assert!(c.has_extension(&RIGHT));
    }

    // ── extension_or_default ────────────────────────────────────────────────

    #[test]
    fn extension_or_default_returns_declared_default_when_absent() {
        const fn seven() -> i32 {
            7
        }
        const E: Extension<Int32> = Extension::with_default(1, CARRIER, seven);
        let c = Carrier::default();
        // extension() still returns None — presence is distinguishable.
        assert_eq!(c.extension(&E), None);
        assert!(!c.has_extension(&E));
        // extension_or_default() returns the declared default.
        assert_eq!(c.extension_or_default(&E), 7);
    }

    #[test]
    fn extension_or_default_set_value_wins() {
        const fn seven() -> i32 {
            7
        }
        const E: Extension<Int32> = Extension::with_default(1, CARRIER, seven);
        let mut c = Carrier::default();
        c.set_extension(&E, 99);
        // Set value wins over the declared default.
        assert_eq!(c.extension_or_default(&E), 99);
        assert_eq!(c.extension(&E), Some(99));
    }

    #[test]
    fn extension_or_default_falls_back_to_type_default() {
        // No [default] declared → type default (0 for i32).
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let c = Carrier::default();
        assert_eq!(c.extension_or_default(&E), 0);
        assert_eq!(c.extension(&E), None);
    }

    #[test]
    fn extension_or_default_string_allocates_per_call() {
        // String default: non-const fn (allocates each call).
        fn hello() -> String {
            String::from("hello")
        }
        const E: Extension<StringCodec> = Extension::with_default(1, CARRIER, hello);
        let c = Carrier::default();
        assert_eq!(c.extension_or_default(&E), "hello");
        // Each call is a fresh allocation — the two strings are distinct.
        let a = c.extension_or_default(&E);
        let b = c.extension_or_default(&E);
        assert_eq!(a, b);
        assert_ne!(a.as_ptr(), b.as_ptr());
    }

    #[test]
    fn extension_or_default_bytes() {
        fn blob() -> Vec<u8> {
            alloc::vec![0xDE, 0xAD]
        }
        const E: Extension<BytesCodec> = Extension::with_default(1, CARRIER, blob);
        let c = Carrier::default();
        assert_eq!(c.extension_or_default(&E), alloc::vec![0xDE, 0xAD]);
    }

    #[test]
    fn extension_or_default_zero_is_present_not_default() {
        // Explicit presence: an explicitly-set 0 is returned, not the default.
        const fn seven() -> i32 {
            7
        }
        const E: Extension<Int32> = Extension::with_default(1, CARRIER, seven);
        let mut c = Carrier::default();
        c.set_extension(&E, 0);
        assert_eq!(c.extension_or_default(&E), 0);
    }

    // ── Singular scalar: roundtrip, last-wins, wire-type mismatch ──────────

    #[test]
    fn singular_scalar_roundtrip() {
        // One representative per wire family plus the zigzag cases.
        // Int32 sign-extend: -7 → `as i64 as u64` = 0xFFFF_FFFF_FFFF_FFF9.
        #[rustfmt::skip]
        let int32_cases: &[(i32, u64)] = &[
            (0,   0),
            (42,  42),
            (-7,  (-7_i64) as u64),
        ];
        for &(v, wire) in int32_cases {
            let mut c = Carrier::default();
            const E: Extension<Int32> = Extension::new(1, CARRIER);
            c.set_extension(&E, v);
            assert_eq!(
                c.unknown.iter().next().unwrap().data,
                UnknownFieldData::Varint(wire),
                "int32 {v}"
            );
            assert_eq!(c.extension(&E), Some(v), "int32 {v}");
        }

        // Sint32 zigzag: -1 → 1, 1 → 2, -2 → 3.
        #[rustfmt::skip]
        let sint32_cases: &[(i32, u64)] = &[
            (0,  0),
            (-1, 1),
            (1,  2),
            (-2, 3),
        ];
        for &(v, wire) in sint32_cases {
            let mut c = Carrier::default();
            const E: Extension<Sint32> = Extension::new(1, CARRIER);
            c.set_extension(&E, v);
            assert_eq!(
                c.unknown.iter().next().unwrap().data,
                UnknownFieldData::Varint(wire),
                "sint32 {v}"
            );
            assert_eq!(c.extension(&E), Some(v), "sint32 {v}");
        }

        // Sfixed32: bit-cast.
        let mut c = Carrier::default();
        const SF32: Extension<Sfixed32> = Extension::new(1, CARRIER);
        c.set_extension(&SF32, -1);
        assert_eq!(
            c.unknown.iter().next().unwrap().data,
            UnknownFieldData::Fixed32(u32::MAX)
        );
        assert_eq!(c.extension(&SF32), Some(-1));

        // Bool: any nonzero varint decodes as true.
        const B: Extension<Bool> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 7));
        assert_eq!(c.extension(&B), Some(true));
        let mut c = Carrier::default();
        c.set_extension(&B, true);
        assert_eq!(
            c.unknown.iter().next().unwrap().data,
            UnknownFieldData::Varint(1)
        );

        // Float/Double: bit roundtrip.
        const F: Extension<Float> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&F, 1.5_f32);
        assert_eq!(c.extension(&F), Some(1.5_f32));
        const D: Extension<Double> = Extension::new(2, CARRIER);
        c.set_extension(&D, -0.25_f64);
        assert_eq!(c.extension(&D), Some(-0.25_f64));

        // Uint32: truncates high bits of oversized varint on decode.
        const U: Extension<Uint32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 0x1_0000_002A)); // top bit truncated
        assert_eq!(c.extension(&U), Some(0x0000_002A));
    }

    #[test]
    fn singular_scalar_last_wins() {
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 5));
        c.unknown.push(varint(1, 7));
        assert_eq!(c.extension(&E), Some(7));
    }

    #[test]
    fn singular_scalar_wrong_wire_type_skipped() {
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 5));
        c.unknown.push(ld(1, vec![0x00])); // wrong wire type, skipped
        assert_eq!(c.extension(&E), Some(5));

        // Only wrong-type records → None.
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x00]));
        assert_eq!(c.extension(&E), None);
    }

    #[test]
    fn singular_absent_returns_none() {
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let c = Carrier::default();
        assert_eq!(c.extension(&E), None);
        // Other field numbers don't count.
        let mut c = Carrier::default();
        c.unknown.push(varint(2, 5));
        assert_eq!(c.extension(&E), None);
    }

    // ── Explicit presence (#8234 invariant) ─────────────────────────────────

    #[test]
    fn explicit_presence_with_zero_value() {
        // Extensions always have explicit presence: setting 0 makes it present.
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        assert!(!c.has_extension(&E));
        c.set_extension(&E, 0);
        assert!(c.has_extension(&E));
        assert_eq!(c.extension(&E), Some(0));
    }

    // ── set/clear/has ───────────────────────────────────────────────────────

    #[test]
    fn set_clears_prior_occurrences() {
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, 5);
        c.set_extension(&E, 7);
        // Exactly one record after second set.
        assert_eq!(c.unknown.iter().filter(|f| f.number == 1).count(), 1);
        assert_eq!(c.extension(&E), Some(7));
    }

    #[test]
    fn set_preserves_other_fields() {
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(2, 99));
        c.set_extension(&E, 5);
        assert_eq!(c.unknown.len(), 2);
        // Field 2 survived.
        assert!(c.unknown.iter().any(|f| f.number == 2));
    }

    #[test]
    fn clear_extension() {
        const E: Extension<Int32> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, 5);
        c.unknown.push(varint(2, 99));
        c.clear_extension(&E);
        assert!(!c.has_extension(&E));
        assert_eq!(c.unknown.len(), 1); // field 2 survived
    }

    // ── String / Bytes ──────────────────────────────────────────────────────

    #[test]
    fn string_roundtrip() {
        const E: Extension<StringCodec> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, "hello".to_string());
        assert_eq!(c.extension(&E), Some("hello".to_string()));
    }

    #[test]
    fn string_invalid_utf8_is_none() {
        const E: Extension<StringCodec> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0xFF, 0xFE]));
        assert_eq!(c.extension(&E), None);
    }

    #[test]
    fn bytes_roundtrip() {
        const E: Extension<BytesCodec> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(c.extension(&E), Some(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    }

    // ── Repeated scalar ─────────────────────────────────────────────────────

    #[test]
    fn repeated_scalar_unpacked() {
        const E: Extension<Repeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 1));
        c.unknown.push(varint(1, 2));
        c.unknown.push(varint(1, 3));
        assert_eq!(c.extension(&E), vec![1, 2, 3]);
    }

    #[test]
    fn repeated_scalar_packed() {
        // Packed varint payload for [1, 2, 300]: 0x01 0x02 0xAC 0x02
        const E: Extension<Repeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x01, 0x02, 0xAC, 0x02]));
        assert_eq!(c.extension(&E), vec![1, 2, 300]);
    }

    #[test]
    fn repeated_scalar_mixed_packed_unpacked() {
        const E: Extension<Repeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 1));
        c.unknown.push(ld(1, vec![0x02, 0x03])); // packed [2, 3]
        c.unknown.push(varint(1, 4));
        assert_eq!(c.extension(&E), vec![1, 2, 3, 4]);
    }

    #[test]
    fn repeated_scalar_fixed32_packed() {
        // Packed fixed32 payload for [1, 2]: 01 00 00 00 02 00 00 00
        const E: Extension<Repeated<Fixed32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![1, 0, 0, 0, 2, 0, 0, 0]));
        assert_eq!(c.extension(&E), vec![1_u32, 2_u32]);
    }

    #[test]
    fn repeated_scalar_wrong_wire_type_skipped() {
        const E: Extension<Repeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 1));
        c.unknown.push(fixed32(1, 0xDEAD)); // wrong, skipped
        c.unknown.push(varint(1, 2));
        assert_eq!(c.extension(&E), vec![1, 2]);
    }

    #[test]
    fn repeated_scalar_set_roundtrip() {
        const E: Extension<Repeated<Sint32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![-1, 0, 1, -100]);
        assert_eq!(c.extension(&E), vec![-1, 0, 1, -100]);
        // One record per element.
        assert_eq!(c.unknown.len(), 4);
    }

    #[test]
    fn repeated_string() {
        const E: Extension<Repeated<StringCodec>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(c.extension(&E), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn repeated_string_malformed_element_skipped() {
        const E: Extension<Repeated<StringCodec>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, b"ok".to_vec()));
        c.unknown.push(ld(1, vec![0xFF])); // invalid UTF-8, skipped
        c.unknown.push(ld(1, b"also ok".to_vec()));
        // StringCodec::decode_packed is a no-op, so the malformed element
        // contributes nothing.
        assert_eq!(
            c.extension(&E),
            vec!["ok".to_string(), "also ok".to_string()]
        );
    }

    #[test]
    fn repeated_empty_set_not_present() {
        const E: Extension<Repeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![]);
        assert!(!c.has_extension(&E));
        assert_eq!(c.extension(&E), Vec::<i32>::new());
    }

    #[test]
    fn packed_repeated_set_twice_one_record() {
        const E: Extension<PackedRepeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![1, 2]);
        c.set_extension(&E, vec![3, 4, 5]);
        assert_eq!(c.unknown.len(), 1);
        assert_eq!(c.extension(&E), vec![3, 4, 5]);
    }

    // ── PackedRepeated ──────────────────────────────────────────────────────

    #[test]
    fn packed_repeated_encode_one_record() {
        const E: Extension<PackedRepeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![1, 2, 300]);
        // Exactly one record.
        assert_eq!(c.unknown.len(), 1);
        assert_eq!(
            c.unknown.iter().next().unwrap().data,
            UnknownFieldData::LengthDelimited(vec![0x01, 0x02, 0xAC, 0x02])
        );
        // And it roundtrips.
        assert_eq!(c.extension(&E), vec![1, 2, 300]);
    }

    #[test]
    fn packed_repeated_decode_accepts_unpacked() {
        // PackedRepeated decode is identical to Repeated decode — accepts both.
        const E: Extension<PackedRepeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(varint(1, 1));
        c.unknown.push(varint(1, 2));
        assert_eq!(c.extension(&E), vec![1, 2]);
    }

    #[test]
    fn packed_repeated_empty_not_present() {
        const E: Extension<PackedRepeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![]);
        assert!(!c.has_extension(&E));
    }

    #[test]
    fn packed_repeated_fixed64_roundtrip() {
        const E: Extension<PackedRepeated<Sfixed64>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, vec![-1_i64, 0, i64::MAX]);
        assert_eq!(c.unknown.len(), 1);
        assert_eq!(c.extension(&E), vec![-1_i64, 0, i64::MAX]);
    }

    // ── MessageCodec ────────────────────────────────────────────────────────

    // Minimal test message: two i32 fields (numbers 1 and 2) + unknown preservation.
    // Hand-written to avoid a codegen dependency in the runtime crate's tests.
    #[derive(Clone, Default, PartialEq, Debug)]
    struct TestMsg {
        a: i32,
        b: i32,
        unknown: UnknownFields,
    }

    impl crate::DefaultInstance for TestMsg {
        fn default_instance() -> &'static Self {
            static INST: crate::__private::OnceBox<TestMsg> = crate::__private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(TestMsg::default()))
        }
    }

    impl crate::Message for TestMsg {
        fn compute_size(&self, _cache: &mut crate::SizeCache) -> u32 {
            let mut n = 0;
            if self.a != 0 {
                n += 1 + crate::encoding::varint_len(self.a as i64 as u64);
            }
            if self.b != 0 {
                n += 1 + crate::encoding::varint_len(self.b as i64 as u64);
            }
            n += self.unknown.encoded_len();
            n as u32
        }
        fn write_to(&self, _cache: &mut crate::SizeCache, buf: &mut impl bytes::BufMut) {
            use crate::encoding::encode_varint;
            if self.a != 0 {
                encode_varint(1 << 3, buf);
                encode_varint(self.a as i64 as u64, buf);
            }
            if self.b != 0 {
                encode_varint(2 << 3, buf);
                encode_varint(self.b as i64 as u64, buf);
            }
            self.unknown.write_to(buf);
        }
        fn merge_field(
            &mut self,
            tag: crate::encoding::Tag,
            buf: &mut impl bytes::Buf,
            _depth: u32,
        ) -> Result<(), crate::DecodeError> {
            match tag.field_number() {
                1 => self.a = crate::types::decode_int32(buf)?,
                2 => self.b = crate::types::decode_int32(buf)?,
                _ => crate::encoding::skip_field(tag, buf)?,
            }
            Ok(())
        }
        fn clear(&mut self) {
            *self = Self::default();
        }
    }

    #[test]
    fn message_single_record() {
        const E: Extension<MessageCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        // Wire for TestMsg{a:5, b:0}: tag(1,varint)=0x08, value=0x05.
        c.unknown.push(ld(1, vec![0x08, 0x05]));
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 5);
        assert_eq!(got.b, 0);
    }

    #[test]
    fn message_split_records_merge() {
        // Proto spec: multiple records at the same field number merge.
        // Record 1 sets a=5, record 2 sets b=7 — result has both.
        const E: Extension<MessageCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x08, 0x05])); // a=5
        c.unknown.push(ld(1, vec![0x10, 0x07])); // b=7
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 5);
        assert_eq!(got.b, 7);
    }

    #[test]
    fn message_absent_is_none() {
        const E: Extension<MessageCodec<TestMsg>> = Extension::new(1, CARRIER);
        let c = Carrier::default();
        assert!(c.extension(&E).is_none());
    }

    #[test]
    fn message_malformed_is_none() {
        const E: Extension<MessageCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        // Truncated varint: high bit set, no continuation.
        c.unknown.push(ld(1, vec![0x08, 0x80]));
        assert!(c.extension(&E).is_none());
    }

    #[test]
    fn message_set_get_roundtrip() {
        const E: Extension<MessageCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(
            &E,
            TestMsg {
                a: 3,
                b: -1,
                unknown: UnknownFields::new(),
            },
        );
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 3);
        assert_eq!(got.b, -1);
    }

    #[test]
    fn message_unknown_fields_survive_roundtrip() {
        // The extension message's *own* unknown fields survive get→set→get.
        const E: Extension<MessageCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        // Wire for TestMsg with a=5 AND an unknown field 99 (tag=99<<3|0=0x318, varint 42).
        // 0x318 = 792 → varint: 0x98 0x06. Value 42 → 0x2A.
        c.unknown.push(ld(1, vec![0x08, 0x05, 0x98, 0x06, 0x2A]));
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 5);
        // The unknown field was preserved inside TestMsg — actually wait,
        // our TestMsg::merge_field skips unknowns rather than storing them.
        // Redo this test with set→get instead.
        let mut inner_unk = UnknownFields::new();
        inner_unk.push(varint(99, 42));
        let msg = TestMsg {
            a: 5,
            b: 0,
            unknown: inner_unk,
        };
        let mut c = Carrier::default();
        c.set_extension(&E, msg);
        // Re-encode to bytes and decode — the unknown bytes are in the payload.
        use crate::Message;
        let mut roundtrip = TestMsg::default();
        if let UnknownFieldData::LengthDelimited(bytes) = &c.unknown.iter().next().unwrap().data {
            // Our TestMsg doesn't actually preserve unknowns on decode, so
            // verify the *wire* contains them instead.
            assert!(bytes.len() > 2, "payload includes unknown-field bytes");
            // Find the tag for field 99 in the encoded bytes.
            assert!(bytes.windows(3).any(|w| w == [0x98, 0x06, 0x2A]));
            // And decode of field 1 still works.
            roundtrip.merge_from_slice(bytes).unwrap();
            assert_eq!(roundtrip.a, 5);
        } else {
            panic!("expected LengthDelimited");
        }
    }

    #[test]
    fn repeated_message() {
        const E: Extension<Repeated<MessageCodec<TestMsg>>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x08, 0x01])); // a=1
        c.unknown.push(ld(1, vec![0x08, 0x02])); // a=2
        let got = c.extension(&E);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].a, 1);
        assert_eq!(got[1].a, 2);
    }

    #[test]
    fn repeated_message_malformed_element_skipped() {
        const E: Extension<Repeated<MessageCodec<TestMsg>>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x08, 0x01])); // a=1, ok
        c.unknown.push(ld(1, vec![0x08, 0x80])); // truncated, skipped
        c.unknown.push(ld(1, vec![0x08, 0x03])); // a=3, ok
        let got = c.extension(&E);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].a, 1);
        assert_eq!(got[1].a, 3);
    }

    // ── GroupCodec ──────────────────────────────────────────────────────────

    // Group-encoded message extensions (proto2 `group` syntax, editions
    // DELIMITED). Wire data is UnknownFieldData::Group(inner) instead of
    // LengthDelimited(bytes); inner is an already-parsed UnknownFields tree.

    /// Build an `UnknownFields` containing field-1-varint-v — matches
    /// `TestMsg{a: v as i32}` when re-serialized and merged.
    fn inner_a(v: u64) -> UnknownFields {
        let mut inner = UnknownFields::new();
        inner.push(varint(1, v));
        inner
    }

    #[test]
    fn group_single_record() {
        const E: Extension<GroupCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(group(1, inner_a(5)));
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 5);
        assert_eq!(got.b, 0);
    }

    #[test]
    fn group_split_records_merge() {
        // Proto spec: multiple records at the same field number merge.
        // Record 1 sets a=5, record 2 sets b=7 — result has both.
        const E: Extension<GroupCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(group(1, inner_a(5)));
        let mut inner_b = UnknownFields::new();
        inner_b.push(varint(2, 7));
        c.unknown.push(group(1, inner_b));
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 5);
        assert_eq!(got.b, 7);
    }

    #[test]
    fn group_absent_is_none() {
        const E: Extension<GroupCodec<TestMsg>> = Extension::new(1, CARRIER);
        let c = Carrier::default();
        assert!(c.extension(&E).is_none());
    }

    #[test]
    fn group_wrong_wire_type_is_none() {
        // A LengthDelimited record at the same number does NOT match GroupCodec
        // — decode skips it (would merge if MessageCodec were used).
        const E: Extension<GroupCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x08, 0x05]));
        assert!(c.extension(&E).is_none());
    }

    #[test]
    fn group_set_get_roundtrip() {
        const E: Extension<GroupCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(
            &E,
            TestMsg {
                a: 3,
                b: -1,
                unknown: UnknownFields::new(),
            },
        );
        // Verify the encoded data is a Group variant.
        match &c.unknown.iter().next().unwrap().data {
            UnknownFieldData::Group(_) => {}
            other => panic!("expected Group, got {other:?}"),
        }
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got.a, 3);
        assert_eq!(got.b, -1);
    }

    #[test]
    fn group_set_empty_message() {
        // An empty message encodes to zero bytes → empty Group inner.
        // Still counts as present (explicit-presence semantics).
        const E: Extension<GroupCodec<TestMsg>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.set_extension(&E, TestMsg::default());
        assert!(c.has_extension(&E));
        let got = c.extension(&E).expect("decoded");
        assert_eq!(got, TestMsg::default());
    }

    #[test]
    fn repeated_group() {
        const E: Extension<Repeated<GroupCodec<TestMsg>>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(group(1, inner_a(1)));
        c.unknown.push(group(1, inner_a(2)));
        let got = c.extension(&E);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].a, 1);
        assert_eq!(got[1].a, 2);
    }

    #[test]
    fn repeated_group_set_get_roundtrip() {
        const E: Extension<Repeated<GroupCodec<TestMsg>>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        let msgs = vec![
            TestMsg {
                a: 10,
                b: 0,
                unknown: UnknownFields::new(),
            },
            TestMsg {
                a: 0,
                b: 20,
                unknown: UnknownFields::new(),
            },
        ];
        c.set_extension(&E, msgs.clone());
        // Two separate Group records.
        assert_eq!(c.unknown.len(), 2);
        for f in c.unknown.iter() {
            assert!(matches!(f.data, UnknownFieldData::Group(_)));
        }
        assert_eq!(c.extension(&E), msgs);
    }

    // ── All remaining scalar types get a spot-check roundtrip ───────────────

    #[test]
    fn scalar_roundtrip_table() {
        macro_rules! rt {
            ($codec:ty, $v:expr) => {{
                const E: Extension<$codec> = Extension::new(1, CARRIER);
                let mut c = Carrier::default();
                c.set_extension(&E, $v);
                assert_eq!(c.extension(&E), Some($v), stringify!($codec));
            }};
        }
        rt!(Int64, -1_i64);
        rt!(Uint64, u64::MAX);
        rt!(Sint64, i64::MIN);
        rt!(Fixed32, u32::MAX);
        rt!(Fixed64, u64::MAX);
        rt!(Sfixed64, -1_i64);
        rt!(EnumI32, 42);
    }

    #[test]
    fn fixed64_packed_decode_partial_tail_ignored() {
        // 8 bytes = one value; 3 trailing bytes are ignored (not enough for a second).
        const E: Extension<Repeated<Fixed64>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        let mut payload = 42_u64.to_le_bytes().to_vec();
        payload.extend_from_slice(&[0x01, 0x02, 0x03]);
        c.unknown.push(ld(1, payload));
        assert_eq!(c.extension(&E), vec![42_u64]);
    }

    #[test]
    fn varint_packed_decode_malformed_tail_stops() {
        // Good varint 1, then truncated varint (high bit, no continuation).
        const E: Extension<Repeated<Int32>> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(ld(1, vec![0x01, 0x80]));
        assert_eq!(c.extension(&E), vec![1]);
    }

    // Verify unused helper silenced by one use here.
    #[test]
    fn fixed64_wrong_wire_type() {
        const E: Extension<Fixed64> = Extension::new(1, CARRIER);
        let mut c = Carrier::default();
        c.unknown.push(fixed64(1, 0xDEADBEEF));
        assert_eq!(c.extension(&E), Some(0xDEADBEEF_u64));
        let mut c = Carrier::default();
        c.unknown.push(fixed32(1, 0)); // wrong wire type
        assert_eq!(c.extension(&E), None);
    }
}

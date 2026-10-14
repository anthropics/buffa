//! Rust types for buffa's custom protobuf options.
//!
//! `buffa/ext/options.proto` declares one option for each kind of schema
//! element that buffa generates Rust for. Each option is a message, and a
//! setting is a field of that message, so an empty option message leaves the
//! generated code unchanged.
//!
//! | Option in `.proto` source | Set on | Extension | Read from | Value |
//! |---|---|---|---|---|
//! | `(buffa.ext.file)` | a file | [`FILE`] | `google.protobuf.FileOptions` | [`FileOptions`] |
//! | `(buffa.ext.message)` | a message | [`MESSAGE`] | `google.protobuf.MessageOptions` | [`MessageOptions`] |
//! | `(buffa.ext.field)` | a field | [`FIELD`] | `google.protobuf.FieldOptions` | [`FieldOptions`] |
//! | `(buffa.ext.oneof)` | a oneof | [`ONEOF`] | `google.protobuf.OneofOptions` | [`OneofOptions`] |
//! | `(buffa.ext.enum)` | an enum | [`ENUM`] | `google.protobuf.EnumOptions` | [`EnumOptions`] |
//! | `(buffa.ext.enum_value)` | an enum value | [`ENUM_VALUE`] | `google.protobuf.EnumValueOptions` | [`EnumValueOptions`] |
//!
//! Every option uses extension number 1381. `options.proto` records the range
//! of numbers that buffa has reserved.
//!
//! The value types share their names with the `descriptor.proto` messages
//! they extend. [`FieldOptions`] here is `buffa.ext.FieldOptions`, the value
//! of the option, and `google.protobuf.FieldOptions` in `buffa-descriptor` is
//! the message that it extends.
//!
//! A setting is a public field of its value type, so construct a value with
//! `..Default::default()`.
//!
//! # Importing the file
//!
//! protoc resolves `import "buffa/ext/options.proto";` from an include path.
//! The file is in this crate's `protos/` directory, and its text is
//! [`OPTIONS_PROTO`]: a build script can write that text to
//! [`OPTIONS_PROTO_PATH`] under a directory of its own, and pass the directory
//! to `protoc -I` or to `buffa_build::Config::includes`.
//!
//! # Examples
//!
//! Read an option from a descriptor with [`buffa::ExtensionSet`]. An option
//! that the schema left unset reads as `None`. The default features are
//! enough for this.
//!
//! ```
//! use buffa::ExtensionSet;
//! use buffa_descriptor::generated::descriptor::FieldDescriptorProto;
//!
//! use buffa_proto_options::{FieldOptions, FIELD};
//!
//! fn buffa_options(field: &FieldDescriptorProto) -> Option<FieldOptions> {
//!     field.options.as_option()?.extension(&FIELD)
//! }
//!
//! assert_eq!(buffa_options(&FieldDescriptorProto::default()), None);
//! ```
//!
//! # Panics
//!
//! Each constant reads only the `descriptor.proto` options message in its
//! "Read from" column. `extension(&FIELD)` on a
//! `google.protobuf.MessageOptions` panics, as do `set_extension` and
//! `clear_extension`. `has_extension(&FIELD)` returns `false` there.
//!
//! # Cargo features
//!
//! - **`std`** (default) — enables `buffa/std`. Without it the crate is
//!   `no_std` + `alloc`.
//! - **`views`** — zero-copy view types (`FieldOptionsView` and the rest).
//! - **`json`** — ProtoJSON serde impls.
//! - **`text`** — textproto impls.
//! - **`arbitrary`** — `arbitrary::Arbitrary` derives. Enables `std`.
//! - **`reflect`** — `buffa_descriptor::reflect::ReflectMessage` impls and
//!   `descriptor_pool()`. Enables `std`.
//!
//! With `json` or `text` on, `register_types` adds the options to a
//! `buffa::type_registry::TypeRegistry`. After
//! `buffa::type_registry::set_type_registry` installs that registry, and with
//! the same feature on in `buffa-descriptor`, a descriptor's options print
//! with `[buffa.ext.field]` keys.
//!
//! # Regenerating
//!
//! The generated code is checked in. After a change to `options.proto` or to
//! codegen output, run `task gen-option-types` from the workspace root.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]

/// The import path of the options file: `buffa/ext/options.proto`.
pub const OPTIONS_PROTO_PATH: &str = "buffa/ext/options.proto";

/// The text of `buffa/ext/options.proto`, the file that the types in this
/// crate are generated from.
pub const OPTIONS_PROTO: &str = include_str!("../protos/buffa/ext/options.proto");

// Package `buffa.ext`, mounted at the crate root so that the extern path for
// `.buffa.ext` is `::buffa_proto_options`. The module is private and glob
// re-exported because the generated code needs an `#[allow]` that a crate
// root would otherwise apply to hand-written code too: a message without
// fields generates a `match` with a single wildcard arm.
#[allow(clippy::match_single_binding)]
mod generated {
    include!("generated/buffa.ext.mod.rs");
}

pub use generated::*;

#[cfg(test)]
mod tests {
    use buffa::{ExtensionSet, Message};
    use buffa_descriptor::generated::descriptor as pb;

    use super::*;

    /// First extension number of buffa's range in the global registry.
    const REGISTERED: u32 = 1381;

    #[test]
    fn extensions_use_the_registered_number_and_extendee() {
        let declared = [
            (
                FILE.number(),
                FILE.extendee(),
                "google.protobuf.FileOptions",
            ),
            (
                MESSAGE.number(),
                MESSAGE.extendee(),
                "google.protobuf.MessageOptions",
            ),
            (
                FIELD.number(),
                FIELD.extendee(),
                "google.protobuf.FieldOptions",
            ),
            (
                ONEOF.number(),
                ONEOF.extendee(),
                "google.protobuf.OneofOptions",
            ),
            (
                ENUM.number(),
                ENUM.extendee(),
                "google.protobuf.EnumOptions",
            ),
            (
                ENUM_VALUE.number(),
                ENUM_VALUE.extendee(),
                "google.protobuf.EnumValueOptions",
            ),
        ];
        for (number, extendee, expected) in declared {
            assert_eq!(number, REGISTERED, "{expected}");
            assert_eq!(extendee, expected);
        }
    }

    #[test]
    fn option_survives_a_descriptor_round_trip() {
        let mut options = pb::FieldOptions::default();
        assert!(!options.has_extension(&FIELD));
        assert_eq!(options.extension(&FIELD), None);

        options.set_extension(&FIELD, FieldOptions::default());
        let decoded = pb::FieldOptions::decode_from_slice(&options.encode_to_vec()).unwrap();

        assert!(decoded.has_extension(&FIELD));
        assert_eq!(decoded.extension(&FIELD), Some(FieldOptions::default()));
    }

    /// `has_extension` compares the extendee as well as the number, which is
    /// what lets the six options share 1381.
    #[test]
    fn option_for_another_element_reads_as_absent() {
        let mut options = pb::FieldOptions::default();
        options.set_extension(&FIELD, FieldOptions::default());

        assert!(!options.has_extension(&MESSAGE));
    }

    /// A setting that a later `options.proto` adds must pass through a tool
    /// built against this one.
    #[test]
    fn unknown_setting_survives_a_round_trip() {
        // Field 1, varint 1: a setting this version does not declare.
        let newer = [0x08, 0x01];
        let decoded = FieldOptions::decode_from_slice(&newer).unwrap();
        assert_eq!(decoded.encode_to_vec(), newer);
    }
}

//! Rust identifier and path construction helpers.
//!
//! These are shared between buffa's codegen and downstream code generators
//! (e.g. `connectrpc-codegen`) that emit Rust code alongside buffa's message
//! types and need identical keyword-escaping and path-tokenization behavior.
//!
//! The guarantee is that if buffa generates `pub struct r#type::Foo { ... }`,
//! downstream callers using [`rust_path_to_tokens`]`("type::Foo")` produce the
//! matching `r#type::Foo` reference.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};

/// Parse a `::`-separated Rust path string into a [`TokenStream`], using raw
/// identifiers (`r#type`) for segments that are Rust keywords.
///
/// Used instead of `syn::parse_str::<syn::Type>` because the latter cannot
/// handle raw identifiers in path position: `"google::type::LatLng"` would
/// fail to parse because `type` is a keyword, but this function correctly
/// produces `google::r#type::LatLng`.
///
/// Path-position keywords (`self`, `super`, `Self`, `crate`) are emitted as
/// plain idents (they're valid in paths) — this differs from
/// [`make_field_ident`], which suffixes them with `_`.
///
/// Leading `::` (absolute path, e.g. `"::buffa::Message"`) is preserved.
///
/// # Panics
///
/// Panics (in debug) if `path` is empty.
pub fn rust_path_to_tokens(path: &str) -> TokenStream {
    debug_assert!(
        !path.is_empty(),
        "rust_path_to_tokens called with empty path"
    );

    // Handle absolute paths (starting with `::`, e.g. extern crate paths).
    let (prefix, rest) = if let Some(stripped) = path.strip_prefix("::") {
        (quote! { :: }, stripped)
    } else {
        (TokenStream::new(), path)
    };

    // For path segments, non-raw-able keywords (`self`, `super`, `Self`,
    // `crate`) are emitted as plain idents because they are valid in path
    // position. This differs from `make_field_ident`, which appends `_` for
    // these keywords since they are invalid as struct field names.
    let segments: Vec<Ident> = rest
        .split("::")
        .map(|seg| {
            if is_rust_keyword(seg) && can_be_raw_ident(seg) {
                Ident::new_raw(seg, Span::call_site())
            } else {
                Ident::new(seg, Span::call_site())
            }
        })
        .collect();

    quote! { #prefix #(#segments)::* }
}

/// Create a field identifier, escaping Rust keywords.
///
/// Most keywords use raw identifiers (`r#type`). The keywords `self`, `super`,
/// `Self`, `crate` cannot be raw identifiers and are suffixed with `_` instead
/// (e.g. `self_`), matching prost's convention.
///
/// This is the identifier of a struct field only when the proto name is the
/// Rust name. A field that sets `(buffa.ext.field).name` has the option's
/// value as its Rust name, and `idiomatic_field_names` converts a proto name
/// before it is escaped.
pub fn make_field_ident(name: &str) -> Ident {
    if is_rust_keyword(name) {
        if can_be_raw_ident(name) {
            Ident::new_raw(name, Span::call_site())
        } else {
            format_ident!("{}_", name)
        }
    } else {
        format_ident!("{}", name)
    }
}

/// Returns the Rust name of a generated message or enum type, before
/// raw-identifier escaping.
///
/// The name of a primitive type that generated code uses, such as `bool`,
/// `u32` or `str`, gets a trailing `_`. So does a keyword that cannot be a
/// raw identifier (`Self`, `self`, `super`, `crate`), as in
/// [`make_field_ident`]. A struct named `bool` would otherwise replace the
/// primitive for every item in its module, including the code that derive
/// macros expand there. Any other name is returned unchanged, and
/// [`make_type_ident`] makes it a raw identifier when it is a keyword.
pub(crate) fn escape_type_name(name: &str) -> std::borrow::Cow<'_, str> {
    if is_generated_primitive(name) || (is_rust_keyword(name) && !can_be_raw_ident(name)) {
        std::borrow::Cow::Owned(format!("{name}_"))
    } else {
        std::borrow::Cow::Borrowed(name)
    }
}

/// Returns the Rust name that buffa derives for a locally generated message
/// or enum type without a `name` option:
/// [`CodeGenConfig::type_name_prefix`](crate::CodeGenConfig::type_name_prefix),
/// then the proto simple name, with [`escape_type_name`] applied to the two
/// joined. Prefix `Pb` and `bool` give `Pbbool`, with the suffix left off.
pub(crate) fn local_type_name(type_name_prefix: &str, proto_name: &str) -> String {
    escape_type_name(&format!("{type_name_prefix}{proto_name}")).into_owned()
}

/// Create the identifier for a generated message or enum type, for its
/// declaration and its impls. `rust_name` must come from
/// [`escape_type_name`], [`local_type_name`], or a `name` option that
/// [`name_override`](crate::name_override) accepted: a keyword becomes
/// `r#type`, and any other name is used as written.
pub(crate) fn make_type_ident(rust_name: &str) -> Ident {
    make_field_ident(rust_name)
}

/// Is `name` a primitive type that generated code names without a path?
///
/// These are the Rust types of the proto scalars, plus `u8` and `usize`.
/// The other primitive type names (`char`, `i8`, `u16`) are left out:
/// generated code does not name them, so a message with one of those names
/// compiles as written.
pub(crate) fn is_generated_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool" | "str" | "u8" | "usize" | "i32" | "i64" | "u32" | "u64" | "f32" | "f64"
    )
}

/// Convert a protobuf enum value name to `UpperCamelCase`.
///
/// Word boundaries are underscores **and** case transitions, so the conversion
/// works on the canonical `SHOUTY_SNAKE_CASE` (`RULE_LEVEL_HIGH` → `RuleLevelHigh`)
/// as well as non-canonical mixed-case inputs: a lower→upper transition starts a
/// word (`myValue` → `MyValue`) and an acronym ends a word at the upper→lower
/// transition (`HTTPServer` → `HttpServer`). Each word's first character is
/// upper-cased and the rest lower-cased.
///
/// The conversion is intentionally lossy: `FOO_BAR` and `FOO__BAR` both collapse
/// to `FooBar`, and `HTTPServer` and `HTTP_SERVER` both produce `HttpServer`. The
/// caller is responsible for detecting the resulting collisions.
///
/// A leading digit in the output is only reachable when the caller has stripped
/// a prefix first (e.g. `VERSION_2` → `2`); it is preserved verbatim, so callers
/// that need a valid Rust identifier must check for it themselves.
#[must_use]
pub fn to_upper_camel_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut start_of_word = true;
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            start_of_word = true;
            continue;
        }
        // Within a run of non-underscore characters, detect a word boundary at
        // case transitions so mixed-case input splits correctly.
        if !start_of_word && i > 0 {
            let prev = chars[i - 1];
            let lower_to_upper = prev.is_lowercase() && ch.is_uppercase();
            let acronym_end = prev.is_uppercase()
                && ch.is_uppercase()
                && chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if lower_to_upper || acronym_end {
                start_of_word = true;
            }
        }
        if start_of_word {
            out.extend(ch.to_uppercase());
            start_of_word = false;
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// Convert a type name to `SHOUTY_SNAKE_CASE`.
///
/// Used to reconstruct the conventional enum-value prefix from an enum's proto
/// name so it can be stripped: `RuleLevel` → `RULE_LEVEL` (then values like
/// `RULE_LEVEL_HIGH` lose the `RULE_LEVEL_` prefix). An underscore is inserted
/// at each lower→upper boundary and at acronym→word boundaries
/// (`HTTPServer` → `HTTP_SERVER`); existing underscores are preserved without
/// doubling.
#[must_use]
pub fn to_shouty_snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            out.push('_');
            continue;
        }
        if i > 0 && ch.is_uppercase() && chars[i - 1] != '_' {
            let prev = chars[i - 1];
            let prev_starts_word = prev.is_lowercase() || prev.is_ascii_digit();
            let acronym_boundary =
                prev.is_uppercase() && chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if prev_starts_word || acronym_boundary {
                out.push('_');
            }
        }
        out.extend(ch.to_uppercase());
    }
    out
}

/// Escape a proto package segment for use as a Rust `mod` name.
///
/// Returns `r#` prefix for raw-able keywords, `_` suffix for path-position
/// keywords (which can't be raw), and the name as-is otherwise.
///
/// This is a `String` (not `Ident`) because callers typically emit it into
/// source text (e.g. `pub mod {name} { ... }` via `format!`), not via `quote!`.
pub fn escape_mod_ident(name: &str) -> String {
    if is_rust_keyword(name) {
        if can_be_raw_ident(name) {
            format!("r#{name}")
        } else {
            format!("{name}_")
        }
    } else {
        name.to_string()
    }
}

/// Is `name` a Rust keyword (strict, edition-2018+, edition-2024+, or reserved)?
///
/// Covers all editions up to 2024. See `scripts/check-keywords.py` for the
/// maintenance script that diffs this list against the upstream rustc source.
pub fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        // Strict keywords — all editions
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            // Strict keywords — edition 2018+
            | "async"
            | "await"
            | "dyn"
            // Strict keywords — edition 2024+
            | "gen"
            // Reserved for future use (all editions)
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

/// Can `name` be used as a raw identifier (`r#name`)?
///
/// `self`, `super`, `Self`, `crate` are valid path segments and cannot be
/// prefixed with `r#`. They get a `_` suffix in field/mod position instead.
fn can_be_raw_ident(name: &str) -> bool {
    !matches!(name, "self" | "super" | "Self" | "crate")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_path_simple() {
        assert_eq!(rust_path_to_tokens("Foo").to_string(), "Foo");
    }

    #[test]
    fn rust_path_nested() {
        assert_eq!(
            rust_path_to_tokens("foo::bar::Baz").to_string(),
            "foo :: bar :: Baz"
        );
    }

    #[test]
    fn rust_path_keyword_segment() {
        // `type` is a keyword → raw identifier.
        assert_eq!(
            rust_path_to_tokens("google::type::LatLng").to_string(),
            "google :: r#type :: LatLng"
        );
    }

    #[test]
    fn rust_path_absolute() {
        assert_eq!(
            rust_path_to_tokens("::buffa::Message").to_string(),
            ":: buffa :: Message"
        );
    }

    #[test]
    fn rust_path_super_segment() {
        // `super` is valid in path position → plain ident (no r# or _).
        assert_eq!(
            rust_path_to_tokens("super::super::Foo").to_string(),
            "super :: super :: Foo"
        );
    }

    #[test]
    fn type_name_escape() {
        for name in [
            "bool", "str", "u8", "usize", "i32", "i64", "u32", "u64", "f32", "f64",
        ] {
            assert_eq!(escape_type_name(name), format!("{name}_"), "{name}");
        }
        for name in ["Self", "self", "super", "crate"] {
            assert_eq!(escape_type_name(name), format!("{name}_"), "{name}");
        }
        // Case-sensitive, and raw-able keywords are left for the ident.
        // Generated code does not name the last six, so they stay as written.
        for name in [
            "Bool", "U32", "Str", "Crate", "type", "Foo", "char", "i8", "u16", "i128", "isize",
            "f16",
        ] {
            assert_eq!(escape_type_name(name), name, "{name}");
        }
    }

    #[test]
    fn local_type_name_escapes_the_prefixed_name() {
        assert_eq!(local_type_name("", "bool"), "bool_");
        assert_eq!(local_type_name("Pb", "bool"), "Pbbool");
        assert_eq!(local_type_name("Pb", "Foo"), "PbFoo");
    }

    #[test]
    fn type_ident_is_raw_for_keywords() {
        assert_eq!(make_type_ident("type").to_string(), "r#type");
        assert_eq!(make_type_ident("bool_").to_string(), "bool_");
        assert_eq!(make_type_ident("Foo").to_string(), "Foo");
    }

    #[test]
    fn field_ident_normal() {
        assert_eq!(make_field_ident("foo").to_string(), "foo");
    }

    #[test]
    fn field_ident_keyword() {
        assert_eq!(make_field_ident("type").to_string(), "r#type");
    }

    #[test]
    fn field_ident_non_raw_keyword() {
        // `self` can't be r#self → suffixed.
        assert_eq!(make_field_ident("self").to_string(), "self_");
        assert_eq!(make_field_ident("super").to_string(), "super_");
        assert_eq!(make_field_ident("crate").to_string(), "crate_");
        assert_eq!(make_field_ident("Self").to_string(), "Self_");
    }

    #[test]
    fn escape_mod_normal() {
        assert_eq!(escape_mod_ident("foo"), "foo");
    }

    #[test]
    fn escape_mod_keyword() {
        assert_eq!(escape_mod_ident("type"), "r#type");
        assert_eq!(escape_mod_ident("async"), "r#async");
    }

    #[test]
    fn escape_mod_non_raw_keyword() {
        assert_eq!(escape_mod_ident("self"), "self_");
        assert_eq!(escape_mod_ident("super"), "super_");
    }

    #[test]
    fn upper_camel_basic() {
        assert_eq!(to_upper_camel_case("RULE_LEVEL_HIGH"), "RuleLevelHigh");
        assert_eq!(to_upper_camel_case("UNKNOWN"), "Unknown");
        assert_eq!(to_upper_camel_case("low_priority"), "LowPriority");
        assert_eq!(to_upper_camel_case("HTTP_SERVER"), "HttpServer");
    }

    #[test]
    fn upper_camel_lossy_collisions() {
        // Doubled and absent underscores collapse to the same identifier — the
        // caller must detect this.
        assert_eq!(to_upper_camel_case("FOO_BAR"), "FooBar");
        assert_eq!(to_upper_camel_case("FOO__BAR"), "FooBar");
        // Acronym vs snake also collapse — both must resolve to one identifier
        // so the caller can detect the collision.
        assert_eq!(to_upper_camel_case("HTTPServer"), "HttpServer");
        assert_eq!(to_upper_camel_case("HTTP_SERVER"), "HttpServer");
    }

    #[test]
    fn upper_camel_mixed_case_input() {
        // Case transitions are word boundaries, so an already-CamelCase value
        // round-trips (and is later skipped as a redundant alias).
        assert_eq!(to_upper_camel_case("MyValue"), "MyValue");
        assert_eq!(to_upper_camel_case("fooBar"), "FooBar");
        assert_eq!(to_upper_camel_case("Active"), "Active");
    }

    #[test]
    fn upper_camel_digit_and_empty() {
        // Reachable only after a prefix strip; preserved verbatim for the
        // caller's validity check.
        assert_eq!(to_upper_camel_case("2"), "2");
        assert_eq!(to_upper_camel_case(""), "");
        assert_eq!(to_upper_camel_case("FOO_2"), "Foo2");
    }

    #[test]
    fn upper_camel_keyword_source() {
        // `SELF` folds to the keyword `Self`; identifier escaping is the
        // caller's job (via `make_field_ident`).
        assert_eq!(to_upper_camel_case("SELF"), "Self");
    }

    #[test]
    fn shouty_snake_basic() {
        assert_eq!(to_shouty_snake_case("RuleLevel"), "RULE_LEVEL");
        assert_eq!(to_shouty_snake_case("NullValue"), "NULL_VALUE");
        assert_eq!(to_shouty_snake_case("Type"), "TYPE");
    }

    #[test]
    fn shouty_snake_acronym() {
        assert_eq!(to_shouty_snake_case("HTTPServer"), "HTTP_SERVER");
    }

    #[test]
    fn shouty_snake_already_snakey() {
        // Idempotent on names that already carry underscores.
        assert_eq!(to_shouty_snake_case("RULE_LEVEL"), "RULE_LEVEL");
    }

    #[test]
    fn keyword_coverage() {
        assert!(is_rust_keyword("type"));
        assert!(is_rust_keyword("async"));
        assert!(is_rust_keyword("gen")); // 2024
        assert!(is_rust_keyword("yield")); // reserved
        assert!(!is_rust_keyword("foo"));
        assert!(!is_rust_keyword("Type")); // case-sensitive
    }
}

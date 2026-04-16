//! Parsing of proto `default_value` strings into Rust expression `TokenStream`s.

use crate::generated::descriptor::FieldDescriptorProto;
use proc_macro2::TokenStream;
use quote::quote;

use crate::context::CodeGenContext;
use crate::features::ResolvedFeatures;
use crate::CodeGenError;

/// Parse a proto `default_value` string into a Rust expression `TokenStream`.
///
/// Returns `Ok(None)` if `default_value` is absent, or `Ok(Some(expr))` where
/// `expr` is the Rust literal or constructor for the default.
///
/// # Errors
///
/// Returns an error if the default value cannot be parsed for the field's type,
/// or if the enum variant lookup fails.
pub fn parse_default_value(
    field: &FieldDescriptorProto,
    ctx: &CodeGenContext,
    current_package: &str,
    features: &ResolvedFeatures,
    nesting: usize,
) -> Result<Option<TokenStream>, CodeGenError> {
    use crate::generated::descriptor::field_descriptor_proto::Type;

    let default_str = match field.default_value.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(None),
    };

    // Custom defaults only apply to fields with explicit presence (proto2,
    // or editions with `features.field_presence = EXPLICIT`). Proto3 implicit
    // fields and editions with implicit presence ignore `default_value`.
    if features.field_presence != crate::features::FieldPresence::Explicit {
        return Ok(None);
    }

    let field_name = field.name.as_deref().unwrap_or("<unknown>");
    let ty = field.r#type.unwrap_or_default();
    let expr = match ty {
        Type::TYPE_BOOL => match default_str {
            "true" => quote! { true },
            "false" => quote! { false },
            _ => {
                return Err(CodeGenError::Other(format!(
                    "field '{field_name}': invalid bool default: '{default_str}'"
                )))
            }
        },
        Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32 => {
            let val: i32 = default_str.parse().map_err(|_| {
                CodeGenError::Other(format!(
                    "field '{field_name}': invalid i32 default: '{default_str}'"
                ))
            })?;
            quote! { #val }
        }
        Type::TYPE_UINT32 | Type::TYPE_FIXED32 => {
            let val: u32 = default_str.parse().map_err(|_| {
                CodeGenError::Other(format!(
                    "field '{field_name}': invalid u32 default: '{default_str}'"
                ))
            })?;
            quote! { #val }
        }
        Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64 => {
            let val: i64 = default_str.parse().map_err(|_| {
                CodeGenError::Other(format!(
                    "field '{field_name}': invalid i64 default: '{default_str}'"
                ))
            })?;
            quote! { #val }
        }
        Type::TYPE_UINT64 | Type::TYPE_FIXED64 => {
            let val: u64 = default_str.parse().map_err(|_| {
                CodeGenError::Other(format!(
                    "field '{field_name}': invalid u64 default: '{default_str}'"
                ))
            })?;
            quote! { #val }
        }
        Type::TYPE_FLOAT => parse_float_default::<f32>(default_str)?,
        Type::TYPE_DOUBLE => parse_float_default::<f64>(default_str)?,
        Type::TYPE_STRING => {
            // default_value is the raw text, not escaped. When strict_utf8_mapping
            // normalizes this field to bytes, emit the literal as a Vec<u8> —
            // the proto source literal is valid UTF-8 by definition.
            if crate::impl_message::effective_type(ctx, field, features) == Type::TYPE_BYTES {
                quote! { ::buffa::alloc::string::String::from(#default_str).into_bytes() }
            } else {
                quote! { ::buffa::alloc::string::String::from(#default_str) }
            }
        }
        Type::TYPE_BYTES => {
            let bytes = unescape_c_escape_string(default_str)?;
            let byte_literals = bytes.iter().map(|b| quote! { #b });
            quote! { ::buffa::alloc::vec![#(#byte_literals),*] }
        }
        Type::TYPE_ENUM => {
            // default_str is the proto value name (e.g. "BAR").
            let type_name = field
                .type_name
                .as_deref()
                .ok_or(CodeGenError::MissingField("field.type_name"))?;
            let path_str = ctx
                .rust_type_relative(type_name, current_package, nesting)
                .ok_or_else(|| {
                    CodeGenError::Other(format!(
                        "enum type '{type_name}' not found in descriptor set"
                    ))
                })?;
            let ty = crate::message::rust_path_to_tokens(&path_str);
            // Must use the same keyword-escaping as enumeration.rs so that
            // e.g. `[default = type]` → `r#type` and `[default = Self]` → `Self_`
            // match the actual variant ident emitted in the enum definition.
            let variant_ident = crate::message::make_field_ident(default_str);
            // Proto2 enum fields use bare E (closed enums).
            quote! { #ty::#variant_ident }
        }
        _ => return Ok(None),
    };

    Ok(Some(expr))
}

/// Parse a float/double default value, handling special values "inf", "-inf", "nan".
fn parse_float_default<F>(s: &str) -> Result<TokenStream, CodeGenError>
where
    F: std::str::FromStr + std::fmt::Display,
{
    let is_f32 = std::mem::size_of::<F>() == 4;
    match s {
        "inf" | "infinity" => {
            if is_f32 {
                Ok(quote! { f32::INFINITY })
            } else {
                Ok(quote! { f64::INFINITY })
            }
        }
        "-inf" | "-infinity" => {
            if is_f32 {
                Ok(quote! { f32::NEG_INFINITY })
            } else {
                Ok(quote! { f64::NEG_INFINITY })
            }
        }
        "nan" => {
            if is_f32 {
                Ok(quote! { f32::NAN })
            } else {
                Ok(quote! { f64::NAN })
            }
        }
        _ => {
            if is_f32 {
                let val: f32 = s
                    .parse()
                    .map_err(|_| CodeGenError::Other(format!("invalid f32 default: '{s}'")))?;
                Ok(quote! { #val })
            } else {
                let val: f64 = s
                    .parse()
                    .map_err(|_| CodeGenError::Other(format!("invalid f64 default: '{s}'")))?;
                Ok(quote! { #val })
            }
        }
    }
}

/// Unescape a C-escaped byte string as used in protobuf `default_value` for
/// bytes fields.
///
/// Based on `google::protobuf::UnescapeCEscapeString`.
///
/// # Errors
///
/// Returns an error for invalid escape sequences instead of panicking.
pub fn unescape_c_escape_string(s: &str) -> Result<Vec<u8>, CodeGenError> {
    let src = s.as_bytes();
    let len = src.len();
    let mut dst = Vec::with_capacity(len);
    let mut p = 0;

    while p < len {
        if src[p] != b'\\' {
            dst.push(src[p]);
            p += 1;
        } else {
            p += 1;
            if p == len {
                return Err(CodeGenError::Other(format!(
                    "invalid c-escaped default binary value ({s}): ends with '\\'"
                )));
            }
            match src[p] {
                b'a' => {
                    dst.push(0x07);
                    p += 1;
                }
                b'b' => {
                    dst.push(0x08);
                    p += 1;
                }
                b'f' => {
                    dst.push(0x0C);
                    p += 1;
                }
                b'n' => {
                    dst.push(0x0A);
                    p += 1;
                }
                b'r' => {
                    dst.push(0x0D);
                    p += 1;
                }
                b't' => {
                    dst.push(0x09);
                    p += 1;
                }
                b'v' => {
                    dst.push(0x0B);
                    p += 1;
                }
                b'\\' => {
                    dst.push(0x5C);
                    p += 1;
                }
                b'?' => {
                    dst.push(0x3F);
                    p += 1;
                }
                b'\'' => {
                    dst.push(0x27);
                    p += 1;
                }
                b'"' => {
                    dst.push(0x22);
                    p += 1;
                }
                b'0'..=b'7' => {
                    let mut octal = 0u8;
                    for _ in 0..3 {
                        if p < len && src[p] >= b'0' && src[p] <= b'7' {
                            octal = octal.wrapping_mul(8).wrapping_add(src[p] - b'0');
                            p += 1;
                        } else {
                            break;
                        }
                    }
                    dst.push(octal);
                }
                b'x' | b'X' => {
                    if p + 3 > len {
                        return Err(CodeGenError::Other(format!(
                            "invalid c-escaped default binary value ({s}): incomplete hex value"
                        )));
                    }
                    let b = u8::from_str_radix(&s[p + 1..p + 3], 16).map_err(|_| {
                        CodeGenError::Other(format!(
                            "invalid c-escaped default binary value ({s}): invalid hex value"
                        ))
                    })?;
                    dst.push(b);
                    p += 3;
                }
                _ => {
                    return Err(CodeGenError::Other(format!(
                        "invalid c-escaped default binary value ({s}): invalid escape"
                    )));
                }
            }
        }
    }
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_plain_text() {
        assert_eq!(
            unescape_c_escape_string("hello world").unwrap(),
            b"hello world"
        );
    }

    #[test]
    fn unescape_null() {
        assert_eq!(unescape_c_escape_string(r"\0").unwrap(), b"\0");
    }

    #[test]
    fn unescape_octal() {
        assert_eq!(
            unescape_c_escape_string(r"\012\156").unwrap(),
            &[0o012, 0o156]
        );
    }

    #[test]
    fn unescape_hex() {
        assert_eq!(
            unescape_c_escape_string(r"\x01\x02").unwrap(),
            &[0x01, 0x02]
        );
    }

    #[test]
    fn unescape_all_escapes() {
        assert_eq!(
            unescape_c_escape_string(r#"\0\001\a\b\f\n\r\t\v\\\'\"\xfe"#).unwrap(),
            b"\0\x01\x07\x08\x0C\n\r\t\x0B\\\'\"\xFE"
        );
    }

    #[test]
    fn unescape_incomplete_hex() {
        assert!(unescape_c_escape_string(r"\x1").is_err());
    }

    #[test]
    fn unescape_trailing_backslash() {
        assert!(unescape_c_escape_string(r"\").is_err());
    }

    #[test]
    fn unescape_question_mark() {
        assert_eq!(unescape_c_escape_string(r"\?").unwrap(), &[0x3F]);
    }

    #[test]
    fn unescape_uppercase_hex() {
        assert_eq!(unescape_c_escape_string(r"\XAB").unwrap(), &[0xAB]);
    }

    #[test]
    fn unescape_invalid_escape() {
        assert!(unescape_c_escape_string(r"\z").is_err());
    }

    #[test]
    fn unescape_invalid_hex_digits() {
        assert!(unescape_c_escape_string(r"\xGG").is_err());
    }

    #[test]
    fn unescape_empty_input() {
        assert_eq!(unescape_c_escape_string("").unwrap(), &[] as &[u8]);
    }

    #[test]
    fn parse_float_inf() {
        let ts = parse_float_default::<f32>("inf").unwrap();
        assert!(ts.to_string().contains("INFINITY"));
    }

    #[test]
    fn parse_float_neg_inf() {
        let ts = parse_float_default::<f64>("-inf").unwrap();
        assert!(ts.to_string().contains("NEG_INFINITY"));
    }

    #[test]
    fn parse_float_nan() {
        let ts = parse_float_default::<f32>("nan").unwrap();
        assert!(ts.to_string().contains("NAN"));
    }

    #[test]
    fn parse_float_infinity_long_form() {
        let ts = parse_float_default::<f32>("infinity").unwrap();
        assert!(ts.to_string().contains("INFINITY"));
    }

    #[test]
    fn parse_float_neg_infinity_long_form() {
        let ts = parse_float_default::<f64>("-infinity").unwrap();
        assert!(ts.to_string().contains("NEG_INFINITY"));
    }

    #[test]
    fn parse_float_normal() {
        let ts = parse_float_default::<f64>("3.14").unwrap();
        let s = ts.to_string();
        assert!(s.contains("3.14"), "expected 3.14 in '{s}'");
    }

    #[test]
    fn parse_float_invalid() {
        assert!(parse_float_default::<f32>("not_a_number").is_err());
    }
}

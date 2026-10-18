//! Parsing of explicit field defaults from `FieldDescriptorProto`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::desc::{FieldDefault, FieldKind, ScalarType, SingularKind};

pub(crate) fn parse(raw: &str, kind: FieldKind) -> Result<FieldDefault, ()> {
    let FieldKind::Singular(kind) = kind else {
        return Err(());
    };

    match kind {
        SingularKind::Enum(_) => Ok(FieldDefault::EnumName(raw.to_string())),
        SingularKind::Message(_) => Err(()),
        SingularKind::Scalar(scalar) => parse_scalar(raw, scalar),
    }
}

fn parse_scalar(raw: &str, scalar: ScalarType) -> Result<FieldDefault, ()> {
    Ok(match scalar {
        ScalarType::Double => FieldDefault::F64(parse_float(raw)?),
        ScalarType::Float => FieldDefault::F32(parse_float(raw)?),
        ScalarType::Int64 | ScalarType::Sfixed64 | ScalarType::Sint64 => {
            FieldDefault::I64(parse_signed_integer(raw)?.try_into().map_err(|_| ())?)
        }
        ScalarType::Uint64 | ScalarType::Fixed64 => FieldDefault::U64(parse_unsigned_integer(raw)?),
        ScalarType::Int32 | ScalarType::Sfixed32 | ScalarType::Sint32 => {
            FieldDefault::I32(parse_signed_integer(raw)?.try_into().map_err(|_| ())?)
        }
        ScalarType::Uint32 | ScalarType::Fixed32 => {
            FieldDefault::U32(parse_unsigned_integer(raw)?.try_into().map_err(|_| ())?)
        }
        ScalarType::Bool => FieldDefault::Bool(match raw {
            "true" => true,
            "false" => false,
            _ => return Err(()),
        }),
        // String defaults are already unescaped in a descriptor set.
        ScalarType::String => FieldDefault::String(raw.to_string()),
        ScalarType::Bytes => FieldDefault::Bytes(unescape_c_escape_string(raw)?),
    })
}

fn parse_signed_integer(raw: &str) -> Result<i128, ()> {
    let (negative, magnitude) = parse_integer_magnitude(raw, true)?;
    let magnitude = i128::from(magnitude);
    Ok(if negative { -magnitude } else { magnitude })
}

fn parse_unsigned_integer(raw: &str) -> Result<u64, ()> {
    let (negative, magnitude) = parse_integer_magnitude(raw, false)?;
    if negative {
        return Err(());
    }
    Ok(magnitude)
}

fn parse_integer_magnitude(raw: &str, allow_negative: bool) -> Result<(bool, u64), ()> {
    let (negative, unsigned) = match raw.strip_prefix('-') {
        Some(rest) if allow_negative => (true, rest),
        Some(_) => return Err(()),
        None if raw.starts_with('+') => return Err(()),
        None => (false, raw),
    };
    let (radix, digits) = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (16, hex)
    } else if unsigned.len() > 1 && unsigned.starts_with('0') {
        (8, &unsigned[1..])
    } else {
        (10, unsigned)
    };
    if digits.is_empty() {
        return Err(());
    }
    u64::from_str_radix(digits, radix)
        .map(|magnitude| (negative, magnitude))
        .map_err(|_| ())
}

fn parse_float<F: core::str::FromStr>(raw: &str) -> Result<F, ()> {
    if raw.starts_with('+') {
        return Err(());
    }
    match raw {
        "inf" => "inf".parse().map_err(|_| ()),
        "-inf" => "-inf".parse().map_err(|_| ()),
        "nan" => "NaN".parse().map_err(|_| ()),
        "-nan" => "-NaN".parse().map_err(|_| ()),
        "infinity" | "-infinity" => Err(()),
        _ => match radix_integer_to_decimal(raw)? {
            Some(decimal) => decimal.parse().map_err(|_| ()),
            None if is_decimal_number(raw) => raw.parse().map_err(|_| ()),
            None => Err(()),
        },
    }
}

fn is_decimal_number(raw: &str) -> bool {
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    if unsigned.is_empty() || raw.starts_with('+') {
        return false;
    }

    let bytes = unsigned.as_bytes();
    let mut pos = 0;
    let mut has_digit = false;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        has_digit = true;
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'.' {
        pos += 1;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            has_digit = true;
            pos += 1;
        }
    }
    if !has_digit {
        return false;
    }
    if pos < bytes.len() && matches!(bytes[pos], b'e' | b'E') {
        pos += 1;
        if pos < bytes.len() && matches!(bytes[pos], b'+' | b'-') {
            pos += 1;
        }
        let exponent_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if exponent_start == pos {
            return false;
        }
    }
    pos == bytes.len()
}

/// Convert an octal or hexadecimal integer default to decimal before parsing
/// it as a float. Rust's float parser reads leading-zero values as decimal and
/// does not accept hexadecimal integers, while protobuf uses base-zero integer
/// syntax for these defaults.
fn radix_integer_to_decimal(raw: &str) -> Result<Option<String>, ()> {
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    let radix_integer = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        Some((16, hex))
    } else if unsigned.len() > 1
        && unsigned.starts_with('0')
        && unsigned.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        Some((8, &unsigned[1..]))
    } else {
        None
    };
    let Some((_radix, digits)) = radix_integer else {
        return Ok(None);
    };
    if digits.is_empty() {
        return Err(());
    }

    let (negative, magnitude) = parse_integer_magnitude(raw, true)?;
    let decimal = magnitude.to_string();
    let mut result = String::with_capacity(decimal.len() + if negative { 1 } else { 0 });
    if negative {
        result.push('-');
    }
    result.push_str(&decimal);
    Ok(Some(result))
}

/// Decode the C-style byte escapes used by protobuf bytes defaults.
fn unescape_c_escape_string(raw: &str) -> Result<Vec<u8>, ()> {
    let src = raw.as_bytes();
    let mut dst = Vec::with_capacity(src.len());
    let mut pos = 0;

    while pos < src.len() {
        if src[pos] != b'\\' {
            dst.push(src[pos]);
            pos += 1;
            continue;
        }

        pos += 1;
        if pos == src.len() {
            return Err(());
        }
        match src[pos] {
            b'a' => dst.push(0x07),
            b'b' => dst.push(0x08),
            b'f' => dst.push(0x0c),
            b'n' => dst.push(b'\n'),
            b'r' => dst.push(b'\r'),
            b't' => dst.push(b'\t'),
            b'v' => dst.push(0x0b),
            b'\\' => dst.push(b'\\'),
            b'?' => dst.push(b'?'),
            b'\'' => dst.push(b'\''),
            b'"' => dst.push(b'"'),
            b'0'..=b'7' => {
                let mut value = 0u32;
                for _ in 0..3 {
                    if pos == src.len() || !(b'0'..=b'7').contains(&src[pos]) {
                        break;
                    }
                    value = value * 8 + u32::from(src[pos] - b'0');
                    pos += 1;
                }
                dst.push(value as u8);
                continue;
            }
            b'x' | b'X' => {
                pos += 1;
                if pos == src.len() || !src[pos].is_ascii_hexdigit() {
                    return Err(());
                }
                let mut value = 0u32;
                // Protobuf consumes at most two digits for a hex escape; a
                // following hex character is ordinary string content.
                for _ in 0..2 {
                    if pos == src.len() || !src[pos].is_ascii_hexdigit() {
                        break;
                    }
                    value = value
                        .saturating_mul(16)
                        .saturating_add((src[pos] as char).to_digit(16).ok_or(())?);
                    pos += 1;
                }
                dst.push(value as u8);
                continue;
            }
            _ => return Err(()),
        }
        pos += 1;
    }

    Ok(dst)
}

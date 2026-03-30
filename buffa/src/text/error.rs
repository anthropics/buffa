//! Parse error type for the textproto format.
//!
//! Every parse error carries the 1-based `line:col` position into the input
//! string, mirroring the protobuf C++ and Go text-format diagnostics.

/// An error encountered while parsing textproto input.
///
/// The `line` and `col` fields are 1-based and point at (or immediately after)
/// the offending byte in the input. `col` counts Unicode scalar values, not
/// bytes, so multi-byte UTF-8 characters advance the column by one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("text format parse error (line {line}:{col}): {kind}")]
#[non_exhaustive]
pub struct ParseError {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column (Unicode scalar count from start of line).
    pub col: u32,
    /// What went wrong.
    pub kind: ParseErrorKind,
}

impl ParseError {
    /// Construct a new parse error at the given position.
    #[inline]
    pub(crate) fn new(line: u32, col: u32, kind: ParseErrorKind) -> Self {
        Self { line, col, kind }
    }
}

/// The category of textproto parse error.
///
/// Variants are all zero-allocation: they hold `&'static str` or small `Copy`
/// payloads. The full diagnostic (including position) lives in [`ParseError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// Input ended while a value was still being parsed.
    #[error("unexpected end of input")]
    UnexpectedEof,

    /// A token appeared where the grammar didn't permit it.
    ///
    /// The `expected` string describes what the parser wanted — e.g.
    /// `"field name"`, `"scalar value"`, `"',' or ']'"`.
    #[error("unexpected token, expected {expected}")]
    UnexpectedToken {
        /// Human-readable description of what was expected.
        expected: &'static str,
    },

    /// A number literal couldn't be interpreted as the target numeric type
    /// (out of range, wrong base, or float where integer expected).
    #[error("invalid number literal")]
    InvalidNumber,

    /// A string literal contained a malformed escape sequence.
    ///
    /// Holds the reason produced by the unescaper — e.g.
    /// `"invalid \\x escape"`, `"invalid unicode escape"`.
    #[error("invalid string: {0}")]
    InvalidString(&'static str),

    /// A `string` field's unescaped bytes were not valid UTF-8.
    ///
    /// `bytes` fields use [`TextDecoder::read_bytes`](super::TextDecoder::read_bytes)
    /// and do not produce this error.
    #[error("invalid UTF-8 in string field")]
    InvalidUtf8,

    /// A field name appeared that the message's `merge_text` does not
    /// recognise, and the caller did not ask to skip unknowns.
    #[error("unknown field")]
    UnknownField,

    /// An enum variant name was not recognised, or a closed enum received a
    /// numeric value outside its defined set.
    ///
    /// Open (proto3) enums accept any in-range integer and never produce this
    /// error for numeric input. Closed (proto2) enums reject unknown numbers.
    #[error("unknown enum value")]
    UnknownEnumValue,

    /// Message nesting exceeded [`RECURSION_LIMIT`](crate::RECURSION_LIMIT).
    #[error("recursion limit exceeded")]
    RecursionLimitExceeded,

    /// A message was opened with `{` but closed with `>`, or vice versa.
    ///
    /// Textproto allows either `{...}` or `<...>` around sub-messages, but
    /// the pair must match.
    #[error("mismatched message delimiters")]
    DelimiterMismatch,

    /// An internal invariant was violated during parsing.
    ///
    /// This indicates a bug in buffa, not a problem with the input. Please
    /// report it if encountered.
    #[error("internal error (this is a buffa bug): {0}")]
    Internal(&'static str),
}

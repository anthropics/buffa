//! Textproto tokenizer.
//!
//! Produces a flat stream of [`Token`]s borrowing from the input `&str`. The
//! tokenizer is **syntactic only**: it recognises the shape of numbers, string
//! literals, and identifiers but does not interpret them. `42`, `"foo"`, and
//! `true` all come out as [`TokenKind::Scalar`] with `raw` pointing at the
//! relevant slice. Type-directed interpretation (is this i32? enum variant?
//! bool?) happens in the decoder layer, so `expected i32, got string` errors
//! are possible.
//!
//! The grammar is a simplified state machine driven by the last-returned token
//! kind — reference implementation is protobuf-go's
//! `internal/encoding/text/decode.go`. Colons, commas, and semicolons between
//! tokens are consumed internally and never surfaced.

use super::error::{ParseError, ParseErrorKind};

/// The kind of a textproto token.
///
/// Colons, commas, and semicolons are consumed by the tokenizer between
/// tokens and are not exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// End of input.
    Eof,
    /// A field name: either a bare identifier (`foo`), a bracketed extension
    /// name (`[pkg.ext]`), a bracketed Any URL (`[type.googleapis.com/Foo]`),
    /// or a decimal field number (`42`).
    Name,
    /// A scalar value: number literal, string literal, enum identifier, or
    /// `true`/`false`/`inf`/`nan`. Uninterpreted — the decoder parses `raw`
    /// according to the expected field type.
    Scalar,
    /// `{` or `<` — start of a nested message.
    MessageOpen,
    /// `}` or `>` — end of a nested message.
    MessageClose,
    /// `[` — start of a repeated-scalar list.
    ListOpen,
    /// `]` — end of a repeated-scalar list.
    ListClose,
}

/// Secondary classification of a [`TokenKind::Name`] token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// Bare identifier: `foo_bar`.
    Ident,
    /// Bracketed type name: `[pkg.ext]` or `[type.googleapis.com/pkg.Msg]`.
    /// The brackets are included in `raw`.
    TypeName,
    /// Decimal integer used as a field number: `42`.
    FieldNumber,
}

/// Secondary classification of a [`TokenKind::Scalar`] token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    /// A numeric literal: integer (dec/hex/oct) or float.
    Number,
    /// One or more adjacent quoted string literals.
    String,
    /// An identifier in value position: enum variant name, `true`, `false`,
    /// `inf`, `nan`, etc. May have a leading `-`.
    Literal,
}

/// A single textproto token, borrowing from the input.
#[derive(Debug, Clone, Copy)]
pub struct Token<'a> {
    /// What kind of token this is.
    pub kind: TokenKind,
    /// The raw input slice covered by this token. For strings, this includes
    /// the quotes and any adjacent concatenated literals. For bracketed type
    /// names, it includes the `[` and `]`.
    pub raw: &'a str,
    /// Byte offset of `raw` into the original input. Use
    /// [`Tokenizer::line_col`] to convert to 1-based line:col for errors.
    pub pos: usize,
    /// For `Name` tokens: which kind of name. `Ident` for non-`Name` tokens.
    pub name_kind: NameKind,
    /// For `Scalar` tokens: which kind of scalar. `Number` for non-`Scalar`.
    pub scalar_kind: ScalarKind,
    /// For `Name` tokens: was this name followed by a `:` separator?
    /// The textproto grammar makes `:` optional before messages but required
    /// before scalars.
    pub has_separator: bool,
}

/// Internal state: what the tokenizer is currently nested inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    /// Not inside anything — top-level message body.
    Top,
    /// Inside `{ ... }` (close with `}`) or `< ... >` (close with `>`).
    /// Holds the expected close byte.
    Message(u8),
    /// Inside `[ ... ]`.
    List,
}

/// Internal: the last-emitted token kind, driving the state machine.
/// Slightly richer than [`TokenKind`] because it tracks the consumed
/// separators.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LastKind {
    /// Beginning of file — no token emitted yet.
    Bof,
    Name,
    Scalar,
    MessageOpen,
    MessageClose,
    ListOpen,
    ListClose,
    Comma,
    Semicolon,
}

/// Stateful textproto tokenizer.
///
/// Holds a cursor into the input string and a small stack of open delimiters.
/// [`peek`](Self::peek) caches its result so it is cheap to call repeatedly
/// before [`read`](Self::read).
pub struct Tokenizer<'a> {
    input: &'a str,
    cursor: usize,
    last_kind: LastKind,
    /// Stack of `{`, `<`, `[` bytes. Capped at
    /// [`RECURSION_LIMIT`](crate::RECURSION_LIMIT) — pushing beyond that
    /// fails with [`ParseErrorKind::RecursionLimitExceeded`].
    open_stack: [u8; crate::message::RECURSION_LIMIT as usize],
    open_depth: usize,
    /// Peek cache: last-produced token + its `last_kind` + cursor after it.
    peeked: Option<(Token<'a>, LastKind, usize)>,
}

impl<'a> Tokenizer<'a> {
    /// Create a tokenizer positioned at the start of `input`.
    ///
    /// A leading UTF-8 byte-order mark (`U+FEFF`) is silently skipped — some
    /// editors prepend one when saving, and it is not a valid identifier
    /// start character.
    pub fn new(input: &'a str) -> Self {
        // BOM is 3 bytes in UTF-8 (EF BB BF).
        let cursor = if input.starts_with('\u{FEFF}') { 3 } else { 0 };
        Tokenizer {
            input,
            cursor,
            last_kind: LastKind::Bof,
            open_stack: [0u8; crate::message::RECURSION_LIMIT as usize],
            open_depth: 0,
            peeked: None,
        }
    }

    /// Look at the next token without consuming it.
    ///
    /// The result is cached: repeated `peek()` calls do not re-parse.
    ///
    /// # Errors
    ///
    /// Any tokenization error at the upcoming position.
    pub fn peek(&mut self) -> Result<Token<'a>, ParseError> {
        if let Some((tok, _, _)) = self.peeked {
            return Ok(tok);
        }
        // Speculatively tokenize: save scalar state, run `parse_next`, restore.
        //
        // open_stack invariant: `parse_next` may *write* to
        // `open_stack[save_depth]` (when it sees an open delimiter) or *read*
        // `open_stack[save_depth - 1]` (when it sees a close). It never
        // touches any other slot. Restoring `open_depth` here leaves the
        // written byte in place — harmless, because the slot is beyond
        // depth and unread by `current_open()`. When `read()` commits the
        // cached token below, it re-increments depth and the byte is
        // exactly where it needs to be. The single-token cache means we
        // can never have two speculative pushes at different depths
        // without an intervening commit.
        let save_cursor = self.cursor;
        let save_depth = self.open_depth;
        let save_last = self.last_kind;
        let tok = self.parse_next()?;
        let new_last = self.last_kind;
        let new_cursor = self.cursor;
        // Restore — `read()` commits from the cache.
        self.cursor = save_cursor;
        self.open_depth = save_depth;
        self.last_kind = save_last;
        self.peeked = Some((tok, new_last, new_cursor));
        Ok(tok)
    }

    /// Consume and return the next token.
    ///
    /// # Errors
    ///
    /// Any tokenization error at the current position.
    pub fn read(&mut self) -> Result<Token<'a>, ParseError> {
        if let Some((tok, last, cur)) = self.peeked.take() {
            // Commit the peeked state. `peek()` rolled back `open_depth`
            // but left the speculatively-written stack byte in place; we
            // only need to re-apply the depth delta. See the invariant
            // comment in `peek()`.
            self.last_kind = last;
            self.cursor = cur;
            match tok.kind {
                TokenKind::MessageOpen | TokenKind::ListOpen => self.open_depth += 1,
                TokenKind::MessageClose | TokenKind::ListClose => self.open_depth -= 1,
                _ => {}
            }
            return Ok(tok);
        }
        self.parse_next()
    }

    /// Convert a byte offset into the input into a 1-based (line, column).
    ///
    /// Column counts Unicode scalar values, not bytes. `pos` past the end of
    /// input clamps to the final position.
    ///
    /// Line and column use `u32`: inputs larger than ~4 GiB would wrap, but
    /// that is not a realistic textproto size and this is error-reporting only.
    pub fn line_col(&self, pos: usize) -> (u32, u32) {
        let pos = pos.min(self.input.len());
        let before = &self.input[..pos];
        let line = before.bytes().filter(|&b| b == b'\n').count() as u32 + 1;
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = before[line_start..].chars().count() as u32 + 1;
        (line, col)
    }

    // ── internals ───────────────────────────────────────────────────────────

    fn err(&self, pos: usize, kind: ParseErrorKind) -> ParseError {
        let (line, col) = self.line_col(pos);
        ParseError::new(line, col, kind)
    }

    fn err_here(&self, kind: ParseErrorKind) -> ParseError {
        self.err(self.cursor, kind)
    }

    #[inline]
    fn rest(&self) -> &'a [u8] {
        &self.input.as_bytes()[self.cursor..]
    }

    /// What we're currently nested inside, based on the top of the open stack.
    fn current_open(&self) -> OpenKind {
        if self.open_depth == 0 {
            return OpenKind::Top;
        }
        match self.open_stack[self.open_depth - 1] {
            b'{' => OpenKind::Message(b'}'),
            b'<' => OpenKind::Message(b'>'),
            b'[' => OpenKind::List,
            _ => unreachable!("open_stack holds only {{, <, ["),
        }
    }

    fn push_open(&mut self, ch: u8) -> Result<(), ParseError> {
        if self.open_depth >= self.open_stack.len() {
            return Err(self.err_here(ParseErrorKind::RecursionLimitExceeded));
        }
        self.open_stack[self.open_depth] = ch;
        self.open_depth += 1;
        Ok(())
    }

    fn pop_open(&mut self) {
        debug_assert!(self.open_depth > 0);
        self.open_depth -= 1;
    }

    /// Advance `self.cursor` past `n` bytes and then any following
    /// whitespace or `#`-to-EOL comments.
    fn consume(&mut self, n: usize) {
        self.cursor += n;
        loop {
            match self.rest().first() {
                Some(&c) if is_textproto_ws(c) => self.cursor += 1,
                Some(b'#') => {
                    // Skip to end of line (or end of input).
                    let rest = self.rest();
                    match rest.iter().position(|&b| b == b'\n') {
                        Some(i) => self.cursor += i + 1,
                        None => self.cursor = self.input.len(),
                    }
                }
                _ => break,
            }
        }
    }

    /// If the next byte is `c`, consume it (and trailing whitespace) and return true.
    fn try_consume_char(&mut self, c: u8) -> bool {
        if self.rest().first() == Some(&c) {
            self.consume(1);
            true
        } else {
            false
        }
    }

    /// Core state machine: produce the next token based on `self.last_kind`
    /// and the current nesting. Skips over comma/semicolon separators.
    ///
    /// Reference: protobuf-go `decode.go` `parseNext`.
    fn parse_next(&mut self) -> Result<Token<'a>, ParseError> {
        // Outer loop so we can re-dispatch after consuming a comma/semicolon.
        loop {
            self.consume(0);
            let at_eof = self.rest().is_empty();
            let open = self.current_open();

            match self.last_kind {
                LastKind::Bof => {
                    // Top-level: expect EOF or a field name.
                    if at_eof {
                        return self.emit_eof();
                    }
                    return self.parse_field_name();
                }

                LastKind::Name => {
                    // After a name: MessageOpen, ListOpen, or Scalar.
                    if at_eof {
                        return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                    }
                    let ch = self.rest()[0];
                    match ch {
                        b'{' | b'<' => {
                            self.push_open(ch)?;
                            return self.emit(TokenKind::MessageOpen, 1);
                        }
                        b'[' => {
                            self.push_open(ch)?;
                            return self.emit(TokenKind::ListOpen, 1);
                        }
                        _ => return self.parse_scalar(),
                    }
                }

                LastKind::Scalar | LastKind::MessageClose | LastKind::ListClose => {
                    // After a value: either close the current container,
                    // see a separator, or start the next field name.
                    match open {
                        OpenKind::Top => {
                            if at_eof {
                                return self.emit_eof();
                            }
                            match self.rest()[0] {
                                b',' => {
                                    self.consume(1);
                                    self.last_kind = LastKind::Comma;
                                    continue;
                                }
                                b';' => {
                                    self.consume(1);
                                    self.last_kind = LastKind::Semicolon;
                                    continue;
                                }
                                _ => return self.parse_field_name(),
                            }
                        }
                        OpenKind::Message(close) => {
                            if at_eof {
                                return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                            }
                            let ch = self.rest()[0];
                            if ch == close {
                                self.pop_open();
                                return self.emit(TokenKind::MessageClose, 1);
                            }
                            if ch == other_close(close) {
                                return Err(self.err_here(ParseErrorKind::DelimiterMismatch));
                            }
                            match ch {
                                b',' => {
                                    self.consume(1);
                                    self.last_kind = LastKind::Comma;
                                    continue;
                                }
                                b';' => {
                                    self.consume(1);
                                    self.last_kind = LastKind::Semicolon;
                                    continue;
                                }
                                _ => return self.parse_field_name(),
                            }
                        }
                        OpenKind::List => {
                            if at_eof {
                                return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                            }
                            let ch = self.rest()[0];
                            match ch {
                                b']' => {
                                    self.pop_open();
                                    return self.emit(TokenKind::ListClose, 1);
                                }
                                b',' => {
                                    self.consume(1);
                                    self.last_kind = LastKind::Comma;
                                    continue;
                                }
                                _ => {
                                    return Err(self.err_here(ParseErrorKind::UnexpectedToken {
                                        expected: "',' or ']'",
                                    }));
                                }
                            }
                        }
                    }
                }

                LastKind::MessageOpen => {
                    // After `{` or `<`: MessageClose (empty message) or Name.
                    if at_eof {
                        return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                    }
                    let OpenKind::Message(close) = open else {
                        unreachable!("MessageOpen always pushes a Message frame")
                    };
                    let ch = self.rest()[0];
                    if ch == close {
                        self.pop_open();
                        return self.emit(TokenKind::MessageClose, 1);
                    }
                    if ch == other_close(close) {
                        return Err(self.err_here(ParseErrorKind::DelimiterMismatch));
                    }
                    return self.parse_field_name();
                }

                LastKind::ListOpen => {
                    // After `[`: ListClose (empty list), MessageOpen, or Scalar.
                    if at_eof {
                        return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                    }
                    let ch = self.rest()[0];
                    match ch {
                        b']' => {
                            self.pop_open();
                            return self.emit(TokenKind::ListClose, 1);
                        }
                        b'{' | b'<' => {
                            self.push_open(ch)?;
                            return self.emit(TokenKind::MessageOpen, 1);
                        }
                        _ => return self.parse_scalar(),
                    }
                }

                LastKind::Comma | LastKind::Semicolon => {
                    // After a separator: close or next name/value.
                    match open {
                        OpenKind::Top => {
                            if at_eof {
                                return self.emit_eof();
                            }
                            return self.parse_field_name();
                        }
                        OpenKind::Message(close) => {
                            if at_eof {
                                return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                            }
                            let ch = self.rest()[0];
                            if ch == close {
                                self.pop_open();
                                return self.emit(TokenKind::MessageClose, 1);
                            }
                            if ch == other_close(close) {
                                return Err(self.err_here(ParseErrorKind::DelimiterMismatch));
                            }
                            return self.parse_field_name();
                        }
                        OpenKind::List => {
                            // Semicolon inside a list is unreachable by
                            // construction (the Scalar→List arm doesn't
                            // emit semicolons). Comma: next element.
                            if at_eof {
                                return Err(self.err_here(ParseErrorKind::UnexpectedEof));
                            }
                            let ch = self.rest()[0];
                            match ch {
                                b'{' | b'<' => {
                                    self.push_open(ch)?;
                                    return self.emit(TokenKind::MessageOpen, 1);
                                }
                                _ => return self.parse_scalar(),
                            }
                        }
                    }
                }
            }
        }
    }

    fn emit(&mut self, kind: TokenKind, len: usize) -> Result<Token<'a>, ParseError> {
        let pos = self.cursor;
        let raw = &self.input[pos..pos + len];
        self.consume(len);
        self.last_kind = match kind {
            TokenKind::Name => LastKind::Name,
            TokenKind::Scalar => LastKind::Scalar,
            TokenKind::MessageOpen => LastKind::MessageOpen,
            TokenKind::MessageClose => LastKind::MessageClose,
            TokenKind::ListOpen => LastKind::ListOpen,
            TokenKind::ListClose => LastKind::ListClose,
            TokenKind::Eof => LastKind::Bof, // unused; emit_eof handles Eof
        };
        Ok(Token {
            kind,
            raw,
            pos,
            name_kind: NameKind::Ident,
            scalar_kind: ScalarKind::Number,
            has_separator: false,
        })
    }

    fn emit_eof(&mut self) -> Result<Token<'a>, ParseError> {
        Ok(Token {
            kind: TokenKind::Eof,
            raw: &self.input[self.input.len()..],
            pos: self.input.len(),
            name_kind: NameKind::Ident,
            scalar_kind: ScalarKind::Number,
            has_separator: false,
        })
    }

    /// Parse a field name: identifier, `[type.name]`, or field number.
    /// Also consumes a trailing `:` if present, recording it in
    /// `has_separator`.
    fn parse_field_name(&mut self) -> Result<Token<'a>, ParseError> {
        let start = self.cursor;
        let rest = self.rest();

        // Bracketed type name: extension or Any URL.
        if rest[0] == b'[' {
            // Scan to the matching `]`. Whitespace inside is permitted and
            // preserved in `raw`; the decoder strips it when comparing.
            let mut i = 1;
            while i < rest.len() && rest[i] != b']' {
                i += 1;
            }
            if i >= rest.len() {
                return Err(self.err(start, ParseErrorKind::UnexpectedEof));
            }
            let len = i + 1; // include `]`
            let raw = &self.input[start..start + len];
            self.consume(len);
            self.last_kind = LastKind::Name;
            let has_separator = self.try_consume_char(b':');
            return Ok(Token {
                kind: TokenKind::Name,
                raw,
                pos: start,
                name_kind: NameKind::TypeName,
                scalar_kind: ScalarKind::Number,
                has_separator,
            });
        }

        // Plain identifier.
        let ilen = parse_ident(rest, false);
        if ilen > 0 {
            let raw = &self.input[start..start + ilen];
            self.consume(ilen);
            self.last_kind = LastKind::Name;
            let has_separator = self.try_consume_char(b':');
            return Ok(Token {
                kind: TokenKind::Name,
                raw,
                pos: start,
                name_kind: NameKind::Ident,
                scalar_kind: ScalarKind::Number,
                has_separator,
            });
        }

        // Decimal field number.
        if let Some(num) = lex_number(rest) {
            if !num.neg && num.kind == NumKind::Dec {
                // Validate it fits in i32 (field numbers are 29-bit, but
                // protobuf-go checks i32 range).
                let s = &self.input[start..start + num.len];
                if s.parse::<i32>().is_ok() {
                    let raw = s;
                    self.consume(num.len);
                    self.last_kind = LastKind::Name;
                    let has_separator = self.try_consume_char(b':');
                    return Ok(Token {
                        kind: TokenKind::Name,
                        raw,
                        pos: start,
                        name_kind: NameKind::FieldNumber,
                        scalar_kind: ScalarKind::Number,
                        has_separator,
                    });
                }
            }
        }

        Err(self.err(
            start,
            ParseErrorKind::UnexpectedToken {
                expected: "field name",
            },
        ))
    }

    /// Parse a scalar: string, literal identifier, or number.
    fn parse_scalar(&mut self) -> Result<Token<'a>, ParseError> {
        let start = self.cursor;
        let rest = self.rest();
        let first = rest[0];

        // String literal (possibly adjacent-concatenated).
        if first == b'"' || first == b'\'' {
            let len = lex_string_run(rest).ok_or_else(|| {
                self.err(start, ParseErrorKind::InvalidString("unterminated string"))
            })?;
            let raw = &self.input[start..start + len];
            self.consume(len);
            self.last_kind = LastKind::Scalar;
            return Ok(Token {
                kind: TokenKind::Scalar,
                raw,
                pos: start,
                name_kind: NameKind::Ident,
                scalar_kind: ScalarKind::String,
                has_separator: false,
            });
        }

        // Literal identifier (enum variant, true/false, -inf, etc.).
        let ilen = parse_ident(rest, true);
        if ilen > 0 {
            let raw = &self.input[start..start + ilen];
            self.consume(ilen);
            self.last_kind = LastKind::Scalar;
            return Ok(Token {
                kind: TokenKind::Scalar,
                raw,
                pos: start,
                name_kind: NameKind::Ident,
                scalar_kind: ScalarKind::Literal,
                has_separator: false,
            });
        }

        // Number.
        if let Some(num) = lex_number(rest) {
            let raw = &self.input[start..start + num.len];
            self.consume(num.len);
            self.last_kind = LastKind::Scalar;
            return Ok(Token {
                kind: TokenKind::Scalar,
                raw,
                pos: start,
                name_kind: NameKind::Ident,
                scalar_kind: ScalarKind::Number,
                has_separator: false,
            });
        }

        Err(self.err(
            start,
            ParseErrorKind::UnexpectedToken {
                expected: "scalar value",
            },
        ))
    }
}

// ── lexing helpers (free functions, no tokenizer state) ──────────────────────

/// The textproto close character for the *other* delimiter style.
/// `}` ↔ `>`.
#[inline]
fn other_close(c: u8) -> u8 {
    match c {
        b'}' => b'>',
        b'>' => b'}',
        _ => unreachable!(),
    }
}

/// Is `c` a token-delimiting byte? (Negation of the identifier/number
/// character set.)
#[inline]
fn is_delim(c: u8) -> bool {
    !(c == b'-' || c == b'+' || c == b'.' || c == b'_' || c.is_ascii_alphanumeric())
}

/// Is `c` a textproto whitespace byte?
///
/// Per <https://protobuf.dev/reference/protobuf/textformat-spec/#whitespace>
/// this includes vertical tab and form feed, which `u8::is_ascii_whitespace`
/// and the common `matches!(c, b' ' | b'\t' | b'\r' | b'\n')` idiom miss.
#[inline]
pub(super) const fn is_textproto_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0B' | b'\x0C')
}

/// Parse an identifier `[_a-zA-Z][_a-zA-Z0-9]*`, optionally with a leading
/// `-`. Returns the byte length, or 0 if no identifier is present.
///
/// Reference: protobuf-go `parseIdent`.
fn parse_ident(s: &[u8], allow_neg: bool) -> usize {
    let mut i = 0;
    if allow_neg && s.first() == Some(&b'-') {
        i = 1;
        // The grammar permits whitespace between the sign and the literal
        // (`FLOAT = [ "-" ] , FLOAT_LIT` where `,` means "may be separated
        // by whitespace"), so `- inf` is as valid as `-inf`. Matches what
        // `lex_number` does for `- 42`. The decoder side trims after
        // stripping `-` so the raw span's internal whitespace is harmless.
        let after = consume_ws(&s[1..]);
        i += s.len() - 1 - after.len();
        if s.get(i).is_none() {
            return 0;
        }
    }
    match s.get(i) {
        Some(b'_') | Some(b'a'..=b'z') | Some(b'A'..=b'Z') => i += 1,
        _ => return 0,
    }
    while let Some(&c) = s.get(i) {
        if c == b'_' || c.is_ascii_alphanumeric() {
            i += 1;
        } else {
            break;
        }
    }
    // Must be followed by a delimiter (or EOF), else this is some glued
    // nonsense like `foo.bar` in name position without brackets.
    if let Some(&c) = s.get(i) {
        if !is_delim(c) {
            return 0;
        }
    }
    i
}

/// Numeric literal classification.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NumKind {
    Dec,
    Hex,
    Oct,
    Float,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LexNumber {
    pub kind: NumKind,
    pub neg: bool,
    /// Byte length of the entire lexeme, including `-` and `f` suffix.
    pub len: usize,
    /// If `neg`, number of whitespace/comment bytes between `-` and the
    /// digits. Needed to reconstruct a parseable string.
    pub sep: usize,
}

/// Lex a number: decimal, `0x` hex, `0` octal, or float with optional
/// `e`/`E` exponent and `f`/`F` suffix.
///
/// Returns `None` if no number is present or if it's malformed.
///
/// Reference: protobuf-go `parseNumber`.
pub(super) fn lex_number(input: &[u8]) -> Option<LexNumber> {
    let mut s = input;
    let mut len = 0;
    let mut neg = false;
    let mut sep = 0;
    let mut kind = NumKind::Dec;

    if s.is_empty() {
        return None;
    }

    if s[0] == b'-' {
        neg = true;
        s = &s[1..];
        len += 1;
        // Consume whitespace/comments between `-` and the digits.
        let before = s.len();
        s = consume_ws(s);
        sep = before - s.len();
        len += sep;
        if s.is_empty() {
            return None;
        }
    }

    match s[0] {
        b'0' => {
            if s.len() > 1 {
                match s[1] {
                    b'x' | b'X' => {
                        kind = NumKind::Hex;
                        let mut n = 2;
                        while s.get(n).is_some_and(|c| c.is_ascii_hexdigit()) {
                            n += 1;
                        }
                        if n == 2 {
                            return None; // `0x` with no digits
                        }
                        len += n;
                        s = &s[n..];
                        return finish_number(s, kind, neg, len, sep);
                    }
                    b'0'..=b'7' => {
                        kind = NumKind::Oct;
                        let mut n = 2;
                        while s.get(n).is_some_and(|&c| (b'0'..=b'7').contains(&c)) {
                            n += 1;
                        }
                        len += n;
                        s = &s[n..];
                        return finish_number(s, kind, neg, len, sep);
                    }
                    _ => {}
                }
            }
            s = &s[1..];
            len += 1;
        }
        b'1'..=b'9' => {
            let mut n = 1;
            while s.get(n).is_some_and(u8::is_ascii_digit) {
                n += 1;
            }
            s = &s[n..];
            len += n;
        }
        b'.' => {
            // Leading dot: must have digits after. Flag intent and fall through.
            kind = NumKind::Float;
        }
        _ => return None,
    }

    // Optional `.` followed by zero-or-more digits.
    if s.first() == Some(&b'.') {
        let mut n = 1;
        // If the leading-dot case brought us here with no digits yet,
        // require at least one digit after the dot.
        let had_digits = kind != NumKind::Float;
        while s.get(n).is_some_and(u8::is_ascii_digit) {
            n += 1;
        }
        if !had_digits && n == 1 {
            return None; // `.` alone
        }
        s = &s[n..];
        len += n;
        kind = NumKind::Float;
    }

    // Optional `e`/`E` [`+`|`-`] digits.
    if s.len() >= 2 && matches!(s[0], b'e' | b'E') {
        kind = NumKind::Float;
        let mut n = 1;
        if matches!(s[1], b'+' | b'-') {
            n = 2;
            if s.len() <= 2 {
                return None;
            }
        }
        let start = n;
        while s.get(n).is_some_and(u8::is_ascii_digit) {
            n += 1;
        }
        if n == start {
            return None; // `e` with no digits
        }
        s = &s[n..];
        len += n;
    }

    // Optional `f`/`F` suffix.
    if matches!(s.first(), Some(b'f' | b'F')) {
        kind = NumKind::Float;
        s = &s[1..];
        len += 1;
    }

    finish_number(s, kind, neg, len, sep)
}

#[inline]
fn finish_number(
    rest: &[u8],
    kind: NumKind,
    neg: bool,
    len: usize,
    sep: usize,
) -> Option<LexNumber> {
    // Must end at a delimiter or EOF.
    if let Some(&c) = rest.first() {
        if !is_delim(c) {
            return None;
        }
    }
    Some(LexNumber {
        kind,
        neg,
        len,
        sep,
    })
}

/// Given a number token's raw text, produce a string suitable for feeding
/// to Rust's integer/float parsers: strip any `f` suffix and any whitespace
/// between `-` and the digits.
pub(super) fn number_for_parse<'a>(raw: &'a str, num: &LexNumber) -> alloc::borrow::Cow<'a, str> {
    let bytes = raw.as_bytes();
    let mut end = bytes.len();
    if num.kind == NumKind::Float && matches!(bytes.last(), Some(b'f' | b'F')) {
        end -= 1;
    }
    if num.neg && num.sep > 0 {
        // Stitch: `-` + digits (skip the separator run).
        let mut s = alloc::string::String::with_capacity(end - num.sep);
        s.push('-');
        s.push_str(&raw[1 + num.sep..end]);
        alloc::borrow::Cow::Owned(s)
    } else {
        alloc::borrow::Cow::Borrowed(&raw[..end])
    }
}

/// Consume leading whitespace and `#`-to-EOL comments.
fn consume_ws(mut s: &[u8]) -> &[u8] {
    loop {
        match s.first() {
            Some(&c) if is_textproto_ws(c) => s = &s[1..],
            Some(b'#') => match s.iter().position(|&b| b == b'\n') {
                Some(i) => s = &s[i + 1..],
                None => return &[],
            },
            _ => return s,
        }
    }
}

/// Lex a run of one-or-more adjacent string literals. Returns the total byte
/// length including all quotes and any inter-literal whitespace.
///
/// Escapes are handled only insofar as needed to find the close quote:
/// `\"` inside a `"..."` literal does not terminate it.
fn lex_string_run(s: &[u8]) -> Option<usize> {
    let mut i = 0;
    loop {
        // One literal.
        let quote = *s.get(i)?;
        debug_assert!(quote == b'"' || quote == b'\'');
        i += 1;
        loop {
            match s.get(i)? {
                &c if c == quote => {
                    i += 1;
                    break;
                }
                b'\n' | 0 => return None,
                b'\\' => {
                    // Skip the escape introducer and the next byte, whatever
                    // it is. Validation of the escape happens in `unescape`.
                    i += 2;
                    if i > s.len() {
                        return None;
                    }
                }
                _ => i += 1,
            }
        }
        // After a closing quote: skip inter-literal whitespace and check for
        // another opener. No `#` comments here — protobuf-go doesn't skip
        // comments between adjacent literals either (see `parseStringValue`).
        let mut j = i;
        while s.get(j).is_some_and(|&c| is_textproto_ws(c)) {
            j += 1;
        }
        if matches!(s.get(j), Some(b'"') | Some(b'\'')) {
            i = j;
            continue;
        }
        return Some(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Drain a tokenizer, collecting (kind, raw) pairs until Eof.
    fn drain(input: &str) -> Result<Vec<(TokenKind, &str)>, ParseError> {
        let mut t = Tokenizer::new(input);
        let mut out = Vec::new();
        loop {
            let tok = t.read()?;
            if tok.kind == TokenKind::Eof {
                return Ok(out);
            }
            out.push((tok.kind, tok.raw));
        }
    }

    // ── whole-input tokenization ────────────────────────────────────────────

    #[test]
    fn tokenize_simple_field() {
        let toks = drain("foo: 42").unwrap();
        assert_eq!(toks, [(TokenKind::Name, "foo"), (TokenKind::Scalar, "42")]);
    }

    #[test]
    fn tokenize_nested_message() {
        let toks = drain("child { a: 1 b: 2 }").unwrap();
        assert_eq!(
            toks,
            [
                (TokenKind::Name, "child"),
                (TokenKind::MessageOpen, "{"),
                (TokenKind::Name, "a"),
                (TokenKind::Scalar, "1"),
                (TokenKind::Name, "b"),
                (TokenKind::Scalar, "2"),
                (TokenKind::MessageClose, "}"),
            ]
        );
    }

    #[test]
    fn tokenize_angle_delimiters() {
        let toks = drain("m < x: 1 >").unwrap();
        assert_eq!(toks[1], (TokenKind::MessageOpen, "<"));
        assert_eq!(toks[4], (TokenKind::MessageClose, ">"));
    }

    #[test]
    fn tokenize_list() {
        let toks = drain("r: [1, 2, 3]").unwrap();
        assert_eq!(
            toks,
            [
                (TokenKind::Name, "r"),
                (TokenKind::ListOpen, "["),
                (TokenKind::Scalar, "1"),
                (TokenKind::Scalar, "2"),
                (TokenKind::Scalar, "3"),
                (TokenKind::ListClose, "]"),
            ]
        );
    }

    #[test]
    fn tokenize_empty_list() {
        let toks = drain("r: []").unwrap();
        assert_eq!(
            toks,
            [
                (TokenKind::Name, "r"),
                (TokenKind::ListOpen, "["),
                (TokenKind::ListClose, "]"),
            ]
        );
    }

    #[test]
    fn tokenize_message_list() {
        // List of messages: `[{...}, {...}]`
        let toks = drain("r: [{a: 1}, {a: 2}]").unwrap();
        assert_eq!(toks[1], (TokenKind::ListOpen, "["));
        assert_eq!(toks[2], (TokenKind::MessageOpen, "{"));
        assert_eq!(toks[5], (TokenKind::MessageClose, "}"));
        assert_eq!(toks[6], (TokenKind::MessageOpen, "{"));
        assert_eq!(toks[9], (TokenKind::MessageClose, "}"));
        assert_eq!(toks[10], (TokenKind::ListClose, "]"));
    }

    #[test]
    fn separators_consumed() {
        // Comma and semicolon between fields are silently eaten.
        let a = drain("a: 1 b: 2").unwrap();
        let b = drain("a: 1, b: 2").unwrap();
        let c = drain("a: 1; b: 2").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn comments_skipped() {
        let toks = drain("# leading\nfoo: 1 # trailing\nbar: 2").unwrap();
        assert_eq!(
            toks,
            [
                (TokenKind::Name, "foo"),
                (TokenKind::Scalar, "1"),
                (TokenKind::Name, "bar"),
                (TokenKind::Scalar, "2"),
            ]
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(drain("").unwrap(), []);
        assert_eq!(drain("  \n\t  ").unwrap(), []);
        assert_eq!(drain("# just a comment").unwrap(), []);
    }

    // ── name variants ───────────────────────────────────────────────────────

    #[test]
    fn name_kinds() {
        #[rustfmt::skip]
        let cases: &[(&str, NameKind, &str, bool)] = &[
            ("foo: 1",                  NameKind::Ident,       "foo",                  true),
            ("foo_bar: 1",              NameKind::Ident,       "foo_bar",              true),
            ("_private: 1",             NameKind::Ident,       "_private",             true),
            ("msg {}",                  NameKind::Ident,       "msg",                  false), // no colon before message
            ("[pkg.ext]: 1",            NameKind::TypeName,    "[pkg.ext]",            true),
            ("[a.b.c/d.e]: 1",          NameKind::TypeName,    "[a.b.c/d.e]",          true),
            ("[type.googleapis.com/Foo] {}", NameKind::TypeName, "[type.googleapis.com/Foo]", false),
            ("42: 1",                   NameKind::FieldNumber, "42",                   true),
        ];
        for &(input, want_kind, want_raw, want_sep) in cases {
            let mut t = Tokenizer::new(input);
            let tok = t.read().unwrap();
            assert_eq!(tok.kind, TokenKind::Name, "input: {input}");
            assert_eq!(tok.name_kind, want_kind, "input: {input}");
            assert_eq!(tok.raw, want_raw, "input: {input}");
            assert_eq!(tok.has_separator, want_sep, "input: {input}");
        }
    }

    // ── scalar variants ─────────────────────────────────────────────────────

    #[test]
    fn scalar_kinds() {
        #[rustfmt::skip]
        let cases: &[(&str, ScalarKind, &str)] = &[
            ("f: 42",           ScalarKind::Number,  "42"),
            ("f: -7",           ScalarKind::Number,  "-7"),
            ("f: 0x1F",         ScalarKind::Number,  "0x1F"),
            ("f: 0777",         ScalarKind::Number,  "0777"),
            ("f: 1.5",          ScalarKind::Number,  "1.5"),
            ("f: 1.5e-3",       ScalarKind::Number,  "1.5e-3"),
            ("f: .5",           ScalarKind::Number,  ".5"),
            ("f: 1f",           ScalarKind::Number,  "1f"),
            ("f: 1.5F",         ScalarKind::Number,  "1.5F"),
            (r#"f: "hello""#,   ScalarKind::String,  r#""hello""#),
            (r#"f: 'world'"#,   ScalarKind::String,  r#"'world'"#),
            (r#"f: "a" "b""#,   ScalarKind::String,  r#""a" "b""#), // adjacent concat
            ("f: true",         ScalarKind::Literal, "true"),
            ("f: False",        ScalarKind::Literal, "False"),
            ("f: FOO_BAR",      ScalarKind::Literal, "FOO_BAR"),    // enum name
            ("f: inf",          ScalarKind::Literal, "inf"),
            ("f: -inf",         ScalarKind::Literal, "-inf"),       // `-` + ident
            ("f: nan",          ScalarKind::Literal, "nan"),
        ];
        for &(input, want_kind, want_raw) in cases {
            let mut t = Tokenizer::new(input);
            t.read().unwrap(); // consume name
            let tok = t.read().unwrap();
            assert_eq!(tok.kind, TokenKind::Scalar, "input: {input}");
            assert_eq!(tok.scalar_kind, want_kind, "input: {input}");
            assert_eq!(tok.raw, want_raw, "input: {input}");
        }
    }

    #[test]
    fn string_escape_not_closing() {
        // `\"` inside a string literal must not close it.
        let mut t = Tokenizer::new(r#"f: "say \"hi\"""#);
        t.read().unwrap();
        let tok = t.read().unwrap();
        assert_eq!(tok.scalar_kind, ScalarKind::String);
        assert_eq!(tok.raw, r#""say \"hi\"""#);
    }

    #[test]
    fn adjacent_strings_no_whitespace() {
        let mut t = Tokenizer::new(r#"f: "a"'b'"c""#);
        t.read().unwrap();
        let tok = t.read().unwrap();
        assert_eq!(tok.raw, r#""a"'b'"c""#);
    }

    // ── number lexing ───────────────────────────────────────────────────────

    #[test]
    fn lex_number_table() {
        // Some((kind, len)) = success, None = not-a-number.
        #[rustfmt::skip]
        let cases: &[(&str, Option<(NumKind, usize)>)] = &[
            ("0",          Some((NumKind::Dec,   1))),
            ("42",         Some((NumKind::Dec,   2))),
            ("-7",         Some((NumKind::Dec,   2))),
            ("- 7",        Some((NumKind::Dec,   3))),  // whitespace after `-` ok
            ("0x1F",       Some((NumKind::Hex,   4))),
            ("0XFF",       Some((NumKind::Hex,   4))),
            ("-0x1",       Some((NumKind::Hex,   4))),
            ("0777",       Some((NumKind::Oct,   4))),
            ("1.5",        Some((NumKind::Float, 3))),
            (".5",         Some((NumKind::Float, 2))),
            ("1.",         Some((NumKind::Float, 2))),
            ("1e3",        Some((NumKind::Float, 3))),
            ("1.5e-3",     Some((NumKind::Float, 6))),
            ("1.5E+3",     Some((NumKind::Float, 6))),
            ("1f",         Some((NumKind::Float, 2))),
            ("1.5F",       Some((NumKind::Float, 4))),
            // non-numbers / malformed:
            ("",           None),
            ("abc",        None),
            ("0x",         None),    // no hex digits
            (".",          None),    // dot alone
            ("1e",         None),    // `e` with no digits
            ("1e+",        None),    // `e+` with no digits
            ("-",          None),    // bare `-`
            ("0x1g",       None),    // glued to non-delim
            ("42abc",      None),    // glued to non-delim
        ];
        for &(input, want) in cases {
            let got = lex_number(input.as_bytes()).map(|n| (n.kind, n.len));
            assert_eq!(got, want, "input: {input:?}");
        }
    }

    #[test]
    fn number_for_parse_strips_suffix() {
        let n = lex_number(b"1.5f").unwrap();
        assert_eq!(number_for_parse("1.5f", &n), "1.5");
    }

    #[test]
    fn number_for_parse_strips_separator() {
        let n = lex_number(b"- 42").unwrap();
        assert_eq!(n.sep, 1);
        assert_eq!(number_for_parse("- 42", &n), "-42");
    }

    #[test]
    fn number_for_parse_neg_hex_with_sep() {
        // `- 0xFF`: both sep-stripping AND hex-prefix-stripping paths.
        let n = lex_number(b"- 0xFF").unwrap();
        assert_eq!(n.kind, NumKind::Hex);
        assert!(n.neg);
        assert_eq!(n.sep, 1);
        assert_eq!(number_for_parse("- 0xFF", &n), "-0xFF");
    }

    // ── errors ──────────────────────────────────────────────────────────────

    #[test]
    fn delimiter_mismatch() {
        // Open with `{`, close with `>`.
        let err = drain("m { a: 1 >").unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::DelimiterMismatch);
    }

    #[test]
    fn delimiter_mismatch_angle() {
        let err = drain("m < a: 1 }").unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::DelimiterMismatch);
    }

    #[test]
    fn unexpected_eof_in_message() {
        let err = drain("m { a: 1").unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::UnexpectedEof);
    }

    #[test]
    fn unexpected_eof_in_list() {
        let err = drain("r: [1, 2").unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::UnexpectedEof);
    }

    #[test]
    fn unterminated_string() {
        let err = drain(r#"f: "oops"#).unwrap_err();
        assert!(matches!(err.kind, ParseErrorKind::InvalidString(_)));
    }

    #[test]
    fn list_missing_comma() {
        // `[1 2]` — missing comma between elements.
        let err = drain("r: [1 2]").unwrap_err();
        assert!(matches!(
            err.kind,
            ParseErrorKind::UnexpectedToken {
                expected: "',' or ']'"
            }
        ));
    }

    #[test]
    fn recursion_limit() {
        // 101 levels of `m {` should trip the limit.
        let depth = crate::message::RECURSION_LIMIT as usize + 1;
        let mut s = alloc::string::String::new();
        for _ in 0..depth {
            s.push_str("m { ");
        }
        let err = drain(&s).unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::RecursionLimitExceeded);
    }

    // ── peek ────────────────────────────────────────────────────────────────

    #[test]
    fn peek_does_not_advance() {
        let mut t = Tokenizer::new("a: 1 b: 2");
        let p1 = t.peek().unwrap();
        let p2 = t.peek().unwrap();
        assert_eq!(p1.raw, "a");
        assert_eq!(p2.raw, "a");
        let r = t.read().unwrap();
        assert_eq!(r.raw, "a");
        let next = t.read().unwrap();
        assert_eq!(next.raw, "1");
    }

    #[test]
    fn peek_then_read_preserves_nesting() {
        // Peek a MessageOpen, then read it — depth must be correct.
        let mut t = Tokenizer::new("m { a: 1 }");
        t.read().unwrap(); // name
        let p = t.peek().unwrap();
        assert_eq!(p.kind, TokenKind::MessageOpen);
        t.read().unwrap(); // commit the peeked open
                           // Now reading inside the message should work.
        assert_eq!(t.read().unwrap().raw, "a");
        assert_eq!(t.read().unwrap().raw, "1");
        assert_eq!(t.read().unwrap().kind, TokenKind::MessageClose);
    }

    // ── line/col ────────────────────────────────────────────────────────────

    #[test]
    fn line_col_table() {
        let input = "ab\ncde\nfg";
        let t = Tokenizer::new(input);
        #[rustfmt::skip]
        let cases: &[(usize, (u32, u32))] = &[
            (0, (1, 1)),   // 'a'
            (1, (1, 2)),   // 'b'
            (2, (1, 3)),   // '\n'
            (3, (2, 1)),   // 'c'
            (5, (2, 3)),   // 'e'
            (7, (3, 1)),   // 'f'
            (999, (3, 3)), // past end → clamp
        ];
        for &(pos, want) in cases {
            assert_eq!(t.line_col(pos), want, "pos: {pos}");
        }
    }

    #[test]
    fn line_col_unicode() {
        // 'é' is 2 bytes but 1 column.
        let t = Tokenizer::new("éx");
        assert_eq!(t.line_col(0), (1, 1));
        assert_eq!(t.line_col(2), (1, 2)); // byte 2 = 'x', column 2
    }

    #[test]
    fn error_has_correct_position() {
        let err = drain("a: 1\nb: 2\nm { x: 1 >").unwrap_err();
        assert_eq!(err.line, 3);
        // `>` is at byte offset 9 on line 3 → column 10
        assert_eq!(err.col, 10);
    }

    #[test]
    fn bom_is_skipped() {
        // UTF-8 BOM (U+FEFF = EF BB BF) at file start should be transparent.
        let toks = drain("\u{FEFF}a: 1").unwrap();
        assert_eq!(toks, &[(TokenKind::Name, "a"), (TokenKind::Scalar, "1")]);
    }

    #[test]
    fn bom_only_is_empty() {
        let toks = drain("\u{FEFF}").unwrap();
        assert!(toks.is_empty());
    }

    #[test]
    fn vertical_tab_and_form_feed_are_whitespace() {
        // Spec: https://protobuf.dev/reference/protobuf/textformat-spec/#whitespace
        let toks = drain("a:\x0B1\x0Cb: 2").unwrap();
        assert_eq!(
            toks,
            &[
                (TokenKind::Name, "a"),
                (TokenKind::Scalar, "1"),
                (TokenKind::Name, "b"),
                (TokenKind::Scalar, "2"),
            ]
        );
    }

    #[test]
    fn signed_literal_with_whitespace() {
        // `- inf` is as valid as `-inf` per the grammar. Raw span includes
        // the whitespace; decoder handles it.
        let toks = drain("f: - inf").unwrap();
        assert_eq!(
            toks,
            &[(TokenKind::Name, "f"), (TokenKind::Scalar, "- inf")]
        );
    }
}

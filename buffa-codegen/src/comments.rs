//! Source code comment extraction from protobuf descriptors.
//!
//! Protobuf stores source comments in `SourceCodeInfo`, attached to each
//! `FileDescriptorProto`. Comments are indexed by a *path* — a sequence of
//! field numbers and repeated-field indices that navigates from the
//! `FileDescriptorProto` root to a specific descriptor element.
//!
//! Rather than exposing these raw index-based paths to the rest of codegen,
//! this module translates them into an FQN-keyed map at construction time.
//! This trades a small up-front descriptor walk for significantly simpler
//! call sites: codegen functions look up comments by proto FQN (which they
//! already have) instead of threading index-based paths through every level
//! of the call stack.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;

use crate::generated::descriptor::{DescriptorProto, FileDescriptorProto};

// ── Descriptor field numbers (from google/protobuf/descriptor.proto) ────────
// FileDescriptorProto
const FILE_MESSAGE_TYPE: i32 = 4;
const FILE_ENUM_TYPE: i32 = 5;

// DescriptorProto
const MSG_FIELD: i32 = 2;
const MSG_NESTED_TYPE: i32 = 3;
const MSG_ENUM_TYPE: i32 = 4;
const MSG_ONEOF_DECL: i32 = 8;

// EnumDescriptorProto
const ENUM_VALUE: i32 = 2;

/// Walk a file descriptor's `SourceCodeInfo` and produce an FQN-keyed comment map.
///
/// Returns `(fqn -> comment_string)` entries for messages, fields, enums,
/// enum values, and oneofs. FQNs use the same dotted form as `proto_fqn`
/// throughout codegen (no leading dot), e.g. `"example.v1.Person"`,
/// `"example.v1.Person.name"`.
pub fn fqn_comments(file: &FileDescriptorProto) -> HashMap<String, String> {
    let path_map = build_path_map(file);
    if path_map.is_empty() {
        return HashMap::new();
    }

    let package = file.package.as_deref().unwrap_or("");
    let mut result = HashMap::new();

    // Top-level enums
    for (i, enum_type) in file.enum_type.iter().enumerate() {
        let enum_name = enum_type.name.as_deref().unwrap_or("");
        let fqn = fqn_join(package, enum_name);
        let path = vec![FILE_ENUM_TYPE, i as i32];
        collect_enum_comments(&path_map, &path, &fqn, enum_type, &mut result);
    }

    // Top-level messages
    for (i, msg) in file.message_type.iter().enumerate() {
        let msg_name = msg.name.as_deref().unwrap_or("");
        let fqn = fqn_join(package, msg_name);
        let path = vec![FILE_MESSAGE_TYPE, i as i32];
        collect_message_comments(&path_map, &path, &fqn, msg, &mut result);
    }

    result
}

/// Build the raw path-based comment map from `SourceCodeInfo`.
fn build_path_map(file: &FileDescriptorProto) -> HashMap<Vec<i32>, String> {
    let mut map = HashMap::new();
    let source_code_info = match file.source_code_info.as_option() {
        Some(sci) => sci,
        None => return map,
    };
    for location in &source_code_info.location {
        if let Some(comment) = format_comment(location) {
            map.insert(location.path.clone(), comment);
        }
    }
    map
}

/// Recursively collect comments for a message and all its children.
fn collect_message_comments(
    path_map: &HashMap<Vec<i32>, String>,
    msg_path: &[i32],
    msg_fqn: &str,
    msg: &DescriptorProto,
    out: &mut HashMap<String, String>,
) {
    // Message itself
    if let Some(comment) = path_map.get(msg_path) {
        out.insert(msg_fqn.to_string(), comment.clone());
    }

    // Fields
    for (i, field) in msg.field.iter().enumerate() {
        let field_name = field.name.as_deref().unwrap_or("");
        let fqn = format!("{}.{}", msg_fqn, field_name);
        let mut path = msg_path.to_vec();
        path.extend_from_slice(&[MSG_FIELD, i as i32]);
        if let Some(comment) = path_map.get(&path) {
            out.insert(fqn, comment.clone());
        }
    }

    // Oneofs
    for (i, oneof) in msg.oneof_decl.iter().enumerate() {
        let oneof_name = oneof.name.as_deref().unwrap_or("");
        let fqn = format!("{}.{}", msg_fqn, oneof_name);
        let mut path = msg_path.to_vec();
        path.extend_from_slice(&[MSG_ONEOF_DECL, i as i32]);
        if let Some(comment) = path_map.get(&path) {
            out.insert(fqn, comment.clone());
        }
    }

    // Nested enums
    for (i, enum_type) in msg.enum_type.iter().enumerate() {
        let enum_name = enum_type.name.as_deref().unwrap_or("");
        let fqn = format!("{}.{}", msg_fqn, enum_name);
        let mut path = msg_path.to_vec();
        path.extend_from_slice(&[MSG_ENUM_TYPE, i as i32]);
        collect_enum_comments(path_map, &path, &fqn, enum_type, out);
    }

    // Nested messages (recurse)
    for (i, nested) in msg.nested_type.iter().enumerate() {
        let nested_name = nested.name.as_deref().unwrap_or("");
        let fqn = format!("{}.{}", msg_fqn, nested_name);
        let mut path = msg_path.to_vec();
        path.extend_from_slice(&[MSG_NESTED_TYPE, i as i32]);
        collect_message_comments(path_map, &path, &fqn, nested, out);
    }
}

/// Collect comments for an enum and its values.
fn collect_enum_comments(
    path_map: &HashMap<Vec<i32>, String>,
    enum_path: &[i32],
    enum_fqn: &str,
    enum_desc: &crate::generated::descriptor::EnumDescriptorProto,
    out: &mut HashMap<String, String>,
) {
    // Enum itself
    if let Some(comment) = path_map.get(enum_path) {
        out.insert(enum_fqn.to_string(), comment.clone());
    }

    // Enum values
    for (i, value) in enum_desc.value.iter().enumerate() {
        let value_name = value.name.as_deref().unwrap_or("");
        let fqn = format!("{}.{}", enum_fqn, value_name);
        let mut path = enum_path.to_vec();
        path.extend_from_slice(&[ENUM_VALUE, i as i32]);
        if let Some(comment) = path_map.get(&path) {
            out.insert(fqn, comment.clone());
        }
    }
}

/// Join a package and a name into an FQN (no leading dot).
fn fqn_join(package: &str, name: &str) -> String {
    if package.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", package, name)
    }
}

/// Build `#[doc = "..."]` token stream attributes from an optional proto comment,
/// resolving AIP-192 cross-references into rustdoc intra-doc links where possible.
///
/// `scope_fqn` is the dotless FQN of the **enclosing proto type** whose lexical
/// scope governs ref resolution — always the message or enum FQN, never the
/// field/value/oneof's own FQN (e.g. `"google.example.v1.Book"` for a message
/// comment, a field comment, or a oneof comment inside `Book`). Unresolvable
/// refs and extern-crate types fall back to escaped-literal behaviour.
pub(crate) fn doc_attrs_resolved(
    comment: Option<&str>,
    scope_fqn: &str,
    type_map: &HashMap<String, String>,
) -> TokenStream {
    match comment {
        None => quote! {},
        Some(text) => doc_lines_with_refs(text, scope_fqn, type_map),
    }
}

/// Like [`doc_attrs_resolved`] but appends a `tag` line after a blank separator.
///
/// Useful for adding a "Field N: `name`" annotation after the proto comment body.
pub(crate) fn doc_attrs_with_tag_resolved(
    comment: Option<&str>,
    tag: &str,
    scope_fqn: &str,
    type_map: &HashMap<String, String>,
) -> TokenStream {
    match comment {
        None => doc_lines_with_refs(tag, scope_fqn, type_map),
        Some(text) => {
            // Render the two separately rather than concatenating first: the
            // comment is untrusted, and a code fence left open at its end
            // would otherwise swallow the tag into the code block. Rendering
            // the comment on its own closes any such fence before the tag.
            let body = doc_lines_with_refs(text, scope_fqn, type_map);
            let tag = doc_lines_with_refs(tag, scope_fqn, type_map);
            quote! { #body #[doc = ""] #tag }
        }
    }
}

/// Convert text into `#[doc = " ..."]` tokens — test helper with no proto context.
/// Passes an empty type_map so all proto refs fall back to escaping.
#[cfg(test)]
fn doc_lines_to_tokens(text: &str) -> TokenStream {
    doc_lines_impl(text, |line| {
        sanitize_line_with_refs(line, "", &HashMap::new())
    })
}

fn doc_lines_with_refs(
    text: &str,
    scope_fqn: &str,
    type_map: &HashMap<String, String>,
) -> TokenStream {
    doc_lines_impl(text, |line| {
        sanitize_line_with_refs(line, scope_fqn, type_map)
    })
}

/// Core line-by-line doc-comment formatter.
///
/// Handles indented-block fencing (4-space/tab blocks → ```` ```text ````),
/// user-written markdown fences (content passes through; the opener's info
/// string is rewritten by [`fence_info`] so rustdoc never compiles it), and
/// blank-line preservation. Prose lines are processed via the `sanitize`
/// closure.
///
/// Fence open/close detection follows CommonMark: a closer must have at
/// least the opener's tick count and no info string, and a marker indented
/// 4+ spaces is code rather than a fence. An unterminated fence is closed at
/// the end of the comment, so it cannot swallow doc text emitted after it.
fn doc_lines_impl<F: Fn(&str) -> String>(text: &str, sanitize: F) -> TokenStream {
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut lines: Vec<String> = Vec::with_capacity(raw_lines.len());
    let mut in_code_block = false;
    // The character and run length of the author's fence we are inside, if
    // any — a closer must match both.
    let mut open_fence: Option<(char, usize)> = None;

    for (idx, line) in raw_lines.iter().enumerate() {
        if let Some(open) = open_fence {
            // Anything that does not close the fence (a shorter run, a
            // ```lang line, the other fence character) is fence content.
            if fence_marker(line).is_some_and(|f| f.closes(open)) {
                open_fence = None;
            }
            lines.push(pad(line));
            continue;
        }

        if in_code_block {
            if is_indented(line) {
                lines.push(pad(&strip_indent(line)));
                continue;
            }
            // A blank line inside an indented block only closes it when no
            // more indented content follows.
            if line.is_empty() {
                let next_is_indented = raw_lines[idx + 1..]
                    .iter()
                    .find(|l| !l.is_empty())
                    .is_some_and(|l| is_indented(l));
                if next_is_indented {
                    lines.push(String::new());
                    continue;
                }
            }
            lines.push(" ```".to_string());
            in_code_block = false;
            // Fall through: this line still needs classifying.
        }

        if let Some(fence) = fence_marker(line) {
            open_fence = Some((fence.ch, fence.len));
            let run = fence.ch.to_string().repeat(fence.len);
            let info = fence_info(fence.info);
            lines.push(pad(&format!("{}{run}{info}", fence.indent)));
        } else if is_indented(line) {
            lines.push(" ```text".to_string());
            in_code_block = true;
            lines.push(pad(&strip_indent(line)));
        } else if line.is_empty() {
            lines.push(String::new());
        } else {
            lines.push(pad(&sanitize(line)));
        }
    }

    if in_code_block {
        lines.push(" ```".to_string());
    }
    if let Some((ch, len)) = open_fence {
        // An unterminated fence would swallow every doc line emitted after
        // the comment; close it with a matching fence.
        let closer = ch.to_string().repeat(len);
        lines.push(format!(" {closer}"));
    }

    quote! { #( #[doc = #lines] )* }
}

/// Give a rendered line the single leading space `#[doc = "…"]` needs to
/// unparse as `/// …` rather than `///…`. Blank lines stay blank.
fn pad(line: &str) -> String {
    if line.is_empty() {
        String::new()
    } else if line.starts_with(' ') {
        line.to_string()
    } else {
        format!(" {line}")
    }
}

/// Width of one indentation level, in columns (CommonMark's tab stop).
const INDENT_COLUMNS: usize = 4;

/// Byte offset at which `line` reaches `columns` of leading whitespace, with
/// tabs advancing to the next multiple of [`INDENT_COLUMNS`] as CommonMark
/// requires. `None` if the leading whitespace never gets that far, so a line
/// mixing spaces and tabs (`"  \t"` reaches column 4) is measured the way
/// rustdoc measures it rather than by counting literal spaces.
fn indent_split(line: &str, columns: usize) -> Option<usize> {
    let mut column = 0;
    for (offset, ch) in line.char_indices() {
        if column >= columns {
            return Some(offset);
        }
        column += match ch {
            ' ' => 1,
            '\t' => INDENT_COLUMNS - (column % INDENT_COLUMNS),
            _ => return None,
        };
    }
    (column >= columns).then_some(line.len())
}

/// An indented-code-block line: leading whitespace reaching 4+ columns.
fn is_indented(line: &str) -> bool {
    indent_split(line, INDENT_COLUMNS).is_some()
}

/// Remove one level of code-block indentation, but keep it on a line that
/// would otherwise read as a fence marker — inside the synthetic
/// ```` ```text ```` fence a de-indented ```` ``` ```` run would close it
/// early, while CommonMark ignores fence markers indented 4+ spaces.
fn strip_indent(line: &str) -> String {
    let Some(offset) = indent_split(line, INDENT_COLUMNS) else {
        return line.to_string();
    };
    let stripped = &line[offset..];
    if stripped.trim_start().starts_with("```") {
        line.to_string()
    } else {
        stripped.to_string()
    }
}

/// If `line` is a fence marker — an optionally ≤3-space-indented run of 3+
/// backticks *or* tildes — return the fence character, the run length, and
/// the info string after it. rustdoc's markdown parser treats `~~~` exactly
/// like ` ``` `, so both must be inerted.
///
/// Lines indented 4+ spaces are indented code, not fences, and a backtick
/// fence's info string may not itself contain a backtick — in both cases
/// CommonMark says this is not a fence, so neither does this.
fn fence_marker(line: &str) -> Option<Fence<'_>> {
    if is_indented(line) {
        return None;
    }
    let indent_len = line.len() - line.trim_start_matches(' ').len();
    let (indent, rest) = line.split_at(indent_len);
    let ch = rest.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let info = rest.trim_start_matches(ch);
    let len = rest.len() - info.len();
    if len < 3 || (ch == '`' && info.contains('`')) {
        return None;
    }
    Some(Fence {
        indent,
        ch,
        len,
        info,
    })
}

/// A fenced-code-block marker line.
struct Fence<'a> {
    /// The 0-3 spaces before the run.
    indent: &'a str,
    /// The fence character: a backtick or a tilde.
    ch: char,
    /// How many of them the run has. A closer needs at least as many.
    len: usize,
    /// Whatever follows the run — the language, plus any rustdoc attributes.
    info: &'a str,
}

impl Fence<'_> {
    /// Whether this marker closes `open` — same character, a run at least as
    /// long, and nothing but spaces or tabs after it (CommonMark).
    fn closes(&self, open: (char, usize)) -> bool {
        self.ch == open.0 && self.len >= open.1 && self.info.chars().all(|c| c == ' ' || c == '\t')
    }
}

/// The info string to emit for a fence opener, so that rustdoc never
/// *compiles* the fence body. Proto comments are untrusted input: their
/// examples have no imports, name proto types rather than Rust ones, and
/// would be compiled — and run — by the consuming crate's
/// `cargo test --doc`.
///
/// Deciding "is this Rust?" the way rustdoc does is a trap: an explicit
/// `rust` keeps the block Rust even beside an unknown word
/// (`rust,noplayground`), error-code tokens (`compile_fail,E0277`) and
/// `{class=…}` attributes keep it Rust too, and the verdict is even
/// order-dependent. So this does not classify at all — it makes every
/// fence inert:
///
/// - `ignore-<target>` tokens are dropped. rustdoc reads them as a target
///   *list* that replaces a plain `ignore`, so the block would still be
///   compiled for every other target.
/// - a bare `ignore` is then ensured. It is the only attribute that
///   reliably stops compilation: `no_run` still type-checks,
///   `should_panic` runs, and `compile_fail` merely inverts the verdict.
///
/// The author's language annotation is preserved, so a ` ```rust ` fence
/// still gets Rust syntax highlighting as ` ```rust,ignore `. An `ignore`
/// on a non-Rust fence is inert, and an unannotated fence becomes `text`
/// rather than guessing a language — rustdoc highlights nothing but Rust,
/// so identifying JSON or YAML would buy no rendering benefit.
fn fence_info(info: &str) -> String {
    let mut dropped_target_ignore = false;
    let mut tokens: Vec<&str> = Vec::new();
    for tok in info.split([',', ' ', '\t']).filter(|t| !t.is_empty()) {
        if tok.starts_with("ignore-") {
            dropped_target_ignore = true;
        } else {
            tokens.push(tok);
        }
    }

    if tokens.is_empty() {
        // A fence annotated only with `ignore-<target>` was Rust, so keep
        // it highlighted; an unannotated one is language-agnostic.
        return if dropped_target_ignore {
            "rust,ignore".to_string()
        } else {
            "text".to_string()
        };
    }

    if !tokens.contains(&"ignore") {
        tokens.push("ignore");
    }
    tokens.join(",")
}

/// Sanitize one prose line of proto comment text for rustdoc, resolving
/// AIP-192 cross-references into intra-doc links where possible and escaping
/// everything else. `scope_fqn` and `type_map` may be empty for test-only callers
/// that just need the base escaping behaviour.
fn sanitize_line_with_refs(
    line: &str,
    scope_fqn: &str,
    type_map: &HashMap<String, String>,
) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'`' {
            let run_start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let run_len = i - run_start;
            if let Some(close_end) = find_backtick_closer(bytes, i, run_len) {
                out.push_str(&line[run_start..close_end]);
                i = close_end;
            } else {
                out.push_str(&line[run_start..i]);
            }
            continue;
        }

        match b {
            b'\\' => {
                out.push('\\');
                i += 1;
                if i < bytes.len() {
                    i += push_char_at(&mut out, line, i);
                }
            }
            b'[' => {
                if let Some(end) = find_inline_link_end(bytes, i) {
                    out.push_str(&line[i..=end]);
                    i = end + 1;
                } else if let Some((display, ref_target, end)) = find_ref_link(bytes, i, line) {
                    match resolve_proto_ref(display, ref_target, scope_fqn, type_map) {
                        Some(resolved) => out.push_str(&resolved),
                        None => {
                            out.push_str("\\[");
                            out.push_str(&escape_angle_brackets(display));
                            out.push_str("\\]\\[");
                            out.push_str(&escape_angle_brackets(ref_target));
                            out.push_str("\\]");
                        }
                    }
                    i = end;
                } else {
                    out.push_str("\\[");
                    i += 1;
                }
            }
            b']' => {
                out.push_str("\\]");
                i += 1;
            }
            b'<' => {
                if let Some(end) = find_autolink_end(bytes, i) {
                    out.push_str(&line[i..=end]);
                    i = end + 1;
                } else {
                    out.push_str("\\<");
                    i += 1;
                }
            }
            b'>' => {
                out.push_str("\\>");
                i += 1;
            }
            b'h' => {
                if let Some(end) = find_bare_url_end(bytes, i) {
                    out.push('<');
                    out.push_str(&line[i..end]);
                    out.push('>');
                    i = end;
                } else {
                    out.push('h');
                    i += 1;
                }
            }
            _ => {
                i += push_char_at(&mut out, line, i);
            }
        }
    }
    out
}

/// Push the UTF-8 char at byte index `i` of `s` into `out`, returning its
/// byte length. `i` must be a char boundary and `< s.len()`.
fn push_char_at(out: &mut String, s: &str, i: usize) -> usize {
    let ch = s[i..]
        .chars()
        .next()
        .expect("i is in bounds and on a char boundary");
    out.push(ch);
    ch.len_utf8()
}

/// Starting at `from` (just past an opening run of `run_len` backticks),
/// return the past-the-end index of the matching closing run, or `None`.
fn find_backtick_closer(bytes: &[u8], from: usize, run_len: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            if i - start == run_len {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }
    None
}

/// If `bytes[start..]` is a complete `[text](url)`, return the index of the
/// closing `)`. Nested `(`/`)` inside the URL are balanced one level deep so
/// fragments like `…#method()` survive.
fn find_inline_link_end(bytes: &[u8], start: usize) -> Option<usize> {
    debug_assert_eq!(bytes[start], b'[');
    let mut j = start + 1;
    while j < bytes.len() && bytes[j] != b']' {
        if bytes[j] == b'[' {
            return None;
        }
        j += 1;
    }
    if j + 1 >= bytes.len() || bytes[j + 1] != b'(' {
        return None;
    }
    let mut depth = 1i32;
    let mut k = j + 2;
    while k < bytes.len() {
        match bytes[k] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// If `bytes[start..]` is `[display][ref]` or `[display][]`, return
/// `(display_text, ref_target, past_end_index)`.
///
/// Requires no space between the closing `]` of display and the opening `[`
/// of the ref target. Returns `None` for anything else including `[text](url)`
/// (already matched by `find_inline_link_end`) and nested brackets.
fn find_ref_link<'a>(
    bytes: &[u8],
    start: usize,
    line: &'a str,
) -> Option<(&'a str, &'a str, usize)> {
    debug_assert_eq!(bytes[start], b'[');
    let mut j = start + 1;
    while j < bytes.len() {
        if bytes[j] == b'[' {
            return None;
        }
        if bytes[j] == b']' {
            break;
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    if j + 1 >= bytes.len() || bytes[j + 1] != b'[' {
        return None;
    }
    let ref_open = j + 1;
    let mut k = ref_open + 1;
    while k < bytes.len() && bytes[k] != b']' {
        if bytes[k] == b'[' {
            return None;
        }
        k += 1;
    }
    if k >= bytes.len() {
        return None;
    }
    // Reject [text]( — that is find_inline_link_end's territory.
    if k + 1 < bytes.len() && bytes[k + 1] == b'(' {
        return None;
    }
    let display = &line[start + 1..j];
    let ref_target = &line[ref_open + 1..k];
    Some((display, ref_target, k + 1))
}

/// Escape `<` and `>` in a display or ref-target string so rustdoc doesn't
/// treat them as HTML tags (`rustdoc::invalid_html_tags` under `-D warnings`).
///
/// Content inside backtick code spans is left verbatim — escaping `\<` inside
/// a code span would render the backslash literally, corrupting the output.
/// `[` and `]` are already excluded by `find_ref_link`.
fn escape_angle_brackets(s: &str) -> String {
    if !s.contains(['<', '>']) {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 4);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run_start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let run_len = i - run_start;
            if let Some(close_end) = find_backtick_closer(bytes, i, run_len) {
                out.push_str(&s[run_start..close_end]);
                i = close_end;
            } else {
                out.push_str(&s[run_start..i]);
            }
        } else {
            match bytes[i] {
                b'<' => {
                    out.push_str("\\<");
                    i += 1;
                }
                b'>' => {
                    out.push_str("\\>");
                    i += 1;
                }
                _ => i += push_char_at(&mut out, s, i),
            }
        }
    }
    out
}

/// Escape keyword segments in a `::` -separated Rust path for use in a
/// rustdoc intra-doc link string (e.g. `google::type::LatLng` →
/// `google::r#type::LatLng`). Type-map values are crate-relative and do not
/// contain `super`/`self`/`crate`, so `escape_mod_ident` is safe to apply to
/// every segment.
fn escape_path_for_link(path: &str) -> String {
    if !path.split("::").any(crate::idents::is_rust_keyword) {
        return path.to_owned();
    }
    path.split("::")
        .map(crate::idents::escape_mod_ident)
        .collect::<Vec<_>>()
        .join("::")
}

/// Returns `true` for Rust paths that cannot be linked by prepending `crate::`.
///
/// `::` paths are global (extern crate) paths. `crate::`-prefixed values are
/// rejected because the link is constructed as `crate::{p}`, which would
/// mangle to `crate::crate::...` for an already-prefixed value. Mirrors the
/// corresponding check in `context.rs`.
fn is_extern_path(rust_path: &str) -> bool {
    rust_path.starts_with("::") || rust_path.starts_with("crate::")
}

/// Returns the Rust path from `type_map` for `type_ref` resolved from `scope_fqn`.
///
/// Tries fully-qualified first (`".{type_ref}"`), then walks up the scope
/// components stripping one segment at a time (proto lexical scoping).
fn resolve_type_fqn<'m>(
    type_ref: &str,
    scope_fqn: &str,
    type_map: &'m HashMap<String, String>,
) -> Option<&'m str> {
    let fq_key = format!(".{type_ref}");
    if let Some(path) = type_map.get(&fq_key) {
        return Some(path.as_str());
    }
    if scope_fqn.is_empty() {
        return None;
    }
    let mut scope = scope_fqn;
    loop {
        let candidate = format!(".{scope}.{type_ref}");
        if let Some(path) = type_map.get(&candidate) {
            return Some(path.as_str());
        }
        match scope.rfind('.') {
            Some(pos) => scope = &scope[..pos],
            None => break,
        }
    }
    None
}

/// Try to resolve an AIP-192 cross-reference into a rustdoc inline link string.
///
/// Returns `Some("[display](crate::rust::path)")` on success, or `None` if the
/// ref cannot be resolved (caller falls back to escaping).
///
/// `scope_fqn` is the dotless FQN of the **enclosing proto type** (message or
/// enum) — never a field/value/oneof FQN. `ref_target` may be empty for the
/// implied form `[display][]`.
///
/// Unlinkable paths (see [`is_extern_path`]) return `None`.
fn resolve_proto_ref(
    display: &str,
    ref_target: &str,
    scope_fqn: &str,
    type_map: &HashMap<String, String>,
) -> Option<String> {
    let effective_ref = if ref_target.is_empty() {
        display.trim()
    } else {
        ref_target.trim()
    };
    if effective_ref.is_empty() {
        return None;
    }

    if let Some(rust_path) = resolve_type_fqn(effective_ref, scope_fqn, type_map) {
        if is_extern_path(rust_path) {
            return None;
        }
        let d = escape_angle_brackets(display);
        let p = escape_path_for_link(rust_path);
        return Some(format!("[{d}](crate::{p})"));
    }

    None
}

/// If `bytes[start..]` is `<http(s)://…>`, return the index of the `>`.
fn find_autolink_end(bytes: &[u8], start: usize) -> Option<usize> {
    debug_assert_eq!(bytes[start], b'<');
    let rest = &bytes[start + 1..];
    if !(rest.starts_with(b"http://") || rest.starts_with(b"https://")) {
        return None;
    }
    rest.iter().position(|&b| b == b'>').map(|p| start + 1 + p)
}

/// If `bytes[start..]` begins a bare `http(s)://` URL, return the
/// past-the-end byte index. The URL ends at whitespace or `)`.
fn find_bare_url_end(bytes: &[u8], start: usize) -> Option<usize> {
    let rest = &bytes[start..];
    if !(rest.starts_with(b"http://") || rest.starts_with(b"https://")) {
        return None;
    }
    let mut j = start;
    while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b')' {
        j += 1;
    }
    Some(j)
}

/// Format a `SourceCodeInfo.Location` into a doc-comment string.
///
/// Combines leading detached comments, leading comments, and trailing
/// comments. Returns `None` if no comments are present.
///
/// Proto comments use `//` or `/* */` syntax. protoc strips the leading
/// `// ` or ` * ` prefix and stores plain text. Each line is separated by
/// `\n`. We preserve this structure so that `#[doc = "..."]` renders
/// correctly in rustdoc.
///
/// Leading newlines and trailing whitespace are stripped, but leading
/// spaces on the first content line are preserved so that indented code
/// blocks survive for the fencing heuristic in [`doc_lines_to_tokens`].
///
/// When multiple parts (detached, leading, trailing) are present they are
/// joined with a blank line. If an indented code block spans across parts,
/// it will be fenced as two separate `text` blocks — this is a known
/// limitation and acceptable since each proto comment section is
/// conceptually distinct.
fn format_comment(
    location: &crate::generated::descriptor::source_code_info::Location,
) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();

    for detached in &location.leading_detached_comments {
        let trimmed = detached.trim_start_matches('\n').trim_end();
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
    }

    if let Some(ref leading) = location.leading_comments {
        let trimmed = leading.trim_start_matches('\n').trim_end();
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
    }

    if let Some(ref trailing) = location.trailing_comments {
        let trimmed = trailing.trim_start_matches('\n').trim_end();
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::source_code_info::Location;
    use crate::generated::descriptor::{
        EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto, OneofDescriptorProto,
        SourceCodeInfo,
    };

    fn make_location(path: Vec<i32>, leading: Option<&str>, trailing: Option<&str>) -> Location {
        Location {
            path,
            leading_comments: leading.map(|s| s.to_string()),
            trailing_comments: trailing.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    fn make_file_with_locations(
        package: &str,
        messages: Vec<DescriptorProto>,
        enums: Vec<EnumDescriptorProto>,
        locations: Vec<Location>,
    ) -> FileDescriptorProto {
        FileDescriptorProto {
            package: Some(package.to_string()),
            message_type: messages,
            enum_type: enums,
            source_code_info: SourceCodeInfo {
                location: locations,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }
    }

    fn make_field(name: &str) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    fn make_enum(name: &str, values: &[&str]) -> EnumDescriptorProto {
        EnumDescriptorProto {
            name: Some(name.to_string()),
            value: values
                .iter()
                .enumerate()
                .map(|(i, v)| EnumValueDescriptorProto {
                    name: Some(v.to_string()),
                    number: Some(i as i32),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_empty_source_code_info() {
        let file = FileDescriptorProto::default();
        let map = fqn_comments(&file);
        assert!(map.is_empty());
    }

    #[test]
    fn test_message_comment() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Person".to_string()),
                ..Default::default()
            }],
            vec![],
            vec![make_location(vec![4, 0], Some("A test message.\n"), None)],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Person").map(|s| s.as_str()),
            Some("A test message.")
        );
    }

    #[test]
    fn test_field_comment() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("User".to_string()),
                field: vec![make_field("email")],
                ..Default::default()
            }],
            vec![],
            vec![make_location(
                vec![4, 0, 2, 0],
                Some("The user's email.\n"),
                None,
            )],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.User.email").map(|s| s.as_str()),
            Some("The user's email.")
        );
    }

    #[test]
    fn test_enum_and_value_comments() {
        let file = make_file_with_locations(
            "pkg",
            vec![],
            vec![make_enum("Status", &["UNKNOWN", "ACTIVE"])],
            vec![
                make_location(vec![5, 0], Some("Status enum.\n"), None),
                make_location(vec![5, 0, 2, 0], None, Some("Unknown status.\n")),
                make_location(vec![5, 0, 2, 1], Some("Active status.\n"), None),
            ],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Status").map(|s| s.as_str()),
            Some("Status enum.")
        );
        assert_eq!(
            map.get("pkg.Status.UNKNOWN").map(|s| s.as_str()),
            Some("Unknown status.")
        );
        assert_eq!(
            map.get("pkg.Status.ACTIVE").map(|s| s.as_str()),
            Some("Active status.")
        );
    }

    #[test]
    fn test_oneof_comment() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Event".to_string()),
                oneof_decl: vec![OneofDescriptorProto {
                    name: Some("payload".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            vec![],
            vec![make_location(
                vec![4, 0, 8, 0],
                Some("The payload.\n"),
                None,
            )],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Event.payload").map(|s| s.as_str()),
            Some("The payload.")
        );
    }

    #[test]
    fn test_nested_message_comment() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Outer".to_string()),
                nested_type: vec![DescriptorProto {
                    name: Some("Inner".to_string()),
                    field: vec![make_field("value")],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            vec![],
            vec![
                make_location(vec![4, 0, 3, 0], Some("A nested type.\n"), None),
                make_location(vec![4, 0, 3, 0, 2, 0], Some("The value.\n"), None),
            ],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Outer.Inner").map(|s| s.as_str()),
            Some("A nested type.")
        );
        assert_eq!(
            map.get("pkg.Outer.Inner.value").map(|s| s.as_str()),
            Some("The value.")
        );
    }

    #[test]
    fn test_nested_enum_in_message_comment() {
        // Path [4, 0, 4, 0] = message_type[0].enum_type[0] (MSG_ENUM_TYPE = 4).
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Container".to_string()),
                enum_type: vec![make_enum("Kind", &["UNSET", "A"])],
                ..Default::default()
            }],
            vec![],
            vec![
                make_location(vec![4, 0, 4, 0], Some("Kind of thing.\n"), None),
                make_location(vec![4, 0, 4, 0, 2, 1], Some("The A kind.\n"), None),
            ],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Container.Kind").map(|s| s.as_str()),
            Some("Kind of thing.")
        );
        assert_eq!(
            map.get("pkg.Container.Kind.A").map(|s| s.as_str()),
            Some("The A kind.")
        );
    }

    #[test]
    fn test_leading_and_trailing_combined() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Msg".to_string()),
                ..Default::default()
            }],
            vec![],
            vec![make_location(
                vec![4, 0],
                Some("Leading.\n"),
                Some("Trailing.\n"),
            )],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Msg").map(|s| s.as_str()),
            Some("Leading.\n\nTrailing.")
        );
    }

    #[test]
    fn test_detached_comments() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Msg".to_string()),
                ..Default::default()
            }],
            vec![],
            vec![{
                let mut loc = make_location(vec![4, 0], Some("Main.\n"), None);
                loc.leading_detached_comments = vec!["Detached.\n".to_string()];
                loc
            }],
        );
        let map = fqn_comments(&file);
        assert_eq!(
            map.get("pkg.Msg").map(|s| s.as_str()),
            Some("Detached.\n\nMain.")
        );
    }

    #[test]
    fn test_whitespace_only_comments_ignored() {
        let file = make_file_with_locations(
            "pkg",
            vec![DescriptorProto {
                name: Some("Msg".to_string()),
                ..Default::default()
            }],
            vec![],
            vec![make_location(vec![4, 0], Some("  \n  "), Some("  "))],
        );
        let map = fqn_comments(&file);
        assert!(!map.contains_key("pkg.Msg"));
    }

    #[test]
    fn test_empty_package() {
        let file = make_file_with_locations(
            "",
            vec![DescriptorProto {
                name: Some("Root".to_string()),
                field: vec![make_field("id")],
                ..Default::default()
            }],
            vec![],
            vec![
                make_location(vec![4, 0], Some("Root msg.\n"), None),
                make_location(vec![4, 0, 2, 0], Some("The id.\n"), None),
            ],
        );
        let map = fqn_comments(&file);
        assert_eq!(map.get("Root").map(|s| s.as_str()), Some("Root msg."));
        assert_eq!(map.get("Root.id").map(|s| s.as_str()), Some("The id."));
    }

    // --- doc_lines_to_tokens -----------------------------------------------

    fn doc_tokens(text: &str) -> String {
        doc_lines_to_tokens(text).to_string()
    }

    #[test]
    fn test_doc_plain_text_gets_leading_space() {
        let out = doc_tokens("hello world");
        assert_eq!(out, "# [doc = \" hello world\"]");
    }

    #[test]
    fn test_doc_line_already_spaced_kept_as_is() {
        let out = doc_tokens(" already spaced");
        assert_eq!(out, "# [doc = \" already spaced\"]");
    }

    #[test]
    fn test_doc_empty_line_preserved() {
        let out = doc_tokens("a\n\nb");
        assert_eq!(out, "# [doc = \" a\"] # [doc = \"\"] # [doc = \" b\"]");
    }

    #[test]
    fn test_doc_indented_block_gets_text_fence() {
        let out = doc_tokens("Example:\n    x = 1;\n    y = 2;");
        assert!(out.contains("```text"), "should open text fence: {out}");
        assert!(out.contains("\" x = 1;\""), "indent stripped: {out}");
        assert!(out.ends_with("# [doc = \" ```\"]"), "should close: {out}");
    }

    #[test]
    fn test_doc_blank_line_within_indented_block_keeps_fence_open() {
        let out = doc_tokens("    line1\n\n    line2");
        let fence_count = out.matches("```").count();
        assert_eq!(
            fence_count, 2,
            "one open + one close, not two blocks: {out}"
        );
    }

    #[test]
    fn test_doc_trailing_unclosed_block_gets_closing_fence() {
        let out = doc_tokens("text\n    code");
        assert!(out.ends_with("# [doc = \" ```\"]"), "trailing close: {out}");
    }

    #[test]
    fn test_doc_tab_indent_detected() {
        let out = doc_tokens("\tcode line");
        assert!(out.contains("```text"), "tab triggers fence: {out}");
    }

    #[test]
    fn test_doc_indent_measured_in_columns_not_literal_spaces() {
        // CommonMark expands a tab to the next 4-column stop, so 1-3 spaces
        // followed by a tab reaches column 4 and is an indented code block —
        // rustdoc compiles such a line as a doctest unless it is fenced.
        for prefix in [" \t", "  \t", "   \t", "\t"] {
            let out = doc_tokens(&format!("{prefix}this is not rust !!!"));
            assert!(
                out.contains("```text"),
                "{prefix:?} reaches column 4 and must be fenced: {out}"
            );
        }
        // Under four columns stays prose.
        for prefix in [" ", "  ", "   "] {
            let out = doc_tokens(&format!("{prefix}just prose"));
            assert!(
                !out.contains("```text"),
                "{prefix:?} is under column 4 and must stay prose: {out}"
            );
        }
    }

    #[test]
    fn test_doc_empty_input() {
        assert_eq!(doc_tokens(""), "");
    }

    #[test]
    fn test_doc_user_markdown_fence_keeps_language_and_gains_ignore() {
        // Proto authors may write markdown fences directly. The author's
        // language survives (so the block still highlights) and the content
        // is not double-fenced, but the opener gains `ignore` so rustdoc
        // cannot compile the body as a doctest.
        let out = doc_tokens("Example:\n```go\nx := 1\n```");
        assert!(out.contains("\" ```go,ignore\""), "opener inerted: {out}");
        assert_eq!(
            out.matches("```").count(),
            2,
            "user fence preserved, not double-fenced: {out}"
        );
        assert!(!out.contains("```text"), "no synthetic fence: {out}");
    }

    #[test]
    fn test_doc_user_fence_with_indented_content_not_double_fenced() {
        // Edge case: user-written fence with 4-space-indented content inside.
        // The indented-block heuristic must not fire inside an existing fence.
        let out = doc_tokens("```\n    int x = 1;\n```");
        assert_eq!(
            out.matches("```").count(),
            2,
            "no nested fence inside user fence: {out}"
        );
    }

    // --- format_comment indentation preservation ----------------------------

    #[test]
    fn test_format_comment_preserves_leading_indent() {
        let loc = Location {
            leading_comments: Some("    int x = 1;\n    int y = 2;\n".to_string()),
            ..Default::default()
        };
        let out = format_comment(&loc).unwrap();
        assert!(
            out.starts_with("    "),
            "leading indent must survive for fencing: {out:?}"
        );
    }

    #[test]
    fn test_format_comment_strips_leading_newlines_keeps_spaces() {
        let loc = Location {
            leading_comments: Some("\n\n hello\n".to_string()),
            ..Default::default()
        };
        assert_eq!(format_comment(&loc).as_deref(), Some(" hello"));
    }

    // --- sanitize_line_with_refs (base escaping, no proto context) -------------
    // These cases use an empty type_map and empty scope so all proto refs fall
    // back to escaping — identical to the old sanitize_line behaviour.

    fn sl(line: &str) -> String {
        sanitize_line_with_refs(line, "", &HashMap::new())
    }

    #[test]
    fn test_sanitize_line() {
        let cases: &[(&str, &str, &str)] = &[
            ("plain text", "plain text", "plain"),
            ("hello world", "hello world", "h_not_url"),
            // Brackets
            (
                "see [google.protobuf.Duration][]",
                r"see \[google.protobuf.Duration\]\[\]",
                "collapsed_ref_link",
            ),
            ("a [foo] b", r"a \[foo\] b", "shortcut_link"),
            ("[foo][bar]", r"\[foo\]\[bar\]", "full_ref_link"),
            ("[.{frac_sec}]Z", r"\[.{frac_sec}\]Z", "format_string"),
            // Inline links preserved
            (
                "[RFC 3339](https://ietf.org/rfc/rfc3339.txt)",
                "[RFC 3339](https://ietf.org/rfc/rfc3339.txt)",
                "inline_link",
            ),
            (
                "[m()](https://e.com/#m())",
                "[m()](https://e.com/#m())",
                "inline_link_nested_parens",
            ),
            // Already escaped
            (r"\[foo\]", r"\[foo\]", "pre_escaped"),
            // Backtick spans untouched
            ("`[foo]` bar", "`[foo]` bar", "backtick_brackets"),
            ("`Option<T>` bar", "`Option<T>` bar", "backtick_generics"),
            ("``[foo]``", "``[foo]``", "double_backtick"),
            ("`` `<T>` ``", "`` `<T>` ``", "double_backtick_inner"),
            ("``` x ` y ```", "``` x ` y ```", "triple_backtick"),
            ("`` no closer", "`` no closer", "unclosed_backticks"),
            (
                "[résumé](http://e.com)",
                "[résumé](http://e.com)",
                "utf8_link_text",
            ),
            // Bare URLs wrapped
            (
                "see https://example.com/x for details",
                "see <https://example.com/x> for details",
                "bare_url",
            ),
            (
                "(https://example.com)",
                "(<https://example.com>)",
                "bare_url_in_parens",
            ),
            // Existing autolinks preserved
            ("<https://example.com>", "<https://example.com>", "autolink"),
            // Angle brackets escaped
            ("Option<T>", r"Option\<T\>", "generics"),
            ("HashMap<K, V>", r"HashMap\<K, V\>", "generics_multi"),
            // UTF-8 passthrough
            ("café — ok", "café — ok", "utf8"),
            ("`café` [x]", r"`café` \[x\]", "utf8_backtick"),
        ];
        for (input, want, name) in cases {
            assert_eq!(sl(input), *want, "case: {name}");
        }
    }

    #[test]
    fn test_sanitize_line_unbalanced() {
        // Unmatched delimiters are escaped, not crashed on.
        assert_eq!(sl("[foo"), r"\[foo");
        assert_eq!(sl("foo]"), r"foo\]");
        assert_eq!(sl("[foo]("), r"\[foo\](");
        assert_eq!(sl("<http://x"), r"\<<http://x>");
        assert_eq!(sl("a > b"), r"a \> b");
        assert_eq!(sl("trailing \\"), "trailing \\");
    }

    // ── find_ref_link ──────────────────────────────────────────────────────────

    #[test]
    fn test_find_ref_link_full_form() {
        let line = "[Book][google.example.v1.Book]";
        let bytes = line.as_bytes();
        let result = find_ref_link(bytes, 0, line);
        assert_eq!(result, Some(("Book", "google.example.v1.Book", line.len())));
    }

    #[test]
    fn test_find_ref_link_implied_form() {
        let line = "[Book][]";
        let bytes = line.as_bytes();
        let result = find_ref_link(bytes, 0, line);
        assert_eq!(result, Some(("Book", "", line.len())));
    }

    #[test]
    fn test_find_ref_link_not_matched_for_inline_link() {
        let line = "[text](https://example.com)";
        let bytes = line.as_bytes();
        assert_eq!(find_ref_link(bytes, 0, line), None);
    }

    #[test]
    fn test_find_ref_link_bare_bracket_not_matched() {
        let line = "[foo]";
        let bytes = line.as_bytes();
        assert_eq!(find_ref_link(bytes, 0, line), None);
    }

    // ── resolve_proto_ref ──────────────────────────────────────────────────────

    #[test]
    fn test_resolve_proto_ref_fully_qualified() {
        let mut map = HashMap::new();
        map.insert(
            ".google.example.v1.Book".into(),
            "google::example::v1::Book".into(),
        );
        let result = resolve_proto_ref("Book", "google.example.v1.Book", "any.scope", &map);
        assert_eq!(
            result.as_deref(),
            Some("[Book](crate::google::example::v1::Book)")
        );
    }

    #[test]
    fn test_resolve_proto_ref_scope_relative() {
        let mut map = HashMap::new();
        map.insert(
            ".google.example.v1.Genre".into(),
            "google::example::v1::Genre".into(),
        );
        let result = resolve_proto_ref("Genre", "Genre", "google.example.v1.Book", &map);
        assert_eq!(
            result.as_deref(),
            Some("[Genre](crate::google::example::v1::Genre)")
        );
    }

    #[test]
    fn test_resolve_proto_ref_implied_form() {
        let mut map = HashMap::new();
        map.insert(
            ".google.example.v1.Book".into(),
            "google::example::v1::Book".into(),
        );
        let result = resolve_proto_ref("Book", "", "google.example.v1.Library", &map);
        assert_eq!(
            result.as_deref(),
            Some("[Book](crate::google::example::v1::Book)")
        );
    }

    #[test]
    fn test_resolve_proto_ref_member_ref_returns_none() {
        let mut map = HashMap::new();
        map.insert(".pkg.Genre".into(), "pkg::Genre".into());
        assert_eq!(
            resolve_proto_ref("Genre.GENRE_SCI_FI", "", "pkg.Book", &map),
            None,
        );
        assert_eq!(
            resolve_proto_ref("X", "pkg.Genre.GENRE_SCI_FI", "pkg.Book", &map),
            None,
        );
    }

    #[test]
    fn test_resolve_proto_ref_extern_returns_none() {
        let mut map = HashMap::new();
        map.insert(
            ".google.protobuf.Timestamp".into(),
            "::buffa_types::google::protobuf::Timestamp".into(),
        );
        let result =
            resolve_proto_ref("Timestamp", "google.protobuf.Timestamp", "my.pkg.Msg", &map);
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_proto_ref_crate_extern_returns_none() {
        // extern_path mappings that start with `crate::` must also be rejected —
        // they live in another crate re-exported under this crate's root and
        // cannot be linked with `crate::crate::...`.
        let mut map = HashMap::new();
        map.insert(
            ".google.api.Foo".into(),
            "crate::vendored::google::api::Foo".into(),
        );
        let result = resolve_proto_ref("Foo", "google.api.Foo", "my.pkg.Msg", &map);
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_proto_ref_keyword_segment_escaped() {
        // Package `google.type` → path `google::type::LatLng`; `type` must be
        // escaped to `r#type` so rustdoc can resolve the intra-doc link.
        let mut map = HashMap::new();
        map.insert(".google.type.LatLng".into(), "google::type::LatLng".into());
        let result = resolve_proto_ref("LatLng", "google.type.LatLng", "my.pkg.Msg", &map);
        assert_eq!(
            result.as_deref(),
            Some("[LatLng](crate::google::r#type::LatLng)")
        );
    }

    #[test]
    fn test_resolve_proto_ref_unknown_returns_none() {
        let map = HashMap::new();
        let result = resolve_proto_ref("Foo", "NoSuchType", "my.pkg.Msg", &map);
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_proto_ref_empty_effective_ref_returns_none() {
        let map = HashMap::new();
        let result = resolve_proto_ref("", "", "my.pkg.Msg", &map);
        assert_eq!(result, None);
    }

    // ── sanitize_line_with_refs ────────────────────────────────────────────────

    #[test]
    fn test_sanitize_with_refs_resolves_fq() {
        let mut map = HashMap::new();
        map.insert(
            ".google.example.v1.Book".into(),
            "google::example::v1::Book".into(),
        );
        let out = sanitize_line_with_refs(
            "See [Book][google.example.v1.Book] for details.",
            "google.example.v1.Library",
            &map,
        );
        assert!(
            out.contains("[Book](crate::google::example::v1::Book)"),
            "got: {out}"
        );
    }

    #[test]
    fn test_sanitize_with_refs_falls_back_for_unknown() {
        let out = sanitize_line_with_refs("[Foo][unknown.Type]", "my.pkg.Msg", &HashMap::new());
        assert_eq!(out, r"\[Foo\]\[unknown.Type\]");
    }

    #[test]
    fn test_sanitize_with_refs_preserves_inline_links() {
        let line = "[RFC 3339](https://ietf.org/rfc/rfc3339.txt)";
        let out = sanitize_line_with_refs(line, "my.pkg.Msg", &HashMap::new());
        assert_eq!(out, line);
    }

    #[test]
    fn test_sanitize_with_refs_implied_form() {
        let mut map = HashMap::new();
        map.insert(".my.pkg.Foo".into(), "my::pkg::Foo".into());
        let out = sanitize_line_with_refs("See [Foo][].", "my.pkg.Bar", &map);
        assert!(out.contains("[Foo](crate::my::pkg::Foo)"), "got: {out}");
    }

    #[test]
    fn test_sanitize_with_refs_display_angle_brackets_escaped() {
        // Display text containing < or > must be escaped (invalid_html_tags lint).
        let mut map = HashMap::new();
        map.insert(".my.pkg.Foo".into(), "my::pkg::Foo".into());
        // Resolved: display gets < > escaped.
        let out = sanitize_line_with_refs("[Foo<T>][my.pkg.Foo]", "my.pkg.Bar", &map);
        assert_eq!(out, r"[Foo\<T\>](crate::my::pkg::Foo)");
        // Fallback: display also gets < > escaped.
        let out2 = sanitize_line_with_refs("[Foo<T>][unknown.Type]", "my.pkg.Bar", &HashMap::new());
        assert_eq!(out2, r"\[Foo\<T\>\]\[unknown.Type\]");
    }

    #[test]
    fn test_sanitize_with_refs_display_backtick_span_preserved() {
        // < > inside a backtick code span in the display must NOT be escaped —
        // \< inside a code span renders the backslash literally.
        let mut map = HashMap::new();
        map.insert(".my.pkg.Foo".into(), "my::pkg::Foo".into());
        let out = sanitize_line_with_refs("[`Option<T>`][my.pkg.Foo]", "my.pkg.Bar", &map);
        assert_eq!(out, "[`Option<T>`](crate::my::pkg::Foo)");
        // Fallback: backtick span inside display still preserved.
        let out2 =
            sanitize_line_with_refs("[`Option<T>`][unknown.Type]", "my.pkg.Bar", &HashMap::new());
        assert_eq!(out2, r"\[`Option<T>`\]\[unknown.Type\]");
    }

    #[test]
    fn test_sanitize_with_refs_ref_target_angle_brackets_escaped() {
        // < > in a ref_target (malformed proto FQN) must also be escaped on
        // the fallback path so they don't trigger invalid_html_tags.
        let out = sanitize_line_with_refs("[Foo][a<b>c]", "my.pkg.Msg", &HashMap::new());
        assert_eq!(out, r"\[Foo\]\[a\<b\>c\]");
    }

    #[test]
    fn test_sanitize_with_refs_code_span_untouched() {
        let mut map = HashMap::new();
        map.insert(".my.pkg.Foo".into(), "my::pkg::Foo".into());
        let out = sanitize_line_with_refs("`[Foo][my.pkg.Foo]`", "my.pkg.Bar", &map);
        assert!(
            out.contains("`[Foo][my.pkg.Foo]`"),
            "code span unchanged: {out}"
        );
    }

    #[test]
    fn test_doc_tokens_sanitizes_prose_not_code() {
        // Indented code block content must NOT be sanitized.
        let out = doc_tokens("Prose [foo].\n    code [bar]\nMore.");
        assert!(out.contains(r"\\[foo\\]"), "prose escaped: {out}");
        assert!(out.contains("code [bar]"), "code untouched: {out}");
        // User-written fence content must NOT be sanitized.
        let out = doc_tokens("```\n[x](y)\nOption<T>\n```");
        assert!(out.contains("Option<T>"), "fence untouched: {out}");
    }

    // --- fence inerting ------------------------------------------------------

    /// Every one of these reaches rustdoc's compiler without an `ignore`:
    /// an unannotated fence is Rust by default, `no_run` still type-checks,
    /// `should_panic` runs, `compile_fail` inverts the verdict, an error code
    /// or an mdBook-style word keeps the block Rust, and `ignore-<target>` is
    /// a target *list* that replaces a plain `ignore`, so the block compiles
    /// on every other target.
    #[test]
    fn test_fence_info_strings_are_made_inert() {
        let cases = [
            ("", "text"),
            ("rust", "rust,ignore"),
            ("no_run", "no_run,ignore"),
            ("should_panic", "should_panic,ignore"),
            ("compile_fail", "compile_fail,ignore"),
            ("compile_fail,E0277", "compile_fail,E0277,ignore"),
            ("rust,noplayground", "rust,noplayground,ignore"),
            ("edition2018", "edition2018,ignore"),
            // `ignore-<target>` is dropped, not kept alongside `ignore`.
            ("ignore-wasm32", "rust,ignore"),
            ("rust,ignore-wasm32", "rust,ignore"),
            // Non-Rust fences keep their language; the added `ignore` is
            // inert for them, and uniformity beats classifying.
            ("json", "json,ignore"),
            ("proto", "proto,ignore"),
        ];
        for (info, expected) in cases {
            assert_eq!(fence_info(info), expected, "info string: {info:?}");
        }
    }

    #[test]
    fn test_already_ignored_fences_are_untouched() {
        for info in ["rust,ignore", "ignore", "ignore,json"] {
            assert_eq!(fence_info(info), info, "info string: {info}");
        }
    }

    #[test]
    fn test_bare_fence_body_is_not_a_doctest() {
        let out = doc_tokens("Example:\n```\n{\"a\": 1}\n```");
        assert!(out.contains("\" ```text\""), "expected text fence: {out}");
    }

    #[test]
    fn test_tilde_fences_are_inerted_too() {
        // rustdoc's markdown parser treats ~~~ exactly like ``` — a tilde
        // fence's body is compiled as a doctest just the same.
        let out = doc_tokens("~~~rust\nlet x = 1;\n~~~");
        assert!(out.contains("\" ~~~rust,ignore\""), "opener inerted: {out}");

        let bare = doc_tokens("~~~\n{\"a\": 1}\n~~~");
        assert!(bare.contains("\" ~~~text\""), "bare tilde fence: {bare}");

        // A tilde fence is not closed by a backtick run, and vice versa.
        let unterminated = doc_tokens("~~~\nx\n```");
        assert!(
            unterminated.ends_with("# [doc = \" ~~~\"]"),
            "closed with its own fence char: {unterminated}"
        );
    }

    #[test]
    fn test_unterminated_fence_does_not_swallow_the_field_tag() {
        // The tag is rendered separately from the comment, so a fence the
        // author left open cannot pull "Field 1: `name`" into the code
        // block (and out of the rendered docs).
        let out = doc_attrs_with_tag_resolved(
            Some("Example:\n```\nfoo"),
            "Field 1: `name`",
            "",
            &HashMap::new(),
        )
        .to_string();
        let fence_close = out.find("# [doc = \" ```\"]").expect("synthetic closer");
        let tag = out.find("Field 1").expect("tag emitted");
        assert!(
            fence_close < tag,
            "the fence must be closed before the tag: {out}"
        );
    }

    #[test]
    fn test_exotic_leading_whitespace_is_not_a_fence() {
        // Only spaces (0-3) may precede a fence marker. rustdoc reads an
        // NBSP-prefixed run as prose, and so must we — otherwise we "close"
        // a fence rustdoc never opened and leave the run it *does* treat as
        // an opener unguarded.
        let out = doc_tokens("\u{a0}```\nlet x = 1;\n```");
        assert!(
            out.contains("\" ```text\""),
            "the real opener must still be inerted: {out}"
        );
    }

    #[test]
    fn test_backtick_in_info_string_is_not_a_fence() {
        // CommonMark: a backtick fence's info string may not contain a
        // backtick, so rustdoc reads this line as prose, not an opener.
        // Treating it as a fence would desync us from rustdoc and let the
        // *next* ``` open an unannotated (Rust) block.
        let out = doc_tokens("```rust```\nx");
        assert!(!out.contains("ignore"), "not treated as a fence: {out}");
    }

    #[test]
    fn test_unterminated_fence_is_closed() {
        // Otherwise it swallows every doc line emitted after the comment.
        let out = doc_tokens("prose\n```\nlet x = 1;");
        assert!(
            out.ends_with("# [doc = \" ```\"]"),
            "expected a synthetic closer: {out}"
        );
    }

    #[test]
    fn test_longer_fence_not_closed_by_shorter_run() {
        // CommonMark: a closer needs at least the opener's tick count, so
        // the inner ``` line is content, not the end of the fence.
        let out = doc_tokens("````\n```\ncode\n````");
        assert!(out.contains("\" ````text\""), "opener rewritten: {out}");
        assert_eq!(
            out.matches("# [doc = \" ````\"]").count(),
            1,
            "the 4-tick closer must be the only one: {out}"
        );
    }

    #[test]
    fn test_indented_fence_marker_is_code_not_fence() {
        // CommonMark: a marker indented 4+ spaces is an indented code block.
        // The marker keeps its indentation inside the synthetic `text`
        // fence, so it cannot close it early.
        let out = doc_tokens("    ```\n    x");
        assert!(out.contains("\" ```text\""), "expected text fence: {out}");
        assert!(
            out.contains("\"    ```\""),
            "marker kept indented, as content: {out}"
        );
    }
}

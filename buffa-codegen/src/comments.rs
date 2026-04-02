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

/// Convert a comment string into `#[doc = "..."]` token stream attributes.
///
/// Each line of the comment becomes a separate `#[doc = "..."]` attribute
/// so that rustdoc renders them as a contiguous doc block. Returns empty
/// tokens if the comment is `None`.
pub fn doc_attrs(comment: Option<&str>) -> TokenStream {
    match comment {
        None => quote! {},
        Some(text) => doc_lines_to_tokens(text),
    }
}

/// Combine an optional proto comment with a tag line into `#[doc = "..."]` attrs.
///
/// If a proto comment is present, a blank `#[doc = ""]` separator is inserted
/// between it and the tag line so that rustdoc renders them as separate
/// paragraphs.
pub fn doc_attrs_with_tag(comment: Option<&str>, tag: &str) -> TokenStream {
    match comment {
        None => doc_lines_to_tokens(tag),
        Some(text) => {
            let combined = format!("{text}\n\n{tag}");
            doc_lines_to_tokens(&combined)
        }
    }
}

/// Convert text into `#[doc = " ..."]` tokens, ensuring each non-empty line
/// has a leading space so that `prettyplease` renders `/// text` instead of
/// `///text`.
///
/// Indented code blocks (4+ spaces) from proto source comments contain
/// C++/Java/Python examples, not Rust. We wrap them in ```` ```text ````
/// fences so rustdoc renders them as plain text instead of trying to
/// compile them as Rust doc tests.
fn doc_lines_to_tokens(text: &str) -> TokenStream {
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut lines: Vec<String> = Vec::with_capacity(raw_lines.len());
    let mut in_code_block = false;
    let mut in_user_fence = false;

    for (idx, line) in raw_lines.iter().enumerate() {
        // Proto authors may write markdown fences directly. Pass them
        // through and suppress the indented-block heuristic inside so we
        // don't nest a synthetic ```text fence.
        if line.trim_start().starts_with("```") && !in_code_block {
            in_user_fence = !in_user_fence;
            lines.push(if line.starts_with(' ') {
                line.to_string()
            } else {
                format!(" {line}")
            });
            continue;
        }
        if in_user_fence {
            lines.push(if line.is_empty() {
                String::new()
            } else if line.starts_with(' ') {
                line.to_string()
            } else {
                format!(" {line}")
            });
            continue;
        }

        let is_indented = line.starts_with("    ") || line.starts_with('\t');

        if is_indented && !in_code_block {
            // Open a text fence before the first indented line.
            lines.push(" ```text".to_string());
            in_code_block = true;
        } else if in_code_block && !is_indented {
            // Non-indented line (including empty) closes the code block,
            // but only if there isn't another indented line coming next.
            if line.is_empty() {
                // Look ahead: if the next non-empty line is indented, keep
                // the block open (it's a blank line within the example).
                let next_is_indented = raw_lines[idx + 1..]
                    .iter()
                    .find(|l| !l.is_empty())
                    .is_some_and(|l| l.starts_with("    ") || l.starts_with('\t'));
                if next_is_indented {
                    lines.push(String::new());
                    continue;
                }
            }
            lines.push(" ```".to_string());
            in_code_block = false;
        }

        if in_code_block {
            // Strip the 4-space / tab indent since we're inside a fence.
            let stripped = line
                .strip_prefix("    ")
                .or_else(|| line.strip_prefix('\t'))
                .unwrap_or(line);
            if stripped.is_empty() {
                lines.push(String::new());
            } else if stripped.starts_with(' ') {
                lines.push(stripped.to_string());
            } else {
                lines.push(format!(" {stripped}"));
            }
        } else if line.is_empty() {
            lines.push(String::new());
        } else {
            let sanitized = sanitize_line(line);
            if sanitized.starts_with(' ') {
                lines.push(sanitized);
            } else {
                lines.push(format!(" {sanitized}"));
            }
        }
    }

    if in_code_block {
        lines.push(" ```".to_string());
    }

    quote! {
        #( #[doc = #lines] )*
    }
}

/// Escape one prose line of proto comment text for rustdoc.
///
/// Proto comments are written for a cross-language audience and frequently
/// contain constructs that rustdoc misparses:
///
/// - `[foo]` / `[foo][]` — treated as intra-doc links; an error under
///   `deny(rustdoc::broken_intra_doc_links)` when `foo` resembles a Rust
///   path. Escaped to `\[foo\]`.
/// - Bare `http(s)://…` — triggers `rustdoc::bare_urls`. Wrapped in `<…>`.
/// - `Option<T>` — treated as raw HTML; triggers
///   `rustdoc::invalid_html_tags`. Escaped to `Option\<T\>`.
///
/// Left intact:
/// - Single-line inline links `[text](url)`.
/// - Existing autolinks `<http(s)://…>`.
/// - Content inside `` `…` `` backtick code spans.
/// - Already-escaped `\[`, `\]`, `\<`, `\>`.
///
/// This is a per-line pass invoked from [`doc_lines_to_tokens`] on prose
/// lines only — code blocks are left untouched. Multi-line markdown links
/// are conservatively escaped; the link degrades to literal text plus a
/// clickable autolink, which is preferable to a docs.rs build failure.
fn sanitize_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'`' {
            // CommonMark code span: a run of N backticks opens, the next run
            // of exactly N closes. Emit the whole span verbatim. If no
            // closer exists on this line, the run is literal (not a span).
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
    fn test_doc_empty_input() {
        assert_eq!(doc_tokens(""), "");
    }

    #[test]
    fn test_doc_user_markdown_fence_passes_through() {
        // Proto authors may write markdown fences directly. These should
        // pass through to rustdoc unmodified — no extra `text` fence added.
        let out = doc_tokens("Example:\n```go\nx := 1\n```");
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

    // --- sanitize_line ------------------------------------------------------

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
            assert_eq!(sanitize_line(input), *want, "case: {name}");
        }
    }

    #[test]
    fn test_sanitize_line_unbalanced() {
        // Unmatched delimiters are escaped, not crashed on.
        assert_eq!(sanitize_line("[foo"), r"\[foo");
        assert_eq!(sanitize_line("foo]"), r"foo\]");
        assert_eq!(sanitize_line("[foo]("), r"\[foo\](");
        assert_eq!(sanitize_line("<http://x"), r"\<<http://x>");
        assert_eq!(sanitize_line("a > b"), r"a \> b");
        assert_eq!(sanitize_line("trailing \\"), "trailing \\");
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
}

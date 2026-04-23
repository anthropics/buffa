//! Buffa protobuf conformance test binary.
//!
//! Implements the protocol expected by `conformance_test_runner`:
//!   stdin  → [u32-LE length][ConformanceRequest bytes]  (repeated)
//!   stdout → [u32-LE length][ConformanceResponse bytes] (repeated)
//!
//! The envelope is decoded by the hand-rolled parser in `envelope.rs`;
//! `TestAllTypesProto3` is decoded/re-encoded by buffa-generated code.
//!
//! When `conformance/protos/` has not been populated the binary compiles to
//! a stub that prints an error and exits.  Run `task fetch-protos` first.

#![allow(
    clippy::derivable_impls,
    clippy::match_single_binding,
    clippy::useless_asref,
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_variables
)]

mod envelope;

// ── Generated code (only compiled when protos are present) ───────────────

// Well-known types from buffa-types, re-exported under the nested module path
// that the generated test-message code expects (`google::protobuf::*`).  The
// serde impls for proto3 JSON encoding live in buffa-types itself, so no custom
// conformance-only serde code is needed.
#[cfg(not(no_protos))]
pub use buffa_types::google;

// Test messages are included under their package hierarchy so that
// cross-package references (e.g. protobuf_test_messages::proto3::ForeignMessage
// from proto2 code) resolve correctly.
#[cfg(not(no_protos))]
pub mod protobuf_test_messages {
    pub use crate::google;

    pub mod proto3 {
        pub use super::google;
        buffa::include_proto!("protobuf_test_messages.proto3");
    }

    pub mod proto2 {
        pub use super::google;
        buffa::include_proto!("protobuf_test_messages.proto2");
    }
}

// Re-export for convenience in the rest of this binary.
#[cfg(not(no_protos))]
pub use protobuf_test_messages::proto2;
#[cfg(not(no_protos))]
pub use protobuf_test_messages::proto3;

// Editions test messages: proto3/proto2 behavior expressed via edition = "2023".
#[cfg(has_editions_protos)]
pub mod protobuf_test_messages_editions {
    pub use crate::google;

    pub mod proto3 {
        pub use super::google;
        buffa::include_proto!("protobuf_test_messages.editions.proto3");
    }

    pub mod proto2 {
        pub use super::google;
        buffa::include_proto!("protobuf_test_messages.editions.proto2");
    }

    // Pure edition 2023: file-level DELIMITED message encoding. Binary-only
    // — no JSON generation. The package is `protobuf_test_messages.editions`
    // so the module path here matches where the suite expects to find it.
    buffa::include_proto!("protobuf_test_messages.editions");
}

#[cfg(has_editions_protos)]
pub use protobuf_test_messages_editions::proto2 as editions_proto2;
#[cfg(has_editions_protos)]
pub use protobuf_test_messages_editions::proto3 as editions_proto3;

// ── Stub binary when protos are missing ──────────────────────────────────

#[cfg(no_protos)]
fn main() {
    eprintln!(
        "conformance binary not functional: proto files not found.\n\
         Run `task fetch-protos` to extract them from the tools image,\n\
         then rebuild with `cargo build --manifest-path conformance/Cargo.toml`."
    );
    std::process::exit(1);
}

// ── Type registry (Any types + extensions, JSON + text) ──────────────────

/// Populates the global type registry with all well-known types, the
/// generated test-message Any entries, and the conformance proto's extension
/// declarations. Codegen emits `register_types` per file which covers both
/// JSON and text entries for both Any types and extensions.
#[cfg(not(no_protos))]
fn setup_type_registry() {
    use buffa::type_registry::{set_type_registry, TypeRegistry};

    let mut reg = TypeRegistry::new();

    // WKTs hand-registered — buffa_types knows which ones use "value"
    // wrapping in Any JSON (is_wkt: true). Codegen always emits
    // is_wkt: false, so user messages and WKTs don't step on each other.
    buffa_types::register_wkt_types(&mut reg);

    // Generated per-file registration: Any entries for every message type
    // (JSON + text) + extension entries. `test_messages_proto3.proto`
    // has no extensions, so its register_types is Any-only;
    // `test_messages_proto2.proto` declares `extension_int32` at field 120.
    proto3::__buffa::register_types(&mut reg);
    proto2::__buffa::register_types(&mut reg);
    #[cfg(has_editions_protos)]
    {
        editions_proto3::__buffa::register_types(&mut reg);
        editions_proto2::__buffa::register_types(&mut reg);
        // Edition2023's `groupliketype` / `delimited_ext` extensions —
        // needed for the text `[pkg.ext] { ... }` bracket tests.
        protobuf_test_messages_editions::__buffa::register_types(&mut reg);
    }

    set_type_registry(reg);
}

// ── Via-view mode ────────────────────────────────────────────────────────
//
// When `BUFFA_VIA_VIEW=1`, binary input is routed through
// `decode_view → to_owned_message` instead of the direct owned decode.
// Verifies owned/view decoder parity at conformance scale. JSON input is
// skipped (views have no serde) and JSON output is skipped (we'd be
// re-testing the owned encode path which the non-view run already covers).

#[cfg(not(no_protos))]
fn via_view() -> bool {
    // Cache the env lookup — invoked once per test request.
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var("BUFFA_VIA_VIEW").as_deref() == Ok("1"))
}

/// Decode via the view path: `decode_view → to_owned_message`.
/// Produces the same owned message type as the direct decode, enabling
/// reuse of the existing encode helpers.
#[cfg(not(no_protos))]
fn decode_binary_via_view<'a, V>(bytes: &'a [u8]) -> Result<V::Owned, String>
where
    V: buffa::MessageView<'a>,
{
    let view = V::decode_view(bytes).map_err(|e| format!("{e}"))?;
    Ok(view.to_owned_message())
}

// ── Real binary ──────────────────────────────────────────────────────────

#[cfg(not(no_protos))]
fn main() {
    use std::io::{self, Read, Write};

    // Set up the unified JSON registry so that serialization of Any fields
    // and `"[pkg.ext]"` extension keys uses proto3-compliant encoding.
    setup_type_registry();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();

    loop {
        let mut len_buf = [0u8; 4];
        match stdin.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => panic!("stdin read error: {e}"),
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut req_bytes = vec![0u8; len];
        stdin.read_exact(&mut req_bytes).expect("read request body");

        let resp_bytes = handle(&req_bytes);

        stdout
            .write_all(&(resp_bytes.len() as u32).to_le_bytes())
            .expect("write response length");
        stdout.write_all(&resp_bytes).expect("write response body");
        stdout.flush().expect("flush");
    }
}

#[cfg(not(no_protos))]
const MSG_PROTO3: &str = "protobuf_test_messages.proto3.TestAllTypesProto3";
#[cfg(not(no_protos))]
const MSG_PROTO2: &str = "protobuf_test_messages.proto2.TestAllTypesProto2";
#[cfg(has_editions_protos)]
const MSG_EDITIONS_PROTO3: &str = "protobuf_test_messages.editions.proto3.TestAllTypesProto3";
#[cfg(has_editions_protos)]
const MSG_EDITIONS_PROTO2: &str = "protobuf_test_messages.editions.proto2.TestAllTypesProto2";
#[cfg(has_editions_protos)]
const MSG_EDITION_2023: &str = "protobuf_test_messages.editions.TestAllTypesEdition2023";

#[cfg(not(no_protos))]
fn handle(req_bytes: &[u8]) -> Vec<u8> {
    use envelope::{encode_response, parse_request, Response};

    let req = match parse_request(req_bytes) {
        Ok(r) => r,
        Err(e) => {
            return encode_response(Response::RuntimeError(format!(
                "failed to parse ConformanceRequest: {e}"
            )));
        }
    };

    encode_response(process(&req))
}

#[cfg(not(no_protos))]
fn is_supported_message(msg_type: &str) -> bool {
    if msg_type == MSG_PROTO3 || msg_type == MSG_PROTO2 {
        return true;
    }
    #[cfg(has_editions_protos)]
    if msg_type == MSG_EDITIONS_PROTO3
        || msg_type == MSG_EDITIONS_PROTO2
        || msg_type == MSG_EDITION_2023
    {
        return true;
    }
    false
}

#[cfg(not(no_protos))]
fn process(req: &envelope::Request) -> envelope::Response {
    use envelope::{Payload, Response, WireFormat};

    if !is_supported_message(&req.message_type) {
        return Response::Skipped(format!("message type '{}' not supported", req.message_type));
    }

    // Via-view mode: only binary→binary, skip JSON entirely.
    if via_view() {
        return match &req.payload {
            Some(Payload::Protobuf(_)) if req.requested_output_format == WireFormat::Protobuf => {
                process_via_view(req)
            }
            _ => Response::Skipped("view mode: JSON and non-binary I/O skipped".into()),
        };
    }

    let ignore_unknown = req.test_category == envelope::TestCategory::JsonIgnoreUnknownParsing;
    let pu = req.print_unknown_fields;

    match (&req.payload, req.requested_output_format) {
        (None, _) => Response::ParseError("ConformanceRequest has no payload".into()),
        (Some(Payload::Jspb(_)), _) => Response::Skipped("JSPB not in scope".into()),
        (_, WireFormat::Jspb) => Response::Skipped("JSPB output not in scope".into()),
        (_, WireFormat::Unspecified) => Response::Skipped("unspecified output format".into()),

        // Proto3 paths
        (Some(Payload::Protobuf(b)), WireFormat::Protobuf) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(|| decode_proto3_binary(b), encode_proto3_binary)
        }
        (Some(Payload::Protobuf(b)), WireFormat::Json) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(|| decode_proto3_binary(b), encode_proto3_json)
        }
        (Some(Payload::Json(s)), WireFormat::Protobuf) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(
                || decode_proto3_json(s, ignore_unknown),
                encode_proto3_binary,
            )
        }
        (Some(Payload::Json(s)), WireFormat::Json) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(|| decode_proto3_json(s, ignore_unknown), encode_proto3_json)
        }
        (Some(Payload::Protobuf(b)), WireFormat::TextFormat) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(|| decode_proto3_binary(b), |m| encode_proto3_text(m, pu))
        }
        (Some(Payload::Text(s)), WireFormat::Protobuf) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(|| decode_proto3_text(s), encode_proto3_binary)
        }
        (Some(Payload::Text(s)), WireFormat::TextFormat) if req.message_type == MSG_PROTO3 => {
            roundtrip_proto3(|| decode_proto3_text(s), |m| encode_proto3_text(m, pu))
        }

        // Proto2 paths
        (Some(Payload::Protobuf(b)), WireFormat::Protobuf) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(|| decode_proto2_binary(b), encode_proto2_binary)
        }
        (Some(Payload::Protobuf(b)), WireFormat::Json) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(|| decode_proto2_binary(b), encode_proto2_json)
        }
        (Some(Payload::Json(s)), WireFormat::Protobuf) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(
                || decode_proto2_json(s, ignore_unknown),
                encode_proto2_binary,
            )
        }
        (Some(Payload::Json(s)), WireFormat::Json) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(|| decode_proto2_json(s, ignore_unknown), encode_proto2_json)
        }
        (Some(Payload::Protobuf(b)), WireFormat::TextFormat) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(|| decode_proto2_binary(b), |m| encode_proto2_text(m, pu))
        }
        (Some(Payload::Text(s)), WireFormat::Protobuf) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(|| decode_proto2_text(s), encode_proto2_binary)
        }
        (Some(Payload::Text(s)), WireFormat::TextFormat) if req.message_type == MSG_PROTO2 => {
            roundtrip_proto2(|| decode_proto2_text(s), |m| encode_proto2_text(m, pu))
        }

        _ => process_editions(req, ignore_unknown),
    }
}

/// Binary→binary round-trip via `decode_view → to_owned_message → encode`.
/// Dispatches on message type and reuses the existing binary-encode helpers.
#[cfg(not(no_protos))]
fn process_via_view(req: &envelope::Request) -> envelope::Response {
    use envelope::{Payload, Response};
    let Some(Payload::Protobuf(b)) = &req.payload else {
        return Response::RuntimeError("process_via_view called without protobuf payload".into());
    };

    match req.message_type.as_str() {
        MSG_PROTO3 => roundtrip_proto3(
            || decode_binary_via_view::<proto3::__buffa::view::TestAllTypesProto3View<'_>>(b),
            encode_proto3_binary,
        ),
        MSG_PROTO2 => roundtrip_proto2(
            || decode_binary_via_view::<proto2::__buffa::view::TestAllTypesProto2View<'_>>(b),
            encode_proto2_binary,
        ),
        #[cfg(has_editions_protos)]
        MSG_EDITIONS_PROTO3 => roundtrip(
            || decode_binary_via_view::<editions_proto3::__buffa::view::TestAllTypesProto3View<'_>>(b),
            encode_binary,
        ),
        #[cfg(has_editions_protos)]
        MSG_EDITIONS_PROTO2 => roundtrip(
            || decode_binary_via_view::<editions_proto2::__buffa::view::TestAllTypesProto2View<'_>>(b),
            encode_binary,
        ),
        other => Response::Skipped(format!("message type '{other}' not in view dispatch")),
    }
}

/// Handle editions message types.  Returns `Skipped` if the message type
/// is unknown or editions protos are not compiled in.
#[cfg(not(no_protos))]
fn process_editions(
    #[allow(unused_variables)] req: &envelope::Request,
    #[allow(unused_variables)] ignore_unknown: bool,
) -> envelope::Response {
    #[cfg(has_editions_protos)]
    {
        return process_editions_inner(req, ignore_unknown);
    }
    #[cfg(not(has_editions_protos))]
    envelope::Response::Skipped(format!(
        "message type '{}' not supported (editions protos not compiled)",
        req.message_type
    ))
}

#[cfg(has_editions_protos)]
fn process_editions_inner(req: &envelope::Request, ignore_unknown: bool) -> envelope::Response {
    use envelope::{Payload, Response, WireFormat};

    type EdProto3 = editions_proto3::TestAllTypesProto3;
    type EdProto2 = editions_proto2::TestAllTypesProto2;
    type Ed2023 = protobuf_test_messages_editions::TestAllTypesEdition2023;

    let pu = req.print_unknown_fields;

    match (&req.payload, req.requested_output_format) {
        (None, _) => Response::ParseError("ConformanceRequest has no payload".into()),

        // Proto3 via editions
        (Some(Payload::Protobuf(b)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITIONS_PROTO3 =>
        {
            roundtrip(|| decode_binary::<EdProto3>(b), encode_binary)
        }
        (Some(Payload::Protobuf(b)), WireFormat::Json)
            if req.message_type == MSG_EDITIONS_PROTO3 =>
        {
            roundtrip(|| decode_binary::<EdProto3>(b), encode_json)
        }
        (Some(Payload::Json(s)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITIONS_PROTO3 =>
        {
            roundtrip(|| decode_json::<EdProto3>(s, ignore_unknown), encode_binary)
        }
        (Some(Payload::Json(s)), WireFormat::Json) if req.message_type == MSG_EDITIONS_PROTO3 => {
            roundtrip(|| decode_json::<EdProto3>(s, ignore_unknown), encode_json)
        }
        (Some(Payload::Protobuf(b)), WireFormat::TextFormat)
            if req.message_type == MSG_EDITIONS_PROTO3 =>
        {
            roundtrip(|| decode_binary::<EdProto3>(b), |m| encode_text(m, pu))
        }
        (Some(Payload::Text(s)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITIONS_PROTO3 =>
        {
            roundtrip(|| decode_text::<EdProto3>(s), encode_binary)
        }
        (Some(Payload::Text(s)), WireFormat::TextFormat)
            if req.message_type == MSG_EDITIONS_PROTO3 =>
        {
            roundtrip(|| decode_text::<EdProto3>(s), |m| encode_text(m, pu))
        }

        // Proto2 via editions
        (Some(Payload::Protobuf(b)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITIONS_PROTO2 =>
        {
            roundtrip(|| decode_binary::<EdProto2>(b), encode_binary)
        }
        (Some(Payload::Protobuf(b)), WireFormat::Json)
            if req.message_type == MSG_EDITIONS_PROTO2 =>
        {
            roundtrip(|| decode_binary::<EdProto2>(b), encode_json)
        }
        (Some(Payload::Json(s)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITIONS_PROTO2 =>
        {
            roundtrip(|| decode_json::<EdProto2>(s, ignore_unknown), encode_binary)
        }
        (Some(Payload::Json(s)), WireFormat::Json) if req.message_type == MSG_EDITIONS_PROTO2 => {
            roundtrip(|| decode_json::<EdProto2>(s, ignore_unknown), encode_json)
        }
        (Some(Payload::Protobuf(b)), WireFormat::TextFormat)
            if req.message_type == MSG_EDITIONS_PROTO2 =>
        {
            roundtrip(|| decode_binary::<EdProto2>(b), |m| encode_text(m, pu))
        }
        (Some(Payload::Text(s)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITIONS_PROTO2 =>
        {
            roundtrip(|| decode_text::<EdProto2>(s), encode_binary)
        }
        (Some(Payload::Text(s)), WireFormat::TextFormat)
            if req.message_type == MSG_EDITIONS_PROTO2 =>
        {
            roundtrip(|| decode_text::<EdProto2>(s), |m| encode_text(m, pu))
        }

        // Pure edition 2023 (file-level DELIMITED). Binary + text; JSON
        // codegen is on only for the extension registry (the text `[pkg.ext]`
        // bracket syntax resolves through it) — the suite doesn't send JSON
        // for this message type.
        (Some(Payload::Protobuf(b)), WireFormat::Protobuf)
            if req.message_type == MSG_EDITION_2023 =>
        {
            roundtrip(|| decode_binary::<Ed2023>(b), encode_binary)
        }
        (Some(Payload::Protobuf(b)), WireFormat::TextFormat)
            if req.message_type == MSG_EDITION_2023 =>
        {
            roundtrip(|| decode_binary::<Ed2023>(b), |m| encode_text(m, pu))
        }
        (Some(Payload::Text(s)), WireFormat::Protobuf) if req.message_type == MSG_EDITION_2023 => {
            roundtrip(|| decode_text::<Ed2023>(s), encode_binary)
        }
        (Some(Payload::Text(s)), WireFormat::TextFormat)
            if req.message_type == MSG_EDITION_2023 =>
        {
            roundtrip(|| decode_text::<Ed2023>(s), |m| encode_text(m, pu))
        }
        _ if req.message_type == MSG_EDITION_2023 => {
            Response::Skipped("TestAllTypesEdition2023: JSON I/O not supported".into())
        }

        _ => Response::Skipped(format!("unsupported message type '{}'", req.message_type)),
    }
}

// ── Generic decode/encode helpers for editions ──────────────────────────

#[cfg(has_editions_protos)]
fn roundtrip<T>(
    decode: impl FnOnce() -> Result<T, String>,
    encode: impl FnOnce(&T) -> Result<envelope::Response, String>,
) -> envelope::Response {
    match decode() {
        Err(e) => envelope::Response::ParseError(e),
        Ok(msg) => match encode(&msg) {
            Ok(resp) => resp,
            Err(e) => envelope::Response::SerializeError(e),
        },
    }
}

#[cfg(has_editions_protos)]
fn decode_binary<T: buffa::Message + Default>(bytes: &[u8]) -> Result<T, String> {
    T::decode(&mut bytes.as_ref()).map_err(|e| format!("{e}"))
}

#[cfg(has_editions_protos)]
fn decode_json<T: serde::de::DeserializeOwned>(
    json: &str,
    ignore_unknown: bool,
) -> Result<T, String> {
    #[cfg(feature = "buffa-std")]
    if ignore_unknown {
        let opts = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(true);
        return buffa::json::with_json_parse_options(&opts, || {
            serde_json::from_str(json).map_err(|e| format!("{e}"))
        });
    }
    let _ = ignore_unknown;
    serde_json::from_str(json).map_err(|e| format!("{e}"))
}

#[cfg(has_editions_protos)]
fn encode_binary<T: buffa::Message>(msg: &T) -> Result<envelope::Response, String> {
    Ok(envelope::Response::ProtobufPayload(msg.encode_to_vec()))
}

#[cfg(has_editions_protos)]
fn encode_json<T: serde::Serialize>(msg: &T) -> Result<envelope::Response, String> {
    serde_json::to_string(msg)
        .map(envelope::Response::JsonPayload)
        .map_err(|e| format!("{e}"))
}

#[cfg(has_editions_protos)]
fn decode_text<T: buffa::text::TextFormat + Default>(s: &str) -> Result<T, String> {
    buffa::text::decode_from_str(s).map_err(|e| format!("{e}"))
}

#[cfg(has_editions_protos)]
fn encode_text<T: buffa::text::TextFormat>(
    msg: &T,
    print_unknown: bool,
) -> Result<envelope::Response, String> {
    let mut out = String::new();
    let mut enc = buffa::text::TextEncoder::new(&mut out).emit_unknown(print_unknown);
    let _ = msg.encode_text(&mut enc);
    Ok(envelope::Response::TextPayload(out))
}

// ── Proto3 decode/encode helpers ─────────────────────────────────────────

#[cfg(not(no_protos))]
fn roundtrip_proto3(
    decode: impl FnOnce() -> Result<proto3::TestAllTypesProto3, String>,
    encode: impl FnOnce(&proto3::TestAllTypesProto3) -> Result<envelope::Response, String>,
) -> envelope::Response {
    match decode() {
        Err(e) => envelope::Response::ParseError(e),
        Ok(msg) => match encode(&msg) {
            Ok(resp) => resp,
            Err(e) => envelope::Response::SerializeError(e),
        },
    }
}

#[cfg(not(no_protos))]
fn decode_proto3_binary(bytes: &[u8]) -> Result<proto3::TestAllTypesProto3, String> {
    use buffa::Message as _;
    proto3::TestAllTypesProto3::decode(&mut bytes.as_ref()).map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn decode_proto3_json(
    json: &str,
    ignore_unknown: bool,
) -> Result<proto3::TestAllTypesProto3, String> {
    #[cfg(feature = "buffa-std")]
    if ignore_unknown {
        let opts = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(true);
        return buffa::json::with_json_parse_options(&opts, || {
            serde_json::from_str(json).map_err(|e| format!("{e}"))
        });
    }
    let _ = ignore_unknown;
    serde_json::from_str(json).map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn encode_proto3_binary(msg: &proto3::TestAllTypesProto3) -> Result<envelope::Response, String> {
    use buffa::Message as _;
    Ok(envelope::Response::ProtobufPayload(msg.encode_to_vec()))
}

#[cfg(not(no_protos))]
fn encode_proto3_json(msg: &proto3::TestAllTypesProto3) -> Result<envelope::Response, String> {
    serde_json::to_string(msg)
        .map(envelope::Response::JsonPayload)
        .map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn decode_proto3_text(s: &str) -> Result<proto3::TestAllTypesProto3, String> {
    buffa::text::decode_from_str(s).map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn encode_proto3_text(
    msg: &proto3::TestAllTypesProto3,
    print_unknown: bool,
) -> Result<envelope::Response, String> {
    use buffa::text::TextFormat as _;
    let mut out = String::new();
    let mut enc = buffa::text::TextEncoder::new(&mut out).emit_unknown(print_unknown);
    let _ = msg.encode_text(&mut enc);
    Ok(envelope::Response::TextPayload(out))
}

// ── Proto2 decode/encode helpers ─────────────────────────────────────────

#[cfg(not(no_protos))]
fn roundtrip_proto2(
    decode: impl FnOnce() -> Result<proto2::TestAllTypesProto2, String>,
    encode: impl FnOnce(&proto2::TestAllTypesProto2) -> Result<envelope::Response, String>,
) -> envelope::Response {
    match decode() {
        Err(e) => envelope::Response::ParseError(e),
        Ok(msg) => match encode(&msg) {
            Ok(resp) => resp,
            Err(e) => envelope::Response::SerializeError(e),
        },
    }
}

#[cfg(not(no_protos))]
fn decode_proto2_binary(bytes: &[u8]) -> Result<proto2::TestAllTypesProto2, String> {
    use buffa::Message as _;
    proto2::TestAllTypesProto2::decode(&mut bytes.as_ref()).map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn decode_proto2_json(
    json: &str,
    ignore_unknown: bool,
) -> Result<proto2::TestAllTypesProto2, String> {
    #[cfg(feature = "buffa-std")]
    if ignore_unknown {
        let opts = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(true);
        return buffa::json::with_json_parse_options(&opts, || {
            serde_json::from_str(json).map_err(|e| format!("{e}"))
        });
    }
    let _ = ignore_unknown;
    serde_json::from_str(json).map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn encode_proto2_binary(msg: &proto2::TestAllTypesProto2) -> Result<envelope::Response, String> {
    use buffa::Message as _;
    Ok(envelope::Response::ProtobufPayload(msg.encode_to_vec()))
}

#[cfg(not(no_protos))]
fn encode_proto2_json(msg: &proto2::TestAllTypesProto2) -> Result<envelope::Response, String> {
    serde_json::to_string(msg)
        .map(envelope::Response::JsonPayload)
        .map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn decode_proto2_text(s: &str) -> Result<proto2::TestAllTypesProto2, String> {
    buffa::text::decode_from_str(s).map_err(|e| format!("{e}"))
}

#[cfg(not(no_protos))]
fn encode_proto2_text(
    msg: &proto2::TestAllTypesProto2,
    print_unknown: bool,
) -> Result<envelope::Response, String> {
    use buffa::text::TextFormat as _;
    let mut out = String::new();
    let mut enc = buffa::text::TextEncoder::new(&mut out).emit_unknown(print_unknown);
    let _ = msg.encode_text(&mut enc);
    Ok(envelope::Response::TextPayload(out))
}

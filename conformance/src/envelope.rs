//! Hand-rolled parser and builder for the conformance test harness protocol.
// When protos are absent the binary compiles to a stub; suppress dead-code
// warnings for all the envelope functions that only get called in full builds.
#![cfg_attr(no_protos, allow(dead_code))]
//!
//! Directly decodes `ConformanceRequest` and encodes `ConformanceResponse`
//! using the protobuf wire format without code generation.  This avoids a
//! circular dependency: we test buffa's codec correctness via the conformance
//! suite, so we must not use buffa (or a generated codec) to parse the test
//! harness messages themselves.
//!
//! The `conformance.proto` schema — specifically the fields consumed here —
//! has been stable since protobuf 3.0 (~2016) and field numbers are frozen
//! by the protobuf compatibility guarantee.
//!
//! We use `buffa::encoding::{decode_varint, encode_varint}` as low-level wire
//! primitives since buffa is already a dependency and those functions are
//! independent of message codegen.

// ── Types mirroring conformance.proto enums ───────────────────────────────

/// `conformance.WireFormat`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    Unspecified = 0,
    Protobuf = 1,
    Json = 2,
    Jspb = 3,
    TextFormat = 4,
}

impl WireFormat {
    fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Protobuf,
            2 => Self::Json,
            3 => Self::Jspb,
            4 => Self::TextFormat,
            _ => Self::Unspecified,
        }
    }
}

/// `conformance.TestCategory`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCategory {
    Unspecified = 0,
    Binary = 1,
    Json = 2,
    JsonIgnoreUnknownParsing = 3,
    Jspb = 4,
    TextFormat = 5,
}

impl TestCategory {
    fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Binary,
            2 => Self::Json,
            3 => Self::JsonIgnoreUnknownParsing,
            4 => Self::Jspb,
            5 => Self::TextFormat,
            _ => Self::Unspecified,
        }
    }
}

// ── ConformanceRequest ────────────────────────────────────────────────────

/// All payload variants from `ConformanceRequest.payload` oneof.
/// Parsed even when the format is not yet implemented so that the dispatcher
/// can return `Skipped` with a useful message rather than a parse error.
#[derive(Debug)]
pub enum Payload {
    Protobuf(Vec<u8>),
    Json(String),
    Jspb(String),
    Text(String),
}

/// Parsed `ConformanceRequest`.
#[derive(Debug)]
pub struct Request {
    pub payload: Option<Payload>,
    pub message_type: String,
    pub requested_output_format: WireFormat,
    pub test_category: TestCategory,
    pub print_unknown_fields: bool,
}

/// Decode a `ConformanceRequest` from raw protobuf bytes.
pub fn parse_request(mut buf: &[u8]) -> Result<Request, String> {
    let mut payload = None;
    let mut message_type = String::new();
    let mut requested_output_format = WireFormat::Unspecified;
    let mut test_category = TestCategory::Unspecified;
    let mut print_unknown_fields = false;

    while !buf.is_empty() {
        let tag = read_varint(&mut buf)?;
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;

        match (field_number, wire_type) {
            // oneof payload variants
            (1, 2) => {
                payload = Some(Payload::Protobuf(read_ld(&mut buf)?));
            }
            (2, 2) => {
                payload = Some(Payload::Json(read_string(&mut buf)?));
            }
            (7, 2) => {
                payload = Some(Payload::Jspb(read_string(&mut buf)?));
            }
            (8, 2) => {
                payload = Some(Payload::Text(read_string(&mut buf)?));
            }
            // scalar fields
            (3, 0) => {
                requested_output_format = WireFormat::from_u64(read_varint(&mut buf)?);
            }
            (4, 2) => {
                message_type = read_string(&mut buf)?;
            }
            (5, 0) => {
                test_category = TestCategory::from_u64(read_varint(&mut buf)?);
            }
            (9, 0) => {
                print_unknown_fields = read_varint(&mut buf)? != 0;
            }
            // unknown / future fields — skip gracefully
            (_, 0) => {
                read_varint(&mut buf)?;
            }
            (_, 2) => {
                read_ld(&mut buf)?;
            }
            (_, 1) => skip(&mut buf, 8)?,
            (_, 5) => skip(&mut buf, 4)?,
            (fn_, wt) => {
                return Err(format!(
                    "unexpected wire type {wt} for field {fn_} in ConformanceRequest"
                ));
            }
        }
    }

    Ok(Request {
        payload,
        message_type,
        requested_output_format,
        test_category,
        print_unknown_fields,
    })
}

// ── ConformanceResponse ───────────────────────────────────────────────────

/// `ConformanceResponse.result` oneof.
#[derive(Debug)]
pub enum Response {
    /// field 3: binary protobuf re-encoding of the test message.
    ProtobufPayload(Vec<u8>),
    /// field 4: JSON re-encoding of the test message.
    JsonPayload(String),
    /// field 1: the input could not be decoded.
    ParseError(String),
    /// field 6: the decoded message could not be re-encoded.
    SerializeError(String),
    /// field 2: an unexpected runtime error occurred.
    RuntimeError(String),
    /// field 5: test skipped (unsupported format / message type).
    Skipped(String),
    /// field 8: textproto re-encoding of the test message.
    TextPayload(String),
}

/// Encode a `ConformanceResponse` to raw protobuf bytes.
pub fn encode_response(resp: Response) -> Vec<u8> {
    let mut out = Vec::new();
    match resp {
        Response::ParseError(s) => write_ld_field(&mut out, 1, s.as_bytes()),
        Response::RuntimeError(s) => write_ld_field(&mut out, 2, s.as_bytes()),
        Response::ProtobufPayload(b) => write_ld_field(&mut out, 3, &b),
        Response::JsonPayload(s) => write_ld_field(&mut out, 4, s.as_bytes()),
        Response::Skipped(s) => write_ld_field(&mut out, 5, s.as_bytes()),
        Response::SerializeError(s) => write_ld_field(&mut out, 6, s.as_bytes()),
        Response::TextPayload(s) => write_ld_field(&mut out, 8, s.as_bytes()),
    }
    out
}

// ── Wire-format helpers ───────────────────────────────────────────────────

fn read_varint(buf: &mut &[u8]) -> Result<u64, String> {
    buffa::encoding::decode_varint(buf).map_err(|e| format!("varint: {e}"))
}

fn read_ld(buf: &mut &[u8]) -> Result<Vec<u8>, String> {
    let len = read_varint(buf)? as usize;
    if buf.len() < len {
        return Err(format!(
            "unexpected EOF in length-delimited field: need {len}, have {}",
            buf.len()
        ));
    }
    let data = buf[..len].to_vec();
    *buf = &buf[len..];
    Ok(data)
}

fn read_string(buf: &mut &[u8]) -> Result<String, String> {
    let bytes = read_ld(buf)?;
    String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8 in string field: {e}"))
}

fn skip(buf: &mut &[u8], n: usize) -> Result<(), String> {
    if buf.len() < n {
        return Err(format!(
            "unexpected EOF skipping {n} bytes, have {}",
            buf.len()
        ));
    }
    *buf = &buf[n..];
    Ok(())
}

fn write_ld_field(out: &mut Vec<u8>, field_number: u32, data: &[u8]) {
    // tag = (field_number << 3) | 2  (LengthDelimited wire type)
    buffa::encoding::encode_varint(((field_number as u64) << 3) | 2, out);
    buffa::encoding::encode_varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

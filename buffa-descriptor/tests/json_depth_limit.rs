//! Reflective JSON serialization must bound nesting across
//! `google.protobuf.Any` boundaries.
//!
//! `Any.value` is opaque bytes to the binary decoder, so `RECURSION_LIMIT`
//! does not see nesting hidden inside it; the reflective serializer decodes
//! those bytes at serialize time. Without a budget spanning `Any` layers, a
//! sub-megabyte input of N nested `Any`s costs N stack frames and aborts the
//! process with a stack overflow. These tests pin that over-deep input is a
//! serde error instead, and that the budget is shared with the inner decode.
//!
//! Run standalone with (the file compiles to nothing without the features):
//! `cargo test -p buffa-descriptor --features reflect,json --test json_depth_limit`

#![cfg(all(feature = "reflect", feature = "json", feature = "std"))]

use std::sync::Arc;

use buffa::encoding::encode_varint;
use buffa::RECURSION_LIMIT;
use buffa_descriptor::reflect::{DynamicMessage, ReflectMessageMut, Value};
use buffa_descriptor::DescriptorPool;

/// `descriptor.proto` + `any.proto` + `reflect.opt.Envelope { Any payload = 1; }`.
const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test_options.fds");

const ANY_URL: &str = "type.googleapis.com/google.protobuf.Any";
const ENVELOPE_URL: &str = "type.googleapis.com/reflect.opt.Envelope";
const DESCRIPTOR_PROTO_URL: &str = "type.googleapis.com/google.protobuf.DescriptorProto";

const LIMIT: usize = RECURSION_LIMIT as usize;

/// Every serialization here runs on a thread this small. Measured need at
/// the limit in an unoptimised build is ~300 KiB for the serializer and
/// ~600 KiB when an Any payload itself binary-decodes 100 deep; 1 MiB is
/// half of Rust's default spawned-thread stack, so a per-level frame-size
/// regression fails a named test rather than aborting the test binary.
const STACK_FLOOR: usize = 1024 * 1024;

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

/// One nesting layer: `head` bytes, then a length-delimited payload holding
/// the next layer. The innermost payload is empty.
struct Frame(Vec<u8>);

impl Frame {
    /// `google.protobuf.Any { type_url = url; value = <next> }`.
    fn any(url: &str) -> Self {
        let mut head = vec![0x0A];
        encode_varint(url.len() as u64, &mut head);
        head.extend_from_slice(url.as_bytes());
        head.push(0x12);
        Self(head)
    }
    /// `reflect.opt.Envelope { payload = <next> }`.
    fn envelope() -> Self {
        Self(vec![0x0A])
    }
    /// `google.protobuf.DescriptorProto { nested_type = [<next>] }`.
    fn descriptor_proto() -> Self {
        Self(vec![0x1A])
    }
}

/// Encode `frames` outermost-first in O(total bytes): sizes are computed
/// inside-out, then the buffer is written top-down.
fn encode_nested(frames: &[Frame]) -> Vec<u8> {
    let mut payload_len = vec![0usize; frames.len() + 1];
    for (i, Frame(head)) in frames.iter().enumerate().rev() {
        let inner = payload_len[i + 1];
        payload_len[i] = head.len() + varint_len(inner) + inner;
    }
    let mut out = Vec::with_capacity(payload_len[0]);
    for (i, Frame(head)) in frames.iter().enumerate() {
        out.extend_from_slice(head);
        encode_varint(payload_len[i + 1] as u64, &mut out);
    }
    assert_eq!(out.len(), payload_len[0]);
    out
}

fn varint_len(n: usize) -> usize {
    let mut buf = Vec::new();
    encode_varint(n as u64, &mut buf);
    buf.len()
}

fn to_json(type_name: &str, bytes: &[u8]) -> Result<String, serde_json::Error> {
    let p = pool();
    let idx = p.message_index(type_name).unwrap();
    let msg = DynamicMessage::decode(p, idx, bytes)
        .expect("binary decode succeeds: Any payloads are opaque to it");
    on_floor(msg)
}

/// Run only `to_json` on a `STACK_FLOOR`-sized thread.
fn on_floor(msg: DynamicMessage) -> Result<String, serde_json::Error> {
    std::thread::Builder::new()
        .stack_size(STACK_FLOOR)
        .spawn(move || msg.to_json())
        .unwrap()
        .join()
        .unwrap()
}

fn assert_too_deep(result: Result<String, serde_json::Error>) {
    let err = result.expect_err("over-deep nesting must be a serde error, not a stack overflow");
    let text = err.to_string();
    let needle = format!("nesting depth exceeds buffa::RECURSION_LIMIT ({RECURSION_LIMIT})");
    assert!(text.contains(&needle), "unexpected error: {text}");
}

#[test]
fn deeply_nested_any_is_an_error_not_an_abort() {
    // The reported reproducer shape: ~20k layers in well under 1 MB.
    let frames: Vec<Frame> = (0..20_000).map(|_| Frame::any(ANY_URL)).collect();
    let bytes = encode_nested(&frames);
    assert!(bytes.len() < 1024 * 1024, "{} bytes", bytes.len());
    assert_too_deep(to_json("google.protobuf.Any", &bytes));
}

#[test]
fn any_nesting_at_the_limit_serializes() {
    // Any#1 is the top-level message and free; each payload below it costs
    // one level, like a nested message field. LIMIT frames + the trailing
    // empty Any = exactly LIMIT levels below the top.
    let frames: Vec<Frame> = (0..LIMIT).map(|_| Frame::any(ANY_URL)).collect();
    let json = to_json("google.protobuf.Any", &encode_nested(&frames)).expect("at limit");
    assert_eq!(json.matches("\"@type\"").count(), LIMIT);
    assert!(
        json.ends_with(&format!("{{}}{}", "}".repeat(LIMIT))),
        "{json}"
    );

    let frames: Vec<Frame> = (0..=LIMIT).map(|_| Frame::any(ANY_URL)).collect();
    assert_too_deep(to_json("google.protobuf.Any", &encode_nested(&frames)));
}

#[test]
fn any_spreading_a_plain_message_shares_one_budget() {
    // Envelope → Any → Envelope → Any → …: the common "Any wraps a user
    // message" (field-spread) form. Each pair costs two levels — the Any
    // field, then its payload — so LIMIT/2 pairs sit exactly at the limit.
    let pairs = |n: usize| -> Vec<Frame> {
        (0..n)
            .flat_map(|_| [Frame::envelope(), Frame::any(ENVELOPE_URL)])
            .collect()
    };
    let json =
        to_json("reflect.opt.Envelope", &encode_nested(&pairs(LIMIT / 2))).expect("at limit");
    assert_eq!(json.matches("\"payload\"").count(), LIMIT / 2);

    // One more Envelope with a payload field: LIMIT + 1, caught by the inner
    // decode running at zero remaining depth.
    let mut over = pairs(LIMIT / 2);
    over.push(Frame::envelope());
    assert_too_deep(to_json("reflect.opt.Envelope", &encode_nested(&over)));

    // And far past it, cheaply.
    assert_too_deep(to_json(
        "reflect.opt.Envelope",
        &encode_nested(&pairs(5_000)),
    ));
}

#[test]
fn programmatically_built_over_deep_message_is_an_error() {
    // No Any involved: a message assembled one level deeper than `decode`
    // would accept trips the same budget on the plain nested-message path.
    let p = pool();
    let idx = p.message_index("google.protobuf.DescriptorProto").unwrap();
    let nested_type = p.message(idx).field(3).unwrap();
    let build = |levels: usize| {
        let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
        for _ in 0..levels {
            let mut outer = DynamicMessage::new(Arc::clone(&p), idx);
            outer.set(nested_type, Value::List(vec![Value::Message(msg)]));
            msg = outer;
        }
        msg
    };
    on_floor(build(LIMIT)).expect("at limit");
    assert_too_deep(on_floor(build(LIMIT + 1)));
}

#[test]
fn any_payload_decode_continues_the_outer_budget() {
    // A DescriptorProto nested RECURSION_LIMIT deep decodes fine on its own…
    let limit = LIMIT;
    let chain: Vec<Frame> = (0..limit).map(|_| Frame::descriptor_proto()).collect();
    to_json("google.protobuf.DescriptorProto", &encode_nested(&chain)).expect("standalone");

    // …but wrapped in a single Any it sits one level deeper, and the inner
    // decode runs on the remaining budget rather than a fresh one — reported
    // as the same nesting error, not as a decode failure.
    let mut wrapped = vec![Frame::any(DESCRIPTOR_PROTO_URL)];
    wrapped.extend(chain);
    assert_too_deep(to_json("google.protobuf.Any", &encode_nested(&wrapped)));

    // One level shallower fits again.
    wrapped.pop();
    to_json("google.protobuf.Any", &encode_nested(&wrapped)).expect("fits when one shallower");
}

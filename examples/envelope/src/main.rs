//! Demonstrates reading and writing extensions on a buffa-generated message.
//!
//! Run with: `cargo run -p example-envelope`

mod proto {
    include!(concat!(env!("OUT_DIR"), "/_include.rs"));
}

use buffa::{ExtensionSet, Message};
use proto::buffa::examples::envelope::__buffa::ext::{PRIORITY, RETRY_COUNT, ROUTING_HOPS, TRACE};
use proto::buffa::examples::envelope::{Envelope, TraceContext};

fn main() {
    binary_roundtrip();
    default_values();
    json_roundtrip();
    extendee_check();
}

/// The basic get/set/has/clear cycle, roundtripped through binary encode/decode.
fn binary_roundtrip() {
    println!("── binary roundtrip ─────────────────────────────────────────");

    let mut env = Envelope {
        request_id: Some("req-abc123".into()),
        payload: Some(b"hello".to_vec()),
        ..Default::default()
    };

    // Extension absence: `extension()` returns `Option<T>` — `None` means
    // not set. This is more information-dense than the protobuf-go/es
    // approach (which returns default-or-value and needs a separate `has`).
    assert_eq!(env.extension(&RETRY_COUNT), None);
    assert!(!env.has_extension(&RETRY_COUNT));

    // Set a scalar extension. On the wire this is just a field at number 100 —
    // indistinguishable from a regular field except that Envelope's schema
    // doesn't know about it. Storage is in `__buffa_unknown_fields`.
    env.set_extension(&RETRY_COUNT, 3);
    assert_eq!(env.extension(&RETRY_COUNT), Some(3));

    // Set a message-typed extension.
    env.set_extension(
        &TRACE,
        TraceContext {
            trace_id: Some("4bf92f35".into()),
            span_id: Some("00f067aa".into()),
            ..Default::default()
        },
    );

    // Set a repeated extension — `Value` is `Vec<T>` for `Repeated<_>` codecs.
    env.set_extension(
        &ROUTING_HOPS,
        vec!["edge-lhr".into(), "core-dub".into(), "edge-sfo".into()],
    );

    // Roundtrip through wire encoding. Extensions survive because they're
    // unknown-field bytes — buffa preserves those by default.
    let bytes = env.encode_to_vec();
    println!("encoded size: {} bytes", bytes.len());
    let decoded = Envelope::decode_from_slice(&bytes).expect("decode");

    assert_eq!(decoded.request_id.as_deref(), Some("req-abc123"));
    assert_eq!(decoded.extension(&RETRY_COUNT), Some(3));
    let trace = decoded.extension(&TRACE).expect("TRACE present");
    assert_eq!(trace.trace_id.as_deref(), Some("4bf92f35"));
    assert_eq!(
        decoded.extension(&ROUTING_HOPS),
        vec!["edge-lhr", "core-dub", "edge-sfo"]
    );

    // `clear_extension` removes all records at the extension's field number.
    let mut decoded = decoded;
    decoded.clear_extension(&RETRY_COUNT);
    assert_eq!(decoded.extension(&RETRY_COUNT), None);
    // The other extensions are untouched.
    assert!(decoded.has_extension(&TRACE));

    println!("  retry_count:  {:?}", Some(3));
    println!("  trace_id:     {:?}", trace.trace_id);
    println!(
        "  routing_hops: {:?}",
        vec!["edge-lhr", "core-dub", "edge-sfo"]
    );
    println!();
}

/// Proto2 `[default = ...]` — surfaced by `extension_or_default()`.
fn default_values() {
    println!("── [default = ...] ──────────────────────────────────────────");

    let env = Envelope::default();

    // PRIORITY is declared with `[default = 5]`. `extension()` still returns
    // `None` — presence is distinguishable. `extension_or_default()` applies
    // the declared default.
    assert_eq!(env.extension(&PRIORITY), None);
    assert_eq!(env.extension_or_default(&PRIORITY), 5);
    println!(
        "  absent:       extension()={:?}  extension_or_default()={}",
        env.extension(&PRIORITY),
        env.extension_or_default(&PRIORITY)
    );

    // Explicitly set a value — including one equal to the type's zero.
    // Extensions always have explicit presence (protocolbuffers/protobuf#8234),
    // so `set(0)` is distinguishable from absent: the set value wins, not the
    // declared default.
    let mut env = env;
    env.set_extension(&PRIORITY, 0);
    assert_eq!(env.extension(&PRIORITY), Some(0));
    assert_eq!(env.extension_or_default(&PRIORITY), 0);
    println!(
        "  set to 0:     extension()={:?}  extension_or_default()={}",
        env.extension(&PRIORITY),
        env.extension_or_default(&PRIORITY)
    );

    // Clear → back to the declared default.
    env.clear_extension(&PRIORITY);
    assert_eq!(env.extension_or_default(&PRIORITY), 5);
    println!();
}

/// ProtoJSON `"[pkg.ext]"` keys — requires a `TypeRegistry` populated by the
/// codegen-emitted `register_types()` per file.
fn json_roundtrip() {
    println!("── JSON `[pkg.ext]` keys ────────────────────────────────────");

    // Setup: once at startup. Without this, extension bytes stay in
    // `__buffa_unknown_fields` and are silently dropped from JSON output.
    use buffa::type_registry::{set_type_registry, TypeRegistry};
    let mut reg = TypeRegistry::new();
    // Codegen emits this per package. It registers extension JSON
    // converters, extension text converters, and `Any` type entries — one
    // call covers all.
    proto::buffa::examples::envelope::__buffa::register_types(&mut reg);
    set_type_registry(reg);

    let mut env = Envelope {
        request_id: Some("req-xyz789".into()),
        ..Default::default()
    };
    env.set_extension(&RETRY_COUNT, 2);
    env.set_extension(&ROUTING_HOPS, vec!["edge-nrt".into()]);

    // Serialize: extensions appear as `"[fully.qualified.name]"` keys.
    let json = serde_json::to_string_pretty(&env).expect("serialize");
    println!("{json}");
    assert!(json.contains(r#""[buffa.examples.envelope.retry_count]": 2"#));
    assert!(json.contains(r#""[buffa.examples.envelope.routing_hops]": ["#));

    // Deserialize: `"[...]"` keys are resolved against the registry and
    // decoded back into unknown-field records.
    let back: Envelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.extension(&RETRY_COUNT), Some(2));
    assert_eq!(back.extension(&ROUTING_HOPS), vec!["edge-nrt"]);

    // Unregistered `"[...]"` keys are silently dropped by default —
    // `JsonParseOptions::strict_extension_keys(true)` makes them error instead.
    let with_unknown = r#"{"requestId":"req-x","[unknown.extension]":99}"#;
    let parsed: Envelope = serde_json::from_str(with_unknown).expect("lenient parse");
    assert_eq!(parsed.request_id.as_deref(), Some("req-x"));
    println!();
}

/// The extendee identity check: passing an extension for the wrong message
/// is a bug, and it panics (matching protobuf-go and protobuf-es).
fn extendee_check() {
    println!("── extendee identity check ──────────────────────────────────");

    // Each generated `Extension` const carries its extendee's proto FQN:
    println!("  RETRY_COUNT extends: {}", RETRY_COUNT.extendee());
    println!("  Envelope PROTO_FQN:  {}", Envelope::PROTO_FQN);
    assert_eq!(RETRY_COUNT.extendee(), Envelope::PROTO_FQN);

    // `extension()`, `set_extension()`, and `clear_extension()` panic if the
    // extendee doesn't match. `has_extension()` returns `false` gracefully —
    // "is this extension set here" has a legitimate answer when it can't
    // extend here (no).
    //
    // To see the panic, uncomment this — it would fail because TraceContext
    // has no extension ranges and RETRY_COUNT extends Envelope:
    //
    //   let tc = TraceContext::default();
    //   let _ = tc.extension(&RETRY_COUNT);
    //   // ^ panics: "extension at field 100 extends
    //   //   `buffa.examples.envelope.Envelope`, not
    //   //   `buffa.examples.envelope.TraceContext`"
    //
    // (This example can't actually compile the panic-triggering line because
    // TraceContext doesn't implement ExtensionSet — it has no `extensions`
    // range. The check fires when both sides DO implement ExtensionSet but
    // for different messages.)

    println!();
}

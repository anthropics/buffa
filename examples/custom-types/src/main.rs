//! End-to-end demonstration of buffa's pluggable owned types.
//!
//! `build.rs` compiles `proto/record.proto` with every owned-type knob
//! redirected to a crate-local newtype from [`types`]; this `main` builds a
//! [`Record`], round-trips it through binary and JSON, and statically asserts
//! that each field is the custom type — so the example fails to compile if a
//! knob stops applying.
//!
//! Run with `cargo run -p example-custom-types`.

pub mod types;

// Generated code is held to `-D warnings` but not the pedantic/nursery groups,
// so silence those here for integrators who lint at the stricter level.
#[allow(clippy::pedantic, clippy::nursery)]
mod proto {
    include!(concat!(env!("OUT_DIR"), "/_include.rs"));
}

use buffa::Message;
use proto::buffa::examples::customtypes::v1::{Metadata, Record};
use types::{FlexStr, IndexMap, SmallBox, SmallBytes, SmallVec};

/// Compile-time proof that each generated field uses the custom type.
/// Coercing a field reference to the named type is a no-op at runtime; the
/// guarantee comes from the function being **type-checked**, not const-eval —
/// if a knob regresses to the default representation this stops compiling.
#[allow(clippy::missing_const_for_fn)]
fn assert_field_types(r: &Record) {
    let _: &FlexStr = &r.id;
    let _: &SmallBytes = &r.payload;
    let _: &SmallVec<i64> = &r.samples;
    let _: &SmallVec<FlexStr> = &r.tags;
    let _: &IndexMap<i64, FlexStr> = &r.attributes;
    let _: &buffa::MessageField<Metadata, SmallBox<Metadata>> = &r.metadata;
}

fn metadata(author: &str, revision: i64) -> Metadata {
    Metadata {
        author: author.into(),
        revision,
        ..Default::default()
    }
}

fn build_record() -> Record {
    Record {
        id: "rec-001".into(),
        payload: b"hello, custom types".to_vec().into(),
        samples: SmallVec::from(vec![1, 1, 2, 3, 5]),
        tags: ["alpha", "beta"].into_iter().map(FlexStr::from).collect(),
        attributes: [(42, "answer".into()), (7, "lucky".into())]
            .into_iter()
            .collect(),
        metadata: metadata("iain", 3).into(),
        // `From<Metadata> for Option<Source>` wraps via `ProtoBox::new`, so
        // callers never construct the pointer themselves.
        source: metadata("inline", 1).into(),
        ..Default::default()
    }
}

fn main() {
    let record = build_record();
    assert_field_types(&record);

    // Binary round-trip.
    let wire = record.encode_to_vec();
    let decoded = Record::decode_from_slice(&wire).expect("binary decode");
    assert_eq!(record, decoded);
    println!("binary : {} bytes, round-trip OK", wire.len());

    // JSON round-trip — exercises the int64-map-key and oneof-message paths.
    let json = serde_json::to_string_pretty(&record).expect("json encode");
    let from_json: Record = serde_json::from_str(&json).expect("json decode");
    assert_eq!(record, from_json);
    println!("json   : round-trip OK\n{json}");

    // IndexMap preserves insertion order — neither key-sorted (BTreeMap) nor
    // hash-random (the default HashMap) — so encode and JSON are deterministic
    // in the order entries were added.
    let keys: Vec<i64> = record.attributes.0.keys().copied().collect();
    assert_eq!(keys, vec![42, 7]);
    println!("map    : insertion-order keys {keys:?}");
}

//! End-to-end tests for `with_*` setter methods (issue #30).

use crate::with_setters::{Priority, Request};
use buffa::{EnumValue, Message}; // EnumValue used in assertions

fn round_trip(req: &Request) -> Request {
    Request::decode(&mut req.encode_to_vec().as_slice()).expect("decode")
}

#[test]
fn chained_setters_produce_correct_values() {
    let req = Request::default()
        .with_count(42)
        .with_name("alice")
        .with_enabled(true)
        .with_priority(Priority::HIGH); // EnumValue<E>: From<E> — no wrapper needed

    assert_eq!(req.count, Some(42));
    assert_eq!(req.name, Some("alice".to_string()));
    assert_eq!(req.enabled, Some(true));
    assert_eq!(req.priority, Some(EnumValue::Known(Priority::HIGH)));
}

#[test]
fn setter_overwrites_prior_value() {
    let req = Request::default().with_count(1).with_count(99);
    assert_eq!(req.count, Some(99));
}

#[test]
fn unset_fields_remain_none() {
    let req = Request::default().with_count(7);
    assert_eq!(req.count, Some(7));
    assert_eq!(req.name, None);
    assert_eq!(req.enabled, None);
    assert_eq!(req.payload, None);
}

#[test]
fn setters_round_trip() {
    let req = Request::default()
        .with_count(5)
        .with_name("bob")
        .with_payload(b"hello".to_vec())
        .with_enabled(false);

    let rt = round_trip(&req);
    assert_eq!(rt.count, Some(5));
    assert_eq!(rt.name, Some("bob".to_string()));
    assert_eq!(rt.payload, Some(b"hello".to_vec()));
    assert_eq!(rt.enabled, Some(false));
}

#[test]
fn string_setter_accepts_str_ref() {
    // with_name takes impl Into<String>, so &str should compile directly.
    let req = Request::default().with_name("charlie");
    assert_eq!(req.name, Some("charlie".to_string()));
}

#[test]
fn bytes_setter_accepts_byte_array_literal() {
    // Vec<u8> uses impl Into<Vec<u8>>; From<&[u8; N]> for Vec<u8> is stable
    // since Rust 1.74, so b"..." literals work without .to_vec().
    let req = Request::default().with_payload(b"world");
    assert_eq!(req.payload, Some(b"world".to_vec()));
}

#[test]
fn enum_setter_accepts_bare_variant() {
    // EnumValue<E>: From<E> lets callers pass the variant directly.
    let req = Request::default().with_priority(Priority::LOW);
    assert_eq!(req.priority, Some(EnumValue::Known(Priority::LOW)));
}

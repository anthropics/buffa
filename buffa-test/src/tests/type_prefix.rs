//! `type_name_prefix` (#46) — basic.proto compiled with prefix `Rpc` (see
//! `build.rs` `prefix_variant`). Compilation already proves the prefixed
//! declarations and cross-references are consistent; these tests pin the
//! runtime behavior: the prefix is a pure Rust-identifier rename, invisible
//! on the wire and in JSON.

// The oneof tree keeps the proto-derived module name (`person`, not
// `rpc_person`) — only type identifiers carry the prefix.
use crate::basic_prefixed::__buffa::oneof::person::Contact;
use crate::basic_prefixed::{RpcAddress, RpcPerson, RpcPersonLazyView, RpcPersonView, RpcStatus};
use buffa::view::LazyMessageView;
use buffa::{Message, MessageView};

fn sample() -> RpcPerson {
    RpcPerson {
        id: 7,
        name: "ada".to_string(),
        status: RpcStatus::ACTIVE.into(),
        address: buffa::MessageField::some(RpcAddress {
            street: "1 Main St".to_string(),
            city: "Springfield".to_string(),
            zip_code: 12345,
            ..Default::default()
        }),
        tags: vec!["a".to_string(), "b".to_string()],
        contact: Some(Contact::Email("ada@example.com".to_string())),
        ..Default::default()
    }
}

#[test]
fn prefixed_types_round_trip() {
    let person = sample();
    let decoded = super::round_trip(&person);
    assert_eq!(person, decoded);
}

#[test]
fn prefixed_view_decodes_zero_copy() {
    let bytes = sample().encode_to_vec();
    let view = RpcPersonView::decode_view(&bytes).expect("view decode");
    assert_eq!(view.id, 7);
    assert_eq!(view.name, "ada");
    assert_eq!(view.to_owned_message().expect("to_owned"), sample());
}

#[test]
fn prefixed_lazy_view_decodes() {
    // The lazy-view family is re-exported under the prefixed name (the
    // import above is the regression check) and decodes like any other.
    let bytes = sample().encode_to_vec();
    let view = RpcPersonLazyView::decode_lazy(&bytes).expect("lazy decode");
    assert_eq!(view.id, 7);
    assert_eq!(view.name, "ada");
}

#[test]
fn prefix_does_not_change_wire_format() {
    // The same message built from the unprefixed compilation of basic.proto
    // must produce identical bytes — the prefix renames Rust identifiers
    // only.
    let unprefixed = crate::basic::Person {
        id: 7,
        name: "ada".to_string(),
        status: crate::basic::Status::ACTIVE.into(),
        address: buffa::MessageField::some(crate::basic::Address {
            street: "1 Main St".to_string(),
            city: "Springfield".to_string(),
            zip_code: 12345,
            ..Default::default()
        }),
        tags: vec!["a".to_string(), "b".to_string()],
        contact: Some(crate::basic::__buffa::oneof::person::Contact::Email(
            "ada@example.com".to_string(),
        )),
        ..Default::default()
    };
    assert_eq!(sample().encode_to_vec(), unprefixed.encode_to_vec());
}

#[test]
fn prefix_does_not_change_json_names() {
    // JSON field names come from the proto schema, not the Rust type name.
    let json = serde_json::to_value(sample()).expect("serialize");
    assert_eq!(json["name"], "ada");
    assert_eq!(json["status"], "ACTIVE");
    assert!(json.get("RpcPerson").is_none());
}

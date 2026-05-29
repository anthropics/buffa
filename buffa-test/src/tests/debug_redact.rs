//! `[debug_redact = true]`: Debug output must never contain the annotated
//! field's value — owned message, oneof variant, and view.

use crate::debug_redact::__buffa::oneof;
use crate::debug_redact::__buffa::view::CredentialsView;
use crate::debug_redact::*;
use buffa::{Message, MessageView};

fn credentials() -> Credentials {
    Credentials {
        api_key: "sk-ant-super-secret".to_string(),
        org_id: "org_123".to_string(),
        seed: vec![0xde, 0xad, 0xbe, 0xef],
        lookup: Some(oneof::credentials::Lookup::AccessToken(
            "oat-very-secret".to_string(),
        )),
        scopes: vec!["scope-admin-org".to_string()],
        ..Default::default()
    }
}

#[test]
fn owned_message_debug_redacts_annotated_fields() {
    let out = format!("{:?}", credentials());
    assert!(
        !out.contains("sk-ant-super-secret"),
        "api_key leaked: {out}"
    );
    assert!(
        !out.contains("222, 173, 190, 239"),
        "seed bytes leaked: {out}"
    );
    assert!(
        !out.contains("scope-admin-org"),
        "repeated scopes leaked: {out}"
    );
    assert!(
        !out.contains("oat-very-secret"),
        "oneof payload leaked: {out}"
    );
    assert!(out.contains("[REDACTED]"), "placeholder missing: {out}");
    assert!(
        out.contains("org_123"),
        "unannotated field must still print: {out}"
    );
}

#[test]
fn oneof_debug_keeps_unannotated_variant() {
    let lookup = oneof::credentials::Lookup::TokenUuid("uuid-1".to_string());
    let out = format!("{lookup:?}");
    assert!(
        out.contains("uuid-1"),
        "unannotated variant must print: {out}"
    );
}

#[test]
fn view_debug_redacts_annotated_fields() {
    let bytes = credentials().encode_to_vec();
    let view = CredentialsView::decode_view(&bytes).expect("decode_view");
    let out = format!("{view:?}");
    assert!(
        !out.contains("sk-ant-super-secret"),
        "view leaked api_key: {out}"
    );
    assert!(
        !out.contains("oat-very-secret"),
        "view-oneof leaked payload: {out}"
    );
    assert!(
        !out.contains("222, 173, 190, 239"),
        "view leaked seed bytes: {out}"
    );
    assert!(
        !out.contains("scope-admin-org"),
        "view leaked repeated scopes: {out}"
    );
    assert!(out.contains("[REDACTED]"), "placeholder missing: {out}");
    assert!(
        out.contains("org_123"),
        "unannotated field must still print: {out}"
    );
    assert!(
        !out.contains("__buffa_unknown_fields"),
        "redacted view Debug lists proto fields only: {out}"
    );
}

#[test]
fn all_redacted_scalar_oneof_debug_prints_placeholder_only() {
    let secret = oneof::credentials::NumericSecret::Pin(43210099);
    let out = format!("{secret:?}");
    assert!(!out.contains("43210099"), "pin leaked: {out}");
    assert!(out.contains("[REDACTED]"), "placeholder missing: {out}");
}

#[test]
fn view_oneof_without_lifetime_debug_redacts_all_variants() {
    // `numeric_secret` has only scalar payloads, so its view-oneof enum has no
    // lifetime parameter — exercises the un-parameterized manual Debug impl.
    let msg = Credentials {
        numeric_secret: Some(oneof::credentials::NumericSecret::Pin(43210099)),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = CredentialsView::decode_view(&bytes).expect("decode_view");
    let out = format!("{view:?}");
    assert!(!out.contains("43210099"), "view leaked pin: {out}");
    assert!(out.contains("[REDACTED]"), "placeholder missing: {out}");
}

#[test]
fn view_oneof_debug_keeps_unannotated_variant() {
    let msg = Credentials {
        org_id: "org_123".to_string(),
        lookup: Some(oneof::credentials::Lookup::TokenUuid("uuid-1".to_string())),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = CredentialsView::decode_view(&bytes).expect("decode_view");
    let out = format!("{view:?}");
    assert!(
        out.contains("uuid-1"),
        "unannotated view-oneof variant must print: {out}"
    );
}

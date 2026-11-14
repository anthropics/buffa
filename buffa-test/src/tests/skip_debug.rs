//! `skip_debug`: the crate's own `Debug` impls, in `lib.rs`, stand in for the
//! generated ones. That `lib.rs` compiles is the main check, because a
//! generated impl left in place would conflict with them.

use crate::skip_debug::{
    token, Level, Session, SessionView, Token, TokenLazyView, TokenOwnedView, TokenView,
};
use buffa::{LazyMessageView, Message, MessageView, ViewEncode};

fn sample_token() -> Token {
    Token {
        id: "t1".to_string(),
        scope: token::Scope::SCOPE_ADMIN.into(),
        holder: Some(token::Holder::User("root".to_string())),
        ..Default::default()
    }
}

#[test]
fn the_crate_impls_format_the_matched_types() {
    let sample = sample_token();
    assert_eq!(format!("{sample:?}"), "Token#t1");
    assert_eq!(format!("{:?}", sample.holder), "Some(user:root)");
    assert_eq!(format!("{:?}", Level::LEVEL_HIGH), "LEVEL_HIGH");
}

#[test]
fn an_unmatched_parent_formats_the_child_through_the_crate_impl() {
    let session = Session {
        token: buffa::MessageField::some(sample_token()),
        ..Default::default()
    };
    let out = format!("{session:?}");
    assert!(out.starts_with("Session"), "{out}");
    assert!(out.contains("Token#t1"), "{out}");
}

#[test]
fn a_nested_enum_keeps_its_generated_debug() {
    assert_eq!(format!("{:?}", token::Scope::SCOPE_ADMIN), "SCOPE_ADMIN");
}

#[test]
fn matched_views_omit_known_and_preserved_unknown_fields_from_debug() {
    let mut bytes = sample_token().encode_to_vec();
    // The older schema cannot annotate the unknown field for redaction.
    bytes.extend_from_slice(&super::length_delimited_field(99, b"future"));

    let view = TokenView::decode_view(&bytes).unwrap();
    assert_eq!(format!("{view:?}"), "TokenView([REDACTED])");
    assert_eq!(format!("{view:#?}"), "TokenView([REDACTED])");
    assert_eq!(
        format!("{:?}", view.holder.as_ref().unwrap()),
        "Holder([REDACTED])"
    );
    assert_eq!(view.encode_to_vec(), bytes);

    let lazy = TokenLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(format!("{lazy:?}"), "TokenLazyView([REDACTED])");
    assert_eq!(lazy.encode_to_vec(), bytes);

    let owned = TokenOwnedView::decode(buffa::bytes::Bytes::copy_from_slice(&bytes)).unwrap();
    assert_eq!(
        format!("{owned:?}"),
        "TokenOwnedView(TokenView([REDACTED]))"
    );
    assert_eq!(owned.view().encode_to_vec(), bytes);
}

#[test]
fn an_unmatched_parent_view_uses_the_childs_redacted_formatter() {
    let session = Session {
        token: buffa::MessageField::some(sample_token()),
        ..Default::default()
    };
    let bytes = session.encode_to_vec();
    let view = SessionView::decode_view(&bytes).unwrap();
    let out = format!("{view:?}");
    assert!(out.starts_with("SessionView"), "{out}");
    assert!(out.contains("TokenView([REDACTED])"), "{out}");
    assert!(!out.contains("root"), "{out}");
    assert!(!out.contains("t1"), "{out}");
}

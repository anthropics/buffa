//! `skip_debug`: the crate's own `Debug` impls, in `lib.rs`, stand in for the
//! generated ones. That `lib.rs` compiles is the main check, because a
//! generated impl left in place would conflict with them.

use crate::skip_debug::__buffa::view::TokenView;
use crate::skip_debug::{token, Level, Session, Token};
use buffa::{Message, MessageView};

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
fn a_nested_enum_and_the_view_keep_their_generated_debug() {
    assert_eq!(format!("{:?}", token::Scope::SCOPE_ADMIN), "SCOPE_ADMIN");

    let bytes = sample_token().encode_to_vec();
    let view = TokenView::decode_view(&bytes).expect("decode_view");
    let out = format!("{view:?}");
    assert!(out.starts_with("TokenView"), "{out}");
    assert!(out.contains("\"t1\""), "{out}");
}

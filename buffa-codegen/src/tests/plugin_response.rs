//! The response both plugins build their responses from.

use crate::generated::compiler::code_generator_response::Feature;
use crate::generated::descriptor::Edition;
use crate::plugin_response;

#[test]
fn plugin_response_declares_features_and_edition_range() {
    let response = plugin_response();
    assert_eq!(
        response.supported_features,
        Some(Feature::FEATURE_PROTO3_OPTIONAL as u64 | Feature::FEATURE_SUPPORTS_EDITIONS as u64)
    );
    // FEATURE_SUPPORTS_EDITIONS without the range is rejected by buf.
    assert_eq!(
        response.minimum_edition,
        Some(Edition::EDITION_PROTO2 as i32)
    );
    assert_eq!(response.maximum_edition, Some(Edition::EDITION_2024 as i32));
}

#[test]
fn plugin_response_is_otherwise_empty() {
    let response = plugin_response();
    assert_eq!(response.error, None);
    assert!(response.file.is_empty());
}

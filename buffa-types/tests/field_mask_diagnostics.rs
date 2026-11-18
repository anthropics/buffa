//! Serialization failures must not disclose application-owned field paths.
#![cfg(all(feature = "json", feature = "reflect"))]
use buffa::{Message, MessageView};
use buffa_descriptor::reflect::Reflectable;
use buffa_types::google::protobuf::{__buffa::view::FieldMaskView, FieldMask};

#[test]
fn invalid_paths_are_redacted_in_owned_view_and_reflective_json() {
    for path in ["marker_X", "marker..private", "marker_7", "MarkerSecret"] {
        let mask = FieldMask {
            paths: vec![path.into()],
            ..Default::default()
        };
        let expected = "invalid FieldMask path, it has no proto3 JSON representation";
        assert_eq!(
            serde_json::to_string(&mask).unwrap_err().to_string(),
            expected
        );
        let bytes = mask.encode_to_vec();
        let view = FieldMaskView::decode_view(&bytes).unwrap();
        assert_eq!(
            serde_json::to_string(&view).unwrap_err().to_string(),
            expected
        );
        let dynamic = mask.reflect().to_dynamic();
        assert_eq!(
            serde_json::to_string(&dynamic).unwrap_err().to_string(),
            expected
        );
    }
    let valid = FieldMask {
        paths: vec!["account_id".into(), "profile.display_name".into()],
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_string(&valid).unwrap(),
        r#""accountId,profile.displayName""#
    );
}

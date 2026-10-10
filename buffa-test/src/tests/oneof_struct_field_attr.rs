//! `oneof_struct_field_attribute`: a serde attribute on the struct field
//! holding a oneof, read by a `Serialize` that build.rs derives itself. That
//! the module compiles shows the attribute is on the field and not on a
//! variant, where serde rejects `skip_serializing_if`.

use crate::oneof_struct_field_attr::{event, Event};

#[test]
fn unset_oneof_is_skipped_by_the_derived_serialize() {
    let event = Event {
        id: "e1".to_string(),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({ "id": "e1" })
    );
}

#[test]
fn set_oneof_is_serialized_by_the_derived_serialize() {
    let event = Event {
        id: "e1".to_string(),
        payload: Some(event::Payload::Text("hi".to_string())),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({ "id": "e1", "payload": { "Text": "hi" } })
    );
}

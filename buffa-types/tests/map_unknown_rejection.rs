//! Checked-in view code must preserve rejection inside synthetic map entries.
use buffa::{DecodeError, DecodeOptions, Message, MessageView};
use buffa_types::google::protobuf::{Struct, StructView};

#[test]
fn struct_map_entry_unknowns_are_rejected_by_owned_and_view_decoders() {
    // Struct.fields entry containing field 3, outside the entry's key/value pair.
    let wire = [0x0a, 2, 0x18, 1];
    assert!(Struct::decode_from_slice(&wire).is_ok());
    assert!(StructView::decode_view(&wire).is_ok());
    let options = DecodeOptions::new().reject_unknown_fields();
    assert!(matches!(
        options.decode_from_slice::<Struct>(&wire),
        Err(DecodeError::UnknownFieldLimitExceeded)
    ));
    assert!(matches!(
        options.decode_view::<StructView<'_>>(&wire),
        Err(DecodeError::UnknownFieldLimitExceeded)
    ));
}

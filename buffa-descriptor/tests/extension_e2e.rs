//! End-to-end tests for extension reflection: wire round-trip, JSON
//! round-trip, and the `ReflectMessage` accessor surface over extension
//! fields.
//!
//! Extensions are fields declared outside the message they belong to. The
//! reflective model follows protobuf-go: an extension's
//! [`FieldDescriptor`] is passed to the same `get`/`set`/`has` accessors as
//! a declared field, and extension values live in the same per-number map.

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa_descriptor::reflect::{DynamicMessage, ReflectMessage, ReflectMessageMut, Value};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

#[test]
fn extension_set_get_has_through_reflect_message() {
    let p = pool();
    let idx = p.message_index("reflect.ext.Extendable").unwrap();
    let ext = p.extension_by_name("reflect.ext.ext_int32").unwrap();

    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    assert!(!msg.has(ext.field()));
    msg.set(ext.field(), Value::I32(42));
    assert!(msg.has(ext.field()));
    assert!(matches!(
        msg.get(ext.field()),
        buffa_descriptor::reflect::ValueRef::I32(42)
    ));

    // for_each_set visits the extension alongside declared fields.
    let md = p.message_by_name("reflect.ext.Extendable").unwrap();
    msg.set(md.field(1).unwrap(), Value::I32(1));
    let mut seen: Vec<u32> = Vec::new();
    msg.for_each_set(&mut |fd, _| seen.push(fd.number()));
    seen.sort_unstable();
    assert_eq!(seen, vec![1, 100]);
}

#[test]
fn extension_binary_round_trip() {
    let p = pool();
    let idx = p.message_index("reflect.ext.Extendable").unwrap();
    let payload_idx = p.message_index("reflect.ext.Payload").unwrap();
    let md = p.message_by_name("reflect.ext.Extendable").unwrap();

    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    msg.set(md.field(1).unwrap(), Value::I32(7));
    let ext_i32 = p.extension_by_name("reflect.ext.ext_int32").unwrap();
    let ext_str = p.extension_by_name("reflect.ext.ext_string").unwrap();
    let ext_rep = p.extension_by_name("reflect.ext.ext_repeated").unwrap();
    let ext_msg = p.extension_by_name("reflect.ext.ext_message").unwrap();
    let ext_nested = p.extension_by_name("reflect.ext.Scope.ext_nested").unwrap();
    msg.set(ext_i32.field(), Value::I32(100));
    msg.set(ext_str.field(), Value::String("hello".into()));
    msg.set(
        ext_rep.field(),
        Value::List(vec![Value::I32(1), Value::I32(2)]),
    );
    let mut payload = DynamicMessage::new(Arc::clone(&p), payload_idx);
    payload.set(
        p.message_by_name("reflect.ext.Payload")
            .unwrap()
            .field(1)
            .unwrap(),
        Value::String("inner".into()),
    );
    msg.set(ext_msg.field(), Value::Message(payload));
    msg.set(ext_nested.field(), Value::I64(9));

    // Encode → decode → all extensions resolve typed.
    let bytes = msg.encode_to_vec();
    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &bytes).unwrap();
    assert_eq!(decoded.field_by_number(100), Some(&Value::I32(100)));
    assert_eq!(
        decoded.field_by_number(101),
        Some(&Value::String("hello".into()))
    );
    assert_eq!(
        decoded.field_by_number(102),
        Some(&Value::List(vec![Value::I32(1), Value::I32(2)]))
    );
    assert!(matches!(
        decoded.field_by_number(103),
        Some(&Value::Message(_))
    ));
    assert_eq!(decoded.field_by_number(110), Some(&Value::I64(9)));
    // Nothing leaked into unknown fields.
    assert_eq!(decoded.unknown_fields().len(), 0);
    // Re-encoding is stable.
    assert_eq!(decoded.encode_to_vec(), bytes);
}

#[test]
fn unregistered_extension_range_number_round_trips_as_unknown() {
    let p = pool();
    let idx = p.message_index("reflect.ext.Extendable").unwrap();
    // Field 150 is inside `extensions 100 to 199` but no extension is
    // registered there. Hand-craft wire bytes: tag(150, VARINT)=5.
    let mut bytes = Vec::new();
    buffa::encoding::Tag::new(150, buffa::encoding::WireType::Varint).encode(&mut bytes);
    buffa::encoding::encode_varint(5, &mut bytes);
    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &bytes).unwrap();
    // Preserved as an unknown field, not silently dropped.
    assert_eq!(decoded.unknown_fields().len(), 1);
    assert_eq!(decoded.encode_to_vec(), bytes);
}

#[test]
fn open_ended_extension_ranges_skip_reserved_band() {
    let p = pool();
    let md = p.message_by_name("reflect.closed.Contexts").unwrap();
    assert_eq!(
        md.extension_ranges(),
        &[
            (100, buffa::encoding::FIRST_RESERVED_FIELD_NUMBER),
            (
                buffa::encoding::LAST_RESERVED_FIELD_NUMBER + 1,
                buffa::encoding::MAX_FIELD_NUMBER + 1,
            ),
        ]
    );
}

#[cfg(feature = "json")]
mod json {
    use super::*;

    #[test]
    fn extension_json_round_trip() {
        let p = pool();
        let idx = p.message_index("reflect.ext.Extendable").unwrap();
        let ext = p.extension_by_name("reflect.ext.ext_int32").unwrap();
        let nested = p.extension_by_name("reflect.ext.Scope.ext_nested").unwrap();

        let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
        msg.set(ext.field(), Value::I32(42));
        msg.set(nested.field(), Value::I64(9));
        let json = msg.to_json().unwrap();
        assert!(
            json.contains(r#""[reflect.ext.ext_int32]":42"#),
            "extension serialized with bracketed full name: {json}"
        );
        assert!(
            json.contains(r#""[reflect.ext.Scope.ext_nested]":"9""#),
            "int64 extension serialized as quoted string: {json}"
        );

        // Parse it back.
        let parsed = DynamicMessage::from_json(Arc::clone(&p), idx, &json).unwrap();
        assert_eq!(parsed.field_by_number(100), Some(&Value::I32(42)));
        assert_eq!(parsed.field_by_number(110), Some(&Value::I64(9)));
        assert_eq!(parsed, msg);
    }

    #[test]
    fn extension_json_rejects_wrong_extendee_and_unknown() {
        let p = pool();
        // `ext_int32` extends Extendable, not Payload.
        let payload_idx = p.message_index("reflect.ext.Payload").unwrap();
        assert!(DynamicMessage::from_json(
            Arc::clone(&p),
            payload_idx,
            r#"{"[reflect.ext.ext_int32]": 1}"#
        )
        .is_err());
        // An unregistered extension name is an unknown field: error in
        // strict mode, skipped in lenient mode.
        let idx = p.message_index("reflect.ext.Extendable").unwrap();
        let input = r#"{"[reflect.ext.no_such_ext]": 1}"#;
        assert!(DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err());
        let m = DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), idx, input).unwrap();
        assert_eq!(m.encode_to_vec(), Vec::<u8>::new());
    }

    #[test]
    fn extension_json_duplicate_key_rejected() {
        let p = pool();
        let idx = p.message_index("reflect.ext.Extendable").unwrap();
        // The same extension twice — caught by the seen-field-numbers check.
        assert!(DynamicMessage::from_json(
            Arc::clone(&p),
            idx,
            r#"{"[reflect.ext.ext_int32]": 1, "[reflect.ext.ext_int32]": 2}"#
        )
        .is_err());
    }
}

#[test]
fn group_typed_extension_round_trips() {
    // Groups use StartGroup/EndGroup wire framing instead of a length
    // prefix. protoc names the synthetic field after the lowercased group
    // name, so `optional group ExtGroup = 120` registers as
    // `reflect.ext.extgroup` with value type `reflect.ext.ExtGroup`.
    let p = pool();
    let idx = p.message_index("reflect.ext.Extendable").unwrap();
    let ext = p
        .extension_by_name("reflect.ext.extgroup")
        .expect("group extension registered");
    assert!(ext.field().is_delimited(), "groups encode delimited");
    let group_idx = p.message_index("reflect.ext.ExtGroup").unwrap();

    let mut group = DynamicMessage::new(Arc::clone(&p), group_idx);
    group.set(p.message(group_idx).field(1).unwrap(), Value::I32(77));
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    msg.set(ext.field(), Value::Message(group));

    let bytes = msg.encode_to_vec();
    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &bytes).unwrap();
    assert_eq!(decoded.unknown_fields().len(), 0, "group decoded typed");
    let Some(Value::Message(g)) = decoded.field_by_number(120) else {
        panic!("group extension not present after round-trip");
    };
    assert_eq!(g.field_by_number(1), Some(&Value::I32(77)));
    assert_eq!(decoded.encode_to_vec(), bytes);
}

/// Build a minimal FileDescriptorSet by mutating the test fixture's raw
/// descriptors, for exercising the pool's malformed-input rejection paths.
mod malformed {
    use super::*;
    use buffa::Message;
    use buffa_descriptor::generated::descriptor::FileDescriptorSet;

    fn base_set() -> FileDescriptorSet {
        FileDescriptorSet::decode_from_slice(FDS_BYTES).unwrap()
    }

    #[test]
    fn duplicate_extension_number_rejected() {
        let mut set = base_set();
        // Duplicate the first file-level extension under a new name — same
        // extendee, same number.
        let ext_file = set
            .file
            .iter_mut()
            .find(|f| f.package.as_deref() == Some("reflect.ext"))
            .unwrap();
        let mut dup = ext_file.extension[0].clone();
        dup.name = Some("ext_int32_clone".into());
        ext_file.extension.push(dup);
        let err = DescriptorPool::new(set).unwrap_err();
        assert!(
            err.to_string().contains("more than one extension claims"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn extension_outside_declared_range_rejected() {
        let mut set = base_set();
        let ext_file = set
            .file
            .iter_mut()
            .find(|f| f.package.as_deref() == Some("reflect.ext"))
            .unwrap();
        // `extensions 100 to 199` — move an extension to 200.
        ext_file.extension[0].number = Some(200);
        let err = DescriptorPool::new(set).unwrap_err();
        assert!(
            err.to_string().contains("invalid field number"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn implementation_reserved_extension_numbers_rejected() {
        for number in [
            buffa::encoding::FIRST_RESERVED_FIELD_NUMBER as i32,
            buffa::encoding::LAST_RESERVED_FIELD_NUMBER as i32,
        ] {
            let mut set = base_set();
            let ext_file = set
                .file
                .iter_mut()
                .find(|f| f.package.as_deref() == Some("reflect.ext"))
                .unwrap();
            ext_file.extension[0].number = Some(number);
            let extendable = ext_file
                .message_type
                .iter_mut()
                .find(|m| m.name.as_deref() == Some("Extendable"))
                .unwrap();
            extendable.extension_range[0].start = Some(1);
            extendable.extension_range[0].end = Some(buffa::encoding::MAX_FIELD_NUMBER as i32 + 1);

            let err = DescriptorPool::new(set).unwrap_err();
            assert!(
                matches!(
                    &err,
                    buffa_descriptor::PoolError::ReservedFieldNumber { field, number: actual }
                        if field == "reflect.ext.ext_int32" && *actual == number
                ),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn implementation_reserved_extension_ranges_rejected() {
        for (start, end) in [
            (
                buffa::encoding::FIRST_RESERVED_FIELD_NUMBER as i32,
                (buffa::encoding::LAST_RESERVED_FIELD_NUMBER + 1) as i32,
            ),
            (
                (buffa::encoding::FIRST_RESERVED_FIELD_NUMBER - 1) as i32,
                (buffa::encoding::FIRST_RESERVED_FIELD_NUMBER + 1) as i32,
            ),
            (
                buffa::encoding::LAST_RESERVED_FIELD_NUMBER as i32,
                (buffa::encoding::LAST_RESERVED_FIELD_NUMBER + 1) as i32,
            ),
        ] {
            let mut set = base_set();
            let ext_file = set
                .file
                .iter_mut()
                .find(|f| f.package.as_deref() == Some("reflect.ext"))
                .unwrap();
            let extendable = ext_file
                .message_type
                .iter_mut()
                .find(|m| m.name.as_deref() == Some("Extendable"))
                .unwrap();
            extendable.extension_range[0].start = Some(start);
            extendable.extension_range[0].end = Some(end);

            let err = DescriptorPool::new(set).unwrap_err();
            assert!(
                matches!(
                    &err,
                    buffa_descriptor::PoolError::ReservedFieldNumber { field, number }
                        if field == "reflect.ext.Extendable (extension range)"
                            && *number == start.max(buffa::encoding::FIRST_RESERVED_FIELD_NUMBER as i32)
                ),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn extension_ranges_adjacent_to_reserved_band_are_accepted() {
        for (start, end) in [
            (
                (buffa::encoding::FIRST_RESERVED_FIELD_NUMBER - 1) as i32,
                buffa::encoding::FIRST_RESERVED_FIELD_NUMBER as i32,
            ),
            (
                (buffa::encoding::LAST_RESERVED_FIELD_NUMBER + 1) as i32,
                (buffa::encoding::LAST_RESERVED_FIELD_NUMBER + 2) as i32,
            ),
        ] {
            let mut set = base_set();
            let ext_file = set
                .file
                .iter_mut()
                .find(|f| f.package.as_deref() == Some("reflect.ext"))
                .unwrap();
            ext_file.extension.clear();
            for message in &mut ext_file.message_type {
                message.extension.clear();
            }
            let extendable = ext_file
                .message_type
                .iter_mut()
                .find(|m| m.name.as_deref() == Some("Extendable"))
                .unwrap();
            extendable.extension_range[0].start = Some(start);
            extendable.extension_range[0].end = Some(end);

            assert!(
                DescriptorPool::new(set).is_ok(),
                "extension range {start}..{end} should be accepted"
            );
        }
    }

    #[test]
    fn extension_oneof_index_is_scrubbed() {
        let mut set = base_set();
        let ext_file = set
            .file
            .iter_mut()
            .find(|f| f.package.as_deref() == Some("reflect.ext"))
            .unwrap();
        // A hostile FDS marks an extension as a oneof member. Without
        // scrubbing, set() would clear the extendee's declared oneof
        // members at that index.
        ext_file.extension[0].oneof_index = Some(0);
        let p = DescriptorPool::new(set).unwrap();
        let ext = p.extension_by_name("reflect.ext.ext_int32").unwrap();
        assert_eq!(ext.field().oneof_index(), None);
    }
}

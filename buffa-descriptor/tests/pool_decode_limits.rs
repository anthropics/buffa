//! [`DescriptorPool::decode`] applies buffa's untrusted-input decode limits;
//! [`DescriptorPool::decode_with_options`] lets a caller who produced the
//! bytes raise them.
//!
//! The element-memory bound is the one a real schema hits. Descriptor types
//! are wide structs, so a `FileDescriptorSet`'s element footprint runs several
//! times its encoded size and a few hundred `.proto` files can exceed the
//! default. See #331.

#![cfg(feature = "reflect")]

use buffa::{DecodeError, DecodeOptions, Message};
use buffa_descriptor::generated::descriptor::field_descriptor_proto::{Label, Type};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use buffa_descriptor::{DescriptorPool, PoolError};

/// A valid `FileDescriptorSet` with enough files to push its element footprint
/// past `files * messages * fields` elements, without needing a large payload.
fn wide_descriptor_set(files: usize, messages_per_file: usize) -> Vec<u8> {
    let mut set = FileDescriptorSet::default();
    for f in 0..files {
        let mut file = FileDescriptorProto {
            name: Some(format!("wide/f{f}.proto")),
            package: Some(format!("wide.p{f}")),
            syntax: Some("proto3".to_string()),
            ..Default::default()
        };
        for m in 0..messages_per_file {
            file.message_type.push(DescriptorProto {
                name: Some(format!("M{m}")),
                field: (0..8)
                    .map(|k| FieldDescriptorProto {
                        name: Some(format!("f{k}")),
                        number: Some(k + 1),
                        label: Some(Label::Optional),
                        r#type: Some(Type::String),
                        json_name: Some(format!("f{k}")),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            });
        }
        set.file.push(file);
    }
    set.encode_to_vec()
}

#[test]
fn decode_applies_the_default_element_memory_bound() {
    let bytes = wide_descriptor_set(400, 40);

    // Well under the 2 GiB message cap, so size is not what rejects it.
    assert!(bytes.len() < 32 * 1024 * 1024, "{} bytes", bytes.len());

    match DescriptorPool::decode(&bytes) {
        Err(PoolError::Decode(DecodeError::ElementMemoryLimitExceeded)) => {}
        other => panic!("expected the element-memory bound to reject this, got {other:?}"),
    }
}

#[test]
fn decode_with_options_can_lift_the_bound() {
    let bytes = wide_descriptor_set(400, 40);
    let opts = DecodeOptions::new().with_element_memory_limit(usize::MAX);

    let pool = DescriptorPool::decode_with_options(&bytes, &opts)
        .expect("an unbounded decode accepts the same bytes");

    // Decoded the whole thing, not a truncated prefix.
    assert!(pool.message_index("wide.p0.M0").is_some());
    assert!(pool.message_index("wide.p399.M39").is_some());
}

#[test]
fn decode_with_options_still_rejects_beyond_the_given_bound() {
    let bytes = wide_descriptor_set(400, 40);
    let opts = DecodeOptions::new().with_element_memory_limit(64 * 1024);

    // The knob raises *and* lowers: a caller-supplied bound is honoured.
    match DescriptorPool::decode_with_options(&bytes, &opts) {
        Err(PoolError::Decode(DecodeError::ElementMemoryLimitExceeded)) => {}
        other => panic!("expected a caller-supplied bound to be enforced, got {other:?}"),
    }
}

#[test]
fn decode_and_decode_with_options_agree_on_defaults() {
    // A small set both accept, proving the refactor of `decode` onto
    // `decode_with_options` did not change its behaviour.
    let bytes = wide_descriptor_set(2, 2);

    let a = DescriptorPool::decode(&bytes).expect("small set decodes");
    let b = DescriptorPool::decode_with_options(&bytes, &DecodeOptions::new())
        .expect("small set decodes with explicit defaults");

    assert!(a.message_index("wide.p1.M1").is_some());
    assert_eq!(a.message_index("wide.p1.M1"), b.message_index("wide.p1.M1"));
}

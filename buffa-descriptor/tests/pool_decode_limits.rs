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

/// A descriptor set whose bulk is empty nested messages with one-character
/// names — 5 wire bytes each, materializing a whole `DescriptorProto`.
fn short_named_empties(outer: usize, nested_each: usize) -> Vec<u8> {
    let mut set = FileDescriptorSet::default();
    let mut file = FileDescriptorProto {
        name: Some("p.proto".to_string()),
        package: Some("p".to_string()),
        syntax: Some("proto3".to_string()),
        ..Default::default()
    };
    for o in 0..outer {
        let mut msg = DescriptorProto {
            name: Some(format!("Outer{o}")),
            ..Default::default()
        };
        for n in 0..nested_each {
            let c = char::from(b'A' + u8::try_from(n % 26).expect("nested index fits a letter"));
            msg.nested_type.push(DescriptorProto {
                name: Some(c.to_string()),
                ..Default::default()
            });
        }
        file.message_type.push(msg);
    }
    set.file.push(file);
    set.encode_to_vec()
}

#[test]
fn a_length_scaled_bound_needs_the_default_as_a_floor() {
    // Why generated `descriptor_pool()` floors its scaled bound at
    // `DEFAULT_ELEMENT_MEMORY_LIMIT` rather than using `len * 64` alone (#336).
    //
    // The element-to-encoded ratio is a property of the schema's shape, not
    // its size: `size_of::<DescriptorProto>()` is charged for something that
    // costs 5 bytes on the wire, so this set needs more than 64x however
    // small it is. Scaling alone would reject it; the default accepts it.
    //
    // This records the rationale; it does not guard the emitted code, because
    // it rebuilds the bound here rather than reading it out of codegen. The
    // guard is `the_emitted_pool_bound_is_floored_at_the_default` in
    // buffa-codegen, which fails if the `.max(..)` is dropped.
    // 728 bytes — the instance the changelog and #336 cite.
    let bytes = short_named_empties(5, 26);
    assert!(
        bytes.len() < 32 * 1024,
        "stays far under the default: {}",
        bytes.len()
    );

    let scaled_only =
        DecodeOptions::new().with_element_memory_limit(bytes.len().saturating_mul(64));
    match DescriptorPool::decode_with_options(&bytes, &scaled_only) {
        Err(PoolError::Decode(DecodeError::ElementMemoryLimitExceeded)) => {}
        other => panic!("expected len*64 alone to reject this shape, got {other:?}"),
    }

    let floored = DecodeOptions::new().with_element_memory_limit(
        bytes
            .len()
            .saturating_mul(64)
            .max(buffa::DEFAULT_ELEMENT_MEMORY_LIMIT),
    );
    DescriptorPool::decode_with_options(&bytes, &floored)
        .expect("the floored bound accepts what the default always did");

    // And the plain default accepts it, which is what makes the floor correct
    // rather than merely generous: without it this is a regression.
    DescriptorPool::decode(&bytes).expect("the default has always accepted this");
}

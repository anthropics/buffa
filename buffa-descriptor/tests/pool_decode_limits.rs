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

/// A fully-qualified name is capped, which bounds two things at once.
///
/// The amplification: every descendant stores its own copy of the prefix,
/// four times over, so `K` leaves under a `P`-byte prefix cost `4KP` bytes of
/// pool from about `P + 7K` bytes of input. A message's `name` is a singular
/// field, so the decode-time element budget never sees the prefix at all.
///
/// And the depth: pool construction walks nested messages recursively in four
/// places with no depth counter, and each level adds at least one byte to the
/// name, so the name cap is also a depth cap.
#[test]
fn a_long_prefix_is_refused_rather_than_stored_once_per_descendant() {
    use buffa_descriptor::MAX_SYMBOL_LEN;

    /// One message named `name`, holding `leaves` empty single-character
    /// nested messages.
    fn set_with_prefix(name: &str, leaves: usize) -> Vec<u8> {
        let mut outer = DescriptorProto {
            name: Some(name.to_string()),
            ..Default::default()
        };
        for n in 0..leaves {
            let c = char::from(b'a' + u8::try_from(n % 26).expect("fits a letter"));
            outer.nested_type.push(DescriptorProto {
                name: Some(format!("{c}{n}")),
                ..Default::default()
            });
        }
        FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("p.proto".to_string()),
                package: Some("p".to_string()),
                syntax: Some("proto3".to_string()),
                message_type: vec![outer],
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec()
    }

    // Well inside the cap: a perfectly ordinary schema is untouched.
    let ok = set_with_prefix("Outer", 64);
    assert!(DescriptorPool::decode(&ok).is_ok());

    // A prefix past the cap is refused, and the error says so — rather than
    // being silently stored once per leaf.
    let long = "L".repeat(MAX_SYMBOL_LEN + 1);
    match DescriptorPool::decode(&set_with_prefix(&long, 8)) {
        Err(PoolError::NameTooLong { len, limit }) => {
            assert!(len > limit, "{len} should exceed {limit}");
            assert_eq!(limit, MAX_SYMBOL_LEN);
        }
        other => panic!("expected NameTooLong, got {other:?}"),
    }
}

/// The cap is reached in pass 1, before the three later recursive walks run,
/// so deep nesting cannot drive them past it either.
///
/// Built through `DescriptorPool::new` rather than `decode`, deliberately.
/// `new` takes an already-materialized `FileDescriptorSet`, so the decoder's
/// recursion limit never applies — that is the path where the four
/// uncounted recursive walks could previously run to a stack overflow, which
/// is a `SIGSEGV` rather than a catchable error. It also keeps the test from
/// depending on where the decode recursion limit happens to sit.
#[test]
fn nesting_deeper_than_the_name_cap_is_refused() {
    use buffa_descriptor::MAX_SYMBOL_LEN;

    // Past the decoder's own recursion limit, so this nesting could only ever
    // arrive through `new`.
    let depth = 200;
    let mut inner = DescriptorProto {
        name: Some("x".repeat(20)),
        ..Default::default()
    };
    for _ in 0..depth {
        inner = DescriptorProto {
            name: Some("x".repeat(20)),
            nested_type: vec![inner],
            ..Default::default()
        };
    }
    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("d.proto".to_string()),
            package: Some("d".to_string()),
            syntax: Some("proto3".to_string()),
            message_type: vec![inner],
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(
        (depth + 1) * 21 > MAX_SYMBOL_LEN,
        "the fixture must actually exceed the cap"
    );
    assert!(matches!(
        DescriptorPool::new(set),
        Err(PoolError::NameTooLong { .. })
    ));
}

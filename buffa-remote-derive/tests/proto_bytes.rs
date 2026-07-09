use buffa::{ProtoBytes, WirePayload};
use buffa_remote_derive::ProtoBytes as DeriveProtoBytes;
use smallvec::SmallVec;

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoBytes)]
#[buffa(remote = smallvec::SmallVec<[u8; 16]>)]
struct MyBytes(pub SmallVec<[u8; 16]>);

#[test]
fn from_wire_copies_payload() {
    let b = MyBytes::from_wire(WirePayload::borrowed(b"hello bytes")).unwrap();
    assert_eq!(b.as_ref(), b"hello bytes");
}

#[test]
fn deref_and_as_ref_agree() {
    let b = MyBytes::from(b"abc".to_vec());
    assert_eq!(&*b, b"abc");
    assert_eq!(b.as_ref(), b"abc");
}

#[test]
fn from_vec_round_trips() {
    let v = vec![1u8, 2, 3, 4];
    let b = MyBytes::from(v.clone());
    assert_eq!(b.as_ref(), v.as_slice());
}

#[test]
fn as_shared_defaults_to_none() {
    let b = MyBytes::from(vec![1u8, 2, 3]);
    assert!(b.as_shared().is_none());
}

mod share {
    pub fn handle(b: &buffa::bytes::Bytes) -> Option<buffa::bytes::Bytes> {
        Some(b.clone())
    }
}

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoBytes)]
#[buffa(remote = buffa::bytes::Bytes, as_shared = share::handle)]
struct SharedBytes(pub buffa::bytes::Bytes);

#[test]
fn as_shared_override_returns_the_wrapped_handle() {
    let b = SharedBytes(buffa::bytes::Bytes::from(vec![7u8; 32]));
    let shared = b.as_shared().expect("override returns Some");
    // Same allocation, not a copy.
    assert_eq!(shared.as_ptr(), b.0.as_ptr());
    assert_eq!(shared.as_ref(), b.as_ref());
}

/// The override must reach a segmented sink end-to-end: encoding through
/// `put_shared_bytes_field` into a `Rope` splices the wrapped `Bytes` by
/// reference count instead of copying. Contiguous sinks never call
/// `as_shared`, so only this path proves the generated hook is wired up.
#[test]
fn as_shared_override_splices_into_rope() {
    // Ropes copy payloads below their min-segment threshold, so the payload
    // must exceed it for the splice assertion to be meaningful.
    let payload = buffa::bytes::Bytes::from(vec![0xAB; 2 * buffa::DEFAULT_MIN_SEGMENT]);
    let b = SharedBytes(payload.clone());

    let mut rope = buffa::Rope::new();
    buffa::types::put_shared_bytes_field(1, &b, &mut rope);
    let rope_bytes = rope.to_contiguous_bytes();
    let spliced = rope
        .into_segments()
        .into_iter()
        .find(|seg| seg.len() == payload.len())
        .expect("payload segment present");
    assert_eq!(spliced.as_ptr(), payload.as_ptr(), "spliced, not copied");

    // Byte-for-byte parity with a contiguous sink.
    let mut contiguous = Vec::new();
    buffa::types::put_shared_bytes_field(1, &b, &mut contiguous);
    assert_eq!(rope_bytes.as_ref(), contiguous.as_slice());
}

/// The override call shape also covers a named-field newtype
/// (`path(&self.field)` rather than `path(&self.0)`).
#[derive(Clone, PartialEq, Default, Debug, DeriveProtoBytes)]
#[buffa(remote = buffa::bytes::Bytes, as_shared = share::handle)]
struct NamedShared {
    inner: buffa::bytes::Bytes,
}

#[test]
fn as_shared_override_works_on_named_field_newtype() {
    let b = NamedShared {
        inner: buffa::bytes::Bytes::from(vec![3u8; 32]),
    };
    let shared = b.as_shared().expect("override returns Some");
    assert_eq!(shared.as_ptr(), b.inner.as_ptr());
}

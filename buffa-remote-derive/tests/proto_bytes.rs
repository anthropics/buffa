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

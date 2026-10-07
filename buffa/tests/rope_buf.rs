use buffa::{EncodeSink, Rope, RopeBuf};
use bytes::{Buf, Bytes};

#[test]
fn bytes_decode_keeps_shared_payload() {
    let payload = Bytes::from(vec![0xAB; 64]);
    let mut rope = Rope::with_min_segment(1);
    rope.put_u8(64);
    rope.put_shared(payload.clone());
    rope.put_u8(42);
    let mut buf = RopeBuf::from(rope);
    let decoded = buffa::types::decode_bytes_to::<Bytes>(&mut buf).unwrap();
    assert_eq!(decoded.as_ptr(), payload.as_ptr());
    assert_eq!(decoded, payload);
    assert_eq!(buf.remaining(), 1);
    assert_eq!(buf.get_u8(), 42);
}

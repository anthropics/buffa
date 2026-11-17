//! A custom string or bytes representation's `decode_length_delimited` must
//! only ever see bytes of its own field. A length that runs past the message
//! holding the field is rejected before the override runs, so the override
//! never sees the parent's next field, a later frame of a stream, or more
//! bytes than `max_message_size` allows.

use buffa::bytes::Buf;
use buffa::{DecodeError, DecodeOptions, Message};
use buffa_test::decode_hook_frame::{take_windows, Inner, Outer, RecBytes, RecStr};

/// `field` as a length-delimited field: tag, length, contents.
fn ld(field: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![field << 3 | 2];
    let mut len = contents.len() as u64;
    while len >= 0x80 {
        out.push((len as u8) | 0x80);
        len >>= 7;
    }
    out.push(len as u8);
    out.extend_from_slice(contents);
    out
}

/// The parent's next field, which no override inside `inner` may see.
const AFTER: &[u8] = b"\x12\x07SECRETX";

/// An `Outer` whose `inner` holds `inner` and is followed by [`AFTER`].
fn outer_with(inner: &[u8]) -> Vec<u8> {
    let mut wire = ld(1, inner);
    wire.extend_from_slice(AFTER);
    wire
}

/// A field of `Inner` declaring 10 bytes, only the first of which is inside
/// `Inner`; the other nine are [`AFTER`].
fn overrun(field: u8) -> Vec<u8> {
    vec![field << 3 | 2, 10, b'a']
}

/// The contents of an `Inner` in which one field of every custom shape runs
/// past its message (or map entry) into the parent's next field.
fn overrunning_inners() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("singular string", overrun(1)),
        ("singular bytes", overrun(2)),
        ("repeated string", overrun(3)),
        ("oneof string", overrun(5)),
        ("optional string", overrun(6)),
        ("repeated bytes", overrun(7)),
        // A map entry of three bytes whose key or value declares ten.
        ("map key", [&[0x22, 3][..], &overrun(1)].concat()),
        ("map string value", [&[0x22, 3][..], &overrun(2)].concat()),
        ("map bytes value", [&[0x42, 3][..], &overrun(2)].concat()),
        // A map entry that itself runs past `Inner`.
        ("map entry", vec![0x22, 10, 0x0a, 1, b'k']),
    ]
}

#[test]
fn a_field_past_its_message_is_rejected_before_the_override_runs() {
    for (shape, inner) in overrunning_inners() {
        let wire = outer_with(&inner);
        take_windows();
        assert_eq!(
            Outer::decode_from_slice(&wire),
            Err(DecodeError::UnexpectedEof),
            "{shape}"
        );
        assert_eq!(take_windows(), Vec::<Vec<u8>>::new(), "{shape}");

        // The same over a fragmented input, which takes the copying path.
        for split in 1..wire.len() {
            let (head, tail) = wire.split_at(split);
            let mut input = head.chain(tail);
            assert_eq!(
                Outer::decode(&mut input),
                Err(DecodeError::UnexpectedEof),
                "{shape}, split {split}"
            );
            assert_eq!(
                take_windows(),
                Vec::<Vec<u8>>::new(),
                "{shape}, split {split}"
            );
        }
    }
}

#[test]
fn a_failed_merge_leaves_no_bytes_from_outside_the_field() {
    // `Inner` is three bytes, but its `s` declares ten: the review repro.
    let wire = [
        0x0a, 3, 0x0a, 10, b'a', 0x12, 7, b'S', b'E', b'C', b'R', b'E', b'T', b'X',
    ];
    assert_eq!(wire.to_vec(), outer_with(&overrun(1)));

    let mut existing = Outer::default();
    existing.inner.get_or_insert_default().s = RecStr("keep".to_owned());
    take_windows();
    assert_eq!(
        existing.merge_from_slice(&wire),
        Err(DecodeError::UnexpectedEof)
    );
    assert_eq!(take_windows(), Vec::<Vec<u8>>::new());
    assert_eq!(existing.inner.s, RecStr("keep".to_owned()));
}

/// The varint encoding of `n`.
fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

#[test]
fn max_message_size_bounds_every_override_window() {
    const MAX: usize = 16;
    let options = DecodeOptions::new().with_max_message_size(MAX);
    let filler = vec![b'z'; 2_000_000];

    // A five-byte frame whose string field declares a million bytes, which
    // the rest of the stream holds.
    let mut head = vec![5, 0x0a];
    head.extend(varint(1_000_000));
    head.push(b'z');
    assert_eq!(head.len(), 6);
    let mut input = head.as_slice().chain(filler.as_slice());
    take_windows();
    assert_eq!(
        options.decode_length_delimited::<Inner>(&mut input),
        Err(DecodeError::UnexpectedEof)
    );
    assert_eq!(take_windows(), Vec::<Vec<u8>>::new());

    // A five-byte frame whose nested message declares a million bytes, and
    // whose string field fills that message.
    let mut head = vec![5, 0x0a];
    head.extend(varint(1_000_000));
    head.push(0x0a);
    let mut tail = varint(999_996);
    tail.extend(&filler[..999_996]);
    assert_eq!(1 + tail.len(), 1_000_000);
    let mut input = head.as_slice().chain(tail.as_slice());
    assert_eq!(
        options.decode_length_delimited::<Outer>(&mut input),
        Err(DecodeError::UnexpectedEof)
    );
    let windows = take_windows();
    assert!(
        windows.is_empty(),
        "an override saw {:?} bytes",
        windows.iter().map(Vec::len).collect::<Vec<_>>()
    );
}

#[test]
fn every_field_shape_still_decodes_inside_a_nested_message() {
    let mut inner = Inner {
        s: RecStr("s".to_owned()),
        b: RecBytes(b"b".to_vec()),
        rs: vec![RecStr("r1".to_owned()), RecStr("r2".to_owned())],
        choice: Some(
            buffa_test::decode_hook_frame::__buffa::oneof::inner::Choice::Os(RecStr(
                "o".to_owned(),
            )),
        ),
        opt: Some(RecStr("opt".to_owned())),
        rb: vec![RecBytes(b"rb".to_vec())],
        ..Inner::default()
    };
    inner
        .m
        .insert(RecStr("k".to_owned()), RecStr("v".to_owned()));
    inner.mb.insert(7, RecBytes(b"mb".to_vec()));
    let outer = Outer {
        inner: inner.into(),
        after: RecStr("after".to_owned()),
        ..Outer::default()
    };
    let wire = outer.encode_to_vec();

    take_windows();
    assert_eq!(Outer::decode_from_slice(&wire).unwrap(), outer);
    // Every window is exactly one field's contents.
    let mut windows = take_windows();
    windows.sort();
    let mut expected: Vec<Vec<u8>> = [
        "s", "b", "r1", "r2", "k", "v", "o", "opt", "rb", "mb", "after",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    expected.sort();
    assert_eq!(windows, expected);

    for split in 1..wire.len() {
        let (head, tail) = wire.split_at(split);
        let mut input = head.chain(tail);
        assert_eq!(Outer::decode(&mut input).unwrap(), outer, "split {split}");
    }
    take_windows();

    // And as a frame of a length-delimited stream with a frame after it.
    let mut stream = Vec::new();
    outer.encode_length_delimited(&mut stream);
    outer.encode_length_delimited(&mut stream);
    let mut input = stream.as_slice();
    for _ in 0..2 {
        assert_eq!(
            DecodeOptions::new()
                .decode_length_delimited::<Outer>(&mut input)
                .unwrap(),
            outer
        );
    }
    assert!(input.is_empty());
    take_windows();
}

#[test]
fn incomplete_prefixes_do_not_invoke_hooks_after_the_nested_message_ends() {
    // Each inner message ends during its field's length prefix. The following
    // byte completes it as zero, but belongs to the enclosing input instead.
    for tag in [0x0a, 0x12, 0x1a, 0x2a, 0x32, 0x3a] {
        for inner in [vec![tag], vec![tag, 0x80]] {
            let mut wire = ld(1, &inner);
            wire.push(0);
            for split in 0..=wire.len() {
                let (head, tail) = wire.split_at(split);
                take_windows();
                assert_eq!(
                    Outer::decode(&mut head.chain(tail)),
                    Err(DecodeError::UnexpectedEof)
                );
                assert!(take_windows().is_empty(), "tag {tag}, split {split}");
            }
        }
    }
}

#[test]
fn a_truncated_prefix_cannot_be_completed_by_the_next_stream_frame() {
    for tag in [0x0a, 0x12] {
        let wire = [2, tag, 0x80, 0];
        for split in 0..=wire.len() {
            let (head, tail) = wire.split_at(split);
            take_windows();
            assert_eq!(
                DecodeOptions::new().decode_length_delimited::<Inner>(&mut head.chain(tail)),
                Err(DecodeError::UnexpectedEof)
            );
            assert!(take_windows().is_empty(), "tag {tag}, split {split}");
        }
        // The same noncanonical zero prefix is valid when both bytes belong
        // to this frame. The following empty frame must remain untouched.
        let wire = [3, tag, 0x80, 0, 0];
        for split in 0..=wire.len() {
            let (head, tail) = wire.split_at(split);
            let mut input = head.chain(tail);
            take_windows();
            DecodeOptions::new()
                .decode_length_delimited::<Inner>(&mut input)
                .unwrap();
            assert_eq!(take_windows(), vec![Vec::<u8>::new()]);
            assert_eq!(input.remaining(), 1);
            DecodeOptions::new()
                .decode_length_delimited::<Inner>(&mut input)
                .unwrap();
            assert!(!input.has_remaining());
            assert!(take_windows().is_empty());
        }
    }
}

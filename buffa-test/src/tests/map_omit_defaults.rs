//! `map_entries_omit_defaults`: map entries leave a key or value at its
//! default out, as prost writes them, from owned messages and views alike.

use crate::map_omit_defaults::__buffa::view::MapsView;
use crate::map_omit_defaults::{maps, Maps};
use buffa::{Message, MessageView, ViewEncode};

fn sample() -> Maps {
    let mut maps = Maps::default();
    maps.counts.insert(String::new(), 0);
    maps.counts.insert("a".into(), 7);
    maps.names.insert(0, "x".into());
    maps.flags.insert(true, -0.0);
    maps.blobs.insert("k".into(), Vec::new());
    maps.colors.insert(3, maps::Color::COLOR_UNSPECIFIED.into());
    maps.colors.insert(4, maps::Color::COLOR_RED.into());
    maps.leaves.insert(0, maps::Leaf::default());
    maps.leaves.insert(
        1,
        maps::Leaf {
            n: 5,
            ..Default::default()
        },
    );
    maps.floats.insert(-1, 0.0);
    maps
}

#[rustfmt::skip]
const SPARSE: &[u8] = &[
    0x0a, 0x00,                                     // counts: "" => 0, both left out
    0x0a, 0x05, 0x0a, 0x01, b'a', 0x10, 0x07,       // counts: "a" => 7
    0x12, 0x03, 0x12, 0x01, b'x',                   // names: 0 => "x", the key left out
    0x1a, 0x02, 0x08, 0x01,                         // flags: true => -0.0, the value left out
    0x22, 0x03, 0x0a, 0x01, b'k',                   // blobs: "k" => empty
    0x2a, 0x02, 0x08, 0x03,                         // colors: 3 => COLOR_UNSPECIFIED
    0x2a, 0x04, 0x08, 0x04, 0x10, 0x01,             // colors: 4 => COLOR_RED
    0x32, 0x00,                                     // leaves: 0 => empty, both left out
    0x32, 0x06, 0x08, 0x01, 0x12, 0x02, 0x08, 0x05, // leaves: 1 => { n: 5 }
    0x3a, 0x02, 0x08, 0x01,                         // floats: -1 => 0.0
];

#[test]
fn owned_encoding_leaves_defaults_out() {
    let maps = sample();
    assert_eq!(maps.encode_to_vec(), SPARSE);
    assert_eq!(maps.encoded_len() as usize, SPARSE.len());
}

#[test]
fn view_encoding_matches_the_owned_one() {
    let view = MapsView::decode_view(SPARSE).expect("decodes");
    assert_eq!(
        view.compute_size(&mut buffa::SizeCache::default()) as usize,
        SPARSE.len()
    );
    assert_eq!(view.encode_to_vec(), SPARSE);
}

#[test]
fn every_entry_decodes_back() {
    let back = Maps::decode_from_slice(SPARSE).expect("decodes");
    assert_eq!(back, sample());
    // `-0.0` is left out like `0.0` and comes back as `0.0`.
    assert!(back.flags[&true].is_sign_positive());
}

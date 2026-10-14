//! `map_entries_omit_defaults`: map entries leave a key or value at its
//! default out, as prost writes them, from owned messages, views and lazy
//! views alike.

use crate::map_omit_defaults::{maps, HolderLazyView, HolderView, Maps, MapsLazyView, MapsView};
use buffa::view::LazyMessageView;
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

/// `sample()` with both fields of every entry written, as without the option.
#[rustfmt::skip]
const DENSE: &[u8] = &[
    0x0a, 0x04, 0x0a, 0x00, 0x10, 0x00,             // counts: "" => 0
    0x0a, 0x05, 0x0a, 0x01, b'a', 0x10, 0x07,       // counts: "a" => 7
    0x12, 0x05, 0x08, 0x00, 0x12, 0x01, b'x',       // names: 0 => "x"
    0x1a, 0x0b, 0x08, 0x01,                         // flags: true => -0.0
    0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,
    0x22, 0x05, 0x0a, 0x01, b'k', 0x12, 0x00,       // blobs: "k" => empty
    0x2a, 0x04, 0x08, 0x03, 0x10, 0x00,             // colors: 3 => COLOR_UNSPECIFIED
    0x2a, 0x04, 0x08, 0x04, 0x10, 0x01,             // colors: 4 => COLOR_RED
    0x32, 0x04, 0x08, 0x00, 0x12, 0x00,             // leaves: 0 => empty
    0x32, 0x06, 0x08, 0x01, 0x12, 0x02, 0x08, 0x05, // leaves: 1 => { n: 5 }
    0x3a, 0x07, 0x08, 0x01, 0x15, 0x00, 0x00, 0x00, 0x00, // floats: -1 => 0.0
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
fn lazy_view_encoding_matches_the_owned_one() {
    let lazy = MapsLazyView::decode_lazy(SPARSE).expect("decodes");
    assert_eq!(lazy.encoded_len() as usize, SPARSE.len());
    assert_eq!(lazy.encode_to_vec(), SPARSE);
}

/// Defaults written on the wire are left out by every encoder.
#[test]
fn written_defaults_are_left_out() {
    let owned = Maps::decode_from_slice(DENSE).expect("decodes");
    assert_eq!(owned, sample());
    assert_eq!(owned.encode_to_vec(), SPARSE);
    let view = MapsView::decode_view(DENSE).expect("decodes");
    assert_eq!(view.encode_to_vec(), SPARSE);
    let lazy = MapsLazyView::decode_lazy(DENSE).expect("decodes");
    assert_eq!(lazy.encode_to_vec(), SPARSE);
}

/// A lazy view writes a message field, which it keeps undecoded, as it was
/// read; a view decodes it and leaves the defaults out.
#[test]
fn lazy_view_writes_a_message_field_as_read() {
    let wrap = |maps: &[u8]| {
        let mut holder = vec![0x0a, maps.len() as u8];
        holder.extend_from_slice(maps);
        holder
    };
    let holder = wrap(DENSE);
    let lazy = HolderLazyView::decode_lazy(&holder).expect("decodes");
    assert_eq!(lazy.encode_to_vec(), holder);
    let view = HolderView::decode_view(&holder).expect("decodes");
    assert_eq!(view.encode_to_vec(), wrap(SPARSE));
}

#[test]
fn every_entry_decodes_back() {
    let back = Maps::decode_from_slice(SPARSE).expect("decodes");
    assert_eq!(back, sample());
    // `-0.0` is left out like `0.0` and comes back as `0.0`.
    assert!(back.flags[&true].is_sign_positive());
}

mod proto2 {
    use crate::map_omit_defaults_proto2::{maps, Maps, MapsLazyView, MapsView};
    use buffa::view::LazyMessageView;
    use buffa::{Message, MessageView, ViewEncode};

    fn sample() -> Maps {
        let mut maps = Maps::default();
        maps.sizes.insert(1, maps::Size::SIZE_SMALL);
        maps.sizes.insert(2, maps::Size::SIZE_LARGE);
        maps.required.insert(
            1,
            maps::Required {
                n: 0,
                ..Default::default()
            },
        );
        maps
    }

    #[rustfmt::skip]
    const SPARSE: &[u8] = &[
        0x0a, 0x02, 0x08, 0x01,                         // sizes: 1 => SIZE_SMALL, the default
        0x0a, 0x04, 0x08, 0x02, 0x10, 0x01,             // sizes: 2 => SIZE_LARGE
        // required: 1 => { n: 0 }, written: it encodes its required field,
        // where prost leaves out a value equal to its default
        0x12, 0x06, 0x08, 0x01, 0x12, 0x02, 0x08, 0x00,
    ];

    #[test]
    fn closed_enum_and_required_values() {
        let maps = sample();
        assert_eq!(maps.encode_to_vec(), SPARSE);
        assert_eq!(maps.encoded_len() as usize, SPARSE.len());
        assert_eq!(Maps::decode_from_slice(SPARSE).expect("decodes"), maps);
        let view = MapsView::decode_view(SPARSE).expect("decodes");
        assert_eq!(
            view.compute_size(&mut buffa::SizeCache::default()) as usize,
            SPARSE.len()
        );
        assert_eq!(view.encode_to_vec(), SPARSE);
        let lazy = MapsLazyView::decode_lazy(SPARSE).expect("decodes");
        assert_eq!(lazy.encoded_len() as usize, SPARSE.len());
        assert_eq!(lazy.encode_to_vec(), SPARSE);
    }
}

//! An editions file whose edition is outside the range the pool defines
//! (`EDITION_PROTO2` through `EDITION_2024`) does not link: the pool cannot
//! know a later edition's feature defaults, so it rejects the file with
//! `PoolError::UnsupportedEdition` instead of silently using the edition
//! 2023 defaults.

#![cfg(feature = "reflect")]

use buffa::Message;
use buffa_descriptor::generated::descriptor::{Edition, FileDescriptorProto, FileDescriptorSet};
use buffa_descriptor::{DescriptorPool, PoolError};

fn editions_file(name: &str, edition: Option<Edition>) -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some(name.into()),
        syntax: Some("editions".into()),
        edition,
        ..Default::default()
    }
}

fn link(file: FileDescriptorProto) -> Result<DescriptorPool, PoolError> {
    DescriptorPool::new(FileDescriptorSet {
        file: vec![file],
        ..Default::default()
    })
}

fn assert_unsupported(file: FileDescriptorProto, name: &str, edition: i32) {
    match link(file) {
        Ok(_) => panic!("edition {edition} links, expected UnsupportedEdition"),
        Err(err) => assert!(
            matches!(&err, PoolError::UnsupportedEdition { file, edition: got }
                if file == name && *got == edition),
            "got {err:?}",
        ),
    }
}

#[test]
fn defined_editions_outside_the_supported_range_are_rejected() {
    for edition in [
        Edition::EDITION_LEGACY,
        Edition::EDITION_UNSTABLE,
        Edition::EDITION_1_TEST_ONLY,
        Edition::EDITION_2_TEST_ONLY,
        Edition::EDITION_99997_TEST_ONLY,
        Edition::EDITION_99998_TEST_ONLY,
        Edition::EDITION_99999_TEST_ONLY,
        Edition::EDITION_MAX,
    ] {
        assert_unsupported(
            editions_file("future.proto", Some(edition)),
            "future.proto",
            edition as i32,
        );
    }
}

#[test]
fn an_edition_number_the_pool_does_not_define_is_rejected_with_its_number() {
    // Edition 1002 (a future EDITION_2026) cannot be written through the
    // typed field: `Edition` is a closed enum, so decoding a real descriptor
    // keeps the number as an unknown field. Hand-encode the set instead:
    // FileDescriptorSet { file { name: "future.proto", syntax: "editions",
    // edition: 1002 } }.
    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }
    let mut proto = Vec::new();
    proto.push(0x0a); // FileDescriptorProto.name (field 1, length-delimited)
    varint(12, &mut proto);
    proto.extend_from_slice(b"future.proto");
    proto.push(0x62); // FileDescriptorProto.syntax (field 12, length-delimited)
    varint(8, &mut proto);
    proto.extend_from_slice(b"editions");
    proto.push(0x70); // FileDescriptorProto.edition (field 14, varint)
    varint(1002, &mut proto);
    let mut set = Vec::new();
    set.push(0x0a); // FileDescriptorSet.file (field 1, length-delimited)
    varint(proto.len() as u64, &mut set);
    set.extend_from_slice(&proto);

    let decoded = FileDescriptorSet::decode(&mut &set[..]).expect("set decodes");
    match DescriptorPool::new(decoded) {
        Ok(_) => panic!("edition 1002 links, expected UnsupportedEdition"),
        Err(err) => assert!(
            matches!(&err, PoolError::UnsupportedEdition { file, edition: 1002 }
                if file == "future.proto"),
            "got {err:?}",
        ),
    }
}

#[test]
fn supported_editions_still_link() {
    for edition in [
        Edition::EDITION_PROTO2,
        Edition::EDITION_PROTO3,
        Edition::EDITION_2023,
        Edition::EDITION_2024,
    ] {
        link(editions_file("ok.proto", Some(edition)))
            .unwrap_or_else(|err| panic!("edition {edition:?} should link: {err:?}"));
    }
}

#[test]
fn unset_and_unknown_editions_are_still_missing_edition() {
    for edition in [None, Some(Edition::EDITION_UNKNOWN)] {
        match link(editions_file("plain.proto", edition)) {
            Ok(_) => panic!("edition {edition:?} links, expected MissingEdition"),
            Err(err) => assert!(
                matches!(&err, PoolError::MissingEdition { file } if file == "plain.proto"),
                "got {err:?}",
            ),
        }
    }
}

#[test]
fn the_edition_gate_only_applies_to_editions_files() {
    // A proto3 file that happens to carry an edition value is judged by its
    // syntax, as before; the edition field means nothing there.
    let mut file = editions_file("legacy.proto", Some(Edition::EDITION_UNSTABLE));
    file.syntax = Some("proto3".into());
    link(file).expect("a proto3 file links regardless of its edition field");
}

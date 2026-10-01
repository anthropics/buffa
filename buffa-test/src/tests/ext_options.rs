//! `buffa.ext` options, read back from the descriptors that codegen embeds
//! for `ext_options.proto`.

use buffa::ExtensionSet;
use buffa_descriptor::DescriptorPool;

fn pool() -> &'static DescriptorPool {
    crate::ext_options::descriptor_pool()
}

#[test]
fn each_option_reads_back_from_its_element() {
    let pool = pool();

    let file = pool.file_by_name("ext_options.proto").unwrap();
    assert_eq!(
        file.options
            .as_option()
            .unwrap()
            .extension(&buffa_proto_options::FILE),
        Some(buffa_proto_options::FileOptions::default())
    );

    let message = pool.message_by_name("test.extoptions.Annotated").unwrap();
    assert_eq!(
        message
            .options()
            .unwrap()
            .extension(&buffa_proto_options::MESSAGE),
        Some(buffa_proto_options::MessageOptions::default())
    );
    let mass = message.field_by_name("mass").unwrap();
    assert_eq!(
        mass.options()
            .unwrap()
            .extension(&buffa_proto_options::FIELD),
        Some(buffa_proto_options::FieldOptions::default())
    );
    let [kind] = message.oneofs() else {
        panic!("Annotated declares one oneof");
    };
    assert_eq!(
        kind.options()
            .unwrap()
            .extension(&buffa_proto_options::ONEOF),
        Some(buffa_proto_options::OneofOptions::default())
    );

    let color = pool.enum_by_name("test.extoptions.Color").unwrap();
    assert_eq!(
        color
            .options()
            .unwrap()
            .extension(&buffa_proto_options::ENUM),
        Some(buffa_proto_options::EnumOptions::default())
    );
    let unspecified = color.value_by_name("COLOR_UNSPECIFIED").unwrap();
    assert_eq!(
        unspecified
            .options()
            .unwrap()
            .extension(&buffa_proto_options::ENUM_VALUE),
        Some(buffa_proto_options::EnumValueOptions::default())
    );
}

#[test]
fn element_without_the_option_reads_as_none() {
    let pool = pool();

    // Both controls set `deprecated`, so `options()` is present and
    // `extension` runs against a message that lacks field 1381.
    let message = pool.message_by_name("test.extoptions.Annotated").unwrap();
    let plain = message.field_by_name("plain").unwrap();
    assert_eq!(
        plain
            .options()
            .unwrap()
            .extension(&buffa_proto_options::FIELD),
        None
    );

    let color = pool.enum_by_name("test.extoptions.Color").unwrap();
    let red = color.value_by_name("COLOR_RED").unwrap();
    assert_eq!(
        red.options()
            .unwrap()
            .extension(&buffa_proto_options::ENUM_VALUE),
        None
    );
}

/// The embedded pool holds `buffa/ext/options.proto` itself, so a reflective
/// reader resolves each option by name.
#[test]
fn pool_resolves_each_option_by_name() {
    let pool = pool();
    assert!(pool
        .file_by_name(buffa_proto_options::OPTIONS_PROTO_PATH)
        .is_some());

    // The six options share a number, so the extendee tells them apart.
    for (name, extendee) in [
        ("buffa.ext.file", "google.protobuf.FileOptions"),
        ("buffa.ext.message", "google.protobuf.MessageOptions"),
        ("buffa.ext.field", "google.protobuf.FieldOptions"),
        ("buffa.ext.oneof", "google.protobuf.OneofOptions"),
        ("buffa.ext.enum", "google.protobuf.EnumOptions"),
        ("buffa.ext.enum_value", "google.protobuf.EnumValueOptions"),
    ] {
        let extension = pool
            .extension_by_name(name)
            .unwrap_or_else(|| panic!("{name} is in the pool"));
        assert_eq!(extension.full_name(), name);
        assert_eq!(extension.field().number(), 1381, "{name}");
        assert_eq!(pool.message(extension.extendee()).full_name(), extendee);
    }
}

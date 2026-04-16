fn main() {
    buffa_build::Config::new()
        .files(&["proto/addressbook.proto"])
        .includes(&["proto/"])
        // WKT types (Timestamp, etc.) are automatically mapped to
        // buffa-types — no extern_path needed.
        .generate_views(true)
        .generate_text(true)
        // Custom attribute: mark the legacy `freeform_address` variant of
        // the `address` oneof as deprecated. Downstream code that reads
        // or writes this variant then gets a standard Rust deprecation
        // warning, surfacing the migration to `structured_address` at
        // compile time. This path targets the oneof variant's FQN —
        // `{Msg}.{oneof}.{variant}` — and demonstrates that custom
        // attributes reach inside oneof enums (not just top-level
        // message fields).
        .field_attribute(
            ".buffa.examples.addressbook.v1.Person.address.freeform_address",
            r#"#[deprecated(note = "legacy address field — prefer structured_address")]"#,
        )
        .include_file("_include.rs")
        .compile()
        .expect("protobuf compilation failed");
}

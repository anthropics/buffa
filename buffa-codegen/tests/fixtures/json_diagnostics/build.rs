fn main() {
    buffa_build::Config::new()
        .files(&["proto/compat.proto"])
        .includes(&["proto"])
        .generate_views(false)
        .generate_json(true)
        .string_type_custom_in(
            "crate::Text",
            &[".compat.CustomOptional", ".compat.CustomOneof"],
        )
        .repeated_type_custom_in(
            "crate::EnvelopeList<*>",
            &[
                ".compat.CustomCollections",
                ".compat.CustomCollectionsWithOneof",
            ],
        )
        .map_type_custom_in(
            "crate::EnvelopeMap",
            &[
                ".compat.CustomCollections",
                ".compat.CustomCollectionsWithOneof",
            ],
        )
        .compile()
        .unwrap();
}

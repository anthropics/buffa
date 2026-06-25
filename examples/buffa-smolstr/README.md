# buffa-smolstr — example only, not published

This crate is the worked reference for wrapping a foreign string type (`smol_str::SmolStr`) in a crate-local newtype that implements `buffa::ProtoString`, so it can be used as the owned representation of `string` fields via `buffa_build`'s `string_type_custom` knob.

**It is not published to crates.io.** Publishing it would tie consumers to this repository's `smol_str` version pin; the point of the pluggable-type design is that you bring your own representation against your own dependency versions. Copy [`src/lib.rs`](src/lib.rs) into your own crate, swap `smol_str` for whatever inline-string type you depend on, and point `string_type_custom` at your newtype's path.

For the end-to-end walkthrough that covers all five pluggable-type knobs (string, bytes, repeated, message-field pointer, and map), see [`examples/custom-types/`](../custom-types/).

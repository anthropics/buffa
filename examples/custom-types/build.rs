//! Compile `proto/record.proto` with every owned-type knob pointed at a
//! crate-local newtype from `src/types.rs`.
//!
//! The string and bytes knobs take a complete type path. The repeated and
//! box knobs take a *template* with a literal `*` placeholder that codegen
//! substitutes with the element/pointee type. The map knob uses the
//! built-in `BTreeMap` preset rather than a custom container.

use buffa_build::MapRepr;

fn main() {
    buffa_build::Config::new()
        .files(&["proto/record.proto"])
        .includes(&["proto/"])
        .generate_json(true)
        // string -> crate::types::FlexStr (newtype over flexstr::SharedStr)
        .string_type_custom("crate::types::FlexStr")
        // bytes -> crate::types::SmallBytes (newtype over SmallVec<[u8; 24]>)
        .bytes_type_custom("crate::types::SmallBytes")
        // repeated T -> crate::types::SmallVec<T> (newtype over SmallVec<[T; 4]>)
        .repeated_type_custom("crate::types::SmallVec<*>")
        // boxed message -> crate::types::SmallBox<T> (newtype over smallbox::SmallBox)
        .box_type_custom("crate::types::SmallBox<*>")
        // map<K, V> -> alloc::collections::BTreeMap<K, V>
        .map_type(MapRepr::BTreeMap)
        .include_file("_include.rs")
        .compile()
        .expect("protobuf compilation failed");
}

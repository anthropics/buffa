//! Compile `proto/record.proto` with every owned-type knob pointed at a
//! crate-local newtype from `src/types.rs`.
//!
//! The string and bytes knobs take a complete type path. The repeated and
//! box knobs take a *template* with a literal `*` placeholder that codegen
//! substitutes with the element/pointee type. The map knob takes a bare
//! path that codegen applies as `path<K, V>`.

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
        // map<K, V> -> crate::types::IndexMap<K, V> (newtype over indexmap).
        // For a no-newtype ordered map, `.map_type(MapRepr::BTreeMap)` is the
        // built-in preset.
        .map_type_custom("crate::types::IndexMap")
        .include_file("_include.rs")
        .compile()
        .expect("protobuf compilation failed");
}

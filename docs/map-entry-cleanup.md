# Temporary map-entry cleanup

With the optional `buffa/zeroize` feature enabled, ordinary and table binary map decoders call the destination collection's `MapStorage::storage_erase_key` and `storage_erase_value` hooks before dropping an entry they cannot insert. This includes malformed input and entries discarded because their final closed-enum value is unknown. Successfully inserted values transfer to the destination without invoking these hooks.

Both hooks have no-op defaults, so existing `MapStorage` implementations remain source-compatible. Standard hash maps and BTreeMap retain those defaults. Enabling the feature does not turn an ordinary map into a wipe-owning collection. With the feature disabled, temporary entries use ordinary Rust drop and the hooks are not called.

A custom collection can override each hook to erase the current live element. Hooks must not allocate or panic and must leave a valid value: the element's destructor runs afterward. The decoder then clears the vacated inline slot through `MaybeUninit` storage, including after transferring ownership to the destination. It never overwrites a live value with an invalid zero representation.

The collection and its element representations still own their broader lifecycle:

- Duplicate key or scalar-value fields can replace an earlier decoded element within an entry. The final-slot hooks do not intercept those earlier replacements; element representations must erase allocations they release themselves, typically through Drop.
- Successfully inserted elements, replaced entries, removals, collection growth and destruction belong to the destination's storage contract.
- Preserving an unknown entry can retain its original encoded payload in `UnknownFields`. These hooks do not erase or prevent that copy.
- Caller-owned input, parser copies and compiler-generated moves are outside the temporary slot's ownership.

JSON decoding is unchanged by these hooks. There is no new code-generation setting, and ordinary generated bindings retain their existing behavior.

The runtime supports zeroize 1.8.2 and newer compatible releases. Rust 1.75 is tested with an MSRV-compatible lockfile selecting 1.8.2; zeroize 1.9 requires Rust 1.85. For an older compiler, declare the project's `rust-version`, generate the lockfile with current Cargo and `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback` (or select `zeroize = "=1.8.2"` explicitly), then build with `--locked`. Fallback cannot satisfy a graph that requires a newer incompatible dependency. No upper bound prevents modern consumers from selecting zeroize 1.9.

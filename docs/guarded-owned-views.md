# Wiping wire ownership

Enable `buffa/zeroize` to use `sensitive::GuardedOwnedView<V>`. It owns a private
wire allocation and exposes borrowed view/byte access, without Clone or a Bytes
handle. Decoding copies caller input; direct encoding from a message writes
into pre-reserved wiping storage without an intermediate wire buffer. Size
limits, reservation failures, encoder length mismatches and decode failures
return typed errors. Failed construction also wipes the private allocation.

The guarantee covers this wire allocation. Decoded view-owned collections and
values follow their view type's cleanup contract. `ZeroizeOnDrop` is provided
only when the view implements it too. Caller input, returned owned messages,
application copies and encoder/parser scratch remain with their owners. The
ordinary OwnedView API and generation defaults remain unchanged.

`Bytes::from_owner` requires bytes 1.9 or newer.

The runtime supports zeroize 1.8.2 and newer compatible releases. Rust 1.75 is tested with an MSRV-compatible lockfile selecting 1.8.2; zeroize 1.9 requires Rust 1.85. For an older compiler, declare the project's `rust-version`, generate the lockfile with current Cargo and `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback` (or select `zeroize = "=1.8.2"` explicitly), then build with `--locked`. Fallback cannot satisfy a graph that requires a newer incompatible dependency. No upper bound prevents modern consumers from selecting zeroize 1.9.

# buffa-build

Build-time integration for compiling Protocol Buffers schemas into Buffa Rust
types. It runs from your crate's `build.rs`; generated messages use the
separate `buffa` runtime crate.

## Quick start

Add the runtime and build dependencies. Add `buffa-types` too when your
schemas use protobuf well-known types such as `Timestamp` or `Any`.

```toml
[dependencies]
buffa = "0.9"
buffa-types = "0.9"

[build-dependencies]
buffa-build = "0.9"
```

Compile your schemas and generate a module tree:

```rust,ignore
// build.rs
fn main() {
    buffa_build::Config::new()
        .files(&["proto/my_service.proto"])
        .includes(&["proto/"])
        .include_file("_include.rs")
        .compile()
        .unwrap();
}
```

Include the generated modules in your crate:

```rust,ignore
// src/lib.rs
mod proto {
    include!(concat!(env!("OUT_DIR"), "/_include.rs"));
}
```

`include_file` creates the nested module tree for the protobuf packages,
including packages that refer to one another.

## Protobuf compiler

By default, `buffa-build` runs `protoc`. Install it on `PATH` or set the
`PROTOC` environment variable. To use `buf`, install it and select it with
`.use_buf()`. You can also pass a precompiled descriptor set with
`.descriptor_set()`.

See the [user guide](https://github.com/anthropics/buffa/blob/main/docs/guide.md#using-buffa-build-in-buildrs)
for compiler setup and the full set of code generation options.

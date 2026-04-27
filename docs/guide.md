# Buffa User Guide

A comprehensive guide to using buffa for Protocol Buffers in Rust.

## Installation

Add buffa to your project:

```toml
# Cargo.toml
[dependencies]
buffa = "0.4"
buffa-types = "0.4"       # well-known types (Timestamp, Duration, Any, etc.)

[build-dependencies]
buffa-build = "0.4"
```

### Feature flags

Both `buffa` and `buffa-types` share the same feature flag names:

| Feature | Default | Enables |
|---------|---------|---------|
| `std` | Yes | `std::io::Read` decoders, `HashMap` for map fields, `JsonParseOptions` thread-local (`buffa`); `std::time::{SystemTime, Duration}` conversions (`buffa-types`) |
| `json` | No | Proto-canonical JSON via serde (works with `no_std` + `alloc`) |
| `arbitrary` | No | `arbitrary::Arbitrary` derive on generated types, for fuzzing |

```toml
# Enable JSON support
buffa = { version = "0.4", features = ["json"] }
buffa-types = { version = "0.4", features = ["json"] }
```

## Prerequisites

### buf (recommended)

[buf](https://buf.build/docs/cli/) is the easiest way to compile `.proto` files with buffa. It has a built-in protobuf compiler, so you only need to install buf itself and the `protoc-gen-buffa` plugin — no separate `protoc` required.

```sh
# Install buf — see https://buf.build/docs/installation for other methods
brew install bufbuild/buf/buf   # macOS
npm install -g @bufbuild/buf    # any platform with Node.js
```

buf handles proto dependency management, linting, and breaking change detection out of the box. It also supports all protobuf editions without version constraints.

### protoc (alternative)

If you prefer protoc (or are using `buffa-build` without `.use_buf()`), install it via your package manager:

```sh
brew install protobuf          # macOS (v33+)
apt install protobuf-compiler  # Debian/Ubuntu (v21.12)
nix-env -i protobuf            # Nix (v29+)
```

Or set the `PROTOC` environment variable to point to a specific binary.

**Minimum version: v21.12.** The minimum varies by feature:

| Feature | Minimum protoc |
|---|---|
| Proto2 + proto3 | v21.12 |
| Editions 2023 | v27.0 |
| Editions 2024 | v33.0 |

Note that the protoc version shipped by Debian and Ubuntu (`apt install protobuf-compiler`) is v21.12, which does not support editions. If you need editions, install a newer protoc from [GitHub releases](https://github.com/protocolbuffers/protobuf/releases) or use buf instead.

## Build setup

There are two ways to generate Rust code from `.proto` files:

1. **`buf generate`** (recommended) — uses the buf CLI with `protoc-gen-buffa` as a local plugin. No `protoc` required, no `build.rs` needed.
2. **`buffa-build`** — a `build.rs` helper that invokes `protoc` (or `buf`) at compile time, similar to `prost-build` or `tonic-build`.

### Using `buf generate` (recommended)

See the [Using buf](#using-buf) section below for full configuration details. Quick start:

```yaml
# buf.gen.yaml
version: v2
plugins:
  - local: protoc-gen-buffa
    out: src/gen
  - local: protoc-gen-buffa-packaging
    out: src/gen
    strategy: all
```

```sh
buf generate
```

```rust,ignore
// src/main.rs or src/lib.rs
mod gen;  // generated mod.rs handles #[allow] and module hierarchy
```

### Using `buffa-build` in `build.rs`

This approach compiles protos at build time via `build.rs`, which is familiar if you've used `prost-build` or `tonic-build`. It requires `protoc` on PATH (or `buf` if `.use_buf()` is configured).

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

Include the generated code in your crate:

```rust,ignore
// src/lib.rs
mod proto {
    include!(concat!(env!("OUT_DIR"), "/_include.rs"));
}
```

The `.include_file("_include.rs")` option generates a module tree file that sets up nested `pub mod` blocks matching your protobuf package hierarchy. This is the recommended approach — it handles cross-package type references automatically and avoids manual module wiring.

**Without `include_file`:** You can include each package's generated stitcher file individually via `buffa::include_proto!`, which is what `_include.rs` expands to under the hood:

```rust,ignore
// Manual approach (not recommended for multi-package projects)
pub mod my_package {
    buffa::include_proto!("my.package");  // dotted protobuf package name
}
```

The macro pulls in `OUT_DIR/<dotted.pkg>.mod.rs`, which in turn includes the per-proto content files and sets up the `__buffa::` ancillary module (see [Generated module layout](#generated-module-layout)). Do not `include!` the per-proto `.rs` files directly — they reference sibling `__buffa::oneof::` / `__buffa::view::` modules that only exist once the stitcher wires them up.

### Config options

| Method | Default | Description |
|--------|---------|-------------|
| `.files(&[...])` | — | Proto files to compile (required) |
| `.includes(&[...])` | — | Include directories for imports |
| `.out_dir(path)` | `$OUT_DIR` | Output directory for generated files |
| `.generate_views(bool)` | `true` | Generate zero-copy view types |
| `.generate_json(bool)` | `false` | Generate serde Serialize/Deserialize for proto3 JSON |
| `.generate_text(bool)` | `false` | Generate `impl buffa::text::TextFormat` for textproto encoding/decoding |
| `.preserve_unknown_fields(bool)` | `true` | Preserve unknown fields for round-trip fidelity |
| `.generate_arbitrary(bool)` | `false` | Emit `#[derive(arbitrary::Arbitrary)]` gated behind the `arbitrary` feature (for fuzzing) |
| `.strict_utf8_mapping(bool)` | `false` | Map `utf8_validation = NONE` string fields to `Vec<u8>` / `&[u8]` instead of `String` (see [Skipping UTF-8 validation](#skipping-utf-8-validation)) |
| `.extern_path(proto, rust)` | — | Map a proto package to an external Rust crate (see below) |
| `.use_bytes_type()` | — | Use `bytes::Bytes` for all bytes fields |
| `.use_bytes_type_in(&[...])` | — | Use `bytes::Bytes` for matching bytes fields |
| `.use_buf()` | — | Use `buf build` instead of `protoc` for descriptor generation |
| `.include_file(name)` | — | Generate a module tree file for `include!` (recommended) |
| `.descriptor_set(path)` | — | Use a pre-compiled `FileDescriptorSet` file |

### Well-known types

Well-known types (`google.protobuf.Timestamp`, `Duration`, `Any`, etc.) are automatically mapped to `buffa-types` — no configuration needed. Any proto that imports `google/protobuf/timestamp.proto` (or other WKTs) will reference `::buffa_types::google::protobuf::Timestamp` in the generated code.

This requires `buffa-types` as a dependency in your `Cargo.toml`:

```toml
[dependencies]
buffa-types = "0.4"
```

`buffa-types` is a pure source crate — it does **not** run `protoc` or any code generation at build time. If your protos use WKTs but you generate your own Rust code ahead-of-time (via `buf generate` or a `protoc` script), then `buffa` + `buffa-types` is your entire runtime dependency surface.

If you omit this dependency, your proto files don't use any WKTs, or you provide custom implementations via `extern_path` (see below), then `buffa-types` is not required.

**Overriding WKT implementations:** To use your own types instead of `buffa-types`, set an explicit `extern_path` for `.google.protobuf`:

```rust,ignore
buffa_build::Config::new()
    .extern_path(".google.protobuf", "::my_custom_wkts")
    // ...
```

This disables the automatic mapping and routes all `google.protobuf.*` references to your crate. Your types must implement `buffa::Message` with the same wire format as the standard WKT definitions.

### External type paths

When multiple crates compile protos that reference each other, use `extern_path` to tell buffa that types under a proto package already exist in another Rust crate:

```rust,ignore
// build.rs — service crate that imports from a shared common-protos crate
buffa_build::Config::new()
    .extern_path(".my.common", "::common_protos")
    .files(&["proto/my_service.proto"])
    .includes(&["proto/"])
    .compile()
    .unwrap();
```

With this configuration, any reference to a type like `my.common.SharedMessage` in `my_service.proto` will generate `::common_protos::SharedMessage` instead of a locally-generated struct.

The proto path must start with `.` (fully qualified), though the leading dot is optional and will be added automatically. When multiple extern paths match, the longest prefix wins.

**View types:** When view generation is enabled (the default), the codegen also expects a `FooView<'a>` type at `<extern_crate>::__buffa::view::FooView` for each extern-mapped message `Foo`. If you're using extern_path to reference types from another buffa-generated crate, the views are already there. If you're mapping to [custom type implementations](#custom-type-implementations), see that section for how to provide the view type.

### Multi-package projects

When your proto files span multiple packages that reference each other, buffa uses `super::`-based relative paths so cross-package types resolve automatically. This works when the module tree matches the protobuf package hierarchy — which `include_file` (for `buffa-build`) and `protoc-gen-buffa-packaging` (for the protoc plugin path) ensure.

**Example:** Two packages that reference each other:

```protobuf
// context/v1/context.proto
package myapp.context.v1;
message RequestContext { string request_id = 1; }

// api/v1/service.proto
package myapp.api.v1;
import "context/v1/context.proto";
message Request {
  myapp.context.v1.RequestContext context = 1;
}
```

With `include_file` or `protoc-gen-buffa-packaging`, the generated module tree is:

```text
pub mod myapp {
    pub mod context {
        pub mod v1 {
            // RequestContext defined here
        }
    }
    pub mod api {
        pub mod v1 {
            // Request defined here, references
            // super::super::context::v1::RequestContext
        }
    }
}
```

The `Request` struct's `context` field references `super::super::context::v1::RequestContext` — navigating up from `api::v1` to the `myapp` module root, then down into `context::v1`. This works regardless of where the module tree is placed in your crate.

**`extern_path` is only needed for types in a different crate** (other than well-known types, which are handled automatically). You do **not** need `extern_path` for sibling packages compiled together or for WKTs.

#### Quirks and gotchas

**Module tree depth matches package depth.** The generated module tree has one `pub mod` level per package segment. A package like `com.example.myapp.api.v1` produces five levels of nesting. Your `use` statements must traverse the full hierarchy:

```rust,ignore
// This works:
use proto::com::example::myapp::api::v1::MyMessage;

// This does NOT work (skipping levels):
use proto::api::v1::MyMessage;  // error: can't find `api` in `proto`
```

**The module tree must be at a consistent position.** All generated code assumes the module tree root is at the same level. If you include the module tree inside `mod proto { ... }`, all types are under `proto::`. If you include it at the crate root, types are at the crate root. Pick one and be consistent.

**Rust keywords in package names** are escaped automatically. A proto package `google.type` becomes `pub mod r#type { ... }` in the module tree. References to types in this package use `r#type` in paths:

```rust,ignore
use proto::google::r#type::LatLng;
```

This is the standard Rust mechanism for using keywords as identifiers. It applies to all Rust keywords (`type`, `match`, `async`, `mod`, etc.).

**Rust keywords in field names** are also escaped. Most keywords use raw identifiers (`r#type`, `r#match`), but `self`, `super`, `Self`, and `crate` cannot be raw identifiers and are suffixed with `_` instead (`self_`, `super_`). This matches prost's convention.

**Generated files are named by proto file path, not package.** The file `proto/api/v1/service.proto` produces `api.v1.service.rs` regardless of the `package` declaration. The module tree generator uses the package from the file descriptor (not the file name) to build the `pub mod` nesting. This means the file name and module path may not correspond — the file `api.v1.service.rs` might be included inside `pub mod myapp { pub mod api { pub mod v1 { ... } } }` if the package is `myapp.api.v1`.

**Recursive message types** work automatically: singular message fields use `MessageField<T>` (which is `Option<Box<T>>` internally), and message-typed oneof variants are boxed. Both direct recursion (`message T { oneof k { T self = 1; } }`) and mutual recursion (`A ↔ B`) compile without workarounds.

### Installing the protoc plugins

There are two binaries: `protoc-gen-buffa` (the codegen plugin) and `protoc-gen-buffa-packaging` (the module-tree assembler). Both are released together.

**From source (requires Rust toolchain):**

From crates.io (recommended):

```sh
cargo install --locked protoc-gen-buffa protoc-gen-buffa-packaging
```

Or from a git ref, for unreleased changes:

```sh
cargo install --locked --git https://github.com/anthropics/buffa protoc-gen-buffa protoc-gen-buffa-packaging
```

**From GitHub releases:**

Download the binaries for your platform from the [releases page](https://github.com/anthropics/buffa/releases) using the `gh` CLI:

```sh
# Download binaries + cosign signatures + certificates (both plugins match)
gh release download v0.4.0 --repo anthropics/buffa \
    --pattern 'protoc-gen-buffa*-linux-x86_64*'

# Verify with GitHub attestations (requires gh CLI ≥ 2.49)
gh attestation verify protoc-gen-buffa-v0.4.0-linux-x86_64 --repo anthropics/buffa
gh attestation verify protoc-gen-buffa-packaging-v0.4.0-linux-x86_64 --repo anthropics/buffa

# Or with cosign (standalone, no gh required) — shown for one binary
cosign verify-blob \
    --signature protoc-gen-buffa-v0.4.0-linux-x86_64.sig \
    --certificate protoc-gen-buffa-v0.4.0-linux-x86_64.pem \
    --certificate-identity-regexp "github.com/anthropics/buffa" \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    protoc-gen-buffa-v0.4.0-linux-x86_64

# Install both
chmod +x protoc-gen-buffa-v0.4.0-linux-x86_64 protoc-gen-buffa-packaging-v0.4.0-linux-x86_64
mv protoc-gen-buffa-v0.4.0-linux-x86_64 ~/.local/bin/protoc-gen-buffa
mv protoc-gen-buffa-packaging-v0.4.0-linux-x86_64 ~/.local/bin/protoc-gen-buffa-packaging
```

Available platforms: `linux-x86_64`, `linux-aarch64`, `darwin-x86_64`, `darwin-aarch64`, `windows-x86_64` (`.exe`). All releases include SHA-256 checksums, Sigstore cosign signatures, and signed SLSA build provenance for supply chain verification.

### Using buf

[buf](https://buf.build/docs/cli/) is the recommended way to invoke the plugins. It has a built-in protobuf compiler and handles dependency management, so no separate `protoc` install is needed.

Create a `buf.gen.yaml`:

```yaml
version: v2
plugins:
  - local: protoc-gen-buffa
    out: src/gen
  - local: protoc-gen-buffa-packaging
    out: src/gen
    strategy: all
```

Then run:

```sh
buf generate
```

This generates per-file `.rs` output plus a `mod.rs` module tree in `src/gen/`. Include the module in your crate:

```rust,ignore
// src/main.rs or src/lib.rs
mod gen;  // no #[allow] needed — the generated mod.rs handles it
```

No hand-written bridge file is needed. The generated `mod.rs` includes `#![allow(...)]` for generated code lints and sets up the full module hierarchy.

**`protoc-gen-buffa`** emits one `.rs` file per proto file. It does not emit `mod.rs` and does not require `strategy: all` — buf can invoke it per-directory.

**`protoc-gen-buffa-packaging`** reads the full proto file set (hence `strategy: all`) and emits a `mod.rs` with nested `pub mod` blocks that `include!` each generated file at the right package nesting. Cross-package type references use `super::` relative paths within this tree, so sibling packages resolve automatically without `extern_path`. Run it once per output directory; if you have multiple codegen plugins emitting to different directories, invoke it once per directory with the appropriate `out:`.

Plugin options (passed via `opt:`):

| Option | Description |
|--------|-------------|
| `views=true` | Generate zero-copy view types (default: true) |
| `json=true` | Generate serde Serialize/Deserialize for proto3 JSON |
| `unknown_fields=false` | Disable unknown field preservation |
| `arbitrary=true` | Emit `#[derive(arbitrary::Arbitrary)]` for fuzzing |
| `extern_path=.pkg=::rust` | Map a proto package to an external Rust path |

**Remote plugin (planned):** Once published to the Buf Schema Registry, the plugin will be available as a remote plugin without requiring a local install:

```yaml
version: v2
plugins:
  - remote: buf.build/anthropic/buffa:v0.4.0
    out: src/generated
    opt: [views=true]
```

This is not yet published. Custom remote plugins require a Pro or Enterprise BSR plan, or can be installed in a self-hosted BSR instance. For now, use the `local:` plugin reference with `protoc-gen-buffa` on your PATH.

### Using protoc directly

If you prefer to use `protoc` without buf:

```sh
protoc --buffa_out=. --plugin=protoc-gen-buffa my_service.proto

# With extern_path:
protoc --buffa_out=. \
    --buffa_opt=extern_path=.my.common=::common_protos \
    --plugin=protoc-gen-buffa my_service.proto
```

See the [protoc (alternative)](#protoc-alternative) section in the Prerequisites for minimum version requirements.

### Requirements summary

**`buf generate`** requires `buf` on your PATH and `protoc-gen-buffa` locally (or a remote plugin reference in `buf.gen.yaml`). No `protoc` needed.

**`buffa-build`** requires `protoc` on your PATH (or set via `PROTOC`), unless `.use_buf()` is configured (which uses `buf` instead).

## Generated code shape

For a proto message:

```protobuf
message Person {
  string name = 1;
  int32 id = 2;
  repeated string tags = 3;
  Address address = 4;
  optional string nickname = 5;
}
```

Buffa generates:

```rust,ignore
pub struct Person {
    pub name: String,
    pub id: i32,
    pub tags: Vec<String>,
    pub address: buffa::MessageField<Address>,
    pub nickname: Option<String>,
    #[doc(hidden)]
    pub __buffa_unknown_fields: buffa::UnknownFields,
}
```

Key design choices:

- **`MessageField<T>`** for sub-message fields (not `Option<Box<T>>`)
- **`EnumValue<E>`** for open enum fields (not raw `i32`)
- **`__buffa_unknown_fields`** preserves fields from newer schema versions
- **Module nesting** for nested message types (`outer::Inner`, not `OuterInner`)
- **No serialization state** — sizes live in an external [`SizeCache`](https://docs.rs/buffa/latest/buffa/struct.SizeCache.html), so the struct holds only its proto fields plus the unknown-fields plumbing, with no interior mutability

### Generated module layout

Owned message structs and their nested-type modules sit at the package level, exactly as the proto package hierarchy implies. Everything else codegen emits — view structs, oneof enums, view-of-oneof enums, extension consts, and `register_types` — lives under a single reserved sentinel module `__buffa::` so it cannot collide with proto-derived names:

| Item | Path |
|---|---|
| Owned message | `pkg::Foo` |
| Nested owned | `pkg::foo::Bar` |
| View struct | `pkg::__buffa::view::FooView<'a>` |
| Nested view | `pkg::__buffa::view::foo::BarView<'a>` |
| Oneof enum | `pkg::__buffa::oneof::foo::Kind` |
| View-of-oneof | `pkg::__buffa::view::oneof::foo::Kind<'a>` |
| Extension const | `pkg::__buffa::ext::MY_EXT` |
| Registration fn | `pkg::__buffa::register_types` |

`__buffa` is the **only** name codegen reserves at user scope. It aligns with the `__buffa_` reserved field-name prefix (`__buffa_unknown_fields`, `__buffa_phantom`), so the rule is uniformly "anything starting `__buffa` is buffa-internal." A proto message, file-level enum, or package segment that snake-cases to `__buffa` is rejected at codegen time.

A common pattern is to alias the ancillary trees once at the top of a module that uses them heavily:

```rust,ignore
use my_crate::pkg;
use my_crate::pkg::__buffa::{oneof, view};
// then: pkg::Foo, view::FooView, oneof::foo::Kind, view::oneof::foo::Kind
```

### `MessageField<T>` — ergonomic optional messages

`MessageField<T>` wraps `Option<Box<T>>` internally but implements `Deref` to a static default instance when unset, eliminating unwrap ceremony:

```rust,ignore
// Reading — no unwrap needed, derefs to default when unset
println!("{}", msg.address.street);  // "" if address is unset

// Checking presence
if msg.address.is_set() { /* address was explicitly set */ }

// Setting
msg.address = MessageField::some(Address {
    street: "123 Main St".into(),
    ..Default::default()
});

// Or initialize-and-mutate
msg.address.get_or_insert_default().street = "123 Main St".into();

// Modify multiple fields at once (initializes if unset)
msg.address.modify(|a| {
    a.street = "123 Main St".into();
    a.city = "Springfield".into();
});

// Clearing
msg.address = MessageField::none();

// Interop with Option
let opt: Option<&Address> = msg.address.as_option();
let taken: Option<Address> = msg.address.take();
```

### `EnumValue<T>` — type-safe open enums

Proto3 enums are open (unknown values must be preserved). Buffa represents them as `EnumValue<E>`, which distinguishes known variants from unknown integer values:

```rust,ignore
// Generated enum
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(i32)]
pub enum Status {
    UNSPECIFIED = 0,
    ACTIVE = 1,
    INACTIVE = 2,
}

// Field type in generated struct
pub status: EnumValue<Status>,
```

```rust,ignore
// Setting
msg.status = EnumValue::from(Status::ACTIVE);
msg.status = EnumValue::from(42);  // Unknown(42) if not a known variant

// Direct comparison (EnumValue<E> implements PartialEq<E>)
if msg.status == Status::ACTIVE { /* ... */ }

// Pattern matching
match msg.status {
    EnumValue::Known(s) => println!("known: {:?}", s),
    EnumValue::Unknown(v) => println!("unknown value: {}", v),
}

// Conversion
let i: i32 = msg.status.to_i32();
let known: Option<Status> = msg.status.as_known();
```

**Proto2 closed enums** use the bare enum type directly (`Status`, not `EnumValue<Status>`). Unknown values on the wire are routed to `unknown_fields` instead.

**Iterating over variants.** Every generated enum implements [`Enumeration::values`], a static slice of all primary variants in proto declaration order:

```rust,ignore
for variant in Status::values() {
    println!("{:?} = {}", variant, variant.to_i32());
}

assert!(Status::values().contains(&Status::ACTIVE));
assert_eq!(Status::values().len(), 3);
```

Aliases (additional names sharing an existing value, allowed by `option allow_alias = true`) are not enum variants in Rust — they're emitted as `pub const` aliases — so they don't appear in `values()`.

### Oneofs

Oneofs are represented as Rust enums in the parallel `__buffa::oneof::` tree. The enum is named `{PascalCase(oneof_name)}` and lives at `__buffa::oneof::<owner_snake_path>::`, mirroring the owned message's nested-module path.

```protobuf
message Contact {
  oneof info {
    string email = 1;
    string phone = 2;
    Address address = 3;
  }
}
```

```rust,ignore
pub struct Contact {
    pub info: Option<__buffa::oneof::contact::Info>,
    // ...
}

// Under pkg::__buffa::oneof::contact
pub enum Info {
    Email(String),
    Phone(String),
    Address(Box<Address>),  // message variants are boxed
}
```

```rust,ignore
use my_crate::pkg::__buffa::oneof;

// Setting
msg.info = Some(oneof::contact::Info::Email("test@example.com".into()));

// Matching
match &msg.info {
    Some(oneof::contact::Info::Email(e)) => println!("email: {}", e),
    Some(oneof::contact::Info::Phone(p)) => println!("phone: {}", p),
    None => println!("not set"),
    _ => {}
}
```

**Message and group variants are always boxed** (`Box<T>`) so that recursive types compile. `From<T>` impls are generated for each boxed variant — one targeting the oneof enum, one targeting `Option<_>` — so that both `Box::new` and `Some` disappear at the call site:

```rust,ignore
msg.info = addr.into();                                       // From<Address> for Option<Info>
msg.info = Some(oneof::contact::Info::from(addr));            // From<Address> for Info
msg.info = Some(oneof::contact::Info::Address(Box::new(addr)));  // fully explicit
```

All three are equivalent. The `From` impls are only generated when the message type appears in **exactly one** variant of the oneof — if two variants share a type (e.g., two `Empty`-typed variants), `From` would be ambiguous and is skipped.

Deref coercion means pattern-matched bindings (`Some(Info::Address(a)) => a.street`) work the same as for unboxed types.

#### Naming

The oneof enum is `{PascalCase(oneof_name)}` — no suffix. The view counterpart (when view generation is enabled) is at `__buffa::view::oneof::<owner>::{PascalCase(oneof_name)}`, also with no suffix. Because oneof enums live in a separate `__buffa::oneof::` tree from nested messages and owned structs, they cannot collide with sibling types regardless of how they're named:

```protobuf
message Contact {
  // Nested message sharing the PascalCase name with the oneof below is fine.
  message Info { ... }
  oneof info {
    string email = 1;
  }
}
```

```rust,ignore
pub mod contact {
    pub struct Info { ... }          // nested message — owned tree
}
// pkg::__buffa::oneof::contact::Info — oneof enum, separate tree
```

Adding or removing sibling types never changes the Rust name of an existing oneof enum.

### Nested types and module structure

Nested proto messages are scoped in Rust modules named after the parent:

```protobuf
message Outer {
  message Inner {
    int32 value = 1;
  }
  Inner child = 1;
}
```

```rust,ignore
pub struct Outer {
    pub child: buffa::MessageField<outer::Inner>,
    // ...
}

pub mod outer {
    pub struct Inner {
        pub value: i32,
        // ...
    }
}
```

## Encoding and decoding

### The `Message` trait

All generated structs implement `buffa::Message`:

```rust,ignore
use buffa::Message;

// Encode to Vec<u8> or bytes::Bytes
let bytes: Vec<u8> = msg.encode_to_vec();
let bytes: buffa::bytes::Bytes = msg.encode_to_bytes();  // zero-copy, for async/networking

// Encode to a BufMut
msg.encode(&mut buf);

// Decode from a byte slice
let msg = Person::decode_from_slice(&bytes)?;

// Decode from a Buf
let msg = Person::decode(&mut buf)?;

// Merge into an existing message (last-write-wins for scalars,
// append for repeated, recursive merge for sub-messages)
msg.merge_from_slice(&more_bytes)?;

// Clear all fields to defaults
msg.clear();
```

### Two-pass serialization

Buffa uses a two-pass model to avoid the exponential-time size computation that affects prost with deeply nested messages:

1. **`compute_size(&self, cache)`** — walks the message tree, recording each length-delimited sub-message's encoded size in a [`SizeCache`](https://docs.rs/buffa/latest/buffa/struct.SizeCache.html).
2. **`write_to(&self, cache, buf)`** — walks the tree again, consuming cached sizes for length-delimited sub-message headers.

`encode()`, `encode_to_vec()`, and `encode_to_bytes()` perform both passes with a fresh `SizeCache` automatically — most callers never name the cache. Use `encoded_len()` if you only need the size.

### Error handling

Encoding is **infallible** — `encode()` and `write_to()` never return errors. The buffer grows as needed via `BufMut`.

Decoding returns `Result<T, DecodeError>`. See [`buffa::DecodeError`](https://docs.rs/buffa/latest/buffa/enum.DecodeError.html)
for the full list of variants (the enum is `#[non_exhaustive]`). Common cases:

- `UnexpectedEof` — truncated input
- `VarintTooLong` — malformed varint (≥ 10 bytes)
- `WireTypeMismatch` — field on wire has a different type than schema expects
- `RecursionLimitExceeded` — too-deeply-nested message (attack or bug)
- `MessageTooLarge` — exceeds configured size limit

### Decode options

For security-sensitive deployments, use `DecodeOptions` to restrict recursion depth and maximum message size:

```rust,ignore
use buffa::DecodeOptions;

// Restrict recursion depth to 50 and message size to 1 MiB:
let msg = DecodeOptions::new()
    .with_recursion_limit(50)
    .with_max_message_size(1024 * 1024)
    .decode::<MyMessage>(&mut buf)?;

// Also works for byte slices, length-delimited, merge, and views:
let msg = DecodeOptions::new()
    .with_max_message_size(64 * 1024)
    .decode_from_slice::<MyMessage>(&bytes)?;

let view = DecodeOptions::new()
    .with_recursion_limit(20)
    .decode_view::<MyMessageView>(&bytes)?;
```

| Option | Default | Description |
|--------|---------|-------------|
| `.with_recursion_limit(n)` | 100 | Max nesting depth for sub-messages |
| `.with_max_message_size(n)` | 2 GiB - 1 | Max total input size in bytes |

The default `Message::decode` / `decode_from_slice` methods use the defaults (100 depth, 2 GiB max). `DecodeOptions` is only needed when you want tighter limits.

## Zero-copy views

For every message, buffa also generates a **view type** under `pkg::__buffa::view::` that borrows directly from the input buffer:

```rust,ignore
// pkg::__buffa::view::PersonView
pub struct PersonView<'a> {
    pub name: &'a str,           // borrowed, no allocation
    pub id: i32,                 // scalars decoded by value
    pub tags: buffa::RepeatedView<'a, &'a str>,
    pub address: buffa::MessageFieldView<AddressView<'a>>,
    pub nickname: Option<&'a str>,
    // internal: __buffa_unknown_fields: buffa::UnknownFieldsView<'a>,
}
```

```rust,ignore
use buffa::MessageView;

// Zero-copy decode
let view = PersonView::decode_view(&bytes)?;
println!("name: {}", view.name);  // &str, no allocation

// Convert to owned when needed (e.g., for storage or mutation)
let owned: Person = view.to_owned_message();
```

Views are ideal for read-only request handlers where the message doesn't outlive the input buffer. They're typically 1.5-4x faster than owned decoding.

Repeated fields use `RepeatedView<T>` (a `Vec`-backed sequence); map fields use
`MapView<K, V>`, which stores entries as a Vec and does **O(n) linear lookup** —
appropriate for typical small protobuf maps but not for large in-memory indices.
For larger maps, collect into a `HashMap`: `let m: HashMap<_,_> = view.labels.into_iter().collect();`

### `OwnedView<V>` — views with `'static` lifetime

The `'a` lifetime on `PersonView<'a>` ties the view to the input buffer, preventing it from being used across async boundaries, in tower services, or anywhere a `'static` bound is required. `OwnedView<V>` solves this by storing the `bytes::Bytes` buffer alongside the decoded view, producing a `'static + Send + Sync` type:

```rust,ignore
use buffa::view::OwnedView;
use bytes::Bytes;

// Decode from a Bytes buffer (e.g., from hyper's request body)
let bytes: Bytes = receive_body().await;
let view = OwnedView::<PersonView>::decode(bytes)?;

// Direct field access via Deref — same ergonomics as a scoped view
println!("name: {}", view.name);
println!("id: {}", view.id);

// Convert to owned if needed for storage or mutation
let owned: Person = view.to_owned_message();
```

`OwnedView` implements `Deref<Target = V>`, so you access view fields directly without `.get()` calls. It also implements `Clone` (cheap — `Bytes` clone is an O(1) refcount bump), `Debug`, `PartialEq`, and `Eq` when the underlying view type does.

**When to use which:**

| Type | Lifetime | Use case |
|------|----------|----------|
| `PersonView<'a>` | Scoped (`'a`) | Synchronous processing, tests, CLI tools — when the buffer outlives all access |
| `OwnedView<PersonView>` | `'static` | RPC handlers, `tokio::spawn`, tower services, channels — when `'static + Send` is required |
| `Person` | Owned | Building messages, long-lived storage, mutation |

**Decode options** work with `OwnedView` via `decode_with_options`:

```rust,ignore
use buffa::DecodeOptions;

let view = OwnedView::<PersonView>::decode_with_options(
    bytes,
    &DecodeOptions::new()
        .with_recursion_limit(50)
        .with_max_message_size(1024 * 1024),
)?;
```

**Recovering the buffer:** If you need the underlying `Bytes` back after processing the view (e.g., for forwarding), use `into_bytes`:

```rust,ignore
let bytes = view.into_bytes(); // view is dropped, buffer returned
```

### `OwnedView` in async trait implementations

`OwnedView` works directly with `async fn` in trait implementations whose
return type carries `+ Send`. View borrows may be held across `.await` points
with no ceremony:

```rust,ignore
impl MyService for MyServer {
    async fn my_method(
        &self,
        ctx: Context,
        req: OwnedView<MyRequestView<'static>>,
    ) -> Result<(MyResponse, Context), ConnectError> {
        let name = req.name;      // &str, zero-copy borrow into the buffer
        db.lookup(name).await;    // borrow held across .await — fine
        let count = req.items.len();
        Ok((MyResponse { count: count as i32, ..Default::default() }, ctx))
    }
}
```

`OwnedView<V>` is auto-`Send`/`Sync` when `V` is. Generated view types are
auto-`Send + Sync` via their `&'static str` / `&'static [u8]` fields, so
`OwnedView<FooView<'static>>` satisfies the `Send` bound on the returned future
naturally.

#### When `to_owned_message()` is needed

Most handlers can work with view fields directly. Call `to_owned_message()`
only when you need to:

- **Pass the full message to `tokio::spawn`** — the spawned task needs
  `'static` ownership, and `OwnedView` borrows can't be moved out of the
  parent async block. Extract individual fields instead when possible.
- **Store the message** in a collection or struct that outlives the handler.
- **Mutate fields** — views are read-only.

When only one or two fields need to cross the boundary, clone just those —
view fields are standard borrowed types, so standard conversions apply
(`&str` → `.to_owned()`, `&[u8]` → `.to_vec()`, scalars are `Copy`).
`to_owned_message()` allocates every string and bytes field in the message;
reserve it for when you actually need the whole thing owned.

If background work needs many fields, move the `OwnedView` itself — it is
`Send + 'static` and moving it is a pointer-sized copy, not a data copy.

```rust,ignore
async fn handle(
    &self,
    ctx: Context,
    req: OwnedView<LogRequestView<'static>>,
) -> Result<(Response, Context), ConnectError> {
    // One field needed → clone just that field.
    let service_name = req.records[0].service_name.to_owned();
    tokio::spawn(async move { log_metrics(service_name).await });

    // Many fields needed → move the whole OwnedView (zero-copy).
    // `req` is consumed here; anything needed afterwards must be
    // extracted beforehand.
    tokio::spawn(async move { process_in_background(req).await });

    Ok((Response::default(), ctx))
}
```

### Encoding from views (`ViewEncode`)

View types also implement `ViewEncode<'a>`, which provides the same
two-pass `compute_size`/`write_to` model as `Message`. This lets you
build a message from borrowed `&str` / `&[u8]` data and serialize it
**without** allocating intermediate `String` / `Vec<u8>` fields:

```rust,ignore
use buffa::ViewEncode;
use my_pkg::__buffa::view::LogRecordView;

let labels: &[(&str, &str)] = &[("env", "prod"), ("region", "us-west-2")];

let view = LogRecordView {
    message: "request handled",
    severity: 3,
    labels: labels.iter().copied().collect(),  // MapView from borrowed pairs
    ..Default::default()
};
let wire: Vec<u8> = view.encode_to_vec();
```

This is the natural fit for high-throughput emit paths (logging, metrics,
tracing) where the source data is already borrowed. Benchmarks show ~6×
speedup over the equivalent owned `Message` build+encode for a 15-label
string-map message — the win is the eliminated per-field allocation, not
the wire write itself.

`ViewEncode` is also useful as a **proxy fast path**: decode a request
view, inspect a few fields, re-encode the same view onward — no
`to_owned_message()` round-trip:

```rust,ignore
let view = RequestView::decode_view(&inbound)?;
if view.tenant_id != expected { return Err(..); }
let outbound = view.encode_to_vec();   // wire-identical to inbound for set fields
```

`MapView` gains `From<Vec<(K, V)>>` and `FromIterator<(K, V)>` constructors
to make hand-building map views ergonomic.

## JSON serialization

Enable the `json` feature and `generate_json(true)` in your build config:

```toml
# Cargo.toml
[dependencies]
buffa = { version = "0.4", features = ["json"] }
serde_json = "1"
```

```rust,ignore
// build.rs
buffa_build::Config::new()
    .files(&["proto/my_service.proto"])
    .includes(&["proto/"])
    .generate_json(true)
    .compile()
    .unwrap();
```

The generated serde impls follow the [proto3 JSON mapping](https://protobuf.dev/programming-guides/proto3/#json):

- Field names use camelCase (`my_field` → `"myField"`)
- `int64`/`uint64` serialize as quoted strings (JavaScript precision)
- `bytes` serialize as base64
- Enums serialize as string names (`"ACTIVE"`, not `1`)
- Default-valued fields are omitted from output
- Well-known types use their canonical JSON representations

```rust,ignore
// Encode to JSON
let json = serde_json::to_string(&msg)?;

// Decode from JSON
let msg: Person = serde_json::from_str(&json)?;
```

### JSON parse options

For lenient parsing (e.g., ignoring unknown enum string values):

```rust,ignore
use buffa::json::{JsonParseOptions, with_json_parse_options};

let opts = JsonParseOptions::new().ignore_unknown_enum_values(true);
let msg = with_json_parse_options(&opts, || {
    serde_json::from_str::<Person>(json)
})?;
```

## Text format (textproto)

The protobuf text format is a human-readable debug representation — useful
for config files, golden-file tests, and logging. It is **not** a stable
interchange format: the spec permits implementations to vary whitespace and
float formatting. Use binary or JSON for data on the wire.

Enable the `text` feature and `generate_text(true)`:

```toml
# Cargo.toml
[dependencies]
buffa = { version = "0.4", features = ["text"] }
```

```rust,ignore
// build.rs
buffa_build::Config::new()
    .files(&["proto/my_service.proto"])
    .includes(&["proto/"])
    .generate_text(true)
    .compile()
    .unwrap();
```

The generated `TextFormat` impl covers nested messages, repeated fields
(both line-per-element and `[1, 2, 3]` forms on parse), maps, oneofs, and
groups/DELIMITED:

```rust,ignore
use buffa::text::{encode_to_string, encode_to_string_pretty, decode_from_str};

// Single-line: `name: "Alice" id: 42`
let compact = encode_to_string(&msg);

// Multi-line with 2-space indent
let pretty = encode_to_string_pretty(&msg);

// Parse
let msg: Person = decode_from_str(&compact)?;
```

For streaming to a `Write` sink or tuning options (e.g. printing unknown
fields), use `TextEncoder` / `TextDecoder` directly:

```rust,ignore
use buffa::text::{TextEncoder, TextFormat};

let mut out = String::new();
let mut enc = TextEncoder::new_pretty(&mut out)
    .emit_unknown(true);  // print unknown fields by number (debug-only)
msg.encode_text(&mut enc)?;
```

`Any` expansion (`[type.googleapis.com/pkg.Type] { ... }`) and the
`[pkg.ext] { ... }` extension bracket syntax both consult the `TypeRegistry`
— see [Extensions](#extensions-custom-options). If you already call
`register_types`, text format picks up those types alongside JSON. The `json`
and `text` features are independently enableable.

The `text` feature is zero-dependency and fully `no_std` + `alloc`.

## Well-known types reference

The `buffa-types` crate provides pre-generated types for Google's well-known proto files:

| Type | Proto | Rust |
|------|-------|------|
| Timestamp | `google.protobuf.Timestamp` | `buffa_types::google::protobuf::Timestamp` |
| Duration | `google.protobuf.Duration` | `buffa_types::google::protobuf::Duration` |
| Any | `google.protobuf.Any` | `buffa_types::google::protobuf::Any` |
| Struct | `google.protobuf.Struct` | `buffa_types::google::protobuf::Struct` |
| Value | `google.protobuf.Value` | `buffa_types::google::protobuf::Value` |
| ListValue | `google.protobuf.ListValue` | `buffa_types::google::protobuf::ListValue` |
| FieldMask | `google.protobuf.FieldMask` | `buffa_types::google::protobuf::FieldMask` |
| Empty | `google.protobuf.Empty` | `buffa_types::google::protobuf::Empty` |
| Wrappers | `google.protobuf.*Value` | `buffa_types::google::protobuf::Int32Value`, etc. |

### Timestamp and Duration

With the `std` feature, `Timestamp` and `Duration` convert to/from `std::time` types:

```rust,ignore
use buffa_types::google::protobuf::Timestamp;

// From SystemTime
let ts = Timestamp::now();
let ts = Timestamp::from(std::time::SystemTime::now());

// To SystemTime
let time: std::time::SystemTime = ts.try_into()?;

// From components
let ts = Timestamp::from_unix(1_700_000_000, 500_000_000);
let ts = Timestamp::from_unix_secs(1_700_000_000);
```

### Any

Pack and unpack messages into `Any`:

```rust,ignore
use buffa_types::google::protobuf::Any;
use buffa::Message;

// Pack
let any = Any::pack(&my_message, MyMessage::TYPE_URL);

// Check type
if any.is_type(MyMessage::TYPE_URL) { /* ... */ }

// Unpack
let msg: Option<MyMessage> = any.unpack_if::<MyMessage>(MyMessage::TYPE_URL)?;
```

### Value and Struct

Ergonomic builders for dynamic JSON-like values:

```rust,ignore
use buffa_types::{Value, Struct, ListValue};

let val = Value::from("hello");
let val = Value::from(42.0);
let val = Value::from(true);
let val = Value::null();

let list = ListValue::from_values(vec![
    Value::from(1.0),
    Value::from("two"),
]);

let obj = Struct::from_fields([
    ("name", Value::from("Alice")),
    ("age", Value::from(30.0)),
]);
```

## `no_std` usage

Buffa works without `std` (requires `alloc`):

```toml
buffa = { version = "0.4", default-features = false }
buffa-types = { version = "0.4", default-features = false }
```

In `no_std` mode:

- Map fields use `hashbrown::HashMap` instead of `std::collections::HashMap`
- `std::time` conversions on Timestamp/Duration are unavailable
- Scoped [`with_json_parse_options`] is unavailable (requires thread-local); use [`set_global_json_parse_options`] to set options process-wide once at startup. Note: the global API supports singular-enum accept-with-default but not repeated/map container filtering (unknown entries still error).
- JSON serialization via serde works fully (both `serde` and `serde_json` support `no_std` + `alloc`)

[`with_json_parse_options`]: https://docs.rs/buffa/latest/buffa/json/fn.with_json_parse_options.html
[`set_global_json_parse_options`]: https://docs.rs/buffa/latest/buffa/json/fn.set_global_json_parse_options.html

## Proto2 support

Buffa supports proto2 with these semantics:

- **`optional` scalars** → `Option<T>` (explicit presence)
- **`required` scalars** → bare `T` (always encoded, no default suppression)
- **`repeated`** → `Vec<T>` (unpacked by default, unlike proto3)
- **Closed enums** → bare `E` type (not `EnumValue<E>`); unknown wire values are routed to `unknown_fields`
- **Custom defaults** → custom `Default` impl using `[default = ...]` values
- **Extensions** → fully supported — see [Extensions (custom options)](#extensions-custom-options) below
- **Groups** → fully supported (both generated types and StartGroup/EndGroup wire format). Group types are emitted as nested message structs with `MessageField<GroupName>` fields, exactly like regular message fields.

## Extensions (custom options)

> **Runnable example:** [`examples/envelope/`](../examples/envelope/) —
> a standalone crate demonstrating binary get/set/has/clear, `[default = ...]`,
> `"[pkg.ext]"` JSON keys via `TypeRegistry`, and the extendee identity check.
> Run with `cargo run --manifest-path examples/envelope/Cargo.toml`.

Extensions are how protobuf attaches custom metadata to descriptor options —
`(buf.validate.field)`, `(google.api.http)`, `(grpc.gateway.protoc_gen_openapiv2.options.openapiv2_schema)`,
and so on. They're declared with `extend <OptionsType> { ... }` and attached
in proto source as `[(my.option) = {...}]`.

A common misconception: editions did not remove extensions. Proto3 removed
*general-purpose* message extensions (extending arbitrary user messages) in
favor of `google.protobuf.Any`, but `descriptor.proto` still declares
`extensions 1000 to max;` on every `*Options` message. Custom options remain
the sanctioned use of `extend` across proto2, proto3, and editions.

### Generated code

For each `extend` declaration, codegen emits a `pub const` extension descriptor under `pkg::__buffa::ext::`:

```proto
// buf/validate/validate.proto
extend google.protobuf.FieldOptions {
  optional FieldRules field = 1159;
}
```

```rust,ignore
// Generated at buf_validate::__buffa::ext::FIELD — users never write this by hand
pub const FIELD: buffa::Extension<buffa::extension::codecs::MessageCodec<FieldRules>>
    = buffa::Extension::new(1159, "google.protobuf.FieldOptions");
```

The codec type (`MessageCodec<FieldRules>`) is a zero-sized marker carrying
only type-level information. You never name it — type inference flows from the
`const` to the call site.

### Reading and writing

The extendee message implements `ExtensionSet`:

```rust,ignore
use buffa::ExtensionSet;
use buf_validate::__buffa::ext::FIELD;

// A FieldDescriptorProto from some parsed schema
let field: &FieldDescriptorProto = /* ... */;

// Read: Option<T> for singular extensions, Vec<T> for repeated
let rules: Option<FieldRules> = field.options.extension(&FIELD);

// Presence test (fast — checks for the tag, doesn't decode)
if field.options.has_extension(&FIELD) { /* ... */ }

// Write (replaces any prior value)
field_opts.set_extension(&FIELD, my_rules);

// Clear
field_opts.clear_extension(&FIELD);
```

### Extendee identity check

`extension()`, `set_extension()`, and `clear_extension()` **panic** if you
pass an extension declared for a different message — for example, passing a
message-level option to a field-level options struct:

```rust,ignore
// (buf.validate.message) extends MessageOptions, not FieldOptions — this
// is a bug in the caller. Panics with a clear message.
let _ = field.options.extension(&buf_validate::__buffa::ext::MESSAGE);
```

This matches protobuf-go (which panics) and protobuf-es (which throws).
`has_extension()` returns `false` gracefully instead of panicking, since
"is this extension set here" has a legitimate answer (`false`) even when
the extension can't extend here.

### Proto2 `[default = ...]`

Proto2 extension declarations can carry a default value:

```proto
extend MyOptions {
  optional int32 retry_count = 50001 [default = 3];
}
```

`extension_or_default()` returns the declared default when the extension is
absent. `extension()` still returns `None` — presence is distinguishable:

```rust,ignore
use my_pkg::__buffa::ext::RETRY_COUNT;

let retries: i32 = opts.extension_or_default(&RETRY_COUNT);  // 3 if unset
let explicit: Option<i32> = opts.extension(&RETRY_COUNT);    // None if unset
```

### JSON: `"[pkg.ext]"` keys

Proto3 JSON represents extensions with bracketed fully-qualified keys:
`{"[buf.validate.field]": {...}}`. Serializing and deserializing these
requires a populated `TypeRegistry` so serde knows which `"[...]"` keys
belong to which extendee and how to encode them.

Setup (once, at startup):

```rust,ignore
use buffa::type_registry::{TypeRegistry, set_type_registry};

let mut reg = TypeRegistry::new();
// Codegen emits one register_types per package under __buffa; covers Any
// types AND extensions, for both JSON and text:
my_pkg::__buffa::register_types(&mut reg);
buf_validate::__buffa::register_types(&mut reg);
set_type_registry(reg);
```

After setup, `serde_json::to_string(&msg)` and `serde_json::from_str(...)`
handle `"[...]"` keys transparently.

Unregistered `"[...]"` keys are silently dropped on parse by default — this
matches buffa's pre-0.3 behavior for all unknown JSON keys, so upgrading
doesn't break callers whose upstream sends extensions they don't use. To
error instead:

```rust,ignore
use buffa::json::{JsonParseOptions, with_json_parse_options};

let opts = JsonParseOptions::new().strict_extension_keys(true);
let msg = with_json_parse_options(&opts, || serde_json::from_str::<MyMsg>(json))?;
```

### MessageSet

`option message_set_wire_format = true` is a legacy Google-internal wire
format (it predates `extensions` ranges). Codegen errors on it by default.
If you genuinely need it — typically because an upstream dependency uses
it — enable support explicitly:

```rust,ignore
// build.rs
buffa_build::Config::new()
    .allow_message_set(true)
    // ...
```

Neither protobuf-go nor protobuf-es supports MessageSet by default (go hides
it behind `-tags protolegacy`; es has no runtime code for it). Most users
will never encounter this.

### Caching

`extension()` decodes from unknown-field storage on every call — there is no
internal cache. If you read the same extension repeatedly (e.g. in a loop
over many descriptors), hoist the call:

```rust,ignore
let rules = field.options.extension(&FIELD);  // decode once
for constraint in &rules.as_ref().map(|r| &r.constraints).unwrap_or_default() {
    // ...
}
```

## Editions support

Buffa treats proto2 and proto3 as feature presets over the editions model. The code generator reads resolved edition features directly from the `FileDescriptorProto` produced by `protoc`, so there is one code path parameterized by features rather than separate proto2/proto3 branches.

Editions 2023 and 2024 are supported. The relevant features are:

| Feature | Values |
|---------|--------|
| `field_presence` | `EXPLICIT`, `IMPLICIT`, `LEGACY_REQUIRED` |
| `enum_type` | `OPEN`, `CLOSED` |
| `repeated_field_encoding` | `PACKED`, `EXPANDED` |
| `utf8_validation` | `VERIFY`, `NONE` |
| `message_encoding` | `LENGTH_PREFIXED`, `DELIMITED` |
| `json_format` | `ALLOW`, `LEGACY_BEST_EFFORT` |

### Skipping UTF-8 validation

By default, buffa emits `String` / `&str` for all string fields and validates
UTF-8 on decode — regardless of the proto `utf8_validation` feature. This is
stricter than proto2 requires (proto2's default is `NONE`) but matches
ecosystem expectations and keeps the API ergonomic.

For performance-sensitive code where UTF-8 validation is a measurable cost
(it can be 10%+ of decode CPU for string-heavy messages), enable
`.strict_utf8_mapping(true)`. String fields with `utf8_validation = NONE` then
become `Vec<u8>` / `&[u8]` — the only sound Rust type when bytes may not be
valid UTF-8. The caller explicitly decides at each use site:

```rust,ignore
// proto (editions):
//   string raw_name = 1 [features.utf8_validation = NONE];
//   string validated_name = 2;  // default: VERIFY

let msg = MyMessageView::decode_view(&bytes)?;

// validated_name is &str — already checked:
let s: &str = msg.validated_name;

// raw_name is &[u8] — caller chooses:
let s = std::str::from_utf8(msg.raw_name)?;  // checked (same cost as VERIFY)
// SAFETY: sender is our own trusted service, always valid UTF-8.
let s = unsafe { std::str::from_utf8_unchecked(msg.raw_name) };  // fast path
```

**Proto2 warning:** proto2's default `utf8_validation` is `NONE`, so enabling
strict mapping turns ALL proto2 string fields into `Vec<u8>`. Only enable for
new code or editions projects where you control which fields opt into `NONE`.

**JSON encoding:** when strict mapping normalizes a field to bytes, JSON
serialization uses base64 (the proto3 JSON encoding for `bytes`), not a JSON
string. If you need JSON interop with other protobuf implementations that
expect string fields to be JSON strings, keep `strict_utf8_mapping` disabled
for those fields (or use `VERIFY`).

## Unknown field preservation

By default, buffa preserves fields that aren't recognized by the current schema. This is important for:

- **Proxy/middleware** use cases where messages pass through services with different schema versions
- **Round-trip fidelity** — decode and re-encode without data loss

Unknown fields are stored in the `__buffa_unknown_fields` field on every generated struct.

### Disabling preservation

To disable (omits the `UnknownFields` field from generated structs entirely):

```rust,ignore
buffa_build::Config::new()
    .preserve_unknown_fields(false)
    // ...
```

**This is primarily a memory optimization**, not a throughput one. When no
unknown fields appear on the wire — the common case for schema-aligned
services — the decode and encode paths are effectively identical regardless
of this setting (the unknown-field branch simply never fires). The measurable
difference is **24 bytes/message** for the omitted `Vec` header.

Leave preservation enabled unless you are memory-constrained (embedded / `no_std`
targets) or maintain large in-memory collections of small messages where struct
size dominates cache footprint. "I don't need round-trip fidelity" alone is not a
strong reason to disable it.

## Custom type implementations

Sometimes you want a custom Rust representation for a type that's defined in a `.proto` file — for example, mapping a proto `Duration` to `std::time::Duration` instead of the generated struct, or adding validation logic to a message's decode path.

The approach:

1. **Implement `buffa::Message` by hand** for your custom type, matching the wire format defined in the `.proto` file.
2. **Use `extern_path`** in consuming crates to tell the codegen to reference your custom type instead of generating one.

This is how `buffa-types` implements well-known types like `Timestamp` and `Duration` with ergonomic Rust APIs.

### Example: mapping a proto Range to `std::ops::Range`

A common pattern is defining range types in proto for pagination, time windows, or numeric bounds:

```protobuf
// common/range.proto
package my.common;

message Int64Range {
  int64 start = 1;
  int64 end = 2;
}
```

The generated code would produce a struct with `start: i64` and `end: i64` fields. But in Rust, it's more natural to work with `std::ops::Range<i64>`. You can implement `Message` on a thin newtype that wraps the standard range type — no `UnknownFields` field needed for a simple leaf message like this:

```rust,ignore
// my-common-protos/src/lib.rs
use std::ops::{Deref, DerefMut};
use buffa::{Message, SizeCache};
use buffa::error::DecodeError;

/// A protobuf `Int64Range` backed by `std::ops::Range<i64>`.
///
/// Derefs to `Range<i64>` for direct use with iterators, contains,
/// and other range operations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Int64Range {
    inner: std::ops::Range<i64>,
}

impl Int64Range {
    pub fn new(range: std::ops::Range<i64>) -> Self {
        Self { inner: range }
    }
}

impl Deref for Int64Range {
    type Target = std::ops::Range<i64>;
    fn deref(&self) -> &Self::Target { &self.inner }
}

impl DerefMut for Int64Range {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.inner }
}

impl From<std::ops::Range<i64>> for Int64Range {
    fn from(r: std::ops::Range<i64>) -> Self { Self::new(r) }
}

impl From<Int64Range> for std::ops::Range<i64> {
    fn from(r: Int64Range) -> Self { r.inner }
}

impl Message for Int64Range {
    fn compute_size(&self, _cache: &mut SizeCache) -> u32 {
        // Leaf message (no nested message fields), so the cache is unused.
        // For a type with a nested message field `m`, the pattern is:
        //   let slot = cache.reserve();
        //   let inner = self.m.compute_size(cache);
        //   cache.set(slot, inner);
        let mut size = 0u32;
        if self.inner.start != 0 {
            size += 1 + buffa::types::int64_encoded_len(self.inner.start) as u32;
        }
        if self.inner.end != 0 {
            size += 1 + buffa::types::int64_encoded_len(self.inner.end) as u32;
        }
        size
    }

    fn write_to(&self, _cache: &mut SizeCache, buf: &mut impl bytes::BufMut) {
        if self.inner.start != 0 {
            buffa::encoding::Tag::new(1, buffa::encoding::WireType::Varint)
                .encode(buf);
            buffa::types::encode_int64(self.inner.start, buf);
        }
        if self.inner.end != 0 {
            buffa::encoding::Tag::new(2, buffa::encoding::WireType::Varint)
                .encode(buf);
            buffa::types::encode_int64(self.inner.end, buf);
        }
    }

    fn merge_field(
        &mut self,
        tag: buffa::encoding::Tag,
        buf: &mut impl bytes::Buf,
        _depth: u32,
    ) -> Result<(), DecodeError> {
        match tag.field_number() {
            1 => self.inner.start = buffa::types::decode_int64(buf)?,
            2 => self.inner.end = buffa::types::decode_int64(buf)?,
            _ => buffa::encoding::skip_field(tag, buf)?,
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.inner = 0..0;
    }
}

impl buffa::DefaultInstance for Int64Range {
    fn default_instance() -> &'static Self {
        static INST: buffa::__private::OnceBox<Int64Range> =
            buffa::__private::OnceBox::new();
        INST.get_or_init(|| Box::new(Int64Range::default()))
    }
}
```

Note what's *not* needed:

- **`UnknownFields`** — omitted since this is a simple leaf type where round-trip preservation of unknown fields isn't important. Unknown tags are silently skipped via `skip_field`.
- **Any size-caching field** — sizes live in the external `SizeCache` threaded through `compute_size` / `write_to`. A leaf type like this doesn't touch the cache; types with nested message fields reserve a slot before recursing (see the `compute_size` comment above).

### View types for custom implementations

When view generation is enabled (the default), the codegen expects a corresponding `FooView<'a>` type for every message type `Foo`. For extern-mapped types, you must provide this.

For scalar-only types like `Int64Range` (no strings, bytes, or sub-messages to borrow), the view type gains nothing — just alias it to the owned type:

```rust,ignore
/// View type alias — Int64Range contains only scalars, so there's
/// nothing to borrow from the input buffer.
pub type Int64RangeView<'a> = Int64Range;
```

For types with string or bytes fields where zero-copy borrowing is valuable, you would implement `MessageView` by hand, following the same pattern as the generated view types.

Alternatively, pass `.generate_views(false)` in your build config if you don't use views at all.

Then in consuming crates, use `extern_path` to map the proto type:

```rust,ignore
// my-service/build.rs
buffa_build::Config::new()
    .extern_path("my.common", "::my_common_protos")
    .files(&["proto/my_service.proto"])
    .includes(&["proto/"])
    .compile()
    .unwrap();
```

Any field typed as `my.common.Int64Range` in your service proto will now use your custom type. Code that receives the message gets idiomatic Rust ranges:

```rust,ignore
let request = MyRequest::decode_from_slice(&bytes)?;

// Deref gives you Range<i64> directly
for i in request.page_range.clone() {
    // iterate the range
}

if request.page_range.contains(&42) {
    // range operations work directly
}
```

This approach keeps the `.proto` schema as the source of truth for the wire format while giving you full control over the Rust type. Buffa intentionally does not provide `#[derive(Message)]` macros, as defining protobuf types without a `.proto` schema breaks the cross-language contract that makes protobuf valuable.

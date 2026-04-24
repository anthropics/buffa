# Migrating from prost to buffa

A step-by-step guide for migrating an existing prost-based project to buffa.

## 1. Swap dependencies

```diff
 # Cargo.toml
 [dependencies]
-prost = "0.13"
-prost-types = "0.13"
+buffa = "0.4"
+buffa-types = "0.4"

 [build-dependencies]
-prost-build = "0.13"
+buffa-build = "0.4"
```

If you use JSON serialization via `pbjson`:

```diff
 [dependencies]
-pbjson = "0.7"
-pbjson-types = "0.7"
+buffa = { version = "0.4", features = ["json"] }
+buffa-types = { version = "0.4", features = ["json"] }
 serde_json = "1"
-
-[build-dependencies]
-pbjson-build = "0.7"
```

## 2. Rewrite `build.rs`

```diff
 // build.rs
 fn main() {
-    prost_build::compile_protos(
-        &["proto/my_service.proto"],
-        &["proto/"],
-    ).unwrap();
+    buffa_build::Config::new()
+        .files(&["proto/my_service.proto"])
+        .includes(&["proto/"])
+        .compile()
+        .unwrap();
 }
```

With configuration:

```diff
-prost_build::Config::new()
-    .btree_map(&["."])
-    .bytes(&["my_pkg.MyMessage.data"])
-    .type_attribute(".", "#[derive(serde::Serialize)]")
-    .out_dir("src/generated/")
-    .compile_protos(&["src/items.proto"], &["src/"])?;
+buffa_build::Config::new()
+    .files(&["src/items.proto"])
+    .includes(&["src/"])
+    .out_dir("src/generated/")
+    .generate_json(true)       // built-in, proto-conformant JSON
+    .compile()?;
```

In `src/lib.rs`, replace prost's per-file `include!` with buffa's per-package macro:

```diff
 pub mod proto {
-    include!(concat!(env!("OUT_DIR"), "/my.package.rs"));
+    buffa::include_proto!("my.package");
 }
```

Or use `.include_file("_include.rs")` in the build config and `include!` that single file — recommended when your protos span multiple packages.

## 3. Optional message fields

This is the biggest API change. Prost uses `Option<Box<M>>`, which requires explicit unwrapping. Buffa uses `MessageField<M>`, which derefs to a default instance when unset.

```diff
 // Reading a sub-message field
-if let Some(ref addr) = msg.address {
-    println!("{}", addr.street);
-}
+println!("{}", msg.address.street);  // "" if unset
+if msg.address.is_set() {
+    // handle set case
+}

 // Setting
-msg.address = Some(Box::new(Address {
-    street: "123 Main".into(),
-    ..Default::default()
-}));
+msg.address = buffa::MessageField::some(Address {
+    street: "123 Main".into(),
+    ..Default::default()
+});

 // Or use get_or_insert_default for piecewise construction
+msg.address.get_or_insert_default().street = "123 Main".into();

 // Clearing
-msg.address = None;
+msg.address = buffa::MessageField::none();

 // Converting to Option for interop
-let opt: Option<&Address> = msg.address.as_deref();
+let opt: Option<&Address> = msg.address.as_option();

 // Enforcing presence (e.g. in an RPC handler)
-let addr = msg.address.ok_or_else(|| Error::missing("address"))?;
+let addr = msg.address.ok_or_else(|| Error::missing("address"))?;  // same
```

## 4. Enum fields

Prost represents all enum fields as `i32`. Buffa uses `EnumValue<E>` for open enums (proto3 default) and bare `E` for closed enums (proto2).

```diff
 // Setting an enum field
-msg.status = Status::Active as i32;
+msg.status = buffa::EnumValue::from(Status::ACTIVE);
+// or simply:
+msg.status = Status::ACTIVE.into();

 // Reading
-match Status::try_from(msg.status) {
-    Ok(Status::Active) => { /* ... */ }
-    Ok(s) => { /* other known variant */ }
-    Err(_) => { /* unknown value */ }
-}
+match msg.status {
+    buffa::EnumValue::Known(Status::ACTIVE) => { /* ... */ }
+    buffa::EnumValue::Known(s) => { /* other known variant */ }
+    buffa::EnumValue::Unknown(v) => { /* unknown value */ }
+}
+// or use direct comparison:
+if msg.status == Status::ACTIVE { /* ... */ }

 // Getting the raw integer
-let raw: i32 = msg.status;
+let raw: i32 = msg.status.to_i32();
```

## 5. Encoding API

```diff
 use buffa::Message;  // was: use prost::Message;

 // Encode to Vec — unchanged
 let bytes = msg.encode_to_vec();

 // Encode to buffer — now infallible (no Result)
-msg.encode(&mut buf)?;
+msg.encode(&mut buf);

 // encoded_len — now compute_size (returns u32, caches result)
-let len = msg.encoded_len();
+let len = msg.compute_size() as usize;
```

## 6. Decoding API

```diff
 // Decode from bytes::Buf — now takes &mut
-let msg = Person::decode(buf)?;
+let msg = Person::decode(&mut buf)?;

 // Decode from slice — use the convenience method
-let msg = Person::decode(&*bytes)?;
+let msg = Person::decode_from_slice(&bytes)?;

 // Merge
-msg.merge(buf)?;
+msg.merge_from_slice(&bytes)?;
```

### Error types

Prost's `DecodeError` is an opaque struct with a description string. Buffa's `DecodeError` is a structured enum you can match on:

```rust,ignore
match Person::decode_from_slice(&bytes) {
    Ok(msg) => { /* ... */ }
    Err(buffa::DecodeError::InvalidUtf8) => { /* bad string */ }
    Err(buffa::DecodeError::RecursionLimitExceeded) => { /* too deep */ }
    Err(e) => { /* other decode error */ }
}
```

## 7. Well-known type imports

```diff
-use prost_types::Timestamp;
-use prost_types::Duration;
-use prost_types::Any;
-use prost_types::Struct;
-use prost_types::Value;
+use buffa_types::google::protobuf::Timestamp;
+use buffa_types::google::protobuf::Duration;
+use buffa_types::google::protobuf::Any;
+use buffa_types::google::protobuf::Struct;
+use buffa_types::google::protobuf::Value;
```

**Important:** Prost maps some wrapper types to Rust primitives (`google.protobuf.Int32Value` → `i32`, `google.protobuf.Empty` → `()`). Buffa does not — all well-known types are proper structs:

```diff
 // Wrapper types
-let val: Option<i32> = msg.optional_int;       // prost maps to primitive
+let val: buffa::MessageField<buffa_types::google::protobuf::Int32Value> = msg.optional_int;
+let inner: i32 = msg.optional_int.value;        // access the wrapped value
```

## 8. JSON serialization

If you were using `pbjson` / `pbjson-build` for proto-canonical JSON:

```diff
 // build.rs
-prost_build::Config::new()
-    .compile_protos(&["my.proto"], &["proto/"])?;
-pbjson_build::Builder::new()
-    .register_descriptors(&descriptor_bytes)?
-    .build(&[".my_package"])?;
+buffa_build::Config::new()
+    .files(&["my.proto"])
+    .includes(&["proto/"])
+    .generate_json(true)     // built-in, no separate crate needed
+    .compile()?;
```

Usage is the same — `serde_json::to_string` / `serde_json::from_str`.

## 9. New capabilities

These are buffa features with no prost equivalent:

### Zero-copy views

```rust,ignore
use buffa::MessageView;
use my_crate::pkg::__buffa::view::PersonView;

let view = PersonView::decode_view(&bytes)?;
println!("name: {}", view.name);  // &str, no allocation

let owned: Person = view.to_owned_message();
```

### `OwnedView` for async/RPC use

`OwnedView<V>` wraps a view with its backing `Bytes` buffer, producing a `'static + Send + Sync` type that works with tower, `tokio::spawn`, and RPC frameworks:

```rust,ignore
use buffa::view::OwnedView;

let view = OwnedView::<PersonView>::decode(bytes)?;
println!("name: {}", view.name);  // Deref, zero-copy, 'static
```

### Unknown field preservation

Buffa preserves unknown fields by default. Messages decoded from a newer schema version retain the unknown fields through re-encoding.

### Linear-time serialization

Buffa's two-pass `compute_size()` / `write_to()` model avoids the quadratic size computation that affects prost with deeply nested messages.

## 10. Feature comparison

Features that prost supports but buffa does not (yet):

| prost feature | buffa status |
|---------------|-------------|
| `btree_map(&[...])` | Not supported. Maps always use `HashMap`. |
| `bytes(&[...])` | Supported. `.use_bytes_type()` for all, or `.use_bytes_type_in(&[...])` for specific fields. |
| `extern_path(proto, rust)` | Supported. Same API: `.extern_path(".pkg", "::crate")`. |
| `type_attribute(path, attr)` | Not supported. Use `generate_json(true)` for serde. |
| `field_attribute(path, attr)` | Not supported. |
| `service_generator(...)` | Not supported. Services codegen is planned. |
| `#[derive(prost::Message)]` | Not provided. Implement `Message` by hand and use `extern_path` (see [Custom types](guide.md#custom-type-implementations)). |
| `prost::Name` trait | Not supported. The generated `TYPE_URL` associated constant covers the common case (`Any` packing/unpacking). |

Features that buffa has but prost does not:

| buffa feature | prost equivalent |
|---------------|-----------------|
| `MessageField<T>` (deref to default) | `Option<Box<T>>` (manual unwrap) |
| `EnumValue<E>` (typed open enums) | `i32` (raw integer) |
| Zero-copy `MessageView` types | None |
| Unknown field preservation | None |
| Built-in proto-canonical JSON | Requires `pbjson` / `pbjson-build` |
| Two-pass cached-size encoding | None (recomputes at each level) |
| Proto2 closed enum support | Partial (no closed-enum routing to unknown fields) |
| Protobuf editions support | None |

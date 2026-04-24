# Migrating from the `protobuf` crate to buffa

This guide covers migration from both versions of the `protobuf` crate:

- **stepancheg v3** (`protobuf = "3.x"`) — the community-maintained pure-Rust implementation that most existing Rust codebases use.
- **Google official v4** (`protobuf = "4.x"`) — the Google-maintained version backed by the upb C library.

## 1. Swap dependencies

### From stepancheg v3

```diff
 [dependencies]
-protobuf = "3"
+buffa = "0.4"
+buffa-types = "0.4"

 [build-dependencies]
-protobuf-codegen = "3"
-protoc-bin-vendored = "3"    # if using vendored protoc
+buffa-build = "0.4"
```

### From Google v4

```diff
 [dependencies]
-protobuf = "=4.33.1-release"
+buffa = "0.4"
+buffa-types = "0.4"

 [build-dependencies]
-protobuf-codegen = "=4.33.1-release"
+buffa-build = "0.4"
```

## 2. Rewrite `build.rs`

### From v3

```diff
-protobuf_codegen::Codegen::new()
-    .protoc()
-    .protoc_path(&protoc_bin_vendored::protoc_bin_path().unwrap())
-    .includes(&["src/protos"])
-    .input("src/protos/my_message.proto")
-    .cargo_out_dir("protos")
-    .run_from_script();
+buffa_build::Config::new()
+    .files(&["src/protos/my_message.proto"])
+    .includes(&["src/protos"])
+    .compile()
+    .unwrap();
```

### From v4

```diff
-protobuf_codegen::CodeGen::new()
-    .inputs(["my_file.proto"])
-    .include("proto/")
-    .generate_and_compile()
-    .expect("failed to compile protos");
+buffa_build::Config::new()
+    .files(&["my_file.proto"])
+    .includes(&["proto/"])
+    .compile()
+    .unwrap();
```

### Including generated code

```diff
-// v3
-include!(concat!(env!("OUT_DIR"), "/protos/my_message.rs"));
-// v4
-include!(concat!(env!("OUT_DIR"), "/protobuf_generated/generated.rs"));
+// buffa
+buffa::include_proto!("my.package");
```

The macro argument is the dotted protobuf package name. For multi-package builds, prefer `.include_file("_include.rs")` in the build config and `include!` that single file — it sets up the package module tree and `buffa::include_proto!` calls for you.

## 3. Encoding and decoding

### From v3

```diff
-use protobuf::Message;
+use buffa::Message;

 // Decode
-let msg = MyMessage::parse_from_bytes(&bytes)?;
+let msg = MyMessage::decode_from_slice(&bytes)?;

 // Encode
-let bytes = msg.write_to_bytes()?;
+let bytes = msg.encode_to_vec();   // infallible, no Result

 // Merge
-msg.merge_from_bytes(&more_bytes)?;
+msg.merge_from_slice(&more_bytes)?;

 // Clear
 msg.clear();  // unchanged

 // Size
-msg.compute_size();
-let size = msg.cached_size() as usize;
+let size = msg.encoded_len() as usize;
```

### From v4

```diff
-use protobuf::{Parse, Serialize};
+use buffa::Message;

 // Decode
-let msg = MyMessage::parse(&bytes)?;
+let msg = MyMessage::decode_from_slice(&bytes)?;

 // Encode
-let bytes = msg.serialize()?;
+let bytes = msg.encode_to_vec();   // infallible
```

## 4. Optional message fields

Both stepancheg v3 and buffa use a type called `MessageField`, but the APIs differ.

### From v3

```diff
 // Check presence
-msg.sub_msg.is_some()
+msg.sub_msg.is_set()       // renamed

 // Read with default fallback
-let sub: &SubMessage = msg.sub_msg.get_or_default();
+let sub: &SubMessage = &msg.sub_msg;   // Deref gives default automatically

 // Mutable access
-let sub: &mut SubMessage = msg.sub_msg.mut_or_insert_default();
+let sub: &mut SubMessage = msg.sub_msg.get_or_insert_default();

 // Set
-msg.sub_msg = protobuf::MessageField::some(value);
+msg.sub_msg = buffa::MessageField::some(value);

 // Clear
-msg.sub_msg = protobuf::MessageField::none();
+msg.sub_msg = buffa::MessageField::none();

 // To Option
-let opt: Option<SubMessage> = msg.sub_msg.into_option();
+let opt: Option<SubMessage> = msg.sub_msg.into_option();  // consumes field
+let opt: Option<SubMessage> = msg.sub_msg.take();         // leaves field unset
+let opt: Option<&SubMessage> = msg.sub_msg.as_option();   // borrows

 // Required-field check (common in RPC handlers)
+let sub: SubMessage = msg.sub_msg.ok_or_else(|| MyError::missing("sub_msg"))?;
```

### From v4

v4 uses a proxy-based API with `has_foo()` / `foo()` / `foo_mut()` accessors:

```diff
 // Check presence
-msg.has_sub_msg()
+msg.sub_msg.is_set()

 // Read (v4 returns a view proxy; buffa returns &T via Deref)
-let view: SubMsgView<'_> = msg.sub_msg();
+let sub: &SubMsg = &msg.sub_msg;

 // Write (v4 uses set_foo; buffa uses direct field assignment)
-msg.set_sub_msg(value);
+msg.sub_msg = buffa::MessageField::some(value);

 // Clear
-msg.clear_sub_msg();
+msg.sub_msg = buffa::MessageField::none();
```

## 5. Enum fields

### From v3

v3's `EnumOrUnknown<E>` is similar to buffa's `EnumValue<E>`:

```diff
-use protobuf::EnumOrUnknown;
+use buffa::EnumValue;

 // Set
-msg.severity = EnumOrUnknown::new(Severity::INFO);
+msg.severity = EnumValue::from(Severity::INFO);
+// or:
+msg.severity = Severity::INFO.into();

 // Read known value
-let val: Severity = msg.severity.enum_value_or_default();
+let val: Option<Severity> = msg.severity.as_known();
+// or for direct comparison:
+if msg.severity == Severity::INFO { /* ... */ }

 // Read with error handling
-match msg.severity.enum_value() {
-    Ok(known) => { /* ... */ }
-    Err(raw_i32) => { /* ... */ }
-}
+match msg.severity {
+    EnumValue::Known(known) => { /* ... */ }
+    EnumValue::Unknown(raw_i32) => { /* ... */ }
+}

 // Raw integer
-let raw: i32 = msg.severity.value();
+let raw: i32 = msg.severity.to_i32();
```

### From v4

v4 uses `#[repr(transparent)]` newtypes with associated constants:

```diff
 // v4 enums are newtypes: Status(i32) with Status::ACTIVE, Status::INACTIVE
 // buffa enums are proper Rust enums with EnumValue<E> wrapper

 // Set
-msg.set_status(Status::ACTIVE);
+msg.status = Status::ACTIVE.into();

 // Read
-let s: Status = msg.status();    // returns the newtype
+if msg.status == Status::ACTIVE { /* ... */ }
```

## 6. String types

### From v4 only

v4 uses `ProtoStr` / `ProtoString` instead of standard Rust string types. Buffa uses `String` and `&str` directly:

```diff
 // v4 string access
-let name: &ProtoStr = msg.name();
-msg.set_name("hello");
+// buffa
+let name: &str = &msg.name;       // or &msg.name in view: &'a str
+msg.name = "hello".into();
```

## 7. Oneofs

Buffa places oneof enums in a parallel `__buffa::oneof::` tree. The enum is named `{PascalCase(oneof_name)}` (no suffix) at `pkg::__buffa::oneof::<owner_snake_path>::`.

### From v3

v3 uses `Option<OneofEnum>` — same shape, different path:

```diff
+use my_crate::pkg::__buffa::oneof;
 match &msg.value {
-    Some(my_message::Value_oneof::StringValue(s)) => { /* ... */ }
+    Some(oneof::my_message::Value::StringValue(s)) => { /* ... */ }
     None => {}
 }
```

### From v4

v4 uses individual `has_/get/set` accessors plus a case enum:

```diff
+use my_crate::pkg::__buffa::oneof;
 // v4: individual accessors
-if msg.has_string_value() {
-    let s: &ProtoStr = msg.string_value();
-}
+// buffa: pattern match on the oneof enum
+if let Some(oneof::my_message::Value::StringValue(s)) = &msg.value {
+    println!("{}", s);
+}
```

## 8. Repeated and map fields

These are largely unchanged:

```diff
 // Repeated — Vec<T> in both v3 and buffa
 msg.tags.push("foo".into());

 // Maps — HashMap<K, V> in both v3 and buffa
 msg.labels.insert("key".into(), "value".into());
```

v4 uses proxy types (`RepeatedView`/`MapView`):

```diff
 // v4 repeated
-let view: RepeatedView<'_, i32> = msg.values();
+let values: &Vec<i32> = &msg.values;

 // v4 map
-let view: MapView<'_, String, String> = msg.labels();
+let labels: &HashMap<String, String> = &msg.labels;
```

## 9. Well-known types

```diff
-// v3
-use protobuf::well_known_types::timestamp::Timestamp;
-use protobuf::well_known_types::any::Any;
+use buffa_types::google::protobuf::Timestamp;
+use buffa_types::google::protobuf::Any;
```

v4 uses the same `protobuf::well_known_types::*` path as v3.

## 10. Special/unknown fields

### From v3

v3 stores unknown fields in `special_fields: SpecialFields`:

```diff
-msg.special_fields.unknown_fields();
+&msg.__buffa_unknown_fields   // directly on the struct
```

### From v4

v4 manages unknown fields internally through upb. Buffa exposes them as a public field.

## 11. What you gain

| Feature | protobuf v3/v4 | buffa |
|---------|---------------|-------|
| Pure Rust | v3: yes, v4: no (upb C) | Yes |
| Editions support | v3: no, v4: yes | Yes |
| Zero-copy views | v4 has proxy views (upb-backed) | `MessageView` types |
| Built-in JSON | v3: `protobuf-json-mapping`, v4: no | `generate_json(true)` |
| Open enum types | `EnumOrUnknown<E>` (v3), newtypes (v4) | `EnumValue<E>` |
| `MessageField` deref | `get_or_default()` (v3), proxy (v4) | Direct `Deref` |
| Cached-size encoding | Both | Yes |
| Unknown field preservation | Both | Yes |

## 12. Feature comparison

Features in `protobuf` v3/v4 that buffa does not support:

| Feature | buffa status |
|---------|-------------|
| Runtime reflection (`descriptor()`) | Not supported. Server reflection (raw descriptor bytes) is planned. |
| `protobuf::text_format` | Use `generate_text(true)` + `buffa::text::{encode_to_string, decode_from_str}` |
| `protobuf::json` (v3) | Use `generate_json(true)` + `serde_json` instead |
| Lite runtime | Not applicable (buffa is already lightweight) |
| `proto!` construction macro (v4) | Not supported |
| Service generation | Planned |
| `#[derive(Message)]` | Not provided. Implement `Message` by hand and use `extern_path`. |

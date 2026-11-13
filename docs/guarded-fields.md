# Guarded field representations

An application can keep credentials and personal data in field types that
redact `Debug` output, wipe their allocations on drop, and control how the
fields are exported. buffa provides the mechanisms; the representation types
and their security policy belong in the consuming proto crate.

## Configure generated fields

Mark sensitive fields in the schema:

```protobuf
bytes private_key = 3 [debug_redact = true];
repeated float embedding = 4 [debug_redact = true];
```

The standard option redacts the field in the `Debug` output of generated owned
messages, eager and lazy views, owned views, and oneof variants. It does not
change serialization. Public fields such as a media type keep their normal
representation and `Debug` behavior.

Select local representations and their JSON policies in `buf.gen.yaml`:

```yaml
plugins:
  - local: protoc-gen-buffa
    out: src/generated
    opt:
      - json=true
      - views=true
      - bytes_type_custom_in=.example.Request.private_key=crate::SecretBytes
      - string_type_custom_in=.example.Request.identity=crate::SecretString
      - repeated_type_custom_in=.example.Request.embedding=crate::SecretList<*>
      - json_policy_type=crate::SecretBytes
      - json_policy_type=crate::SecretString
```

The `*_custom_in` options are repeatable and take protobuf path prefixes. A
rule can name a field, message, package, or `.` for every field; the last
matching rule wins. Use the field's own protobuf path, also when it belongs to
a oneof. The equivalent `buffa-build` methods have the same names.

A custom type used as a `repeated` element or in a `map` must be local to the
generating crate, because codegen emits element trait implementations for it.
Define a local newtype where needed rather than sharing one foreign type.

A `string` rule on a `map` field covers both its key and its value. With
`strict_utf8_mapping`, a map whose string keys have UTF-8 validation disabled
becomes a byte-key map, and its keys keep ordinary `Vec<u8>` storage; when
its values are bytes too, those values also keep `Vec<u8>`. Keep UTF-8
validation enabled for guarded map keys, and check the generated slot types
when you write representation rules.

## Control binary decoding

`ProtoBytes::decode_length_delimited` and
`ProtoString::decode_length_delimited` let a representation read its own field.
buffa reads and validates the field's length prefix, then passes a `FieldPayload`
bounded to exactly that field: `payload.remaining()` is the validated length.
It exposes only bounded `Buf` operations; overrides cannot widen the limit or
access the underlying input. The
defaults pass a `WirePayload` to `from_wire`, which may first copy a field that spans input
chunks into an ordinary `Bytes`. Override the method when that temporary copy
is not acceptable:

```rust,ignore
fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
    let mut bytes = Zeroizing::new(Vec::new());
    // Reserve before writing, so no reallocation leaves a copy behind.
    bytes
        .try_reserve_exact(payload.remaining())
        .map_err(|_| DecodeError::MessageTooLarge)?;
    while payload.has_remaining() {
        let chunk = payload.chunk();
        bytes.extend_from_slice(chunk);
        let read = chunk.len();
        payload.advance(read);
    }
    Ok(core::mem::take(&mut *bytes).into())
}
```

Validate UTF-8 while the bytes are still in guarded storage.

## Control JSON

List a custom `bytes` or `string` type with `json_policy_type` and implement
`buffa::json_helpers::ProtoBytesJson` or `ProtoStringJson` for it. Generated
code then sends every JSON use of the type through that implementation:
singular, optional, repeated and oneof fields, `map` keys and values, and the
same fields of generated views, lazy views and owned views. The type's own
`serde` impls are not consulted, and there is no default implementation, so a
listed type without the trait fails to compile.

Serialization receives the field's `&[u8]` or `&str`, which is also all a view
holds, so one policy covers owned and borrowed export. A policy can:

- encode with base64 scratch space it owns and wipes;
- refuse export, by returning an error, while still accepting import;
- decode base64 into storage reserved under a wipe owner before any byte is
  written.

The policy never receives JSON `null`. An implicit-presence field becomes the
default value, an optional field or oneof variant becomes unset, and a `null`
element of a `repeated` field or `map` value is rejected, before the policy
runs. Unlisted custom types keep their existing JSON behavior.

A listed policy type that no unshadowed representation rule selects is a codegen
error. A rule selecting a type with the same final name but a different path
from a listed policy type is also an error; list both paths explicitly when
they intentionally name different types. Bindings are checked independently of
which directory a Buf plugin request contains.

To reject unknown JSON fields, use `deny_unknown_json_fields=true`. Its errors
can name the unexpected field, so keep diagnostics from untrusted input out of
logs that must not contain attacker-controlled text.

The runtime and generator do not depend on a secret-storage library. A
representation may use `zeroize`, `secrecy` and its own `base64` dependency.
`buffa-test/src/guarded_repr.rs` and its allocator test show a complete
integration; they are test fixtures, not a general-purpose secure parsing
library.

## Lifecycle and ownership boundaries

A field type that owns and wipes its allocation covers ordinary assignment,
`Message::clear`, drop, duplicate singular wire fields, oneof switches,
replacement of a map value, and fields already decoded when a later part of
the input fails. Keep and wipe spare capacity too: converting a secret `Vec`
into a smaller allocation can release unwiped storage. A custom `repeated`
collection must wipe the allocations it outgrows; `Vec`'s own growth frees the
old buffer without wiping it.

These mechanisms do not cover:

- **Scalar fields.** Numbers, booleans, enums and well-known types such as
  `Timestamp` and `google.protobuf.Value` are stored as ordinary values. They
  are not wiped when replaced, cleared or dropped.
- **`Clone`.** Generated messages implement `Clone`. Every cloned guarded
  field must become its own wipe owner.
- **JSON decoding of `repeated` scalars.** The generated JSON path collects
  elements into a `Vec` before converting it into the custom collection, so
  that `Vec`'s growth is not guarded.
- **Buffers outside the message.** Input buffers, borrowed views, an owned
  view's reference-counted backing bytes, reader buffering, serializer output,
  JSON parser scratch space and parser errors are separate ownership
  boundaries for the application to guard or constrain.

Disabling unknown-field preservation (`unknown_fields=false`) stops buffa from
retaining unknown payloads, but unknown fields are then skipped rather than
rejected, and an unknown-field count limit does not apply to them. An
application that must reject unknown fields, duplicates or noncanonical
encodings needs its own validation.

Resource limits bound how much an attacker can make the decoder allocate;
wiping controls how long owned copies live. Use both:

```rust,ignore
let request: Request = buffa::DecodeOptions::new()
    .with_max_message_size(2 * 1024 * 1024)
    .with_recursion_limit(32)
    .with_element_memory_limit(8 * 1024 * 1024)
    .decode_from_slice(input)?;
```

JSON input needs its own size and recursion limits.

Binding and spelling validation also runs with JSON disabled. Valid policy
configuration then warns that its JSON behavior is inactive; invalid bindings
remain errors. Intentional same-name types must each supply and list a policy.

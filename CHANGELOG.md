# Changelog

All notable changes to buffa will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the [Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html): breaking changes increment the minor version (0.1 → 0.2), additive changes increment the patch version.

## [Unreleased]

### Added

- **`buffa::SizeCachePool` — opt-in reuse of the encode size-cache spill
  allocation** (#225). Every `encode` / `encoded_len` builds a fresh
  `SizeCache`; its inline storage is free, but a message with more than the
  inline capacity of nested length-delimited sub-messages (deeply nested,
  repeated-sub-message shapes) spills to a heap `Vec` on every encode.
  `SizeCachePool` is a caller-owned free-list of those spill buffers — keep one
  in a `thread_local!` or a request/connection context and call `pool.encode`,
  `pool.encode_view`, or `pool.encoded_len` to reuse one allocation across many
  encodes. buffa holds no global state; only the spill `Vec` is pooled (each
  cache's inline array stays on the stack), so routing small messages through a
  pool costs only a `Vec` pop/push of an empty buffer — no allocation, no
  thread-local, no synchronization — and the pool is `alloc`-only (`no_std`-OK).
  Bounded by `max_buffers` (free-list length) and `max_capacity` (per-buffer
  capacity, shrunk on return). Also adds `SizeCache::with_spill_buffer` /
  `into_spill_buffer` to source/sink the spill buffer for manual reuse. Additive
  and non-breaking; the default `encode` path is unchanged.

- **Custom owned `string` types for `map` keys and values** (#156). A `string_type`
  rule (`string_type_custom` / `string_type_custom_in`) now also applies to a
  `map<string, V>` key and a `map<K, string>` value — one rule on the map field
  path covers both slots of a `map<string, string>` — mirroring how `bytes_type`
  already reaches `map<K, bytes>` values. The element decodes/encodes through the
  new sealed `buffa::map_codec::ProtoStringMap<S>` codec; no new build knob. The
  wire format is unchanged and view types still borrow `&str`. Requirements on
  the custom type when used in a map: `Hash + Eq` (or `Ord` for
  `map_type(BTreeMap)`) for a key; `serde::Serialize` / `Deserialize` for JSON;
  and — because the map paths have no per-key generic shim — a crate-local
  newtype (vtable reflection emits `ReflectMapKey` / `ReflectElement` for it) and
  its own `Arbitrary` impl under `generate_arbitrary`. Custom-string-keyed maps
  whose value needs proto3-JSON encoding (int64/float/bytes) serialize through a
  new `proto_str_key_map` `with`-module (the existing `proto_map` requires
  `Display + FromStr`, which a `ProtoString` need not implement).

- **Pluggable owned map container for `map<K, V>` fields** (#156). A new
  `buffa::MapStorage` trait (with associated `Key` / `Value` types) selects the
  owned map collection, via `buffa_build`'s `map_type` / `map_type_custom` knobs.
  The default stays `HashMap`; `BTreeMap` is a zero-dependency built-in giving
  deterministic (reproducible) encoded bytes, and a crate-local newtype can wrap
  any other map (e.g. `IndexMap`). JSON and `arbitrary` work for every proto map
  key/value type regardless of the container — the proto-JSON `with`-modules and
  the `arbitrary` shim are generic over `MapStorage`. The wire format is
  unchanged; only the in-memory collection changes, and view types are
  unaffected.

- **Pluggable owned pointer for message fields** (#156). A new
  `buffa::ProtoBox<T>` trait (`Deref<Target=T> + DerefMut { new, into_inner }`)
  selects the smart pointer that a singular message field's `MessageField` wraps
  — and the pointer of a **boxed oneof message/group variant** — via
  `buffa_build`'s `box_type` / `box_type_custom` knobs (the custom path takes a
  `*`-templated type, e.g. `"::my_crate::SmallBox<*>"`). A oneof variant opted
  into inline storage via `unbox_oneof` takes precedence and gets no pointer;
  recursive variants stay pointered and so accept a custom pointer. The
  default stays `Box<T>` and generated output is byte-identical. Only
  exclusively-owned pointers qualify (`Rc`/`Arc` are excluded — the decoder
  merges in place via `DerefMut`); inline pointers like `SmallBox` avoid the
  per-field heap allocation. **Source-breaking note:** `MessageField<T>` gained
  a defaulted pointer type parameter (`MessageField<T, P = Box<T>>`), so a
  *standalone* `MessageField::some(x)` / `none()` with no pinning context now
  needs a type annotation (`MessageField::<T>::some(x)`); struct-literal and
  typed-assignment construction are unaffected. Added `MessageField::from_pointer`
  (the generic counterpart to the `Box`-only `from_box`).

- **Docker-free conformance runs** (#192). `task conformance-tools-local`
  builds `conformance_test_runner` from the pinned protobuf tag into
  `.local/bin/` and `task conformance-local` executes the same seven runs
  as the Docker path with the same failure lists — for dev environments
  without a Docker daemon or GHCR access.

- **Opt-in lazy views: the additive `FooLazyView` family** (#165). With
  `Config::lazy_views(true)` (plugin: `lazy_views=true`), each message
  additionally generates a `FooLazyView<'a>` implementing the new
  `buffa::LazyMessageView` trait — the eager `FooView` family is unchanged
  and output is byte-identical with or without the flag. `decode_lazy`
  performs one non-recursive scan, recording singular/repeated message
  fields as undecoded byte ranges (`LazyMessageFieldView` /
  `LazyRepeatedView`) that decode on access via fallible by-value accessors
  (`.get()`, `.get_or_default()`, iteration), so reading a few fields of
  many large sub-messages no longer allocates or recurses into untouched
  sub-trees (~12× less allocation churn on the issue's workload; ~200×
  faster when only 1% of items are read). Proto merge semantics are
  preserved via per-occurrence fragments merged on access; the recursion
  depth and unknown-field allowance recorded at each deferred field are
  replayed per access (per-subtree capture of the shared pool), so
  `DecodeOptions` limits flow through `decode_lazy_view`. Conversions are
  fallible (`to_owned_message() -> Result`), the lazy `Serialize` impl
  surfaces deferred errors as serde errors, and re-encoding replays
  recorded fragments verbatim without validating them. Groups, oneof
  message variants, map message values, and extern-typed fields (WKTs,
  `extern_path`) stay eager inside the lazy view; the lazy family has no
  reflection/`OwnedView`/text surface. A dedicated `BUFFA_VIA_LAZY`
  conformance runner mode covers the lazy decoder against the full corpus.

- **Customizable feature-gate names** (#169). `CodeGenConfig::feature_gate_names`
  (exposed as `buffa_build::Config::{json,views,text,reflect}_feature_name` and
  `protoc-gen-buffa`'s `{json,views,text,reflect}_feature=` options) renames the
  crate features that `gate_impls_on_crate_features` conditions the generated
  impls on — e.g. gating the serde JSON impls behind a feature named `serde`
  instead of `json`. Defaults are unchanged; the knob is inert unless gating is
  enabled. A name that is not a valid Cargo feature name fails generation with
  an error when its gate is active — the alternative is a permanently-false
  `#[cfg]` that silently compiles the gated impls away.

- **`buffa-build` / `buffa-codegen`: `oneof_attribute`** (#166) — attach Rust
  attributes to generated oneof enums only (not message structs, not regular
  enums), matched against the oneof's fully-qualified path
  (`.pkg.Message.oneof_name`) with the same prefix rules as `type_attribute`.
  Completes the `type` / `message` / `enum` / `field` attribute family for
  the case where a oneof needs a different attribute set than the
  surrounding types.

- **Zero-copy views enforce the unknown-field limit and coalesce adjacent
  unknown records.** View decoding previously stored one borrowed span (16
  bytes) per unknown wire record with no bound beyond the input size. Spans
  for adjacent unknown records now coalesce into a single span — a
  contiguous run of unknown fields costs one `Vec` slot regardless of field
  count, and re-encodes byte-identically — and each *new* span (one per
  unknown run) is counted against the same unknown-field limit that bounds
  owned-message decoding, configured via
  `DecodeOptions::with_unknown_field_limit` and honored by
  `DecodeOptions::decode_view`. As part of this, the view decode path now
  threads `DecodeContext<'_>`: `MessageView::decode_view_with_limit(buf,
  depth)` is replaced by `decode_view_with_ctx(buf, ctx)`, and generated
  views' hidden `_decode_depth` helpers become `_decode_ctx` (**breaking**
  for code generated by earlier releases, which must be regenerated —
  consistent with the owned-path change below).

- **View-to-owned conversion is now fallible and honors the decode-time
  limit.** `MessageView::to_owned_message` and `to_owned_from_source` (and
  the `OwnedView` wrapper) now return `Result<Owned, DecodeError>`
  (**breaking**): generated conversions previously swallowed unknown-field
  re-materialization errors via `unwrap_or_default()`, silently dropping
  every unknown field. `UnknownFieldsView::to_owned` also now re-materializes
  under the unknown-field allowance that remained when the view recorded its
  first unknown field — so a tight `with_unknown_field_limit` configured at
  `decode_view` time carries through conversion, where each owned
  `UnknownField` counts individually (unlike the coalesced spans the view
  stores). Views built manually via `push_raw` fall back to the default
  limit.

- **Unknown-field decode limit bounds decoder memory amplification.**
  Unknown wire data can occupy ~20× more memory decoded than encoded:
  every 2-byte unknown varint field materializes a ~40-byte
  `UnknownField`, so a 64 MiB payload of minimal unknown fields (flat or
  nested in a group) could force over 1 GiB of heap — not bounded by
  `with_max_message_size`, which only caps input length. Decoding now
  counts every materialized unknown field against a limit shared across
  the whole decode call and fails with the new
  `DecodeError::UnknownFieldLimitExceeded` when it is exceeded. The
  default is 1,000,000 fields per decode (`DEFAULT_UNKNOWN_FIELD_LIMIT`),
  capping slot overhead at ~40 MB, and applies to all decode entry points
  including the trait-level convenience methods; tune it with
  `DecodeOptions::with_unknown_field_limit`. Unknown length-delimited
  payload bytes are not counted against the limit — the decoder only
  allocates them once the sender has actually delivered the bytes, so
  they are bounded by the input size and governed by
  `with_max_message_size`. The limit covers owned-message and
  `DynamicMessage` decoding; zero-copy views store unknown fields as
  borrowed spans and are not affected by the amplification.

- **`chrono` interop for `buffa-types`** (#163). A new off-by-default,
  `no_std`-compatible `chrono` feature adds conversions between the
  well-known `Timestamp` / `Duration` types and `chrono::DateTime` /
  `chrono::TimeDelta`: `From<chrono::DateTime<Tz>> for Timestamp` (any time
  zone; the instant is preserved), `TryFrom<Timestamp> for DateTime<Utc>`,
  `From<TimeDelta> for Duration`, and `TryFrom<Duration> for TimeDelta`. The
  last returns a new `DurationChronoError` because `TimeDelta`'s range
  (±`i64::MAX` milliseconds) is narrower than proto `Duration`'s.
  Contributed by @yordis.

- **New `buffa-yaml` crate: YAML serialization with protobuf-JSON semantics**
  (Phase 1 of protoyaml support, #101). A thin carrier layer that routes
  buffa's generated protobuf-JSON serde impls through `serde_norway`, so YAML
  I/O gets the full protobuf JSON mapping: `camelCase`/`snake_case` field
  names, quoted `int64`/`uint64`, base64 bytes, enum string names, and
  canonical well-known-type encodings. Public API: `to_string`, `to_writer`,
  `from_str`, `from_slice`, `from_reader`, plus `to_string_view` /
  `to_writer_view` for zero-copy views, and an `Error` type exposing a
  carrier-agnostic `Location { line, column }`. Requires message types
  generated with `json = true`. Contributed by @rsd-darshan.

- **Proto2 required-field presence on views** (#170). Generated view types
  (`FooView` and `FooLazyView`) for messages with proto2/editions
  `LEGACY_REQUIRED` singular fields now expose `has_<field>()` accessors
  that distinguish a field absent on the wire from one explicitly encoded
  with its default value. Scalar required fields are tracked via hidden
  `__buffa_required_seen_*` bit words; message/group required fields
  delegate to `MessageFieldView::is_set()` / `LazyMessageFieldView::is_set()`.
  The view `ReflectMessage::has()` implementation consults the same
  tracking, so reflection agrees with the inherent accessors. Owned
  messages are unchanged: they store required fields bare and their
  reflection still reports `has() == false` for a required field at its
  default value. Messages without required fields are byte-identical to
  before. `MessageFieldView::is_set` / `is_unset` are now `const fn`.

- **`type_name_prefix` option** (#46). `buffa_build::Config::type_name_prefix("Rpc")`
  (also `CodeGenConfig::type_name_prefix` and `protoc-gen-buffa`'s
  `type_name_prefix=` option) prepends a prefix to every generated message
  struct and enum type name — `message User {}` generates `struct RpcUser`,
  with views (`RpcUserView`), cross-references, and re-exports following.
  Module names, oneof enums, `extern_path`-mapped types (including
  well-known types), and the wire/JSON format are unaffected. The prefix
  must be PascalCase (an ASCII uppercase letter followed by ASCII letters
  and digits); anything else is rejected at generation time.

### Changed

- **MSRV lowered from 1.87 to 1.75**, and the
  [README MSRV policy](README.md#minimum-supported-rust-version) revised:
  `rust-version` now declares the lowest toolchain the released code actually
  compiles on (verified in CI), with bumps capped at roughly twelve months
  behind stable. The 1.75 floor is set by return-position `impl Trait` in
  traits, used by `MapStorage::storage_iter`. Reaching it required only
  mechanical respellings of newer stdlib conveniences — `Option::is_none_or`,
  `i32::cast_unsigned`, `f64::abs` in `const fn` — and
  gating the six `#[diagnostic::on_unimplemented]` hints behind
  `rustversion::attr(since(1.78), …)` so they remain active on modern
  toolchains. Adds `rustversion` as a dependency of `buffa` and
  `buffa-descriptor`.
- `MapValueDecode::merge` now returns `Result<MapValueDecodeStatus, _>`
  instead of `Result<(), _>`, and a new `merge_entry_with_unknowns` carries
  the closed-enum-map preservation path. The trait is sealed, so downstream
  implementations are unaffected; direct callers of `merge` (rare) must
  handle the new return value. (#218)

- `SizeCache` no longer zeroes its inline slot array on construction. A fresh
  cache is built for every `encode`/`compute_size`, and because it is passed by
  `&mut` to an out-of-line `compute_size` the compiler cannot elide the unused
  tail, so the previous `[0u32; N]` initializer emitted `N/4` SSE stores on
  every encode (confirmed by disassembly). The inline storage is now
  `[MaybeUninit<u32>; N]`, written only for the slots actually used; a slot is
  always written by `reserve` before `len` advances past it and read only at
  indices `< len`, so the single `assume_init` in `consume_next` is sound. This
  invariant is private to the `size_cache` module (no external code can break
  it — worst case is a panic, never UB) and is checked mechanically in CI by a
  Miri job over the `size_cache` tests. No API or wire-format change. (#223)

- **Default `map<K,V>` hasher is now `foldhash::fast::RandomState`** on `std`
  builds (previously `std::hash::RandomState` / SipHash-1-3). The container
  remains `std::collections::HashMap`; only the `S` type parameter changes.
  This brings the `std` build in line with `no_std` (which already used
  `foldhash` via `hashbrown`'s default) and matches the hasher class used by
  Google's `protobuf-v4` (upb / Wyhash). On the LogRecord benchmark — a
  string-and-map-heavy shape — this is roughly a 12% owned-decode speedup.
  `foldhash::fast` is per-instance seeded (from ASLR addresses and process
  start time, not a CSPRNG) and does not advertise HashDoS resistance; treat
  the default as not hardened against adversarial hash flooding. Consumers
  decoding `map` fields with attacker-controlled keys who need a hardened
  bound can select `MapRepr::BTreeMap` (no hashing) or supply a SipHash-backed
  map via `MapRepr::Custom`. The `MapStorage` and
  `ReflectMap` impls are now generic over the hasher `S`, so a custom-hasher
  `std::collections::HashMap` works without a newtype. **Migration:** the
  concrete map field type changes, so code that names
  `std::collections::HashMap<K,V>` (default `S`) for a generated field no
  longer type-checks — use the `buffa::Map<K,V>` alias instead. Construct
  empty maps with `buffa::Map::default()` (`HashMap::new()` /
  `HashMap::with_capacity()` are unavailable on `std` builds because they are
  pinned to std's default hasher; use `default()` on both `std` and `no_std`
  for portability). Array-literal construction via `Map::from([...])` /
  `.into()` is likewise unavailable; use `[...].into_iter().collect()`.
  `buffa::Map` and `buffa::foldhash` are now re-exported at the crate root.

- Generated decode arms (owned merge, view decode, lazy record arms,
  map-entry loops) emit a single `::buffa::encoding::check_wire_type` call
  instead of a seven-line inline wire-type guard (~1,100 sites across a
  generated corpus). Error payloads are byte-identical; the `#[cold]`
  out-of-line error constructor moves construction off the hot decode
  path. Regenerate checked-in code to pick up the shrink. (#193)

- Owned map fields encode/decode through the new `buffa::map_codec` module
  (zero-sized per-proto-type codecs plus generic field helpers) instead of
  ~40-50 inline generated lines per map field. Wire output, decode-limit
  semantics, and the fixed-width sizing fast path are unchanged; everything
  monomorphizes to the previous code. (#194)

- Generated `write_to` bodies use new fused `put_*_field` runtime writers
  (one call per field arm) instead of separate tag-encode + payload-encode
  pairs (~870 sites); owned and view impls share them. Wire output is
  byte-identical. (#195)

- `DefaultInstance` / `DefaultViewInstance` / `ViewReborrow` impls are
  emitted via new public runtime macros (`impl_default_instance!`,
  `impl_default_view_instance!`, `impl_view_reborrow!`) instead of being
  expanded per generated type (~290 sites); hand-written message and view
  types can reuse them. No behavioural change. (#196)

- Generated JSON `Serialize` impls use new internal (`#[doc(hidden)]`)
  `buffa::json_helpers` adapter newtypes (`ProtoJson`, `BytesJson`,
  `MapKeyJson`, sequence variants, ...) instead of ~65 per-site local `_W*`
  wrapper structs. JSON output is unchanged. (#197)
- **Breaking:** the decode-path `Message` trait methods (`merge`,
  `merge_field`, `merge_to_limit`, `merge_group`, `merge_length_delimited`),
  `encoding::decode_unknown_field`, and `message_set::merge_item` now take a
  `DecodeContext<'_>` — carrying the remaining recursion depth and the
  shared unknown-field allowance — in place of the bare `depth: u32`. Code
  generated with earlier releases must be regenerated. Callers of the
  convenience methods (`decode`, `decode_from_slice`, `merge_from_slice`,
  `DecodeOptions`) are unaffected.

- **Breaking:** `MessageView` gains a required `merge_view_field` method,
  and the per-view decode tag loop is now a provided trait method
  (`merge_into_view`), mirroring the owned side's `Message::merge` /
  `merge_field` split. Generated views supply only the field match —
  regenerate code from earlier releases. Hand-written `MessageView` impls
  must add `merge_view_field`; the trait docs include the canonical shape,
  the unknown-field-preserving arm, and the `decode_view` →
  `decode_view_ctx` wiring. Sub-message arms call the new provided
  `decode_view_ctx` / `merge_into_view` instead of the removed inherent
  `_decode_ctx` / `_merge_into_view` helpers. (#198)

### Fixed

- **`DecodeOptions::decode_reader` no longer overflows when
  `max_message_size` is `usize::MAX`.** The internal `read_limited` helper
  computed `max_message_size as u64 + 1` to read one sentinel byte past the
  limit; on 64-bit targets this overflowed — a debug panic, or in release a
  wrap to zero that silently decoded an empty default message. The addition
  now saturates, so `usize::MAX` correctly means an unbounded read. 32-bit
  targets and finite limits are unaffected. (#219)

- **Closed-enum map values now preserve unknown entries correctly.** For
  proto2 `map<K, ClosedEnum>` fields, an unknown enum value now prevents the
  map entry from being inserted and routes the whole original map-entry record
  to unknown fields. This fixes the previous default-valued entry synthesis
  (`key -> E::default()`) and applies to owned and view decode paths.
  Regenerate code with the matching `buffa-codegen` to get preservation;
  with an older codegen, runtime-only upgrades change unknown closed-enum
  map entries from default-insert to drop. (#218)

- **`DecodeOptions::decode_length_delimited_reader` no longer allocates the
  wire-declared length up front.** The method previously allocated a zeroed
  buffer of the declared length before reading, so a source that declared a
  large length (up to `max_message_size`, 2 GiB by default) but delivered
  few or no bytes still forced the full allocation. The buffer now grows
  incrementally as bytes are actually delivered (initial capacity capped at
  64 KiB), so peak allocation tracks delivered data. Truncated streams
  report `UnexpectedEof` exactly as before; behavior for well-formed
  streams is unchanged.

## [0.7.1] - 2026-06-10

This release is a patch bump under the
[Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html):
everything below is additive or a fix, with no breaking changes and no MSRV
change. The new codegen capabilities are opt-in (`unbox_oneof`) or gated on a
proto option (`debug_redact`); the packed view pre-allocation applies to all
regenerated code but is behaviorally invisible — a pure performance hint. Code
regenerated with 0.7.1 calls the new (hidden) `RepeatedView::reserve` hook, so
pair regenerated code with a buffa 0.7.1 runtime — any caret `0.7` requirement
resolves there automatically.

### Added

- **`unbox_oneof` opt-out for `Box`ed message oneof variants** (#126).
  `Config::unbox_oneof_in(&[paths])` stores the matching message-typed oneof
  variants inline in the owned enum instead of behind `Box<T>`, removing an
  allocation per construction; `Config::unbox_oneof()` is the blanket form.
  Recursive variants cannot be inlined: a rule naming one *exactly* is
  rejected at codegen time, while broader prefix rules (including the
  blanket) silently keep recursive variants boxed and inline the rest. View
  oneof variants are unaffected and stay boxed. Enums with an inline message
  variant carry `#[allow(clippy::large_enum_variant)]`. Contributed by
  @sam-shridhar1950f.

- **`[debug_redact = true]` is honored in generated `Debug` impls.** Fields
  carrying the standard `debug_redact` field option print `[REDACTED]` instead
  of their value in the owned message's `Debug` impl, and oneof enums, view
  structs, and view-oneof enums containing such fields swap their
  `#[derive(Debug)]` for a generated impl that redacts those fields/variants.
  Output for messages without the annotation is unchanged. Note this covers
  `Debug` formatting only — text-format and JSON serialization are
  intentionally unaffected. A view struct containing a redacted field now
  lists proto fields only in its `Debug` output (matching owned messages), so
  `__buffa_unknown_fields` / phantom internals no longer appear there.
  The reflective `DynamicMessage` `Debug` impl honors the option as well,
  printing `[REDACTED]` in place of the value of any field whose descriptor
  carries it.

- **Packed repeated view decoders pre-allocate `RepeatedView` capacity.**
  Generated view decode arms for packed repeated scalar / enum fields now
  call `RepeatedView::reserve(_)` before the decode loop, matching the
  existing pre-allocation hint on the owned decode path. Fixed-width kinds
  (`fixed32`, `sfixed32`, `float`, `fixed64`, `sfixed64`, `double`) reserve
  the exact element count; varint kinds (`int32`/`64`, `uint32`/`64`,
  `sint32`/`64`, `bool`, `enum`) reserve `payload.len()` as a safe upper
  bound (every wire varint is ≥ 1 byte). The hidden `RepeatedView::reserve`
  hook is also new but `#[doc(hidden)]`. This trims allocator pressure on
  workloads that decode many small packed repeated fields (MVT-style
  payloads), reported in #171.

### Changed

- **`TimestampError::Overflow`'s `Display` message generalized.** It now
  reads "timestamp is out of range for the target type" instead of naming
  `SystemTime`, since the same error is returned by the new
  `Timestamp` → `chrono::DateTime<Utc>` conversion. Code matching on the
  enum variant is unaffected.

- **`HasMessageView` carries a `#[diagnostic::on_unimplemented]` hint.** When a
  type is used where the generated view family is required but its crate was
  generated without one (buffa older than 0.7.0, or views disabled) or has it
  behind a disabled feature, the compile error now explains the cause and how
  to fix it — regenerate the defining crate with buffa ≥ 0.7.0 and views
  enabled (`generate_views(true)` / `views=true`), or enable the crate's views
  feature — instead of only naming the missing trait bound. Downstream
  consumers such as connect-rust rely on this trait for their request
  wrappers, so the notes land directly in the consumer's build output.

### Fixed

- **Mixed-mode reflection degrades at the boundary as designed** (#179). A
  vtable-mode message embedding owned message types generated in bridge mode
  (another crate or compilation) now reflects them as owned `DynamicMessage`
  snapshots at the boundary instead of failing to compile: vtable accessors
  for message-typed fields route through the field type's own
  `Reflectable::reflect()`, and bridge mode now also emits `ReflectElement`
  so `repeated` / `map` fields degrade too. View reflection still requires
  vtable-grade types throughout — that limitation is now documented. (Code
  matching exhaustively on `ReflectCow` may now observe `Owned` for
  bridge-grade message fields; all-vtable builds are unchanged.)
- **Missing-reflection compile errors point at the fix** (#179).
  `ReflectMessage`, `Reflectable`, and `ReflectElement` carry
  `#[diagnostic::on_unimplemented]` hints, so building vtable codegen against
  an extern-path crate without its reflection feature (e.g. `buffa-types`
  without `reflect`) names the missing cargo feature instead of emitting a
  bare unsatisfied-trait error. The `reflect_mode` docs state the
  requirement.
- The owned message `Debug` impl now labels keyword-named fields without the
  raw-identifier prefix (`type` instead of `r#type`), matching what
  `#[derive(Debug)]` prints and what the view `Debug` impl emits.
- Octal escapes above `\377` (255) in a proto2 bytes field's `default_value`
  are now rejected with a codegen error instead of silently wrapping to a
  wrong byte (`\400` previously decoded to `0x00`), matching protobuf C++'s
  `UnescapeCEscapeString` behavior (#164). Such escapes never appear in
  protoc-emitted descriptors, so this only affects hand-built or corrupted
  `FileDescriptorSet` input.
- Hex escapes in a proto2 bytes field's `default_value` now consume the full
  run of hex digits and reject accumulated values above `\xff` (255) with a
  codegen error, matching protobuf C++'s `UnescapeCEscapeString` behavior
  (#173). Previously exactly two digits were read, so `\xfff` decoded to the
  byte `0xFF` followed by a literal `f` instead of erroring, and a
  single-digit escape such as `\x1` at end of input was wrongly rejected. As
  with the octal fix, such escapes never appear in protoc-emitted
  descriptors, so this only affects hand-built or corrupted
  `FileDescriptorSet` input.

## [0.7.0] - 2026-05-28

This release is a minor bump under the
[Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html).
The breaking changes are the removal of `OwnedView<V>`'s `Deref` impl and the
extension of `use_bytes_type()` to `map<K, bytes>` values (both under
*Changed* below), plus an MSRV raise from 1.85 to 1.87. Consumers with
checked-in generated code should regenerate with the 0.7.0 toolchain to pick
up the new `FooOwnedView` wrappers, `HasMessageView` impls, and
`UpperCamelCase` enum aliases — all additive.

### Added

- **Runtime reflection: `DescriptorPool` and `DynamicMessage`.**
  `buffa-descriptor` gains a `reflect` feature with a descriptor-driven
  reflection runtime. `DescriptorPool::decode` builds linked,
  feature-resolved descriptors (`MessageDescriptor`, `FieldDescriptor`,
  `EnumDescriptor`, `ServiceDescriptor`, …) from a `FileDescriptorSet`,
  treating the input as untrusted (malformed sets return `PoolError` rather
  than panicking) and retaining the raw `FileDescriptorProto`s plus a symbol
  index (`file_by_name`, `file_containing_symbol`) for gRPC server
  reflection. `DynamicMessage` decodes and encodes any message by descriptor
  — no generated types required — with unknown-field preservation, in-place
  mutation (`field_mut` / `field_by_number_mut`), `Any` pack/unpack,
  extension fields, and custom-option access (`options()` on every linked
  descriptor, `DynamicMessage::from_options`). With the `json` feature it
  also speaks proto3 canonical JSON (`Serialize`, `DynamicMessage::from_json`,
  lenient `from_json_ignoring_unknown`, duplicate-key rejection). The
  dyn-safe `ReflectMessage` / `ReflectMessageMut` traits and the
  `ReflectCow` / `Value` / `ValueRef` types are the surface generated types
  plug into (see vtable mode below). Generated code opts in with
  `buffa_build::Config::generate_reflection(true)` (plugin:
  `reflection=true`), which embeds the package's `FileDescriptorSet` and
  exposes a lazily-built pool as `pkg::descriptor_pool()`. The reflection
  codec passes the protobuf conformance suite through a dedicated
  `DynamicMessage`-only runner mode.
- **Vtable reflection mode.** Generated types now implement
  `buffa_descriptor::reflect::ReflectMessage` directly — on both the owned
  structs and the zero-copy view types — so `foo.reflect()` borrows `foo` in
  place (`ReflectCow::Borrowed`) with no encode/decode round-trip and no
  per-field allocation. This is the path a CEL evaluator, transcoding gateway, or
  generic interceptor takes to read fields by descriptor; reflecting a decoded
  view runs several times faster than the previous bridge round-trip. Select the
  mode with the new `buffa_build::ReflectMode` enum:

  ```rust
  buffa_build::Config::new()
      .reflect_mode(buffa_build::ReflectMode::VTable) // or ::Bridge / ::Off
      .compile()?;
  ```

  The `protoc-gen-buffa` equivalent is `reflect_mode=off|bridge|vtable`. Vtable
  mode does not require view generation: with views off, only the owned
  `ReflectMessage` is emitted. `generate_reflection(true)` selects vtable mode;
  `reflect_mode(ReflectMode::Bridge)` opts into the smaller round-trip
  implementation (one `DynamicMessage` encode/decode per `reflect()` call)
  instead of one `impl ReflectMessage` per generated type.
- **`buffa-types` `reflect` feature.** Well-known types (`Timestamp`,
  `Duration`, `Struct`/`Value`, `Any`, wrappers, …) now implement
  `ReflectMessage`, so messages that embed WKTs reflect end to end.
- **Pluggable owned types for `string` and `bytes` fields (#127, #156, #206).**
  Generated `string` / `bytes` fields can use a custom in-memory type chosen at
  code-generation time, with no change to the wire format. `buffa_build::Config`
  gains `string_type(StringRepr)` / `string_type_in` and the convenience
  `string_type_custom("::path::To::Type")` / `string_type_custom_in`, where
  `buffa_build::StringRepr` is `{ String (default), Custom(path) }`. The new
  `bytes` counterpart is `bytes_type(BytesRepr)` / `bytes_type_in` /
  `bytes_type_custom` / `bytes_type_custom_in`, where `BytesRepr` is
  `{ Vec (default), Bytes, Custom(path) }`; `use_bytes_type` / `use_bytes_type_in`
  remain as aliases for `BytesRepr::Bytes`. Rules accumulate and the last match
  wins. Only the owned struct field type changes — view types still borrow
  `&str` / `&[u8]`, and `map` keys/values keep their default type.

  The chosen type must implement the marker traits `buffa::ProtoString` /
  `buffa::ProtoBytes`. Each requires a `from_wire(WirePayload<'_>) -> Result<Self,
  DecodeError>` constructor (alongside the supertraits
  `Clone + PartialEq + Default + Debug + Send + Sync`, `Deref` to `str` / `[u8]`,
  `AsRef`, and `From<String>` / `From<Vec<u8>>`). `from_wire` lets each
  representation own validation and borrow-vs-own:
  [`WirePayload`](https://docs.rs/buffa) is `Borrowed(&[u8])` (zero-copy) or
  `Owned(Bytes)`, with `as_slice()` and `into_bytes()`. A representation that
  enforces extra invariants can reject a value from `from_wire` with the new
  `DecodeError::Custom(&'static str)` variant. buffa ships the built-in
  impls for `String`, `Vec<u8>`, and `bytes::Bytes`; a foreign type (e.g.
  `smol_str::SmolStr`) is wrapped in a local newtype that implements the trait —
  the new **`buffa-smolstr`** crate is the template (an inline, allocation-free
  `from_wire`). A custom type needs no native `Arbitrary` impl (a generic builder
  handles it). A custom type used as the element of a **`repeated`** field — or a
  custom `bytes` type as a **`map<K, bytes>`** value — must be **crate-local**:
  codegen emits `ReflectElement` (vtable) and, for bytes, base64 `ProtoElemJson`
  (JSON) impls for it, which the orphan rule forbids for a foreign type. A custom
  `bytes` map value is honored just like the built-in `Bytes` (only the
  `map<bytes, bytes>` carve-out keeps `Vec<u8>`). Singular / optional / oneof uses
  work with the newtype without the crate-local restriction.

  Why `from_wire` rather than a blanket `From`-based impl: the decode path was
  first built as a blanket impl over `From<String>` / `From<Vec<u8>>` to learn the
  tradeoff, but that path *always* pays `decode_string`'s allocate-and-copy and a
  transient heap allocation even for a short string that an inline type
  (`smol_str`) could store without touching the heap. `from_wire` hands the
  representation the raw payload so it can inline, validate lazily, or take
  ownership zero-copy — so it never disadvantages a custom type.

  **BREAKING (unreleased only):** the earlier unreleased `string_type` shapes are
  removed — both the `StringRepr::{SmolStr, EcoString, CompactString}` presets
  (with the `buffa` / `buffa-descriptor` `smol_str` / `ecow` / `compact_str`
  features and `::buffa::{smol_str, …}` re-exports) and the later blanket
  `From`-based `ProtoString` / `ProtoBytes`. Pointing `string_type_custom` at a
  foreign type directly no longer compiles; use `buffa-smolstr` (or a local
  newtype implementing `from_wire`). Default output (`String` / `Vec<u8>`) is
  byte-for-byte unchanged. `buffa-build` / `buffa-codegen` only — there is no
  `protoc-gen-buffa` plugin option yet.
- **Generated `FooOwnedView` wrapper types.** When views are generated, each
  message now also gets a `FooOwnedView` — re-exported at the package root
  next to `Foo` and `FooView` (canonical path `__buffa::view::FooOwnedView`):
  a self-contained `'static` handle wrapping `OwnedView<FooView<'static>>`
  with one accessor method per field (`owned.name()`, `owned.id()`, …). Every
  accessor borrows from `&self`, so field data can never outlive the
  underlying buffer, and the handle stays `Send + Sync` for async handlers and
  spawned tasks. The wrapper forwards `decode` / `decode_with_options` /
  `from_owned` / `to_owned_message` / `bytes` / `into_bytes`, exposes the full
  view via `view()`, converts to and from the raw `OwnedView`, and serializes
  to protobuf JSON when `generate_json` is enabled. A field or oneof whose
  name collides with one of the wrapper's reserved method names keeps working
  through `view()`; its accessor is skipped with a build warning
  (`CodeGenWarning::OwnedViewAccessorSuppressed`).
- **`HasMessageView` view-family trait.** Generated code now implements
  `buffa::HasMessageView` for every message (when views are generated),
  linking the owned type to its view types: `Foo::View<'a>` = `FooView<'a>`
  and `Foo::ViewHandle` = `FooOwnedView`, with a provided
  `decode_view_handle()` helper. The generated wrapper additionally
  implements `From<OwnedView<FooView<'static>>>` and
  `AsRef<OwnedView<FooView<'static>>>`, so code that is generic over an owned
  message can decode, reborrow, and convert without naming the concrete
  types — the hook an RPC framework needs to accept `M` and work with
  `M::View<'_>` and `M::ViewHandle` generically.
- **Idiomatic `UpperCamelCase` enum value aliases (#13).** Generated enums
  now also carry associated `const` aliases with the enum-name prefix
  stripped and the value converted to `UpperCamelCase` —
  `RuleLevel::RULE_LEVEL_HIGH` is reachable as `RuleLevel::High` — usable in
  expressions and in pattern position with exhaustiveness preserved. The
  `SHOUTY_SNAKE_CASE` variants remain the definitive variants and `Debug`
  output is unchanged, so the aliases are purely additive; consumers with
  checked-in generated code will see new consts on regeneration. If two
  values of an enum would collide after conversion, aliases are suppressed
  for that enum as a whole and reported through the new `CodeGenWarning`
  diagnostics (`buffa_codegen::generate_with_diagnostics`). Default on; opt
  out per compilation unit with
  `buffa_build::Config::idiomatic_enum_aliases(false)` /
  `CodeGenConfig::idiomatic_enum_aliases = false`.

### Changed

- **`OwnedView<V>` no longer implements `Deref<Target = V>`.** **Breaking.**
  The `Deref` impl exposed the inner view as `FooView<'static>`, so borrowed
  fields appeared `'static` to the compiler and could be held past the point
  where the `OwnedView` (and the buffer they point into) was dropped — safe
  code could end up reading freed memory. In practice this required the
  calling application to deliberately store a field reference beyond the
  handle's lifetime, so the practical exposure is limited, but the API should
  not allow it at all. Field access now goes through `reborrow()` (one extra
  call per scope: `let person = owned.reborrow(); person.name`) or, more
  conveniently, the new generated `FooOwnedView` accessor methods, both of
  which tie every borrow to the handle. Serializing the handle directly
  (`serde_json::to_string(&owned_view)`) is unaffected.
- **`use_bytes_type()` / `use_bytes_type_in(...)` now applies to `map<K, bytes>`
  values (#76).** Previously map values were always `Vec<u8>` regardless of
  config — the only `bytes`-context not covered. They now match the type used
  for singular / optional / repeated / oneof bytes fields under the same rule
  (`bytes::Bytes` when configured), so `view → owned` conversion of map values
  participates in the `to_owned_from_source` zero-copy `slice_ref` path just
  like the other shapes. **Breaking** for code that already enabled
  `use_bytes_type()` on a proto containing `map<K, bytes>`: at construction
  sites, rewrite map-value construction from `Vec<u8>` to `bytes::Bytes`
  (`b"v".to_vec()` → `bytes::Bytes::from_static(b"v")` for literals,
  `bytes::Bytes::from(v)` for an owned `Vec<u8>`, or
  `bytes::Bytes::copy_from_slice(s)` for a non-`'static` borrow). At read sites,
  `bytes::Bytes` has no inherent `as_slice`, so any `as_slice()` on the value
  needs replacing — e.g. `map.get(k).map(Vec::as_slice)` becomes
  `map.get(k).map(|b| &b[..])`. One carve-out: an effective `map<bytes, bytes>`
  keeps `Vec<u8>` values; this requires `strict_utf8_mapping(true)` *and* a
  `map<string, bytes>` whose key carries `[features.utf8_validation = NONE]`
  (`strict_utf8_mapping` alone keeps a plain `map<string, bytes>` value as
  `Bytes`). See the `use_bytes_type_in` docs. Under `generate_arbitrary`,
  affected map fields use the new `__private::arbitrary_bytes_map<K>` shim
  (`K: Arbitrary + Eq + Hash` — every proto map-key type satisfies this).
- **MSRV raised from 1.85 to 1.87**, following the
  [README's MSRV policy](README.md#minimum-supported-rust-version) of
  tracking roughly twelve months behind the latest stable release,
  re-evaluated each time a release is cut. While buffa is pre-1.0, an MSRV
  bump rides a minor (0.x) release.

### Fixed

- **Module redefinition error when a message and a sub-package share a name
  (#135).** A message with nested types emits a `snake_case(MessageName)`
  submodule, which collided with a sibling sub-package of the same name
  (protobuf is case-sensitive — `message Oof` and `package foo.oof` legally
  coexist — but both mapped to `mod oof`, producing an E0428). Codegen now
  deconflicts the **nested-types module** by appending `_` (e.g. `oof_`; more
  underscores if several modules collide in the same scope — see DESIGN.md),
  leaving the message struct (`foo::Oof`) and the sub-package module
  (`foo::oof`) at their natural names. This only triggers on a collision that
  previously failed to compile, so existing output is unchanged. Two caveats:
  (1) if you *add* a sub-package whose name collides with an existing message's
  nested-types module, paths to those nested types move from `foo::oof::…` to
  `foo::oof_::…`; (2) both packages must be generated in the same
  `buffa_build::Config::compile()` call — deconfliction cannot span separate
  compilations, since each only sees its own descriptor set.

- **Per-type `extern_path` mappings were silently ignored (#111).** An
  `extern_path` entry naming a single type FQN (e.g.
  `.extern_path(".google.protobuf.Timestamp", "::my_types::Timestamp")`, the
  prost/tonic idiom) parsed but never matched, because resolution only
  considered package prefixes. Type references now resolve per-type: an exact
  type-FQN entry wins over the internal `descriptor.proto` routing, which wins
  over the longest matching package prefix, which wins over local generation.
  Nested types inherit an enclosing message's override, resolving to the
  override's parent module plus the usual `snake_case(MessageName)`
  nested-types module. Note that entries which previously had no effect now
  take effect: a type-FQN entry (including a typo'd one) that was a silent
  no-op before will now change the generated reference, and a wrong target
  surfaces as a compile error in the generated code.

## [0.6.0] - 2026-05-15

### Added

- **Generated message structs now include `with_<field>(value) -> Self`
  builder-style setter methods for every explicit-presence field** (proto3
  `optional`, proto2 `optional`, and editions fields with
  `field_presence = EXPLICIT`). This allows chained construction without
  `Some(...)` wrapping:

  ```rust
  let req = GetSecretRequest::default()
      .with_name("alice")
      .with_timeout_ms(30_000)
      .with_enabled(true);
  ```

  String fields accept `impl Into<String>` (`&str` works directly); bytes
  fields accept `impl Into<Vec<u8>>` or `impl Into<bytes::Bytes>` (byte
  array literals like `b"data"` work directly); enum fields accept
  `impl Into<EnumValue<E>>` (bare variant works directly, no
  `EnumValue::Known(...)` wrapper needed); plain scalars take the bare
  type to keep integer-literal inference unambiguous. Message fields
  (`MessageField<T>`), repeated fields, map fields, oneof variants,
  proto2 `required` fields, and implicit-presence fields are unaffected.
  To clear a field, assign `None` directly. Setters are pure inherent
  methods with no runtime dependency, so they're emitted unconditionally
  regardless of `gate_impls_on_crate_features`. Disable per compilation
  unit with `CodeGenConfig::generate_with_setters = false`,
  `buffa_build::Config::generate_with_setters(false)`, or the
  `with_setters=false` plugin opt. **Consumers with checked-in generated
  code** will see new methods on regen.
  ([#30](https://github.com/anthropics/buffa/issues/30),
  [#93](https://github.com/anthropics/buffa/pull/93), by @tejas-dharani)

- **`buffa::MessageName` trait exposes a generated message's protobuf
  identifiers as compile-time `&'static str` constants.** Codegen emits
  `impl MessageName for #Msg` (and `for #MsgView<'a>`) with four consts:
  `PACKAGE` (`"my.pkg"`, empty for the unnamed root package), `NAME`
  (`"Outer.Inner"` — unqualified, with `.` between nesting levels),
  `FULL_NAME` (`"my.pkg.Outer.Inner"`), and `TYPE_URL`
  (`"type.googleapis.com/my.pkg.Outer.Inner"` — the
  `google.protobuf.Any.type_url` form). All four are computed at codegen
  time as string literals, so there's no runtime allocation or
  concatenation — unlike `prost::Name`, whose `full_name()` and
  `type_url()` are runtime `format!` calls. `PACKAGE` and `NAME` are
  separate consts because the dotted `FULL_NAME` cannot be split
  unambiguously (`foo.Bar.Baz` could be package `foo.Bar` + message `Baz`
  or package `foo` + nested `Bar.Baz`).

  The trait has no supertrait — it doesn't reach into the wire codec —
  so view types implement it too: a generic event-sourcing registry can
  bound on `T: MessageName` and dispatch zero-copy views and owned
  messages identically. Useful for type-erased registries, logging, and
  any code that needs the protobuf name without the descriptor machinery.
  The inherent `Foo::TYPE_URL` const generated since 0.4.0 is unchanged
  and equal to `<Foo as MessageName>::TYPE_URL`; for messages that also
  implement `ExtensionSet`, `FULL_NAME` is equal to
  `ExtensionSet::PROTO_FQN` (all derive from the same codegen source).
  `MessageName` is **not** object-safe (associated `const` only) — use it
  as a bound, not `dyn MessageName`. Migrating from `prost::Name`: rename
  the bound and replace runtime `M::full_name()` / `M::type_url()` calls
  with the consts. ([#108](https://github.com/anthropics/buffa/pull/108),
  by @yordis)

- **`buf.build/anthropics/buffa` is published to the public Buf Schema
  Registry.** `buf generate` can now reference `protoc-gen-buffa` as a
  `remote:` plugin with no local install: `remote: buf.build/anthropics/buffa`
  with `opt: [file_per_package=true]` and a small hand-written `pub mod`
  tree, or paired with a locally-installed `protoc-gen-buffa-packaging`
  for a generated `mod.rs`. The README quick-start, `docs/guide.md`
  ["Using buf"](docs/guide.md#using-buf) section, and a new
  [`examples/bsr-quickstart/`](examples/bsr-quickstart/) project document
  the workflow. The stale in-repo `protoc-gen-buffa/buf.plugin.yaml`
  metadata file is removed — the canonical plugin definition lives in
  [bufbuild/plugins](https://github.com/bufbuild/plugins).

- **`buffa-codegen`: `CodeGenConfig::gate_impls_on_crate_features`.**
  When `true`, generated impls controlled by `generate_json`,
  `generate_views`, and `generate_text` are wrapped in
  `#[cfg(feature = "json" | "views" | "text")]` (or `#[cfg_attr(...)]` for
  derives and field attributes) instead of being emitted unconditionally.
  The consuming crate defines matching Cargo features and enables the
  corresponding runtime support (`buffa/json`, `buffa/text`, `serde`, …)
  behind them. The `generate_*` flags still control *whether* an impl kind
  is emitted; the new flag only controls *how*. Default `false` — no
  change to existing output. This is the codegen mechanism that will let
  `buffa-descriptor` and `buffa-types` ship every impl while keeping the
  codegen toolchain (`buffa-codegen` / `buffa-build` / `protoc-gen-buffa`)
  lean — it depends on them with `default-features = false`. Tracked in
  [#113](https://github.com/anthropics/buffa/issues/113). Exposed as
  `buffa_build::Config::gate_impls_on_crate_features(bool)` and the
  `gate_impls=true` plugin opt, both default-off.

- **`buffa-descriptor`: regenerated with views, JSON, text, and arbitrary
  impls behind crate features.** `descriptor.proto` and
  `compiler/plugin.proto` types now ship the full impl surface — gated on
  `views`, `json`, `text`, and `arbitrary` Cargo features so the codegen
  toolchain (`buffa-codegen` / `buffa-build` / `protoc-gen-buffa`) can
  depend on `buffa-descriptor` with `default-features = false` and stay
  free of `serde` / `serde_json` / `base64` / `arbitrary`. **Consumers
  whose protos reference a `descriptor.proto` type as a field (most
  commonly anything depending on `buf/validate/validate.proto`, or
  `buf.registry.module.v1` / `buf.alpha.image.v1` which embed
  `FileDescriptorSet` / `FileDescriptorProto`) must enable the
  `buffa-descriptor` features matching their codegen modes** —
  `views = ["buffa-descriptor/views"]`, `json = ["buffa-descriptor/json"]`,
  etc., or just `buffa-descriptor = { ..., features = ["views", "json"] }`.
  This closes [#113](https://github.com/anthropics/buffa/issues/113): the
  full `bufbuild/registry` and `bufbuild/buf` modules now generate and
  compile cleanly with `views=true` + `json=true`.

  **Migration:** if your `Cargo.toml` already declares `buffa-descriptor`
  as a dependency, add the features matching your codegen config:

  ```toml
  # build.rs uses .generate_views(true).generate_json(true)
  buffa-descriptor = { version = "0.6", features = ["views", "json"] }
  ```

  If you don't declare `buffa-descriptor` directly, the failure mode is a
  missing-impl error at the embedding type's serde / view call site (e.g.
  `the trait bound FileDescriptorSet: serde::Deserialize is not
  satisfied`); add `buffa-descriptor` with the right features.

  The `buffa_descriptor::generated` module tree now nests
  `google.protobuf.compiler` inside `google.protobuf` to mirror the proto
  package hierarchy (so cross-package `super::*` references in the view
  code resolve); the previous sibling-style
  `buffa_descriptor::generated::compiler` and
  `buffa_descriptor::generated::{FileDescriptorProto, GeneratedCodeInfo}`
  paths are preserved with `pub use` re-exports.

- `serde::Serialize` is now implemented for generated view types when `generate_json` is
  enabled, allowing zero-copy JSON serialization without `.to_owned_message()`.
  `OwnedView<V>` also gains a blanket `Serialize` impl so `serde_json::to_string(&owned_view)`
  works directly. Well-known type views (`TimestampView`, `DurationView`, `AnyView`, etc.)
  also implement `Serialize` (delegating to the owned form) when the `buffa-types/json`
  feature is enabled, so messages that nest WKT fields work out of the box. `MapView` gains
  `iter_unique()` and `len_unique()` helpers (last-write-wins deduplication) so map fields
  with duplicate wire keys serialize to a valid JSON object. The protobuf conformance suite
  gains a `BUFFA_VIEW_JSON=1` run that exercises view-side JSON output against the
  conformance reference assertions.
  **Known limitations:** (1) Extension fields are not included in view JSON output —
  serialize the owned form (`view.to_owned_message()`) to include extensions. (2) The view
  impl uses `serialize_map(None)`, which is fine for `serde_json` but will be rejected at
  runtime by length-prefixed formats like `bincode` or `postcard`; use the owned form for
  those serializers. ([#83](https://github.com/anthropics/buffa/issues/83))

### Fixed

- **`buffa` / `buffa-codegen`: `serde_json` re-exported from `buffa` for
  generated extension JSON deserialize.** Messages with `extensions N to M;`
  ranges and `json=true` codegen get a hand-written `Deserialize` impl that
  buffers `"[pkg.ext]"` JSON keys into a `serde_json::Value` before
  dispatching to `extension_registry::deserialize_extension_key`. The emitted
  path was a bare `::serde_json::Value`, which silently required every
  consumer of `json=true` codegen to declare `serde_json` directly in its own
  `Cargo.toml` — a footgun reported by Buf for `bufbuild_registry_*` SDKs
  generated against `buf/validate/validate.proto` (which has 21 extension
  ranges). `buffa` now re-exports `serde_json` (gated on the `json` feature,
  `#[doc(hidden)]`, matching the existing `bytes` re-export) and codegen
  emits `::buffa::serde_json::Value`, so consumers only need `buffa`,
  `buffa-types`, and `serde` (the latter for the `#[derive]` macro). No
  generated output exists for this path in the checked-in WKTs (none declare
  extension ranges), so no regen.

- **`buffa-codegen`: `descriptor.proto` types now resolve to
  `buffa-descriptor`, not `buffa-types`.** The auto-injected WKT
  extern_path `.google.protobuf` → `::buffa_types::google::protobuf`
  covers everything in the `google.protobuf` package, including
  `descriptor.proto` types — but `buffa-types` only ships the
  JSON-mappable WKTs. Any proto referencing a `descriptor.proto` type as
  a field — e.g. `buf/validate/validate.proto`, which has three `optional
  google.protobuf.FieldDescriptorProto.Type` fields — produced a
  generated path that doesn't exist:
  `::buffa_types::google::protobuf::field_descriptor_proto::Type`. An
  internal **file-level** extern resolution now routes
  `google/protobuf/descriptor.proto` to
  `::buffa_descriptor::generated::descriptor` and
  `google/protobuf/compiler/plugin.proto` to
  `::buffa_descriptor::generated::compiler`, taking priority over the
  package-level WKT mapping. Suppression mirrors the WKT mapping: a user
  `.google.protobuf` extern_path overrides it (preserving the long-standing
  behaviour that the override covers descriptor types too), and a file in
  `files_to_generate` resolves locally. **Consumers whose protos
  `import "google/protobuf/descriptor.proto"` and reference its types as
  fields must add `buffa-descriptor` to their `[dependencies]`** — the
  same way protos that reference WKTs require `buffa-types`. The
  user-facing `extern_path` API is unchanged (still package-prefix keyed).

- **`buffa`: closed-enum JSON helpers no longer require the enum to
  `impl Deserialize`.** `opt_closed_enum`, `repeated_closed_enum`, and
  `map_closed_enum` deserialized via `serde_json::from_value::<E>()`,
  which bound `E: DeserializeOwned`. That meant a closed-enum field whose
  enum type lives in an externally-generated crate built *without*
  `generate_json` — e.g. `google.protobuf.FieldDescriptorProto.Type` from
  `buffa-descriptor`, referenced by `buf/validate/validate.proto` — could
  not satisfy the bound and refused to compile under `json=true` codegen.
  The helpers now decode the buffered `serde_json::Value` directly via the
  `Enumeration` trait (`from_proto_name`, `from_i32`, default for `null`),
  which is the same dispatch the codegen-emitted `Deserialize` impl
  performs anyway. The `DeserializeOwned` bound is removed (a relaxation —
  non-breaking). Lenient mode (`ignore_unknown_enum_values`) is unchanged:
  any element that fails to decode — unknown variant, out-of-range
  integer, or wrong JSON type — is dropped from the container / leaves the
  optional unset, exactly as before. Additionally, that lenient filtering
  for closed-enum containers now works under `no_std`: the previous
  implementation needed the `std`-only scoped strict-mode override to
  surface a distinguishable error from the inner deserialize, but the new
  `Enumeration`-direct dispatch has no inner deserialize to override.

### Changed

- The workspace `[profile.release]` now sets `lto = true` and
  `codegen-units = 1`. This shrinks the prebuilt `protoc-gen-buffa` /
  `protoc-gen-buffa-packaging` release binaries by roughly 20% at the cost of
  ~2× clean release-build time. Cargo only honors profile sections from the
  top-level workspace, so library consumers of `buffa` / `buffa-build` do not
  inherit this — set `[profile.release]` in your own workspace (or
  `CARGO_PROFILE_RELEASE_LTO=true` for `cargo install`) to get the same
  benefit. ([#60](https://github.com/anthropics/buffa/issues/60))

- **`buffa-codegen`: empty ancillary content files and modules are no
  longer emitted.** A `.proto` with no oneofs / no extension declarations
  / `views=false` previously produced placeholder
  `<stem>.__oneof.rs` / `<stem>.__ext.rs` / `<stem>.__view.rs` /
  `<stem>.__view_oneof.rs` files containing only the `@generated` header,
  and the package stitcher unconditionally authored a
  `pub mod __buffa { pub mod oneof { ... } pub mod ext { ... } ... }`
  tree that `include!`d them. Codegen now omits an ancillary content file
  when it would be empty, the stitcher only `include!`s files that exist,
  and the `__buffa` wrapper (and each `view` / `oneof` / `ext` submodule
  inside it) is itself omitted when it would be empty — so a package with
  only owned messages emits no `__buffa` block at all. Eliminates pure
  noise in generated trees, editor file lists, search, and review diffs.
  **Consumers with checked-in generated code** will see file deletions
  and stitcher diffs on regeneration; remove orphaned empty files. The
  `__buffa::*` paths are an internal sentinel namespace (consumers reach
  for the natural-path re-exports added in 0.5.0), so no supported public
  surface changes — but a hand-written
  `use crate::pkg::__buffa::oneof::*;` for a package that has no oneofs
  would now fail to resolve (it was previously a no-op import of an
  empty module). ([#107](https://github.com/anthropics/buffa/pull/107))

## [0.5.2] - 2026-05-07

### Fixed

- **`buffa-codegen`: oneof `Serialize` match arms now use `Self::#variant`.**
  `generate_oneof_serialize` emits the manual JSON serde impl as
  `impl Serialize for #enum_ident { fn serialize(&self, …) { match self { … } } }`,
  where `Self` resolves to the oneof enum. The match arms used the
  fully-qualified `#enum_ident::#variant` form, which trips
  `clippy::use_self` in workspaces that opt it on — particularly visible
  under `connectrpc-build`, which doesn't carry an inner `#![allow(...)]`
  the way `protoc-gen-buffa-packaging` does, so the oneof companion file
  inherits the surrounding mod's lint set. The deserialize arms in
  `oneof_variant_deser_arm` remain qualified because they construct the
  oneof from inside the *message*'s `Deserialize` impl, where `Self` would
  be wrong. No behavioural change.
- **`buffa-codegen`: enum JSON deserialize errors use inlined format args.**
  The enum visitor's range-check and unknown-value error messages used
  positional `format!("enum value {} out of i32 range", v)` etc., which
  trip `clippy::uninlined_format_args` for the same reason as above (the
  enum impls live in the per-proto Owned content, outside the `__buffa`
  `#[allow(...)]` block). Now `format!("enum value {v} out of i32
  range")` etc. — semantically identical, lint-clean regardless of which
  module wrapper covers it.

## [0.5.1] - 2026-05-07

### Fixed

- **`buffa-codegen`: `ALLOW_LINTS` now includes `unused_qualifications`.**
  Cross-proto references within the same package are emitted through the
  canonical `super::super::__buffa::view::…` (and `…::oneof::…`) path even
  though the target lives in the same generated module. The bare name would
  resolve, but the canonical path is stable when a sibling proto defines a
  same-named natural-path re-export. Workspaces that opt
  `unused_qualifications = "warn"` and build with `-D warnings` were getting
  false positives from generated code; the lint is now in the package
  stitcher's `#[allow(...)]` block alongside `dead_code`, `unused_imports`,
  etc.

## [0.5.0] - 2026-05-05

This release is a minor bump under the
[Rust 0.x convention](https://doc.rust-lang.org/cargo/reference/semver.html):
the only API break is `#[non_exhaustive]` on `buffa_codegen::GeneratedFileKind`
(see *Changed* below), which affects downstream code generators only — it does
not change the runtime API. Everything else is additive.

**Consumers with checked-in generated code must regenerate** with the 0.5.0
toolchain before depending on the 0.5.0 runtime crates: generated code from
0.5.0's `buffa-codegen` references `ViewReborrow`, `decode_bytes_to_bytes`,
and `__private::arbitrary_bytes`, none of which exist in `buffa` 0.4.0.

### Breaking changes

- **`buffa_codegen::GeneratedFileKind` is now `#[non_exhaustive]`.** Match it
  with a wildcard arm — future kinds can then be added without a major
  version bump. Build integrations that compare with `==` (the common case,
  including connect-rust) are unaffected.

### Added

- `buffa_codegen::GeneratedFileKind::Companion` and `apply_companions` let
  downstream code generators (e.g. connect-rust) supply extra per-proto
  files that buffa wires into the per-package stitcher, instead of having
  to mislabel them as `GeneratedFileKind::Owned` and rely on filename
  matching. Companion files are `include!`d at package root alongside
  owned message types. ([#81](https://github.com/anthropics/buffa/issues/81))

- `OwnedView<V>` gains a `reborrow<'b>(&'b self) -> &'b V::Reborrowed<'b>` method
  that makes the internal `'static` lifetime visible as `'b` (the lifetime of the
  borrow), so view fields can be passed into functions or return types bounded by
  the `OwnedView`'s lifetime. Requires `V: ViewReborrow`, a safe trait whose
  `reborrow` method body is a covariance-checked subtype coercion; codegen emits
  the `impl` automatically for every generated view type. Hand-written view types
  opt in with `impl ViewReborrow for MyView<'static> { type Reborrowed<'b> =
  MyView<'b>; fn reborrow<'b>(this: &'b Self) -> &'b Self::Reborrowed<'b> { this } }`
  — the body fails to compile for invariant view types, so no `unsafe` is needed.
  ([#82](https://github.com/anthropics/buffa/issues/82))

- Codegen now emits "natural-path" `pub use` re-exports for ancillary types
  (views, oneof enums, view-of-oneof enums, file-level extension consts,
  `register_types`) at the module path you'd write first — `pkg::FooView`,
  `pkg::foo::Kind`, `pkg::foo::KindView`, etc. The canonical `__buffa::`
  paths are unchanged and remain what generated code and downstream codegen
  always reference; the re-exports are purely an ergonomic convenience and
  are silently skipped when the natural name is already taken by a real
  proto item or by another candidate re-export. Because of that skip rule,
  adding a proto type whose name shadows a re-export (e.g. `message FooView`
  next to `message Foo`) can silently rebind a natural path between releases
  — the canonical `__buffa::` path is always stable; use it directly when a
  natural import stops resolving (see `examples/conflicts` for one alias
  convention). ([#80](https://github.com/anthropics/buffa/issues/80))

- Doc comments in generated Rust code now resolve AIP-192 proto type cross-references
  (`[Book][google.example.v1.Book]`, `[Book][]`) to rustdoc intra-doc links.
  Only type-level refs are resolved; member refs such as `[Genre.GENRE_SCI_FI][]`
  fall back to escaped literals. Unknown or cross-crate references also fall back
  silently. ([#26](https://github.com/anthropics/buffa/issues/26))

- `protoc-gen-buffa` and `protoc-gen-buffa-packaging` now respond to
  `--version` / `-V` and `--help` / `-h` instead of blocking on stdin.
  Any other command-line argument prints a "this is a protoc plugin" hint
  to stderr and exits non-zero.

- `buffa::types::decode_bytes_to_bytes` reads a length-delimited `bytes` field
  into a `bytes::Bytes` via `Buf::copy_to_bytes`. When decoding from a
  `Bytes`-backed buffer this is a zero-copy refcount bump. Generated
  `merge_field` arms for `bytes_fields`-tagged fields (singular, optional,
  repeated, and oneof) now use it instead of `Bytes::from(decode_bytes(..)?)`,
  eliminating one allocation + memcpy per field on the owned decode path. Note
  that in the zero-copy case the resulting field aliases the source
  allocation, so the source buffer is freed only once every aliased field is
  dropped. Consumers with checked-in generated code must regenerate to pick
  this up. ([#53](https://github.com/anthropics/buffa/issues/53))

### Fixed

- `buffa-types --features arbitrary` now compiles. `Any.value` is
  `bytes::Bytes` (since 0.4.0 / #51), which has no `Arbitrary` impl.
  Codegen now emits `#[arbitrary(with = ::buffa::__private::arbitrary_bytes*)]`
  on every `bytes_fields`-typed field — singular, optional, and repeated
  struct fields plus oneof variant inner fields — when
  `generate_arbitrary = true`, so the struct-level `derive(Arbitrary)`
  succeeds. Map values are unaffected (they are always `Vec<u8>` regardless
  of `bytes_fields`). The same fix covers any user crate that uses
  `bytes_fields` + `generate_arbitrary`. `cargo doc --workspace
  --all-features` and `cargo clippy --workspace --all-features` are also
  unblocked, and CI now runs `cargo check --workspace --all-features` to
  prevent recurrence.
  ([#88](https://github.com/anthropics/buffa/issues/88))

- `write_to` now emits fields in ascending field-number order regardless of
  cardinality (singular / repeated / map / oneof), matching prost,
  protoc-C++, and the spec's serialize-in-field-order recommendation.
  Previously fields were emitted grouped by kind, which broke
  byte-equivalence with other implementations for messages mixing a
  high-numbered singular field with a lower-numbered repeated/map/oneof.
  Decoders accept any order, so this is not a wire-compat break, but
  consumers content-addressing serialized bytes (e.g. `hash(encode(msg))`)
  will see different hashes for affected message shapes.
  ([#75](https://github.com/anthropics/buffa/issues/75))

## [0.4.0] - 2026-04-27

### Breaking changes

- **Ancillary generated types moved under `pkg::__buffa::`.** View structs,
  oneof enums, view-of-oneof enums, extension consts, and `register_types`
  no longer share the package-level Rust namespace with owned message
  structs. The new layout:

  | Item | Before | After |
  |---|---|---|
  | View struct | `pkg::FooView` | `pkg::__buffa::view::FooView` |
  | Nested view | `pkg::foo::BarView` | `pkg::__buffa::view::foo::BarView` |
  | Oneof enum | `pkg::foo::KindOneof` | `pkg::__buffa::oneof::foo::Kind` |
  | View-of-oneof | `pkg::foo::KindOneofView` | `pkg::__buffa::view::oneof::foo::Kind` |
  | Extension const | `pkg::FOO` | `pkg::__buffa::ext::FOO` |
  | Registration fn | `pkg::register_types` (per file) | `pkg::__buffa::register_types` (per package) |

  Owned message structs and nested-type modules are unchanged. Migration is
  a mechanical path rewrite per the table above. The `Oneof` / `OneofView`
  suffixes are dropped — the parallel module tree disambiguates.

  This makes name collisions between user proto types and codegen-derived
  ancillary names structurally impossible. `__buffa` is the **only** name
  codegen reserves in user namespace; it aligns with the existing `__buffa_`
  reserved field-name prefix. A proto message, file-level enum, or package
  segment that snake-cases to `__buffa` is rejected with
  `CodeGenError::ReservedModuleName`.

- **Consumer include pattern: use `buffa::include_proto!("dotted.pkg")`.**
  Codegen now emits a per-package `<dotted.pkg>.mod.rs` stitcher alongside
  the per-proto content files. Hand-authored
  `include!(concat!(env!("OUT_DIR"), "/my_file.rs"))` blocks no longer
  produce a complete module; replace with:

  ```rust
  pub mod my_pkg {
      buffa::include_proto!("my.pkg");
  }
  ```

  `buffa-build`'s `generate_include_file()` already emits the correct
  structure; consumers using that helper need no change.

- **`__buffa_cached_size` is removed from all generated structs (owned and
  view); `Message::compute_size` / `write_to` and `ViewEncode::compute_size` /
  `write_to` now take a `&mut SizeCache` parameter.** Sizes are recorded in an
  external pre-order `Vec<u32>` cache that the provided `encode*` methods
  construct internally, so generated types contain only their proto fields
  plus `__buffa_unknown_fields` — no interior mutability, structurally
  `Send + Sync`, and concurrent `encode()` of the same `&msg` from multiple
  threads is sound. `Message::cached_size()` and `ViewEncode::cached_size()`
  are removed; use the new provided `encoded_len()` to get the size alone.
  `__private::CachedSize` is removed. Hand-written `Message` / `ViewEncode`
  impls must add the `cache: &mut SizeCache` parameter, drop `cached_size()`,
  and (for nested message fields) wrap recursion in `cache.reserve()` /
  `cache.set()`; see the [custom-types section of the user
  guide](docs/guide.md#custom-type-implementations) for the pattern.
  ([#14](https://github.com/anthropics/buffa/issues/14),
  [#22](https://github.com/anthropics/buffa/pull/22))
- **`DefaultInstance` and `DefaultViewInstance` are no longer `unsafe` traits,
  and `HasDefaultViewInstance` is removed.** The liveness and immutability
  invariants are fully encoded by the return type and cannot be violated by
  a safe implementation. `DefaultViewInstance` is now implemented for
  `FooView<'v>` at every lifetime (not just `'static`), with
  `fn default_view_instance<'a>() -> &'a Self where Self: 'a`; the
  covariant lifetime coercion happens in the impl body where the compiler
  checks it via ordinary subtyping, eliminating the raw pointer cast in
  `Deref for MessageFieldView`. Hand-written impls must drop the `unsafe`
  keyword and adopt the new method signature; the separate
  `HasDefaultViewInstance` impl is no longer needed.
  ([#68](https://github.com/anthropics/buffa/issues/68),
  [#69](https://github.com/anthropics/buffa/issues/69))
- **`CodeGenError::OneofNameConflict` and `::ViewNameConflict` removed.**
  These collisions are now structurally impossible (the inputs that
  previously triggered them produce valid output).
- **`google.protobuf.Any.value` is now `::bytes::Bytes` instead of `Vec<u8>`.**
  Makes `Any::clone()` a cheap refcount bump (up to ~170x faster for large
  payloads) instead of a full memcpy. Call sites constructing an `Any` by hand
  need `.into()` on the payload (e.g. `value: my_vec.into()`, or pass `Bytes`
  directly). Reading `any.value` is unchanged — `Bytes` derefs to `&[u8]`.
  `buffa-types` now depends on `bytes` unconditionally.

### Deprecated

- **`buffa_build::proto_path_to_rust_module`** — consumers should use
  the per-package `<pkg>.mod.rs` stitcher path via `buffa::include_proto!`
  instead.

### Added

- **`ViewEncode<'a>` — serialization from borrowed view types.** Generated
  `*View<'a>` types implement `ViewEncode` (whenever views are generated,
  i.e. `generate_views(true)`, the default) with the same two-pass
  `compute_size`/`write_to` model as `Message`. Views can be constructed
  from borrowed `&'a str` / `&'a [u8]` and encoded without intermediate
  `String`/`Vec` allocation. Benchmarks: parity on serialize-only; ~6× on
  build+encode for a 15-label string-map message.
- **`buffa::include_proto!("dotted.pkg")` macro** — wraps the per-package
  `.mod.rs` stitcher; the canonical consumer integration point.
- **`MapView::new(Vec)` / `From<Vec>` / `FromIterator`** for constructing
  map views directly (for `ViewEncode`).
- **`SizeCache`** — external pre-order size table for the two-pass encode
  protocol (`[u32; 16]` inline + `Vec<u32>` spill, allocation-free for ≤16
  nested LEN sub-messages). The provided `encode*()` methods construct one
  internally; for hot loops, the new `encode_with_cache(&mut SizeCache, buf)`
  reuses a single cache across calls.
- **`Message::encoded_len()` / `ViewEncode::encoded_len()`** — provided
  method returning the serialized size without writing (replaces
  `compute_size()`-then-discard).
- **`Enumeration::values()`** — `&'static [Self]` slice of all variants for
  iteration.
- **`buffa-build` / `buffa-codegen`: `type_attribute`, `field_attribute`,
  `message_attribute`, `enum_attribute`** — attach Rust attributes (e.g.
  `#[derive(...)]`, `#[serde(...)]`) to specific generated types or fields
  by proto path.
- **`protoc-gen-buffa`: `text=true` and `allow_message_set=true` plugin
  parameters** — match the existing `buffa-build` config flags.
- **`#[must_use]` on `Message`/`ViewEncode` `compute_size`, `encoded_len`,
  `encode_to_vec`, `encode_to_bytes`.**

### Fixed

- A proto message named `Option` — anywhere in the proto package, including
  nested in a sibling message or in another file — no longer shadows
  `core::option::Option` in generated optional/oneof field types and the
  JSON deserialize path. Generated code now always emits the
  fully-qualified `::core::option::Option`.
  ([#36](https://github.com/anthropics/buffa/issues/36),
  [#64](https://github.com/anthropics/buffa/issues/64))
- Oneof variant names that PascalCase to a reserved Rust identifier (in
  practice, proto field `self` → variant `Self`) are now escaped.
  ([#47](https://github.com/anthropics/buffa/issues/47))
- Nested type and oneof sharing the same name (the gh#31 `RegionCodes`
  case) and `Foo` next to `FooView` (gh#32) — both now structurally
  resolved by the `__buffa::` namespacing above.

## [0.3.0] - 2026-04-01

### Breaking changes

- **`Extension::new(number)` → `Extension::new(number, extendee)`.** Same for
  `Extension::with_default`. Codegen consumers are unaffected — the `pub const`
  items are regenerated. Hand-written `Extension` consts (unusual) need the
  extendee string added.
- **`ExtensionSet` trait gained a required `const PROTO_FQN: &'static str`.**
  Codegen consumers are unaffected. Hand-written impls need the const added.
- **`extension()`, `set_extension()`, `clear_extension()` now panic on extendee
  mismatch** (previously: silently returned `None` / no-op). `has_extension()`
  returns `false` gracefully. Catches `field_options.extension(&MESSAGE_OPTION)`
  bugs at the first call site; matches protobuf-go (panics) and protobuf-es
  (throws).

### Deprecated

- **`set_any_registry`, `set_extension_registry`** — use
  `buffa::type_registry::set_type_registry` instead, which installs all maps
  in one call. The deprecated functions still work.
- **`AnyTypeEntry` → `JsonAnyEntry`, `ExtensionRegistryEntry` → `JsonExtEntry`.**
  Type aliases for one release cycle. The text-format fields have moved to
  separate `TextAnyEntry` / `TextExtEntry` structs in `type_registry`.

### Added

- **Full extension support.** `Extension<C>` typed descriptors,
  `ExtensionSet` trait with `extension`/`set_extension`/`has_extension`/
  `clear_extension`/`extension_or_default`, codec types for every proto field
  type (including `GroupCodec` for editions `DELIMITED` / proto2 groups),
  proto2 `[default = ...]` on extension declarations, and MessageSet wire
  format behind `CodeGenConfig::allow_message_set`. See the
  [Extensions section of the user guide](docs/guide.md#extensions-custom-options).
- **`TypeRegistry`** — unified registry covering `Any` type entries and
  extension entries for both JSON and text formats. Codegen emits
  `register_types(&mut TypeRegistry)` per file; call once per generated file,
  then `set_type_registry(reg)`. JSON entries (`JsonAnyEntry`, `JsonExtEntry`)
  and text entries (`TextAnyEntry`, `TextExtEntry`) live in feature-split
  maps so `json` and `text` are independently enableable.
- **`JsonParseOptions::strict_extension_keys`** — error on unregistered `"[...]"`
  JSON keys (default: silently drop, matching pre-0.3 behavior for all unknown
  keys).
- **Editions `features.message_encoding = DELIMITED`** — fully supported in
  codegen, previously parsed but ignored. Message fields with this feature use
  the group wire format (StartGroup/EndGroup) instead of length-prefixed.
- **Text format (`textproto`)** — the `buffa::text` module provides
  `TextFormat` trait, `TextEncoder`, `TextDecoder`, and `encode_to_string` /
  `decode_from_str` conveniences. Enable with `features = ["text"]`
  (zero-dependency, `no_std`-compatible) and `Config::generate_text(true)`.
  Covers `Any` expansion (`[type.googleapis.com/...] { ... }`), extension
  brackets (`[pkg.ext] { ... }`), and group/DELIMITED naming. `Any` expansion
  and extension brackets consult the text maps in `TypeRegistry` — the `json`
  and `text` features are independently enableable. Passes the full
  text-format conformance suite (883/883).
- **Conformance:** `TestAllTypesEdition2023` enabled; binary+JSON 5539 → 5549
  passing (std). Text format suite 0 → 883 passing (was entirely skipped).
- **`buffa-descriptor` crate** — `FileDescriptorProto` and friends are now in a
  standalone crate that depends only on `buffa`, so descriptor types are usable
  without pulling in `quote`/`syn`/`prettyplease`. `buffa-codegen` re-exports
  the module so existing `buffa_codegen::generated::*` paths still resolve.
  ([#8](https://github.com/anthropics/buffa/pull/8))
- **Proto source comments → rustdoc.** Comments from `.proto` files are now
  emitted as `///` doc comments on generated structs, fields, enums, variants,
  and view types. Requires `--include_source_info` (set automatically by
  `buffa-build` and the protoc plugins).
  ([#7](https://github.com/anthropics/buffa/pull/7))
- **`buffa::encoding::MAX_FIELD_NUMBER`** constant (`(1 << 29) - 1`), replacing
  the magic number at all call sites.
  ([#21](https://github.com/anthropics/buffa/pull/21))

### Changed

- **`buffa-build` skips writing unchanged outputs**, avoiding mtime bumps that
  trigger needless downstream recompilation.
  ([#17](https://github.com/anthropics/buffa/pull/17))
- **Generated code emits `Self`** in `impl` blocks instead of repeating the
  type name, so consumer crates that enable `clippy::use_self` get clean
  output. ([#15](https://github.com/anthropics/buffa/pull/15))

### Fixed

- **Codegen no longer reports a false name collision** between a nested type
  and a proto3 `optional` field whose synthetic oneof PascalCases to the same
  name. ([#20](https://github.com/anthropics/buffa/pull/20),
  fixes [#12](https://github.com/anthropics/buffa/issues/12))
- **Generated rustdoc no longer breaks on proto comments** containing
  `[foo][]` reference-style links or bare URLs — these are now escaped so
  rustdoc treats them as literal text.
  ([#25](https://github.com/anthropics/buffa/pull/25))

## [0.2.0] - 2026-03-16

### Breaking changes

- **`protoc-gen-buffa`: the `mod_file=<name>` option is removed.** Module tree
  assembly (`mod.rs` generation) is now a separate plugin,
  `protoc-gen-buffa-packaging`. The codegen plugin emits per-file `.rs` only
  and no longer requires `strategy: all`.

  Migration - replace this:

  ```yaml
  plugins:
    - local: protoc-gen-buffa
      out: src/gen
      strategy: all
      opt: [mod_file=mod.rs]
  ```

  with this:

  ```yaml
  plugins:
    - local: protoc-gen-buffa
      out: src/gen
    - local: protoc-gen-buffa-packaging
      out: src/gen
      strategy: all
  ```

  Passing `mod_file=` to the 0.2 plugin is a hard error with a migration hint
  (not a silent no-op).

### Added

- **`protoc-gen-buffa-packaging`** - new protoc plugin that emits a `mod.rs`
  module tree for per-file output. Works with any codegen plugin that follows
  buffa's per-file naming convention (`foo/v1/bar.proto` -> `foo.v1.bar.rs`).
  Invoke once per output tree; compose via multiple buf.gen.yaml entries.
  Optional `filter=services` restricts the tree to proto files that declare
  at least one `service`, for packaging service-stub-only output from plugins
  layered on buffa. Released as standalone binaries for the same five targets
  as `protoc-gen-buffa`, with SLSA provenance and cosign signatures.

- **`buffa-codegen`: `"."` accepted as a catch-all `extern_path` prefix.**
  `extern_path = (".", "crate::proto")` maps every proto package to an
  absolute Rust path rooted at `crate::proto`. More-specific mappings (including
  the auto-injected WKT mapping) still win via longest-prefix-match.

### Library compatibility

`buffa`, `buffa-types`, `buffa-codegen`, and `buffa-build` have no breaking
API changes in this release. The version bump reflects the
`protoc-gen-buffa` CLI change; library consumers upgrading from 0.1 should
see no code changes required.

## [0.1.0] - 2026-03-07

Initial release.

### Protobuf feature coverage

| Feature | Status |
|---|---|
| Binary wire format (proto2, proto3, editions 2023/2024) | ✅ |
| Proto3 JSON canonical mapping | ✅ |
| Well-known types (Timestamp, Duration, Any, Struct, Value, FieldMask, wrappers) | ✅ |
| Unknown field preservation | ✅ (default on) |
| Zero-copy view types | ✅ |
| Open enums (`EnumValue<E>`) with unknown-value preservation | ✅ |
| Closed enums (proto2) with unknown-value routing to unknown fields | ✅¹ |
| proto2 groups (singular, repeated, oneof) | ✅ |
| proto2 custom defaults (`[default = X]`) | ✅ on `required`; `optional` stays `None` |
| Editions feature resolution (`field_presence`, `enum_type`, `repeated_field_encoding`, `utf8_validation`) | ✅ |
| Editions `message_encoding = DELIMITED` | ⚠️ Parsed but ignored — see Known Limitations in README |
| `no_std` + `alloc` (core runtime, views, JSON) | ✅ |
| Text format (`textproto`) | ❌ Not planned |
| proto2 extensions | ❌ Not planned (use `Any`) |
| Runtime reflection | ❌ Not planned for 0.1 |

¹ See Known Limitations for two closed-enum edge cases (packed-repeated in views, map values).

### Conformance

Passes the [protobuf conformance suite](https://github.com/protocolbuffers/protobuf/tree/main/conformance) (v33.5):

- **5,539 passing** binary + JSON tests (std)
- **5,519 passing** binary + JSON tests (no_std — the 20-test gap is `IgnoreUnknownEnumStringValue*` in repeated/map contexts, which requires scoped strict-mode override; `no_std` has `set_global_json_parse_options` for singular-enum accept-with-default but not container filtering)
- **2,797 passing** via-view mode (binary → `decode_view` → `to_owned_message` → encode; direct JSON decode is not supported for views)
- **0 expected failures** across all three runs
- Text-format tests (883) are skipped (not supported)

### Test coverage

- **94.3% line coverage** (workspace, including build-script codegen paths)
- **1,018 unit tests** across runtime, codegen, types, and integration
- **6 fuzz targets**: binary decode (proto2, proto3, WKT), binary encode, JSON round-trip, WKT string parsers
- **googleapis stress test**: codegen compiles all ~3,000 `.proto` files in the Google Cloud API set
- **protoc compatibility**: plugin tested against protoc v21–v33

### Benchmarks (Intel Xeon Platinum 8488C)

Comparison against `prost` 0.13 (lower = buffa faster):

| Operation | buffa vs prost |
|---|---|
| Binary encode | **0.56–0.74×** (26–44% faster) |
| Binary decode | 0.91–1.29× (mixed; deep-nested messages slower) |
| JSON encode | 0.97–1.08× (parity) |
| JSON decode | **0.40–0.88×** (12–60% faster) |

See the [README Performance section](README.md#performance) for charts and raw data.

### Crates

This release publishes:

- `buffa` — core runtime
- `buffa-types` — well-known types (Timestamp, Duration, Any, etc.)
- `buffa-codegen` — descriptor → Rust source (for downstream code generators)
- `buffa-build` — `build.rs` integration
- `protoc-gen-buffa` — protoc plugin binary (also released as standalone binaries for linux-x86_64, linux-aarch64, darwin-x86_64, darwin-aarch64, windows-x86_64)

MSRV: Rust 1.85.

[Unreleased]: https://github.com/anthropics/buffa/compare/v0.7.1...HEAD
[0.7.1]: https://github.com/anthropics/buffa/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/anthropics/buffa/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/anthropics/buffa/compare/v0.5.2...v0.6.0
[0.5.2]: https://github.com/anthropics/buffa/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/anthropics/buffa/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/anthropics/buffa/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/anthropics/buffa/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/anthropics/buffa/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/anthropics/buffa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/anthropics/buffa/releases/tag/v0.1.0

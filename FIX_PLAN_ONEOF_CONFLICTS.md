# Fix Plan: Oneof Name Collision Issues

Context for fixing https://github.com/anthropics/buffa/issues/31 and
https://github.com/anthropics/buffa/issues/33. These are related issues
that share a fix strategy.

## Issues

### gh#31 — NestedTypeOneofConflict

A `message` with a nested type `Items` and `oneof items` produces two
identifiers named `Items` in PascalCase. buffa currently returns a hard
`NestedTypeOneofConflict` error. Example: riderperks `PerkRestrictions`
has `message RegionCodes` + `oneof region_codes`.

### gh#33 — Oneof enum shadows parent message name

When a message's oneof has the same PascalCase name as the message itself
(e.g. `message DataType { oneof data_type { ... } }`), the generated
`pub enum DataType` inside `mod data_type` shadows the parent
`struct DataType` imported via `use super::*`. Nested messages that
reference the parent type resolve to the oneof enum instead, causing
`Default` trait errors.

## Fix Strategy

**Suffix the oneof enum with `Oneof` when its name collides**, rather
than erroring. This only triggers when there would have been a conflict,
so no existing working code changes. The compiler will suggest
`RegionCodesOneof` or `DataTypeOneof` via "did you mean?" hints.

Renaming was chosen over sub-module namespacing because compiler
diagnostics are more helpful with a suffix (users get `did you mean
DataTypeOneof?`) versus a missing-type error with no hint.

## Files to Change

All in `buffa-codegen/src/`:

1. **`oneof.rs`** — `oneof_enum_ident()`: accept a `HashSet<String>` of
   reserved names. If PascalCase name is in the set, append `Oneof`.
   Add `reserved_names_for_msg(msg)` helper to collect nested type/enum
   names from a `DescriptorProto`.

2. **`message.rs`** — Build the reserved set in `generate_message()`.
   For gh#31: nested type + enum names. For gh#33: also include
   `rust_name` (parent message name). Pass to both `oneof_enum_ident`
   call sites (struct field type at ~line 168, enum definition at ~line 305).

3. **`lib.rs`** — Remove `check_nested_type_oneof_conflicts()` function
   and its call site (~line 516). Remove the `NestedTypeOneofConflict`
   error variant from `CodeGenError` (~line 655).

4. **All other `oneof_enum_ident` callers** — Thread `msg` (or the
   reserved set) through:
   - `view.rs`: `oneof_view_struct_fields`, `generate_oneof_view_enum`,
     `oneof_decode_arms` (needs `msg` added to signature), and the
     to-owned conversion loop
   - `impl_text.rs`: `oneof_encode_stmt`, `oneof_merge_arms` (both need
     `msg` added to signature)
   - `impl_message.rs`: `generate_oneof_impls` (needs `msg` added)

5. **Tests** — `tests/naming.rs`: convert the existing
   `test_nested_type_oneof_conflict_detected` test from asserting error
   to asserting success with `Oneof` suffix in generated code. Add new
   test for gh#33 pattern (parent message name = oneof PascalCase name).

## PR Strategy

Two stacked PRs, cross-referenced:
- **PR 1** (base: main): gh#31 fix — reserved set = nested types/enums only
- **PR 2** (base: PR 1): gh#33 fix — add parent message name to reserved set

## WIP Branch

`fix/nested-type-oneof-conflict` has a stashed partial implementation
(`git stash list` to see it). It covers steps 1-4 above with a combined
fix for both issues. To split into two PRs, peel out the
`oneof_reserved_names.insert(rust_name)` line (parent message name) into
the second PR.

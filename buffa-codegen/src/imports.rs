//! Import management for generated code.
//!
//! Prelude types like `Option` are always emitted as fully-qualified paths
//! (`::core::option::Option<T>`) to prevent shadowing by proto-defined types
//! of the same name. This is necessary because the stitcher combines all
//! files from one package into a single module scope via `include!`, so a
//! `message Option` in *any* sibling file would shadow the prelude.
//!
//! `alloc` types (`String`, `Vec`, `Box`) are always emitted as
//! `::buffa::alloc::*` paths because they are not in the `no_std` prelude,
//! consistent with the `HashMap` approach via `::buffa::__private::HashMap`.
//! Buffa runtime types are always emitted as absolute paths since generated
//! files may be combined via `include!`.
//!
//! [`RootImports`] (gated behind `CodeGenConfig::idiomatic_imports`, which
//! requires `file_per_package`) layers package-root `use` directives on top.
//! In `file_per_package` mode the whole package is one single-writer file,
//! so the package-root scope — unlike the multi-file layout, where every
//! `.proto`'s content files are `include!`-merged into it — has fully known
//! contents at generation time, and qualified type paths in it can be
//! shortened to `use`-backed short names.
//!
//! Resolution is two-pass to keep the output deterministic and stable:
//! generation runs once with [`ImportsPhase::Collecting`] active (output
//! discarded) to record every path requested at package root, the collected
//! set is turned into bindings by [`RootImports::assign`] — sorted-order
//! assignment, so the result is independent of `.proto` file order — and
//! generation runs again with [`ImportsPhase::Resolving`] active to emit the
//! short names. Collection happens inside the existing emission code paths
//! (not a separate descriptor walk), so the collected set covers exactly the
//! references the second pass will emit, by construction.

use std::collections::{BTreeMap, BTreeSet};

use crate::idents::rust_path_to_tokens;
use proc_macro2::TokenStream;
use quote::quote;

/// Single source of truth for type-path emission in generated code.
///
/// All prelude types are unconditionally emitted as fully-qualified paths
/// (e.g. `::core::option::Option`) to avoid shadowing by user-defined proto
/// types. This is simpler and more robust than trying to detect collisions:
/// the stitcher's `include!`-based module merging makes it impossible to
/// know at per-file generation time which names will be in scope.
///
/// Stateless — kept as a struct (rather than free functions) so call sites
/// uniformly take `&ImportResolver` and any future per-scope state can be
/// added without re-threading parameters.
///
/// Each method takes the emitting scope's context and module depth and
/// routes through the package-root import registry: at depth 0 with
/// `idiomatic_imports` active it resolves to a registered short name,
/// otherwise it emits the canonical qualified path (always, when the
/// registry is off).
pub(crate) struct ImportResolver;

impl ImportResolver {
    pub fn new() -> Self {
        Self
    }

    // ── Prelude type tokens ─────────────────────────────────────────────

    pub fn option_at(&self, ctx: &crate::context::CodeGenContext, nesting: usize) -> TokenStream {
        ctx.root_runtime_path(OPTION_PATH, nesting)
    }

    // ── Alloc types (no_std-safe via ::buffa::alloc) ─────────────────────

    pub fn string_at(&self, ctx: &crate::context::CodeGenContext, nesting: usize) -> TokenStream {
        ctx.root_runtime_path(STRING_PATH, nesting)
    }

    pub fn vec_at(&self, ctx: &crate::context::CodeGenContext, nesting: usize) -> TokenStream {
        ctx.root_runtime_path(VEC_PATH, nesting)
    }

    // ── Buffa runtime types ──────────────────────────────────────────────

    pub fn message_field_at(
        &self,
        ctx: &crate::context::CodeGenContext,
        nesting: usize,
    ) -> TokenStream {
        ctx.root_runtime_path(MESSAGE_FIELD_PATH, nesting)
    }

    pub fn enum_value_at(
        &self,
        ctx: &crate::context::CodeGenContext,
        nesting: usize,
    ) -> TokenStream {
        ctx.root_runtime_path(ENUM_VALUE_PATH, nesting)
    }

    pub fn hashmap_at(&self, ctx: &crate::context::CodeGenContext, nesting: usize) -> TokenStream {
        ctx.root_runtime_path(HASHMAP_PATH, nesting)
    }
}

const OPTION_PATH: &str = "::core::option::Option";
const STRING_PATH: &str = "::buffa::alloc::string::String";
const VEC_PATH: &str = "::buffa::alloc::vec::Vec";
const MESSAGE_FIELD_PATH: &str = "::buffa::MessageField";
const ENUM_VALUE_PATH: &str = "::buffa::EnumValue";
const HASHMAP_PATH: &str = "::buffa::__private::HashMap";

/// Runtime/prelude types eligible for package-root import, as
/// `(canonical qualified path, short name)`.
///
/// These claim their short names *before* proto-type paths are assigned
/// (and only when the package's own items don't occupy the name), so
/// `String` always means `::buffa::alloc::string::String` in idiomatic
/// output, never a proto type that happens to be named `String`.
const RUNTIME_IMPORTS: &[(&str, &str)] = &[
    (ENUM_VALUE_PATH, "EnumValue"),
    (HASHMAP_PATH, "HashMap"),
    (MESSAGE_FIELD_PATH, "MessageField"),
    (OPTION_PATH, "Option"),
    (STRING_PATH, "String"),
    (VEC_PATH, "Vec"),
];

/// Names that package-root emissions reference *bare*, which proto-type
/// imports must therefore never claim.
///
/// The invariant: every identifier that any emission at the package root
/// references without qualification — prelude traits and macros in derive
/// lists (`#[derive(Clone, …)]`, `impl From<…>`), prelude value constructors
/// (`Some`/`None`), primitive scalar types, and crate names referenced by
/// bare path (`serde` under `json=true`). A proto-type `use` claiming one of
/// these would silently change what those emissions mean. When a new
/// package-root emission references a name bare, it must be added here.
///
/// The [`RUNTIME_IMPORTS`] short names are additionally blocked for
/// proto-type claims in [`RootImports::assign`] regardless of whether the
/// runtime type was itself imported — `String` must never denote a proto
/// type in idiomatic output.
const ROOT_BARE_REFERENCED: &[&str] = &[
    "Box",
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "Eq",
    "From",
    "Hash",
    "Into",
    "None",
    "PartialEq",
    "Result",
    "Self",
    "Send",
    "Some",
    "Sync",
    "bool",
    "f32",
    "f64",
    "i32",
    "i64",
    "serde",
    "str",
    "u8",
    "u32",
    "u64",
];

/// Phase of package-root import handling for the package currently being
/// generated. Held by `CodeGenContext` behind a `RefCell` (same pattern as
/// its warning sink) so the deeply-nested emission helpers participate
/// through the shared `&ctx` without new parameters.
pub(crate) enum ImportsPhase {
    /// `idiomatic_imports` off, or between packages: every path passes
    /// through unchanged. The default.
    Off,
    /// Dry-run generation: record each path requested at package root.
    Collecting(BTreeSet<String>),
    /// Real generation: resolve recorded paths to their assigned short
    /// forms.
    Resolving(RootImports),
}

/// Whether `path` is a candidate for package-root import shortening.
///
/// Only `::`-rooted, `crate::`-rooted, and `super::`-rooted paths qualify.
/// Same-package references at the root are already in their shortest form
/// (`Msg`, `outer::Inner`) — and a relative multi-segment path is not a
/// valid edition-2018 `use` target without a `self::` prefix, so skipping
/// them also keeps every recorded `use` path valid verbatim. Applied at
/// collection time too, so the collected set contains exactly the
/// shortening candidates.
pub(crate) fn shortenable(path: &str) -> bool {
    path.starts_with("::") || path.starts_with("crate::") || path.starts_with("super::")
}

/// Package-root import bindings for one package, produced by
/// [`assign`](Self::assign) between the collection and resolution passes.
pub(crate) struct RootImports {
    /// Qualified path → emitted short form (`"Timestamp"`,
    /// `"protobuf::Timestamp"`). Paths absent here stay fully qualified.
    resolved: BTreeMap<String, String>,
    /// Claimed short name → `use` target path. `BTreeMap` for a
    /// deterministic, alphabetical-by-short-name `use` block.
    bindings: BTreeMap<String, String>,
}

impl RootImports {
    /// Assign short names to the collected paths.
    ///
    /// `occupied` is the set of names already present at the package root:
    /// top-level message and enum names, nested-types module names, the
    /// `__buffa` sentinel, and every root re-export candidate name. It is
    /// computed from the whole package's descriptors, so assignment is
    /// sound for every file in the package.
    ///
    /// Runtime types ([`RUNTIME_IMPORTS`]) claim their short names first;
    /// proto-type paths then walk the alias ladder in sorted order — bare
    /// leaf name, parent-module qualification, fully qualified — making the
    /// result independent of the order paths were requested in.
    pub fn assign(collected: &BTreeSet<String>, occupied: &BTreeSet<String>) -> Self {
        let mut imports = RootImports {
            resolved: BTreeMap::new(),
            bindings: BTreeMap::new(),
        };

        for (path, short) in RUNTIME_IMPORTS {
            if collected.contains(*path) && !occupied.contains(*short) {
                imports
                    .bindings
                    .insert((*short).to_string(), (*path).to_string());
                imports
                    .resolved
                    .insert((*path).to_string(), (*short).to_string());
            }
        }

        let runtime_leaves: BTreeSet<&str> = RUNTIME_IMPORTS.iter().map(|(_, s)| *s).collect();
        let proto_claimable = |name: &str| {
            !occupied.contains(name)
                && !runtime_leaves.contains(name)
                && !ROOT_BARE_REFERENCED.contains(&name)
                && !matches!(name, "self" | "super" | "Self" | "crate")
                && !name.is_empty()
        };

        // Runtime paths are rung-1-or-nothing: when the claim above was
        // refused (package item owns the name), they stay fully qualified
        // rather than falling into the proto-type ladder below (which would
        // bind their parent module, e.g. `string::String`).
        let runtime_paths: BTreeSet<&str> = RUNTIME_IMPORTS.iter().map(|(p, _)| *p).collect();

        for path in collected {
            if imports.resolved.contains_key(path)
                || runtime_paths.contains(path.as_str())
                || !shortenable(path)
            {
                continue;
            }
            let Some((parent, leaf)) = path.rsplit_once("::") else {
                continue;
            };

            // Rung 1: bare leaf name backed by `use <path>;`.
            if proto_claimable(leaf) {
                match imports.bindings.get(leaf) {
                    None => {
                        imports.bindings.insert(leaf.to_string(), path.clone());
                        imports.resolved.insert(path.clone(), leaf.to_string());
                        continue;
                    }
                    Some(existing) if existing == path => {
                        imports.resolved.insert(path.clone(), leaf.to_string());
                        continue;
                    }
                    Some(_) => {}
                }
            }

            // Rung 2: import the parent module, qualify with one segment.
            let parent_leaf = parent.rsplit("::").next().unwrap_or("");
            if proto_claimable(parent_leaf) {
                let claimed = match imports.bindings.get(parent_leaf) {
                    None => {
                        imports
                            .bindings
                            .insert(parent_leaf.to_string(), parent.to_string());
                        true
                    }
                    Some(existing) => existing == parent,
                };
                if claimed {
                    imports
                        .resolved
                        .insert(path.clone(), format!("{parent_leaf}::{leaf}"));
                    continue;
                }
            }

            // Rung 3: stays fully qualified (no entry). `use … as Alias`
            // renames are deliberately not emitted — `outer_b::Inner` reads
            // better than synthetic `OuterBInner`-style aliases.
        }

        imports
    }

    /// The short form assigned to `path`, if any.
    pub fn resolve(&self, path: &str) -> Option<&str> {
        self.resolved.get(path).map(String::as_str)
    }

    /// Emit the `use` block backing the assigned short names, in
    /// deterministic (alphabetical-by-short-name) order. Every binding was
    /// recorded from a collected use site, so `unused_imports` cannot fire.
    pub fn use_items(&self) -> TokenStream {
        let mut out = TokenStream::new();
        for target in self.bindings.values() {
            let path = rust_path_to_tokens(target);
            out.extend(quote! { use #path; });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assign(paths: &[&str], occupied: &[&str]) -> RootImports {
        let collected: BTreeSet<String> = paths.iter().map(|s| s.to_string()).collect();
        let occupied: BTreeSet<String> = occupied.iter().map(|s| s.to_string()).collect();
        RootImports::assign(&collected, &occupied)
    }

    #[test]
    fn cross_package_binds_bare_leaf() {
        let imports = assign(&["super::other::Msg"], &[]);
        assert_eq!(imports.resolve("super::other::Msg"), Some("Msg"));
        assert_eq!(
            imports.use_items().to_string(),
            "use super :: other :: Msg ;"
        );
    }

    #[test]
    fn extern_path_binds_bare_leaf() {
        let imports = assign(&["::buffa_types::google::protobuf::Timestamp"], &[]);
        assert_eq!(
            imports.resolve("::buffa_types::google::protobuf::Timestamp"),
            Some("Timestamp")
        );
    }

    #[test]
    fn same_package_relative_paths_pass_through() {
        // `outer::Inner` is already short, and not a valid bare `use`
        // target in edition 2018 — must not be recorded or shortened.
        let imports = assign(&["outer::Inner", "Msg"], &[]);
        assert_eq!(imports.resolve("outer::Inner"), None);
        assert_eq!(imports.resolve("Msg"), None);
        assert!(imports.use_items().is_empty());
    }

    #[test]
    fn occupied_leaf_falls_to_parent_module() {
        let imports = assign(&["super::other::Msg"], &["Msg"]);
        assert_eq!(imports.resolve("super::other::Msg"), Some("other::Msg"));
        assert_eq!(imports.use_items().to_string(), "use super :: other ;");
    }

    #[test]
    fn occupied_leaf_and_parent_stays_qualified() {
        let imports = assign(&["super::other::Msg"], &["Msg", "other"]);
        assert_eq!(imports.resolve("super::other::Msg"), None);
        assert!(imports.use_items().is_empty());
    }

    #[test]
    fn duplicate_leaf_assignment_is_order_independent() {
        // Sorted assignment: `::a::Dup` wins rung 1 regardless of request
        // order; `::b::Dup` falls to rung 2.
        let forward = assign(&["::a::Dup", "::b::Dup"], &[]);
        let reverse = assign(&["::b::Dup", "::a::Dup"], &[]);
        for imports in [forward, reverse] {
            assert_eq!(imports.resolve("::a::Dup"), Some("Dup"));
            assert_eq!(imports.resolve("::b::Dup"), Some("b::Dup"));
        }
    }

    #[test]
    fn runtime_types_claim_before_proto_types() {
        let imports = assign(
            &["::buffa::alloc::string::String", "super::other::String"],
            &[],
        );
        assert_eq!(
            imports.resolve("::buffa::alloc::string::String"),
            Some("String")
        );
        // The proto type named `String` may not shadow the alloc claim and
        // may not take the bare name even via rung 1 — parent rung only.
        assert_eq!(
            imports.resolve("super::other::String"),
            Some("other::String")
        );
    }

    #[test]
    fn proto_type_never_takes_runtime_leaf_even_when_unused() {
        // No string fields collected — but a proto type named `String`
        // still must not read as bare `String`.
        let imports = assign(&["super::other::String"], &[]);
        assert_eq!(
            imports.resolve("super::other::String"),
            Some("other::String")
        );
    }

    #[test]
    fn runtime_type_blocked_by_package_item() {
        // A package defining `message String` keeps alloc String qualified.
        let imports = assign(&["::buffa::alloc::string::String"], &["String"]);
        assert_eq!(imports.resolve("::buffa::alloc::string::String"), None);
    }

    #[test]
    fn bare_referenced_names_never_claimed() {
        let imports = assign(&["super::other::From"], &[]);
        assert_eq!(imports.resolve("super::other::From"), Some("other::From"));
        // `serde` as a parent module name would shadow the bare crate path.
        let imports = assign(&["super::serde::Inner"], &["Inner"]);
        assert_eq!(imports.resolve("super::serde::Inner"), None);
    }

    #[test]
    fn shared_parent_module_binds_once() {
        let imports = assign(&["super::pkg::A", "super::pkg::B"], &["A", "B"]);
        assert_eq!(imports.resolve("super::pkg::A"), Some("pkg::A"));
        assert_eq!(imports.resolve("super::pkg::B"), Some("pkg::B"));
        assert_eq!(imports.use_items().to_string(), "use super :: pkg ;");
    }

    #[test]
    fn keyword_segments_survive_in_use_targets() {
        // Paths arrive unescaped (`type`, not `r#type`); escaping happens
        // in rust_path_to_tokens when the use block is rendered.
        let imports = assign(&["super::type::LatLng"], &[]);
        assert_eq!(imports.resolve("super::type::LatLng"), Some("LatLng"));
        assert_eq!(
            imports.use_items().to_string(),
            "use super :: r#type :: LatLng ;"
        );
    }

    #[test]
    fn use_items_sorted_by_short_name() {
        let imports = assign(&["::ext::Zebra", "::ext::Alpha"], &[]);
        assert_eq!(
            imports.use_items().to_string(),
            "use :: ext :: Alpha ; use :: ext :: Zebra ;"
        );
    }
}

//! Code generation context and descriptor-to-Rust mapping state.

use std::collections::{HashMap, HashSet};

use crate::features::{self, ResolvedFeatures};
use crate::generated::descriptor::{DescriptorProto, EnumDescriptorProto, FileDescriptorProto};
use crate::oneof::to_snake_case;
use crate::CodeGenConfig;

/// The single reserved module name under which all ancillary generated types
/// (views, oneof enums, extensions, `register_types`) live.
///
/// See `DESIGN.md` → "Generated code layout" for the full layout. The name
/// is checked against proto package segments and message-module names by
/// `validate_file`; a collision is a hard error.
pub const SENTINEL_MOD: &str = "__buffa";

/// A Rust type path split at the target-package boundary.
///
/// Returned by [`CodeGenContext::rust_type_relative_split`]. The full owned
/// path is `to_package + within_package` (concatenated with `::`); ancillary
/// kinds insert their `__buffa::<kind>::` prefix between the two halves.
#[derive(Debug, Clone)]
pub struct SplitPath {
    /// Path from the current emission scope to the **target package root**.
    ///
    /// One of:
    /// - empty (same package, nesting 0)
    /// - `"super::super"` (same package, nesting > 0)
    /// - `"super::…::other_pkg"` (cross-package local)
    /// - `"::extern_crate::pkg"` (extern type — absolute, nesting-independent)
    pub to_package: String,
    /// Path from the target package root to the type itself
    /// (e.g. `"Foo"` or `"outer::Inner"`).
    pub within_package: String,
    /// `true` when `to_package` is an absolute (`::`/`crate::`) extern path.
    /// Extern paths don't depend on the caller's nesting depth.
    pub is_extern: bool,
}

/// Shared context for a code generation run.
///
/// Holds the full set of file descriptors and a mapping from fully-qualified
/// protobuf type names to their Rust type paths. This is needed because a
/// field in one `.proto` file may reference a message defined in another.
pub struct CodeGenContext<'a> {
    /// All file descriptors (both requested and dependencies).
    pub files: &'a [FileDescriptorProto],
    /// Code generation configuration.
    pub config: &'a CodeGenConfig,
    /// Map from fully-qualified protobuf name (e.g., ".my.package.MyMessage")
    /// to Rust type path (e.g., "my::package::MyMessage").
    ///
    /// Nested types use module-qualified paths:
    /// ".pkg.Outer.Inner" → "pkg::outer::Inner" (not "pkg::OuterInner").
    pub type_map: HashMap<String, String>,
    /// Map from fully-qualified protobuf name to its proto package.
    ///
    /// Used by `rust_type_relative` to compute `super::`-based relative
    /// paths for cross-package references within the same compilation.
    package_of: HashMap<String, String>,
    /// Map from fully-qualified enum name to its resolved `enum_type` feature.
    ///
    /// The `enum_type` feature determines whether an enum is OPEN or CLOSED.
    /// It's resolved from the ENUM's own file → message → enum feature chain,
    /// NOT from the referencing field's chain. protoc does not propagate
    /// enum-level `enum_type` into field options (verified 2026-03), so
    /// callers must look this up via `is_enum_closed`.
    enum_closedness: HashMap<String, bool>,
    /// Map from fully-qualified protobuf element name to its source comment.
    ///
    /// Keys use dotted FQN form without a leading dot, matching the `proto_fqn`
    /// values already threaded through codegen: `"pkg.Message"`,
    /// `"pkg.Message.field_name"`, `"pkg.Enum.VALUE_NAME"`,
    /// `"pkg.Message.oneof_name"`.
    ///
    /// Built by walking each file's descriptor tree alongside its
    /// `SourceCodeInfo` (which uses index-based paths). This up-front
    /// translation means codegen call sites can look up comments by the
    /// proto FQN they already have, rather than threading index-based paths
    /// through every function signature.
    comment_map: HashMap<String, String>,
    /// Deconflicted module name for each top-level message, keyed by the
    /// leading-dot FQN (`".pkg.Msg"`).
    ///
    /// A message's nested types live in a `snake_case(Name)` submodule. When
    /// that name would collide with a sub-package module in the same scope
    /// (proto is case-sensitive, so `message Oof` and `package foo.oof` both map
    /// to `mod oof`), a trailing `_` is appended until the name is unique within
    /// the scope's occupied set. Entries exist for every top-level message; the
    /// value equals `snake_case(Name)` when no deconfliction was needed.
    nested_module_names: HashMap<String, String>,
    /// Variant paths (leading-dot form) resolved from
    /// `config.unboxed_oneof_fields` whose oneof variants are stored inline.
    /// Built once by [`resolve_unboxed_variants`](crate::oneof::resolve_unboxed_variants);
    /// never contains recursive variants. See [`oneof_unboxed`](Self::oneof_unboxed).
    unboxed_oneof_variants: HashSet<String>,
    /// Singular message-field paths (leading-dot form) whose resolved
    /// [`PointerRepr`](crate::PointerRepr) is `Inline`. Built once by
    /// [`resolve_inlined_fields`](crate::oneof::resolve_inlined_fields); never
    /// contains recursive fields. [`pointer_repr`](Self::pointer_repr) demotes
    /// any raw `Inline` not in this set to `Box`.
    inlined_message_fields: HashSet<String>,
    /// Non-fatal diagnostics accumulated during generation (e.g. an enum whose
    /// idiomatic CamelCase aliases were suppressed by a naming conflict).
    ///
    /// Interior-mutable so deeply-nested codegen helpers can record a warning
    /// through the shared `&CodeGenContext` without threading a sink through
    /// every signature. Drained by [`generate_with_diagnostics`] after
    /// generation; callers surface them as build warnings. The `RefCell` commits
    /// the context to single-threaded use; switch to a `Mutex` if package
    /// generation is ever parallelized.
    ///
    /// [`generate_with_diagnostics`]: crate::generate_with_diagnostics
    warnings: std::cell::RefCell<Vec<crate::CodeGenWarning>>,
    /// Package-root import phase for `CodeGenConfig::idiomatic_imports`
    /// (file_per_package mode only). [`ImportsPhase::Off`] outside the
    /// two-pass window, so every path resolver below is a no-op by default.
    ///
    /// Interior-mutable for the same reason as `warnings`: the leaf path
    /// resolvers ([`rust_type_relative`](Self::rust_type_relative),
    /// [`root_runtime_path`](Self::root_runtime_path)) participate through
    /// the shared `&CodeGenContext` without threading a registry through
    /// every emission signature.
    ///
    /// [`ImportsPhase::Off`]: crate::imports::ImportsPhase::Off
    imports: std::cell::RefCell<crate::imports::ImportsPhase>,
}

/// The immediate child-package segment names directly under `package`.
///
/// For `package` `"foo"` and known packages `{foo, foo.oof, foo.bar.baz}`, this
/// is `{"oof", "bar"}` — the first segment after the `"foo."` prefix of each
/// deeper package. These are exactly the sub-package module names that will be
/// siblings of `foo`'s message-nesting modules.
fn child_package_segments(package: &str, all_packages: &HashSet<String>) -> HashSet<String> {
    let prefix = if package.is_empty() {
        String::new()
    } else {
        format!("{package}.")
    };
    all_packages
        .iter()
        .filter_map(|p| {
            let rest = if package.is_empty() {
                Some(p.as_str())
            } else {
                p.strip_prefix(&prefix)
            };
            rest.filter(|r| !r.is_empty())
                .map(|r| r.split('.').next().unwrap_or(r).to_string())
        })
        .collect()
}

/// Deconflict the nested-types module names for one package's top-level
/// messages against the sub-package modules in the same scope (issue #135).
///
/// `message_names` are the package's top-level message names in declaration
/// order; `children` are the sub-package segment names that share the package's
/// module scope. Returns one module name per input message, in the same order.
///
/// Each name is `snake_case(Name)` unless it collides with a sub-package
/// segment, in which case `_` is appended until the candidate is unique against:
/// the sub-package segments, every message's raw module name, the `__buffa`
/// sentinel, **and every already-assigned deconflicted name**. That last set —
/// threaded through the shared `taken` set — is what keeps two messages that
/// would otherwise race to the same slot distinct (e.g. `Oof` and `Oof_`
/// alongside sub-packages `oof` and `oof_` resolve to `oof__` and `oof___`,
/// never both to `oof__`).
///
/// Colliding messages are assigned in a stable order (sorted by base name), so
/// the per-message result is independent of declaration order — reordering the
/// input files or messages never changes which name a given message receives.
fn deconflict_package_modules(message_names: &[String], children: &HashSet<String>) -> Vec<String> {
    let bases: Vec<String> = message_names.iter().map(|n| to_snake_case(n)).collect();
    // Seed with everything fixed: sub-package segments, the sentinel, and every
    // message's raw module name. Assigned deconflicted names are added as we go.
    let mut taken: HashSet<String> = children.clone();
    taken.insert(SENTINEL_MOD.to_string());
    taken.extend(bases.iter().cloned());

    // Result starts as the raw bases (correct for every non-colliding message),
    // and colliding messages overwrite their slot. Assign in a stable order
    // (sorted by base name) so the per-message suffix is independent of
    // declaration order; two colliding messages can't both grab the same slot.
    let mut out = bases.clone();
    let mut order: Vec<usize> = (0..bases.len()).collect();
    order.sort_by(|&a, &b| bases[a].cmp(&bases[b]));
    for i in order {
        if !children.contains(&bases[i]) {
            continue;
        }
        let mut candidate = format!("{}_", bases[i]);
        while taken.contains(&candidate) {
            candidate.push('_');
        }
        taken.insert(candidate.clone());
        out[i] = candidate;
    }
    out
}

impl<'a> CodeGenContext<'a> {
    /// Build a context from file descriptors, populating the type map.
    ///
    /// `effective_extern_paths` includes both user-provided mappings and any
    /// auto-injected defaults (e.g., the WKT mapping). These are computed by
    /// `crate::effective_extern_paths` before calling this constructor.
    ///
    /// File-level extern resolution (currently only `descriptor.proto` /
    /// `compiler/plugin.proto` → `buffa-descriptor`) is **not** applied by
    /// this constructor — use [`for_generate`](Self::for_generate), which
    /// computes and passes the file-level mappings, when generating code
    /// that may reference descriptor types.
    pub fn new(
        files: &'a [FileDescriptorProto],
        config: &'a CodeGenConfig,
        effective_extern_paths: &[(String, String)],
    ) -> Self {
        Self::with_extern_resolution(files, config, effective_extern_paths, &[])
    }

    /// Build a context with both package-level and file-level extern
    /// mappings.
    ///
    /// `file_extern_paths` maps a `.proto` file's full path (as reported in
    /// `FileDescriptorProto.name`, e.g. `"google/protobuf/descriptor.proto"`)
    /// to a Rust module root. It takes priority over a package-level
    /// `effective_extern_paths` match, which lets two files in the same proto
    /// package (`google.protobuf`) resolve to different external crates —
    /// `descriptor.proto` types to `buffa-descriptor`, every other
    /// `google.protobuf` WKT to `buffa-types`. Without this split a single
    /// package-keyed mapping would route everything to one crate or the
    /// other.
    ///
    /// Per-type resolution priority (issue #111), most specific first: an
    /// **exact** `extern_path` entry for a type's FQN, then the **file-level**
    /// mapping above, then a **package/prefix** `extern_path` match, then the
    /// **local** package path. See [`resolve_type_path`].
    ///
    /// File-level mappings are an internal mechanism used for the
    /// auto-injected descriptor-types routing. They are not part of the
    /// public `CodeGenConfig` API; user-facing `extern_path` entries are keyed
    /// by proto package *or* type FQN.
    pub(crate) fn with_extern_resolution(
        files: &'a [FileDescriptorProto],
        config: &'a CodeGenConfig,
        effective_extern_paths: &[(String, String)],
        file_extern_paths: &[(String, String)],
    ) -> Self {
        let mut type_map = HashMap::new();
        let mut package_of = HashMap::new();
        let mut enum_closedness = HashMap::new();
        let mut comment_map = HashMap::new();
        let mut nested_module_names = HashMap::new();
        let msg_index = crate::oneof::message_index(files);
        let unboxed_oneof_variants = crate::oneof::resolve_unboxed_variants(
            &msg_index,
            &config.unboxed_oneof_fields,
            &config.pointer_fields,
        );
        let inlined_message_fields = crate::oneof::resolve_inlined_fields(
            &msg_index,
            &config.unboxed_oneof_fields,
            &config.pointer_fields,
        );

        // Pre-pass: collect every package and top-level message name in the
        // descriptor set so nested-types module deconfliction (issue #135) is
        // identical whether the package is generated locally or referenced via
        // `extern_path`. Skipping extern packages here made cross-crate refs
        // use plain `snake_case` (e.g. `money::Currency`) while the owning
        // crate emits deconflicted modules (e.g. `money_::Currency`).
        let mut all_packages: HashSet<String> = HashSet::new();
        let mut pkg_message_names: HashMap<String, Vec<String>> = HashMap::new();
        for file in files {
            let package = file.package.as_deref().unwrap_or("");
            all_packages.insert(package.to_string());
            for msg in &file.message_type {
                if let Some(name) = &msg.name {
                    // A per-type `extern_path` override (issue #111) makes the
                    // message extern: it emits no local module, so it must not
                    // reserve a module name in sub-package deconfliction (#135).
                    let fqn = if package.is_empty() {
                        format!(".{name}")
                    } else {
                        format!(".{package}.{name}")
                    };
                    if effective_extern_paths
                        .iter()
                        .any(|(proto, _)| proto == &fqn)
                    {
                        continue;
                    }
                    pkg_message_names
                        .entry(package.to_string())
                        .or_default()
                        .push(name.clone());
                }
            }
        }

        // Resolve the deconflicted nested-types module name for every top-level
        // message, batched per package so racing deconflictions stay distinct.
        // Each package is an independent scope, so the populated map is the same
        // regardless of `pkg_message_names` iteration order.
        for (package, names) in &pkg_message_names {
            let children = child_package_segments(package, &all_packages);
            let modules = deconflict_package_modules(names, &children);
            for (name, module) in names.iter().zip(modules) {
                let fqn = if package.is_empty() {
                    format!(".{name}")
                } else {
                    format!(".{package}.{name}")
                };
                nested_module_names.insert(fqn, module);
            }
        }

        for file in files {
            comment_map.extend(crate::comments::fqn_comments(file));
            let package = file.package.as_deref().unwrap_or("");
            let file_features = features::for_file(file);
            let proto_prefix = if package.is_empty() {
                String::from(".")
            } else {
                format!(".{}.", package)
            };

            // The file-level extern root (the internal descriptor.proto →
            // buffa-descriptor split) and the local package module. Per-type
            // resolution (issue #111) layers exact/longest-prefix `extern_path`
            // matching on top, via `resolve_type_path`, which preserves the
            // historic priority: per-type exact → file-level → package prefix →
            // local. The file-level check still outranks a package prefix so two
            // files in the same proto package can route to different crates
            // (descriptor.proto → buffa-descriptor, timestamp.proto →
            // buffa-types).
            let file_root = file
                .name
                .as_deref()
                .and_then(|n| resolve_file_extern(n, file_extern_paths));
            let local_module = package.replace('.', "::");

            // Register top-level messages
            for msg in &file.message_type {
                if let Some(name) = &msg.name {
                    let fqn = format!("{}{}", proto_prefix, name);
                    let (rust_path, is_extern) = resolve_type_path(
                        &fqn,
                        name,
                        file_root,
                        &local_module,
                        effective_extern_paths,
                        &config.type_name_prefix,
                    );

                    // The module the message's nested types live in. For a local
                    // message it is `<package>::<module>`, where the module name
                    // is deconflicted against sub-package modules (issue #135),
                    // precomputed above and looked up here so emission and
                    // references share the same value. For an extern/overridden
                    // message no local module is emitted, so the nested module is
                    // the resolved path's parent plus the plain `snake_case` name.
                    let parent_mod = if is_extern {
                        let snake = nested_module_names
                            .get(&fqn)
                            .cloned()
                            .unwrap_or_else(|| to_snake_case(name));
                        match rust_path.rsplit_once("::") {
                            Some((parent, _)) => format!("{parent}::{snake}"),
                            None => snake,
                        }
                    } else {
                        let snake = nested_module_names
                            .get(&fqn)
                            .cloned()
                            .unwrap_or_else(|| to_snake_case(name));
                        join_mod(&local_module, &snake)
                    };

                    type_map.insert(fqn.clone(), rust_path);
                    package_of.insert(fqn.clone(), package.to_string());
                    register_nested_types(
                        &mut type_map,
                        &mut package_of,
                        NestedRegistrationCtx {
                            package,
                            extern_paths: effective_extern_paths,
                            type_name_prefix: &config.type_name_prefix,
                        },
                        &fqn,
                        &parent_mod,
                        msg,
                    );
                    register_nested_enum_closedness(
                        &mut enum_closedness,
                        &fqn,
                        &file_features,
                        msg,
                    );
                }
            }

            // Register top-level enums
            for enum_type in &file.enum_type {
                if let Some(name) = &enum_type.name {
                    let fqn = format!("{}{}", proto_prefix, name);
                    let (rust_path, _) = resolve_type_path(
                        &fqn,
                        name,
                        file_root,
                        &local_module,
                        effective_extern_paths,
                        &config.type_name_prefix,
                    );
                    type_map.insert(fqn.clone(), rust_path);
                    package_of.insert(fqn.clone(), package.to_string());
                    register_enum_closedness(&mut enum_closedness, &fqn, &file_features, enum_type);
                }
            }
        }

        Self {
            files,
            config,
            type_map,
            package_of,
            enum_closedness,
            comment_map,
            nested_module_names,
            unboxed_oneof_variants,
            inlined_message_fields,
            warnings: std::cell::RefCell::new(Vec::new()),
            imports: std::cell::RefCell::new(crate::imports::ImportsPhase::Off),
        }
    }

    /// Record a non-fatal diagnostic to surface as a build warning.
    pub(crate) fn warn(&self, warning: crate::CodeGenWarning) {
        self.warnings.borrow_mut().push(warning);
    }

    /// Drain the diagnostics accumulated during generation.
    ///
    /// `pub(crate)` so it can only be called from [`generate_with_diagnostics`]
    /// after all packages are generated — draining mid-flight would truncate the
    /// diagnostic stream.
    pub(crate) fn take_warnings(&self) -> Vec<crate::CodeGenWarning> {
        self.warnings.take()
    }

    /// Truncate the warning sink back to `len` entries.
    ///
    /// Used by the `idiomatic_imports` collection pass, whose dry-run
    /// generation would otherwise record every diagnostic twice.
    pub(crate) fn truncate_warnings(&self, len: usize) {
        self.warnings.borrow_mut().truncate(len);
    }

    /// The number of warnings recorded so far (a mark for
    /// [`truncate_warnings`](Self::truncate_warnings)).
    pub(crate) fn warnings_len(&self) -> usize {
        self.warnings.borrow().len()
    }

    // ── Package-root import registry (idiomatic_imports) ────────────────

    /// Enter the collection phase: subsequent package-root path
    /// resolutions are recorded instead of shortened.
    pub(crate) fn imports_begin_collecting(&self) {
        *self.imports.borrow_mut() =
            crate::imports::ImportsPhase::Collecting(std::collections::BTreeSet::new());
    }

    /// Leave the collection phase, returning the recorded paths.
    pub(crate) fn imports_take_collected(&self) -> std::collections::BTreeSet<String> {
        match self.imports.replace(crate::imports::ImportsPhase::Off) {
            crate::imports::ImportsPhase::Collecting(set) => set,
            _ => std::collections::BTreeSet::new(),
        }
    }

    /// Enter the resolution phase with assigned bindings.
    pub(crate) fn imports_set_resolving(&self, imports: crate::imports::RootImports) {
        *self.imports.borrow_mut() = crate::imports::ImportsPhase::Resolving(imports);
    }

    /// Reset to [`ImportsPhase::Off`] after a package is generated, so
    /// state never leaks into the next package.
    ///
    /// [`ImportsPhase::Off`]: crate::imports::ImportsPhase::Off
    pub(crate) fn imports_reset(&self) {
        *self.imports.borrow_mut() = crate::imports::ImportsPhase::Off;
    }

    /// The `use` block backing the current package's assigned short names
    /// (empty unless in the resolution phase).
    pub(crate) fn imports_use_block(&self) -> proc_macro2::TokenStream {
        match &*self.imports.borrow() {
            crate::imports::ImportsPhase::Resolving(imports) => imports.use_items(),
            _ => proc_macro2::TokenStream::new(),
        }
    }

    /// Route a package-root type path through the import registry.
    ///
    /// `nesting` is the emitting scope's module depth below the package
    /// root; only depth-0 emissions land in the package-root scope where
    /// the `use` block is visible, so anything deeper passes through
    /// unchanged — as does everything when the registry is off.
    fn root_path(&self, path: String, nesting: usize) -> String {
        if nesting != 0 {
            return path;
        }
        match &mut *self.imports.borrow_mut() {
            crate::imports::ImportsPhase::Off => path,
            crate::imports::ImportsPhase::Collecting(set) => {
                if crate::imports::shortenable(&path) {
                    set.insert(path.clone());
                }
                path
            }
            crate::imports::ImportsPhase::Resolving(imports) => match imports.resolve(&path) {
                Some(short) => short.to_string(),
                None => path,
            },
        }
    }

    /// Route a runtime/prelude type's canonical qualified path through the
    /// import registry, returning the tokens to emit.
    ///
    /// Same depth-0 gating as [`root_path`](Self::root_path); `canonical`
    /// must be one of the `RUNTIME_IMPORTS` paths in `imports.rs`.
    pub(crate) fn root_runtime_path(
        &self,
        canonical: &str,
        nesting: usize,
    ) -> proc_macro2::TokenStream {
        crate::idents::rust_path_to_tokens(&self.root_path(canonical.to_string(), nesting))
    }

    /// The nested-types module name for a top-level message, deconflicted
    /// against sub-package modules (issue #135).
    ///
    /// `package` is the proto package (empty for none), `name` the message's
    /// proto name. Returns the recorded deconflicted name (e.g. `oof_` when
    /// `message Oof` collides with `package <pkg>.oof`), or `snake_case(name)`
    /// when no override was recorded. Both emission and reference resolution go
    /// through the same recorded value, so they always agree.
    pub fn nested_module_name(&self, package: &str, name: &str) -> String {
        let fqn = if package.is_empty() {
            format!(".{name}")
        } else {
            format!(".{package}.{name}")
        };
        self.nested_module_names
            .get(&fqn)
            .cloned()
            .unwrap_or_else(|| to_snake_case(name))
    }

    /// Build a context matching what [`generate()`](crate::generate) uses
    /// internally.
    ///
    /// Computes effective extern paths (user-provided + auto-injected WKT
    /// mapping to `buffa-types` + auto-injected `descriptor.proto` /
    /// `compiler/plugin.proto` file-level mapping to `buffa-descriptor`) and
    /// builds the type map from them.
    ///
    /// Convenience for downstream generators (e.g. `connectrpc-codegen`)
    /// that emit code alongside buffa's message types and need identical
    /// type-path resolution. Using this instead of [`new()`](Self::new) +
    /// manual extern-path computation ensures zero drift with buffa's own
    /// generation.
    pub fn for_generate(
        files: &'a [FileDescriptorProto],
        files_to_generate: &[String],
        config: &'a CodeGenConfig,
    ) -> Self {
        let paths = crate::effective_extern_paths(files, files_to_generate, config);
        let file_paths = crate::effective_file_extern_paths(files_to_generate, config);
        Self::with_extern_resolution(files, config, &paths, &file_paths)
    }

    /// Look up the Rust type path for a fully-qualified protobuf type name.
    pub fn rust_type(&self, proto_fqn: &str) -> Option<&str> {
        self.type_map.get(proto_fqn).map(|s| s.as_str())
    }

    /// Look up the source comment for a protobuf element by FQN.
    ///
    /// `fqn` uses the same dotted form as `proto_fqn` throughout codegen
    /// (no leading dot). For sub-elements, append the element name:
    /// - Message: `"pkg.Message"`
    /// - Field: `"pkg.Message.field_name"`
    /// - Enum value: `"pkg.Enum.VALUE_NAME"`
    /// - Oneof: `"pkg.Message.oneof_name"`
    pub fn comment(&self, fqn: &str) -> Option<&str> {
        self.comment_map.get(fqn).map(|s| s.as_str())
    }

    /// Look up whether an enum (by fully-qualified proto name) is closed.
    ///
    /// Returns `None` if the enum is not in this compilation set (e.g., an
    /// extern_path type), in which case callers should fall back to the
    /// referencing field's feature chain (correct for proto2/proto3 where
    /// `enum_type` is file-level anyway).
    pub fn is_enum_closed(&self, proto_fqn: &str) -> Option<bool> {
        self.enum_closedness.get(proto_fqn).copied()
    }

    /// Look up the Rust type path relative to the current code generation
    /// scope.
    ///
    /// `current_package` is the proto package (e.g., `"google.protobuf"`).
    /// `nesting` is the number of message module levels the generated code
    /// sits inside (0 for struct fields and impls at the package level,
    /// 1 for oneof enums inside a message module, etc.).
    ///
    /// - **Same package**: strips the package prefix and prepends `super::`
    ///   for each nesting level.
    /// - **Cross package (local)**: navigates via `super::` to the common
    ///   ancestor, then descends into the target package. This works
    ///   regardless of where the module tree is placed in the user's crate.
    /// - **Cross package (extern)**: returns the absolute extern path as-is.
    pub fn rust_type_relative(
        &self,
        proto_fqn: &str,
        current_package: &str,
        nesting: usize,
    ) -> Option<String> {
        let full_path = self.type_map.get(proto_fqn)?;

        // Extern types use absolute paths (starting with `::` or `crate::`)
        // and need no relative resolution — they work from any module position.
        if full_path.starts_with("::") || full_path.starts_with("crate::") {
            return Some(self.root_path(full_path.clone(), nesting));
        }

        let target_package = self
            .package_of
            .get(proto_fqn)
            .map(|s| s.as_str())
            .unwrap_or("");

        // Extract the type's path within its package (everything after the
        // package module prefix).
        let target_rust_module = target_package.replace('.', "::");
        let type_suffix = if target_rust_module.is_empty() {
            full_path.as_str()
        } else {
            full_path
                .strip_prefix(&format!("{}::", target_rust_module))
                .unwrap_or(full_path)
        };

        if current_package == target_package {
            // Same package — just the type suffix, with super:: for nesting.
            if nesting == 0 {
                return Some(type_suffix.to_string());
            }
            let supers = (0..nesting).map(|_| "super").collect::<Vec<_>>().join("::");
            return Some(format!("{}::{}", supers, type_suffix));
        }

        // Cross-package local type: compute a super::-based relative path.
        let current_parts: Vec<&str> = if current_package.is_empty() {
            vec![]
        } else {
            current_package.split('.').collect()
        };
        let target_parts: Vec<&str> = if target_package.is_empty() {
            vec![]
        } else {
            target_package.split('.').collect()
        };

        // Find the length of the common package prefix.
        let common_len = current_parts
            .iter()
            .zip(&target_parts)
            .take_while(|(a, b)| a == b)
            .count();

        // Navigate up: one super:: per remaining current package segment,
        // plus one per nesting level (message module depth).
        let up_count = (current_parts.len() - common_len) + nesting;

        // Navigate down: target package segments beyond the common prefix.
        let down_parts = &target_parts[common_len..];

        let mut segments: Vec<&str> = vec!["super"; up_count];
        segments.extend_from_slice(down_parts);

        // Append the type's within-package path.
        let mut result = segments.join("::");
        if !result.is_empty() {
            result.push_str("::");
        }
        result.push_str(type_suffix);

        Some(self.root_path(result, nesting))
    }

    /// Like [`rust_type_relative`](Self::rust_type_relative) but returns the
    /// path split at the target-package boundary.
    ///
    /// Ancillary kinds (views, oneof enums) live in the `__buffa::<kind>::`
    /// sub-tree of each package; callers compose the final path as
    /// `to_package + "::__buffa::" + <kind> + "::" + within_package`.
    ///
    /// `nesting` is the **total** module depth of the caller's emission
    /// scope below the current package root — i.e. message-nesting plus any
    /// `__buffa::<kind>::` levels the caller is already inside (0 for owned
    /// types, +2 for `__buffa::view::`, +3 for `__buffa::view::oneof::`).
    pub fn rust_type_relative_split(
        &self,
        proto_fqn: &str,
        current_package: &str,
        nesting: usize,
    ) -> Option<SplitPath> {
        let full_path = self.type_map.get(proto_fqn)?;

        let target_package = self
            .package_of
            .get(proto_fqn)
            .map(|s| s.as_str())
            .unwrap_or("");

        // Compute the type's path within its package (everything after the
        // package module prefix). For extern types the prefix is the
        // configured rust_module (e.g. `::buffa_types::google::protobuf`),
        // not the bare dotted package, so derive it the same way `new()`
        // populated the map.
        let target_rust_module = if full_path.starts_with("::") || full_path.starts_with("crate::")
        {
            // Reconstruct the extern module prefix by stripping the
            // within-package suffix length. We know the proto FQN's
            // within-package portion (FQN minus package), so the full_path's
            // last N segments correspond to it.
            //
            // Simpler: re-derive via `resolve_extern_prefix` would need the
            // original extern_paths list. Instead, compute within-package
            // from the proto FQN (which we know) and slice full_path.
            let fqn_no_dot = proto_fqn.strip_prefix('.').unwrap_or(proto_fqn);
            let within_proto = if target_package.is_empty() {
                fqn_no_dot
            } else {
                fqn_no_dot
                    .strip_prefix(target_package)
                    .and_then(|s| s.strip_prefix('.'))
                    .unwrap_or(fqn_no_dot)
            };
            // within_proto is dotted (e.g. "Outer.Inner"); within full_path
            // it's `outer::Inner` (snake_case modules + final PascalCase).
            // Count the segments and strip that many from full_path to recover
            // the module the type lives in.
            //
            // For paths buffa builds itself (`<rust_module>::<within>`) and for
            // per-type `extern_path` overrides whose target mirrors the proto
            // nesting (the sensible case — e.g. mapping to another
            // buffa-generated crate), `full_segs.len() >= within_segs`. A
            // pathological override that maps a deeply-nested type to a shorter
            // Rust path can't have a matching `__buffa::` view/oneof tree
            // anyway, so we clamp with `saturating_sub` rather than panic and
            // let the (unresolvable) reference surface as a normal compile error.
            let within_segs = within_proto.split('.').count();
            let full_segs: Vec<&str> = full_path.split("::").collect();
            let cut = full_segs.len().saturating_sub(within_segs);
            full_segs[..cut].join("::")
        } else {
            target_package.replace('.', "::")
        };

        let type_suffix = if target_rust_module.is_empty() {
            full_path.as_str()
        } else {
            full_path
                .strip_prefix(&format!("{}::", target_rust_module))
                .unwrap_or(full_path)
        };

        // Extern: absolute path; nesting irrelevant.
        if full_path.starts_with("::") || full_path.starts_with("crate::") {
            return Some(SplitPath {
                to_package: target_rust_module,
                within_package: type_suffix.to_string(),
                is_extern: true,
            });
        }

        if current_package == target_package {
            let to_package = if nesting == 0 {
                String::new()
            } else {
                (0..nesting).map(|_| "super").collect::<Vec<_>>().join("::")
            };
            return Some(SplitPath {
                to_package,
                within_package: type_suffix.to_string(),
                is_extern: false,
            });
        }

        // Cross-package local.
        let current_parts: Vec<&str> = if current_package.is_empty() {
            vec![]
        } else {
            current_package.split('.').collect()
        };
        let target_parts: Vec<&str> = if target_package.is_empty() {
            vec![]
        } else {
            target_package.split('.').collect()
        };
        let common_len = current_parts
            .iter()
            .zip(&target_parts)
            .take_while(|(a, b)| a == b)
            .count();
        let up_count = (current_parts.len() - common_len) + nesting;
        let down_parts = &target_parts[common_len..];

        let mut segments: Vec<&str> = vec!["super"; up_count];
        segments.extend_from_slice(down_parts);

        Some(SplitPath {
            to_package: segments.join("::"),
            within_package: type_suffix.to_string(),
            is_extern: false,
        })
    }

    /// Collect custom attributes matching a fully-qualified proto path.
    ///
    /// Returns a `TokenStream` of all `#[...]` attributes whose path prefix
    /// matches `fqn`. Each attribute string is parsed via `syn::parse_str`
    /// so the caller can interpolate directly into `quote!`.
    ///
    /// `fqn` uses dotted form without a leading dot (e.g., `"my.pkg.MyMessage"`).
    ///
    /// # Errors
    ///
    /// Returns `CodeGenError::InvalidCustomAttribute` if any matching attribute
    /// string fails to parse as a valid Rust attribute.
    pub(crate) fn matching_attributes(
        attrs: &[(String, String)],
        fqn: &str,
    ) -> Result<proc_macro2::TokenStream, crate::CodeGenError> {
        if attrs.is_empty() {
            return Ok(proc_macro2::TokenStream::new());
        }
        let fqn_dotted = format!(".{fqn}");
        let mut tokens = proc_macro2::TokenStream::new();
        for (prefix, attr_str) in attrs {
            if matches_proto_prefix(prefix, &fqn_dotted) {
                let parsed =
                    syn::parse_str::<proc_macro2::TokenStream>(attr_str).map_err(|err| {
                        crate::CodeGenError::InvalidCustomAttribute {
                            path: prefix.clone(),
                            attribute: attr_str.clone(),
                            detail: err.to_string(),
                        }
                    })?;
                tokens.extend(parsed);
            }
        }
        Ok(tokens)
    }

    /// Resolve the [`BytesRepr`](crate::BytesRepr) for a `bytes` field at the
    /// given proto path.
    ///
    /// `field_fqn` is the fully-qualified proto field path, e.g.,
    /// `".my.pkg.MyMessage.data"`. Rules in `config.bytes_fields` are matched
    /// using proto-segment-aware prefix matching (`"."` matches all,
    /// `".my.pkg"` matches `".my.pkg.Msg.data"` but not `".my.pkgs.X.data"`);
    /// the **last** matching rule wins, letting a specific override follow a
    /// broad default. Fields matching no rule use
    /// [`BytesRepr::Vec`](crate::BytesRepr::Vec).
    pub fn bytes_repr(&self, field_fqn: &str) -> crate::BytesRepr {
        self.config
            .bytes_fields
            .iter()
            .rev()
            .find(|(prefix, _)| matches_proto_prefix(prefix, field_fqn))
            .map_or(crate::BytesRepr::default(), |(_, repr)| repr.clone())
    }

    /// Check whether a message-typed oneof variant at the given proto path is
    /// stored inline (opted out of `Box` wrapping).
    ///
    /// `variant_fqn` is the fully-qualified variant path, e.g.
    /// `".my.pkg.MyMessage.body.small"`. This consults the set resolved at
    /// context construction by the internal `resolve_unboxed_variants` pass,
    /// not the raw config rules: recursive variants matched only by a prefix
    /// rule are excluded there (they stay boxed), so every codegen site that
    /// asks agrees with the emitted enum declaration.
    pub fn oneof_unboxed(&self, variant_fqn: &str) -> bool {
        self.unboxed_oneof_variants.contains(variant_fqn)
    }

    /// Resolve the [`StringRepr`](crate::StringRepr) for a `string` field at the
    /// given proto path.
    ///
    /// `field_fqn` is the fully-qualified proto field path, e.g.
    /// `".my.pkg.MyMessage.name"`. Rules in `config.string_fields` are matched
    /// with the same proto-segment-aware prefix logic as
    /// [`bytes_repr`](Self::bytes_repr); the **last** matching rule wins,
    /// letting a specific override follow a broad default. Fields matching no
    /// rule use [`StringRepr::String`](crate::StringRepr::String).
    pub fn string_repr(&self, field_fqn: &str) -> crate::StringRepr {
        self.config
            .string_fields
            .iter()
            .rev()
            .find(|(prefix, _)| matches_proto_prefix(prefix, field_fqn))
            .map_or(crate::StringRepr::default(), |(_, repr)| repr.clone())
    }

    /// Resolve the [`MapRepr`](crate::MapRepr) for a `map` field at the given
    /// proto path.
    ///
    /// `field_fqn` is the fully-qualified proto field path, e.g.
    /// `".my.pkg.MyMessage.entries"`. Rules in `config.map_fields` are matched
    /// with the same proto-segment-aware prefix logic as
    /// [`string_repr`](Self::string_repr); the **last** matching rule wins,
    /// letting a specific override follow a broad default. Fields matching no
    /// rule use [`MapRepr::HashMap`](crate::MapRepr::HashMap).
    pub fn map_repr(&self, field_fqn: &str) -> crate::MapRepr {
        self.config
            .map_fields
            .iter()
            .rev()
            .find(|(prefix, _)| matches_proto_prefix(prefix, field_fqn))
            .map_or(crate::MapRepr::default(), |(_, repr)| repr.clone())
    }

    /// Resolve the [`PointerRepr`](crate::PointerRepr) for a singular message
    /// field at the given proto path. Last matching rule wins (proto-segment
    /// prefix match); fields matching no rule use
    /// [`PointerRepr::Box`](crate::PointerRepr::Box).
    ///
    /// `Inline` is recursion-aware: a path whose raw rule resolves to
    /// [`PointerRepr::Inline`](crate::PointerRepr::Inline) but is absent from
    /// the precomputed `inlined_message_fields` set
    /// (a recursive singular field, or a non-singular path such as a oneof
    /// variant) is demoted to `Box` so the generated type stays sized.
    pub fn pointer_repr(&self, field_fqn: &str) -> crate::PointerRepr {
        let raw = self
            .config
            .pointer_fields
            .iter()
            .rev()
            .find(|(prefix, _)| matches_proto_prefix(prefix, field_fqn))
            .map_or(crate::PointerRepr::default(), |(_, repr)| repr.clone());
        if raw == crate::PointerRepr::Inline && !self.inlined_message_fields.contains(field_fqn) {
            crate::PointerRepr::Box
        } else {
            raw
        }
    }

    /// Resolve the [`RepeatedRepr`](crate::RepeatedRepr) for a `repeated` field
    /// at the given proto path.
    ///
    /// `field_fqn` is the fully-qualified proto field path, e.g.
    /// `".my.pkg.MyMessage.items"`. Rules in `config.repeated_fields` are matched
    /// with the same proto-segment-aware prefix logic as
    /// [`string_repr`](Self::string_repr); the **last** matching rule wins,
    /// letting a specific override follow a broad default. Fields matching no
    /// rule use [`RepeatedRepr::Vec`](crate::RepeatedRepr::Vec).
    pub fn repeated_repr(&self, field_fqn: &str) -> crate::RepeatedRepr {
        self.config
            .repeated_fields
            .iter()
            .rev()
            .find(|(prefix, _)| matches_proto_prefix(prefix, field_fqn))
            .map_or(crate::RepeatedRepr::default(), |(_, repr)| repr.clone())
    }
}

/// Scope-local context for code generation within a message.
///
/// Bundles the parameters that are constant within a single message's code
/// generation scope and change only when recursing into nested messages.
/// Threading this struct instead of five individual parameters keeps function
/// signatures short and makes adding new scope-level state a one-field change.
#[derive(Clone, Copy)]
pub(crate) struct MessageScope<'a> {
    /// Global codegen context (descriptors, type map, config).
    pub ctx: &'a CodeGenContext<'a>,
    /// Proto package of the file being generated (e.g. `"google.protobuf"`).
    pub current_package: &'a str,
    /// Fully-qualified proto name of the current message
    /// (e.g. `"google.protobuf.Timestamp"`, `"pkg.Outer.Inner"`).
    pub proto_fqn: &'a str,
    /// Resolved edition features for this message scope.
    pub features: &'a ResolvedFeatures,
    /// Module nesting depth — number of `pub mod` levels the generated code
    /// sits inside.  Controls the count of `super::` prefixes in type
    /// references via [`CodeGenContext::rust_type_relative`].
    pub nesting: usize,
}

impl<'a> MessageScope<'a> {
    /// Create a child scope for a nested message (increments nesting by 1).
    pub fn nested(&self, proto_fqn: &'a str, features: &'a ResolvedFeatures) -> MessageScope<'a> {
        MessageScope {
            ctx: self.ctx,
            current_package: self.current_package,
            proto_fqn,
            features,
            nesting: self.nesting + 1,
        }
    }
}

/// Kind of ancillary tree under the [`SENTINEL_MOD`] module.
///
/// `path_segments()` returns the module path *inside* `__buffa::` (not
/// including the sentinel itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AncillaryKind {
    /// `__buffa::oneof::<msg_path>::` — owned oneof enums.
    Oneof,
    /// `__buffa::view::<msg_path>::` — message view structs.
    View,
    /// `__buffa::view::oneof::<msg_path>::` — view oneof enums.
    ViewOneof,
    /// `__buffa::lazy_view::<msg_path>::` — lazy view structs.
    LazyView,
}

impl AncillaryKind {
    fn path_segments(self) -> &'static [&'static str] {
        match self {
            Self::Oneof => &["oneof"],
            Self::View => &["view"],
            Self::ViewOneof => &["view", "oneof"],
            Self::LazyView => &["lazy_view"],
        }
    }
}

/// Build a token-stream path prefix from an emission scope to an ancillary
/// kind's location for the **current** message (`proto_fqn`).
///
/// Always climbs to the package root via `super::` and re-descends through
/// `__buffa::<kind>::<msg_path>::` — uniform regardless of where the caller
/// sits. `from_nesting` is the caller's total module depth below the
/// package root (message-nesting plus any `__buffa::<kind>::` levels the
/// caller is already inside).
///
/// `proto_fqn` follows the dotless convention used throughout codegen
/// (e.g. `"google.protobuf.Value"`, not `".google.protobuf.Value"`).
///
/// Returned tokens always end with `::` so callers append the type
/// identifier directly: `quote! { #prefix #ident }`.
pub(crate) fn ancillary_prefix(
    kind: AncillaryKind,
    current_package: &str,
    proto_fqn: &str,
    from_nesting: usize,
) -> proc_macro2::TokenStream {
    use crate::idents::make_field_ident;
    use quote::quote;

    debug_assert!(
        !proto_fqn.starts_with('.'),
        "ancillary_prefix expects dotless FQN, got {proto_fqn:?}"
    );

    let mut supers_tokens = proc_macro2::TokenStream::new();
    for _ in 0..from_nesting {
        supers_tokens.extend(quote! { super:: });
    }

    let sentinel = make_field_ident(SENTINEL_MOD);
    let kind_segs: Vec<_> = kind
        .path_segments()
        .iter()
        .map(|s| make_field_ident(s))
        .collect();

    // Snake-cased message path within the package (e.g. "outer::inner::").
    let within_pkg = if current_package.is_empty() {
        proto_fqn
    } else {
        proto_fqn
            .strip_prefix(current_package)
            .and_then(|s| s.strip_prefix('.'))
            .unwrap_or(proto_fqn)
    };
    let msg_segs: Vec<_> = within_pkg
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|name| make_field_ident(&to_snake_case(name)))
        .collect();

    quote! { #supers_tokens #sentinel :: #(#kind_segs ::)* #(#msg_segs ::)* }
}

/// Proto-segment-aware prefix match: `prefix` matches `fqn_dotted` if
/// `prefix == "."`, the two are equal, or `fqn_dotted` starts with `prefix`
/// followed by a `.` boundary. Proto identifiers are ASCII, and `.` is ASCII,
/// so byte indexing is safe.
pub(crate) fn matches_proto_prefix(prefix: &str, fqn_dotted: &str) -> bool {
    prefix == "."
        || prefix == fqn_dotted
        || (fqn_dotted.starts_with(prefix)
            && fqn_dotted.as_bytes().get(prefix.len()) == Some(&b'.'))
}

/// Look up a file-level extern mapping by exact proto file path.
///
/// Unlike [`resolve_extern_prefix`], this does no prefix or package
/// arithmetic — file-level mappings are auto-injected for a known closed set
/// of bootstrap protos (`descriptor.proto`, `compiler/plugin.proto`) whose
/// types live in a different crate (`buffa-descriptor`) than the rest of the
/// `google.protobuf` package (`buffa-types`). The mapped Rust module is the
/// root of that file's generated module, with no further suffix.
fn resolve_file_extern<'p>(
    file_name: &str,
    file_extern_paths: &'p [(String, String)],
) -> Option<&'p str> {
    file_extern_paths
        .iter()
        .find(|(name, _)| name == file_name)
        .map(|(_, rust)| rust.as_str())
}

/// Check if a proto package matches any extern_path prefix.
///
/// Returns the Rust module path root if matched, including any remaining
/// package segments converted to `snake_case` modules. For example,
/// extern_path `(".my", "::my_crate")` with package `"my.sub.pkg"` returns
/// `"::my_crate::sub::pkg"`.
///
/// `pub(crate)`: also called from `crate::effective_file_extern_paths` to
/// suppress an auto-injected file-level mapping when a user package-level
/// extern_path already covers that file's package.
pub(crate) fn resolve_extern_prefix(
    package: &str,
    extern_paths: &[(String, String)],
) -> Option<String> {
    let dotted = format!(".{}", package);

    // Try longest prefix first so that more specific mappings take priority
    // over broader ones (e.g., ".my.common" before ".my").
    let mut best: Option<(&str, &str, usize)> = None;

    for (proto_prefix, rust_prefix) in extern_paths {
        if dotted == *proto_prefix {
            // Exact match is always the best.
            return Some(rust_prefix.clone());
        }
        if let Some(rest) = dotted.strip_prefix(proto_prefix.as_str()) {
            // `"."` is the catch-all root; stripping it leaves no leading dot.
            if proto_prefix == "." || rest.starts_with('.') {
                let prefix_len = proto_prefix.len();
                // MSRV: `Option::is_none_or` requires 1.82.
                if best.map_or(true, |(_, _, best_len)| prefix_len > best_len) {
                    best = Some((proto_prefix, rust_prefix, prefix_len));
                }
            }
        }
    }

    let (proto_prefix, rust_prefix, _) = best?;
    let rest = dotted.strip_prefix(proto_prefix)?;
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    let suffix = rest
        .split('.')
        .map(to_snake_case)
        .collect::<Vec<_>>()
        .join("::");
    Some(format!("{}::{}", rust_prefix, suffix))
}

/// Resolve a fully-qualified proto **type** name against `extern_paths`,
/// mirroring prost's `resolve_ident` (issue #111).
///
/// Unlike [`resolve_extern_prefix`] — which only matches a file's *package* and
/// returns the package's Rust module — this matches the type's whole FQN:
///
/// 1. An **exact** `extern_path` entry for the FQN wins (a per-type override,
///    e.g. `.google.protobuf.Timestamp = ::pbjson_types::Timestamp`).
/// 2. Otherwise the **longest dotted-prefix** entry (a package or an enclosing
///    type) applies, with the proto segments past that prefix rendered as
///    `snake_case` modules and the final segment kept as the Rust type name —
///    exactly the path [`CodeGenContext::new`] would otherwise build from
///    [`resolve_extern_prefix`] plus the type name, so package-prefix mappings
///    resolve identically to before.
///
/// `fqn` is the leading-dot form (e.g. `.google.protobuf.Timestamp`). Returns
/// the full Rust type path, or `None` when nothing matches (the caller falls
/// back to the local package path).
fn resolve_extern_type(fqn: &str, extern_paths: &[(String, String)]) -> Option<String> {
    // 1. Exact per-type entry — the mapping *is* the full Rust path.
    if let Some((_, rust)) = extern_paths.iter().find(|(proto, _)| proto == fqn) {
        return Some(rust.clone());
    }

    // 2. Longest dotted-prefix entry (package or enclosing type).
    let mut best: Option<(&str, &str, usize)> = None;
    for (proto_prefix, rust_prefix) in extern_paths {
        let matches = proto_prefix == "."
            || fqn
                .strip_prefix(proto_prefix.as_str())
                .is_some_and(|rest| rest.starts_with('.'));
        // MSRV: `Option::is_none_or` requires 1.82.
        if matches && best.map_or(true, |(_, _, best_len)| proto_prefix.len() > best_len) {
            best = Some((proto_prefix, rust_prefix, proto_prefix.len()));
        }
    }

    let (proto_prefix, rust_prefix, _) = best?;
    // The catch-all `.` leaves the whole FQN (minus its leading dot); any other
    // prefix leaves the `.`-separated remainder after the matched boundary.
    let rest = if proto_prefix == "." {
        fqn.strip_prefix('.').unwrap_or(fqn)
    } else {
        fqn.strip_prefix(proto_prefix)
            .and_then(|r| r.strip_prefix('.'))
            .unwrap_or("")
    };
    let mut segments = rest.split('.').collect::<Vec<_>>();
    // The final segment is the type name (kept verbatim); the rest are modules.
    let type_name = segments.pop()?;
    let mut path = rust_prefix.to_string();
    for module in segments {
        path.push_str("::");
        path.push_str(&to_snake_case(module));
    }
    path.push_str("::");
    path.push_str(type_name);
    Some(path)
}

/// Join a Rust module path and a type name, handling the empty (no-package)
/// module by returning the bare type name.
fn join_mod(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_string()
    } else {
        format!("{module}::{name}")
    }
}

/// Resolve a registered type's Rust path, applying per-type `extern_path`
/// overrides (issue #111).
///
/// Priority, most specific first: an **exact** per-type FQN entry, then a
/// **file-level** mapping (the internal `descriptor.proto` → `buffa-descriptor`
/// split, which must outrank a package prefix), then the **longest
/// dotted-prefix** entry (package or enclosing type, via [`resolve_extern_type`]),
/// then the **local** package path.
///
/// Returns `(rust_path, is_extern)`; `is_extern` is `false` only for the local
/// fallback, telling the caller whether the type emits a local module (and thus
/// participates in sub-package deconfliction, issue #135).
fn resolve_type_path(
    fqn: &str,
    name: &str,
    file_root: Option<&str>,
    local_module: &str,
    extern_paths: &[(String, String)],
    type_name_prefix: &str,
) -> (String, bool) {
    // The exact check is done here (not left to `resolve_extern_type`) so an
    // exact per-type entry outranks the file-level mapping below; once it has
    // failed, `resolve_extern_type`'s own exact pass can only fall through to
    // its prefix logic.
    //
    // `type_name_prefix` applies only to the local fallback: extern-mapped
    // types are named by the external crate, and the file-level root is the
    // internal descriptor.proto → buffa-descriptor split (also external to
    // this codegen run).
    if let Some((_, rust)) = extern_paths.iter().find(|(proto, _)| proto == fqn) {
        (rust.clone(), true)
    } else if let Some(root) = file_root {
        (join_mod(root, name), true)
    } else if let Some(path) = resolve_extern_type(fqn, extern_paths) {
        (path, true)
    } else {
        (
            join_mod(local_module, &format!("{type_name_prefix}{name}")),
            false,
        )
    }
}

/// Per-file invariants of [`register_nested_types`], bundled so the
/// recursion only threads the parent FQN / module that actually vary.
#[derive(Clone, Copy)]
struct NestedRegistrationCtx<'a> {
    package: &'a str,
    extern_paths: &'a [(String, String)],
    type_name_prefix: &'a str,
}

/// Recursively register nested messages and enums with module-qualified paths.
///
/// Each nested message `Parent.Child` maps to `parent_mod::Child` in Rust,
/// where `parent_mod` is the snake_case module path of the enclosing message.
///
/// A per-type `extern_path` override (issue #111) on a nested type's own FQN
/// takes priority over the inherited `parent_mod` path; otherwise the nested
/// type simply lives in `parent_mod` — which already reflects any override of an
/// enclosing type, so a parent override cascades to its descendants.
fn register_nested_types(
    type_map: &mut HashMap<String, String>,
    package_of: &mut HashMap<String, String>,
    reg: NestedRegistrationCtx<'_>,
    parent_fqn: &str,
    parent_mod: &str,
    msg: &crate::generated::descriptor::DescriptorProto,
) {
    let NestedRegistrationCtx {
        package,
        extern_paths,
        type_name_prefix,
    } = reg;
    for nested in &msg.nested_type {
        if let Some(name) = &nested.name {
            let fqn = format!("{}.{}", parent_fqn, name);
            // An exact per-type override wins; the child module is then the
            // override's parent plus the plain snake_case name. Otherwise the
            // type lives in `parent_mod`, named with the configured prefix
            // (the module segment stays the proto-derived snake_case name —
            // modules never collide with type names).
            let (rust_path, child_mod) = match extern_paths.iter().find(|(proto, _)| proto == &fqn)
            {
                Some((_, rust)) => {
                    let child = match rust.rsplit_once("::") {
                        Some((parent, _)) => format!("{parent}::{}", to_snake_case(name)),
                        None => to_snake_case(name),
                    };
                    (rust.clone(), child)
                }
                None => (
                    format!("{parent_mod}::{type_name_prefix}{name}"),
                    format!("{parent_mod}::{}", to_snake_case(name)),
                ),
            };
            type_map.insert(fqn.clone(), rust_path);
            package_of.insert(fqn.clone(), package.to_string());

            // Recurse: nested-of-nested goes in a deeper module.
            register_nested_types(type_map, package_of, reg, &fqn, &child_mod, nested);
        }
    }

    for enum_type in &msg.enum_type {
        if let Some(name) = &enum_type.name {
            let fqn = format!("{}.{}", parent_fqn, name);
            let rust_path = extern_paths
                .iter()
                .find(|(proto, _)| proto == &fqn)
                .map(|(_, rust)| rust.clone())
                .unwrap_or_else(|| format!("{parent_mod}::{type_name_prefix}{name}"));
            type_map.insert(fqn.clone(), rust_path);
            package_of.insert(fqn, package.to_string());
        }
    }
}

/// Resolve and record whether an enum is closed, given its parent's features.
fn register_enum_closedness(
    map: &mut HashMap<String, bool>,
    fqn: &str,
    parent_features: &ResolvedFeatures,
    enum_desc: &EnumDescriptorProto,
) {
    let resolved = features::resolve_child(parent_features, features::enum_features(enum_desc));
    let closed = resolved.enum_type == features::EnumType::Closed;
    map.insert(fqn.to_string(), closed);
}

/// Walk nested messages and register all enum closedness, resolving features
/// through the message hierarchy (file → msg → nested_msg → enum).
fn register_nested_enum_closedness(
    map: &mut HashMap<String, bool>,
    parent_fqn: &str,
    parent_features: &ResolvedFeatures,
    msg: &DescriptorProto,
) {
    let msg_features = features::resolve_child(parent_features, features::message_features(msg));
    for enum_type in &msg.enum_type {
        if let Some(name) = &enum_type.name {
            let fqn = format!("{}.{}", parent_fqn, name);
            register_enum_closedness(map, &fqn, &msg_features, enum_type);
        }
    }
    for nested in &msg.nested_type {
        if let Some(name) = &nested.name {
            let fqn = format!("{}.{}", parent_fqn, name);
            register_nested_enum_closedness(map, &fqn, &msg_features, nested);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::descriptor::{DescriptorProto, EnumDescriptorProto, FileDescriptorProto};

    fn children(segs: &[&str]) -> HashSet<String> {
        segs.iter().map(|s| s.to_string()).collect()
    }

    fn names(ns: &[&str]) -> Vec<String> {
        ns.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn deconflict_no_collision_keeps_base() {
        // No sub-package shares a module name → names are unchanged.
        let out = deconflict_package_modules(&names(&["Oof", "Bar"]), &children(&["other"]));
        assert_eq!(out, vec!["oof".to_string(), "bar".to_string()]);
    }

    #[test]
    fn deconflict_single_collision_appends_underscore() {
        let out = deconflict_package_modules(&names(&["Oof"]), &children(&["oof"]));
        assert_eq!(out, vec!["oof_".to_string()]);
    }

    #[test]
    fn deconflict_repeated_append_when_underscore_slot_also_taken() {
        // Sub-packages `oof` AND `oof_` both exist → grow past both.
        let out = deconflict_package_modules(&names(&["Oof"]), &children(&["oof", "oof_"]));
        assert_eq!(out, vec!["oof__".to_string()]);
    }

    #[test]
    fn deconflict_two_messages_racing_to_same_slot_stay_distinct() {
        // `Oof` (oof) and `Oof_` (oof_), sub-packages `oof` and `oof_`. Without
        // the shared `taken` set both would land on `oof__`.
        let out = deconflict_package_modules(&names(&["Oof", "Oof_"]), &children(&["oof", "oof_"]));
        assert_eq!(out, vec!["oof__".to_string(), "oof___".to_string()]);
        // All distinct and clear of the sub-package modules.
        let set: HashSet<&String> = out.iter().collect();
        assert_eq!(set.len(), out.len());
        assert!(!out.contains(&"oof".to_string()) && !out.contains(&"oof_".to_string()));
    }

    #[test]
    fn deconflict_is_independent_of_declaration_order() {
        // Reordering the input must not change which message gets which name.
        let ch = children(&["oof", "oof_"]);
        let fwd = deconflict_package_modules(&names(&["Oof", "Oof_"]), &ch);
        let rev = deconflict_package_modules(&names(&["Oof_", "Oof"]), &ch);
        // fwd: [Oof, Oof_]; rev: [Oof_, Oof] — same per-name mapping either way.
        assert_eq!(fwd, vec!["oof__".to_string(), "oof___".to_string()]);
        assert_eq!(rev, vec!["oof___".to_string(), "oof__".to_string()]);
    }

    #[test]
    fn deconflict_avoids_other_messages_raw_base() {
        // `Oof` collides with sub-package `oof`; its `oof_` candidate must also
        // avoid the raw module of a sibling message `Oof_`.
        let out = deconflict_package_modules(&names(&["Oof", "Oof_"]), &children(&["oof"]));
        // Oof -> oof_ is taken by Oof_'s raw base, so Oof -> oof__; Oof_ stays.
        assert_eq!(out, vec!["oof__".to_string(), "oof_".to_string()]);
    }

    #[test]
    fn deconflict_never_yields_the_sentinel() {
        let out = deconflict_package_modules(&names(&["Buffa"]), &children(&["__buffa", "buffa"]));
        // base `buffa` collides; `buffa_` is free, so it is chosen (not __buffa).
        assert_eq!(out, vec!["buffa_".to_string()]);
        assert_ne!(out[0], SENTINEL_MOD);
    }

    #[test]
    fn child_package_segments_extracts_immediate_segment() {
        let pkgs = children(&["foo", "foo.oof", "foo.bar.baz", "foobar"]);
        let mut got: Vec<String> = child_package_segments("foo", &pkgs).into_iter().collect();
        got.sort();
        // `foo.oof` -> oof, `foo.bar.baz` -> bar; `foobar` is not a sub-package.
        assert_eq!(got, vec!["bar".to_string(), "oof".to_string()]);
    }

    fn make_file(
        name: &str,
        package: &str,
        messages: Vec<DescriptorProto>,
        enums: Vec<EnumDescriptorProto>,
    ) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(name.to_string()),
            package: if package.is_empty() {
                None
            } else {
                Some(package.to_string())
            },
            message_type: messages,
            enum_type: enums,
            ..Default::default()
        }
    }

    fn msg(name: &str) -> DescriptorProto {
        DescriptorProto {
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    fn msg_with_nested(name: &str, nested: Vec<DescriptorProto>) -> DescriptorProto {
        DescriptorProto {
            name: Some(name.to_string()),
            nested_type: nested,
            ..Default::default()
        }
    }

    fn msg_with_nested_and_enums(
        name: &str,
        nested: Vec<DescriptorProto>,
        enums: Vec<EnumDescriptorProto>,
    ) -> DescriptorProto {
        DescriptorProto {
            name: Some(name.to_string()),
            nested_type: nested,
            enum_type: enums,
            ..Default::default()
        }
    }

    fn enum_desc(name: &str) -> EnumDescriptorProto {
        EnumDescriptorProto {
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    fn enum_with_closed_feature(name: &str) -> EnumDescriptorProto {
        use crate::generated::descriptor::{feature_set, EnumOptions, FeatureSet};
        EnumDescriptorProto {
            name: Some(name.to_string()),
            options: buffa::MessageField::some(EnumOptions {
                features: buffa::MessageField::some(FeatureSet {
                    enum_type: Some(feature_set::EnumType::CLOSED),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn editions_file(
        name: &str,
        package: &str,
        messages: Vec<DescriptorProto>,
        enums: Vec<EnumDescriptorProto>,
    ) -> FileDescriptorProto {
        use crate::generated::descriptor::Edition;
        FileDescriptorProto {
            name: Some(name.to_string()),
            package: Some(package.to_string()),
            syntax: Some("editions".to_string()),
            edition: Some(Edition::EDITION_2023),
            message_type: messages,
            enum_type: enums,
            ..Default::default()
        }
    }

    // ── Type registration tests ──────────────────────────────────────────

    #[test]
    fn test_message_with_package() {
        let files = [make_file(
            "test.proto",
            "my.package",
            vec![msg("Foo")],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".my.package.Foo"), Some("my::package::Foo"));
    }

    #[test]
    fn test_message_no_package() {
        let files = [make_file("test.proto", "", vec![msg("Bar")], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".Bar"), Some("Bar"));
    }

    #[test]
    fn test_nested_message_uses_module_path() {
        let outer = msg_with_nested("Outer", vec![msg("Inner")]);
        let files = [make_file("test.proto", "pkg", vec![outer], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".pkg.Outer"), Some("pkg::Outer"));
        // Nested types use module-qualified paths.
        assert_eq!(ctx.rust_type(".pkg.Outer.Inner"), Some("pkg::outer::Inner"));
    }

    #[test]
    fn test_nested_message_no_package() {
        let outer = msg_with_nested("Outer", vec![msg("Inner")]);
        let files = [make_file("test.proto", "", vec![outer], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".Outer"), Some("Outer"));
        assert_eq!(ctx.rust_type(".Outer.Inner"), Some("outer::Inner"));
    }

    #[test]
    fn test_deeply_nested_message() {
        let deep = msg_with_nested("A", vec![msg_with_nested("B", vec![msg("C")])]);
        let files = [make_file("test.proto", "pkg", vec![deep], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".pkg.A"), Some("pkg::A"));
        assert_eq!(ctx.rust_type(".pkg.A.B"), Some("pkg::a::B"));
        assert_eq!(ctx.rust_type(".pkg.A.B.C"), Some("pkg::a::b::C"));
    }

    #[test]
    fn test_nested_enum_uses_module_path() {
        let outer = msg_with_nested_and_enums("Outer", vec![], vec![enum_desc("Status")]);
        let files = [make_file("test.proto", "pkg", vec![outer], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type(".pkg.Outer.Status"),
            Some("pkg::outer::Status")
        );
    }

    #[test]
    fn test_top_level_enum() {
        let files = [make_file(
            "test.proto",
            "pkg",
            vec![],
            vec![enum_desc("Status")],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".pkg.Status"), Some("pkg::Status"));
    }

    #[test]
    fn test_same_named_nested_types_in_different_parents_are_distinct() {
        let outer1 = msg_with_nested("Outer1", vec![msg("Inner")]);
        let outer2 = msg_with_nested("Outer2", vec![msg("Inner")]);
        let files = [make_file("a.proto", "pkg", vec![outer1, outer2], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Different parent modules make them distinct.
        assert_eq!(
            ctx.rust_type(".pkg.Outer1.Inner"),
            Some("pkg::outer1::Inner")
        );
        assert_eq!(
            ctx.rust_type(".pkg.Outer2.Inner"),
            Some("pkg::outer2::Inner")
        );
        assert_ne!(
            ctx.rust_type(".pkg.Outer1.Inner"),
            ctx.rust_type(".pkg.Outer2.Inner")
        );
    }

    #[test]
    fn test_multiple_files() {
        let files = [
            make_file("a.proto", "ns.a", vec![msg("MsgA")], vec![]),
            make_file("b.proto", "ns.b", vec![msg("MsgB")], vec![]),
        ];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".ns.a.MsgA"), Some("ns::a::MsgA"));
        assert_eq!(ctx.rust_type(".ns.b.MsgB"), Some("ns::b::MsgB"));
    }

    // ── Per-type FQN extern_path resolution (issue #111) ──────────────────
    //
    // `extern_path` historically matched only at the package-prefix level, so
    // a mapping naming a concrete type FQN (e.g.
    // `.google.protobuf.Timestamp=::pbjson_types::Timestamp`, a normal
    // prost/tonic idiom) was silently ignored. These tests pin the prost
    // `resolve_ident` behavior: an exact type FQN entry wins, otherwise the
    // longest dotted-prefix (package or enclosing type) applies.

    #[test]
    fn test_extern_path_exact_per_type_match() {
        let files = [make_file(
            "test.proto",
            "test.pkg",
            vec![msg("Msg")],
            vec![],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".test.pkg.Msg".into(), "::ext_crate::Msg".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // An exact per-type FQN mapping must be honored, not silently dropped.
        assert_eq!(ctx.rust_type(".test.pkg.Msg"), Some("::ext_crate::Msg"));
    }

    #[test]
    fn test_extern_path_per_type_overrides_package_prefix() {
        // Package-level mapping covers the whole package; an exact per-type
        // entry overrides it for that one type (exact-match-first), while a
        // sibling still resolves via the package prefix.
        let files = [make_file(
            "test.proto",
            "test.pkg",
            vec![msg("Msg"), msg("Other")],
            vec![],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![
                (".test.pkg".into(), "::pkg_crate".into()),
                (".test.pkg.Msg".into(), "::ext_crate::Msg".into()),
            ],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".test.pkg.Msg"), Some("::ext_crate::Msg"));
        assert_eq!(ctx.rust_type(".test.pkg.Other"), Some("::pkg_crate::Other"));
    }

    #[test]
    fn test_extern_path_nested_type_inherits_per_type_override() {
        // A per-type override of a parent message cascades to its nested
        // types via the snake_case module of the overridden path.
        let outer = msg_with_nested("Outer", vec![msg("Inner")]);
        let files = [make_file("test.proto", "test.pkg", vec![outer], vec![])];
        let config = CodeGenConfig {
            extern_paths: vec![(".test.pkg.Outer".into(), "::ext::Outer".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".test.pkg.Outer"), Some("::ext::Outer"));
        assert_eq!(
            ctx.rust_type(".test.pkg.Outer.Inner"),
            Some("::ext::outer::Inner")
        );
    }

    #[test]
    fn test_extern_path_exact_per_type_enum() {
        let files = [make_file(
            "test.proto",
            "test.pkg",
            vec![],
            vec![enum_desc("Status")],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".test.pkg.Status".into(), "::ext_crate::Status".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type(".test.pkg.Status"),
            Some("::ext_crate::Status")
        );
    }

    #[test]
    fn test_extern_path_package_prefix_still_resolves() {
        // Guard: package-prefix mappings (the only historically-supported
        // form) must keep resolving exactly as before.
        let files = [make_file(
            "test.proto",
            "test.pkg",
            vec![msg("Msg")],
            vec![],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".test.pkg".into(), "::pkg_crate".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".test.pkg.Msg"), Some("::pkg_crate::Msg"));
    }

    #[test]
    fn test_extern_path_per_type_does_not_affect_unmapped_type() {
        // Guard: a per-type entry must not leak onto an unrelated type.
        let files = [make_file(
            "test.proto",
            "test.pkg",
            vec![msg("Msg"), msg("Other")],
            vec![],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".test.pkg.Msg".into(), "::ext_crate::Msg".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".test.pkg.Msg"), Some("::ext_crate::Msg"));
        // `Other` has no entry — resolves locally.
        assert_eq!(ctx.rust_type(".test.pkg.Other"), Some("test::pkg::Other"));
    }

    #[test]
    fn test_keyword_package_segment_in_type_map() {
        // Proto package `google.type` — the type map stores plain string paths.
        // Keyword escaping happens at the token level, not in the type map.
        let files = [make_file(
            "latlng.proto",
            "google.type",
            vec![msg("LatLng")],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type(".google.type.LatLng"),
            Some("google::type::LatLng")
        );
    }

    #[test]
    fn test_keyword_package_relative_same_package() {
        let files = [make_file(
            "latlng.proto",
            "google.type",
            vec![msg("LatLng"), msg("Expr")],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Same-package reference: just the type name (no module prefix).
        assert_eq!(
            ctx.rust_type_relative(".google.type.LatLng", "google.type", 0),
            Some("LatLng".into())
        );
    }

    #[test]
    fn test_keyword_package_cross_package() {
        let files = [
            make_file("latlng.proto", "google.type", vec![msg("LatLng")], vec![]),
            make_file("svc.proto", "google.cloud", vec![msg("Service")], vec![]),
        ];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Cross-package: relative path via super:: (keyword escaping at token level).
        // From google.cloud, go up one (past "cloud"), then into "type".
        assert_eq!(
            ctx.rust_type_relative(".google.type.LatLng", "google.cloud", 0),
            Some("super::type::LatLng".into())
        );
    }

    #[test]
    fn test_keyword_nested_message_module() {
        // Message named "Type" → module "type" in type map.
        let outer = msg_with_nested("Type", vec![msg("Inner")]);
        let files = [make_file("test.proto", "pkg", vec![outer], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".pkg.Type"), Some("pkg::Type"));
        assert_eq!(ctx.rust_type(".pkg.Type.Inner"), Some("pkg::type::Inner"));
    }

    #[test]
    fn test_unknown_type_returns_none() {
        let files = [make_file("test.proto", "pkg", vec![msg("Foo")], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type(".pkg.Unknown"), None);
    }

    // ── Relative type resolution tests ───────────────────────────────────

    #[test]
    fn test_relative_same_package_top_level() {
        let files = [make_file("a.proto", "pkg", vec![msg("Foo")], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // From top-level in same package: just the type name.
        assert_eq!(
            ctx.rust_type_relative(".pkg.Foo", "pkg", 0),
            Some("Foo".into())
        );
    }

    #[test]
    fn test_relative_cross_package() {
        let files = [
            make_file("a.proto", "pkg_a", vec![msg("Foo")], vec![]),
            make_file("b.proto", "pkg_b", vec![msg("Bar")], vec![]),
        ];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Cross-package: relative via super:: (up one from pkg_b, into pkg_a).
        assert_eq!(
            ctx.rust_type_relative(".pkg_a.Foo", "pkg_b", 0),
            Some("super::pkg_a::Foo".into())
        );
    }

    #[test]
    fn test_relative_no_package() {
        let files = [make_file("a.proto", "", vec![msg("Foo")], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type_relative(".Foo", "", 0), Some("Foo".into()));
    }

    #[test]
    fn test_relative_unknown_returns_none() {
        let files = [make_file("a.proto", "pkg", vec![msg("Foo")], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.rust_type_relative(".pkg.Unknown", "pkg", 0), None);
    }

    #[test]
    fn test_relative_dotted_package() {
        let files = [make_file("a.proto", "my.pkg", vec![msg("Foo")], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type_relative(".my.pkg.Foo", "my.pkg", 0),
            Some("Foo".into())
        );
    }

    #[test]
    fn test_relative_cross_dotted_packages() {
        let files = [
            make_file(
                "timestamp.proto",
                "google.protobuf",
                vec![msg("Timestamp")],
                vec![],
            ),
            make_file(
                "test.proto",
                "protobuf_test_messages.proto3",
                vec![msg("TestAllTypesProto3")],
                vec![],
            ),
        ];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);

        // Cross-package: relative via super:: (no common prefix, up 2 levels).
        assert_eq!(
            ctx.rust_type_relative(
                ".google.protobuf.Timestamp",
                "protobuf_test_messages.proto3",
                0,
            ),
            Some("super::super::google::protobuf::Timestamp".into())
        );
    }

    #[test]
    fn test_relative_nested_type_from_same_package() {
        // Referencing Outer.Inner from the same package.
        let outer = msg_with_nested("Outer", vec![msg("Inner")]);
        let files = [make_file("test.proto", "pkg", vec![outer], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);

        // Same package: strips the package prefix, keeps module path.
        assert_eq!(
            ctx.rust_type_relative(".pkg.Outer.Inner", "pkg", 0),
            Some("outer::Inner".into())
        );
    }

    #[test]
    fn test_relative_shared_prefix_not_confused() {
        let files = [
            make_file("ab.proto", "a.b", vec![msg("Msg1")], vec![]),
            make_file("abc.proto", "a.bc", vec![msg("Msg2")], vec![]),
        ];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);

        // `a.b.Msg1` from `a.bc` context: common prefix "a", up 1, into "b".
        assert_eq!(
            ctx.rust_type_relative(".a.b.Msg1", "a.bc", 0),
            Some("super::b::Msg1".into())
        );
        // `a.bc.Msg2` from `a.b` context: common prefix "a", up 1, into "bc".
        assert_eq!(
            ctx.rust_type_relative(".a.bc.Msg2", "a.b", 0),
            Some("super::bc::Msg2".into())
        );
    }

    // ── Nesting depth tests ────────────────────────────────────────────

    #[test]
    fn test_relative_cross_package_nesting_1() {
        // Simulates a nested message (inside a `pub mod`) referencing a type
        // from a sibling package. E.g., account.business.admin.v1 nested msg
        // referencing account.business.v1.Business.Status.
        let outer = msg_with_nested_and_enums("Business", vec![], vec![enum_desc("Status")]);
        let files = [
            make_file("admin.proto", "a.b.admin.v1", vec![msg("Svc")], vec![]),
            make_file("biz.proto", "a.b.v1", vec![outer], vec![]),
        ];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);

        // nesting=0 (top-level struct in admin.v1): up 2 (v1→admin), into v1
        assert_eq!(
            ctx.rust_type_relative(".a.b.v1.Business.Status", "a.b.admin.v1", 0),
            Some("super::super::v1::business::Status".into())
        );
        // nesting=1 (inside a nested message module): one extra super::
        assert_eq!(
            ctx.rust_type_relative(".a.b.v1.Business.Status", "a.b.admin.v1", 1),
            Some("super::super::super::v1::business::Status".into())
        );
    }

    #[test]
    fn test_relative_same_package_nesting_1() {
        // Nested message referencing a sibling type in the same package.
        let files = [make_file(
            "test.proto",
            "pkg",
            vec![msg("Foo"), msg("Bar")],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);

        // nesting=0: same package, just the name
        assert_eq!(
            ctx.rust_type_relative(".pkg.Foo", "pkg", 0),
            Some("Foo".into())
        );
        // nesting=1: inside a message module, needs one super::
        assert_eq!(
            ctx.rust_type_relative(".pkg.Foo", "pkg", 1),
            Some("super::Foo".into())
        );
        // nesting=2: doubly nested
        assert_eq!(
            ctx.rust_type_relative(".pkg.Foo", "pkg", 2),
            Some("super::super::Foo".into())
        );
    }

    // ── Extern path tests ─────────────────────────────────────────────

    #[test]
    fn test_resolve_file_extern_exact_match_only() {
        let mappings = [(
            "google/protobuf/descriptor.proto".to_string(),
            "::buffa_descriptor::generated::descriptor".to_string(),
        )];
        // Exact file path matches.
        assert_eq!(
            resolve_file_extern("google/protobuf/descriptor.proto", &mappings),
            Some("::buffa_descriptor::generated::descriptor"),
        );
        // No prefix matching — a sibling file in the same directory does
        // not match (its types live in a different generated module).
        assert_eq!(
            resolve_file_extern("google/protobuf/timestamp.proto", &mappings),
            None,
        );
        // No suffix matching either.
        assert_eq!(
            resolve_file_extern("vendor/google/protobuf/descriptor.proto", &mappings),
            None,
        );
    }

    #[test]
    fn test_resolve_extern_prefix_exact_match() {
        let result = resolve_extern_prefix(
            "my.common",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, Some("::common_protos".into()));
    }

    #[test]
    fn test_resolve_extern_prefix_sub_package() {
        let result = resolve_extern_prefix(
            "my.common.sub",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, Some("::common_protos::sub".into()));
    }

    #[test]
    fn test_resolve_extern_prefix_no_match() {
        let result = resolve_extern_prefix(
            "other.pkg",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_extern_prefix_partial_name_no_match() {
        // ".my.common" should not match ".my.commonext"
        let result = resolve_extern_prefix(
            "my.commonext",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_extern_prefix_longest_match_wins() {
        // When multiple prefixes match, the longest one should win.
        let result = resolve_extern_prefix(
            "my.common.sub",
            &[
                (".my".into(), "::crate_a".into()),
                (".my.common".into(), "::crate_b".into()),
            ],
        );
        assert_eq!(result, Some("::crate_b::sub".into()));
    }

    #[test]
    fn test_resolve_extern_prefix_catchall() {
        let result = resolve_extern_prefix("greet.v1", &[(".".into(), "crate::proto".into())]);
        assert_eq!(result, Some("crate::proto::greet::v1".into()));
    }

    #[test]
    fn test_resolve_extern_prefix_catchall_empty_pkg() {
        // Empty package with `.` catch-all hits the exact-match branch
        // (dotted == "." == proto_prefix) and returns the root as-is.
        let result = resolve_extern_prefix("", &[(".".into(), "crate::proto".into())]);
        assert_eq!(result, Some("crate::proto".into()));
    }

    #[test]
    fn test_resolve_extern_prefix_catchall_longest_wins() {
        // `.` catch-all is the shortest possible prefix; any more-specific
        // mapping (including the auto-injected WKT mapping) takes priority.
        let result = resolve_extern_prefix(
            "google.protobuf",
            &[
                (".".into(), "crate::proto".into()),
                (
                    ".google.protobuf".into(),
                    "::buffa_types::google::protobuf".into(),
                ),
            ],
        );
        assert_eq!(result, Some("::buffa_types::google::protobuf".into()));
    }

    #[test]
    fn test_resolve_extern_prefix_catchall_keyword_package() {
        // Keyword segments stay unescaped at the string level; escaping to
        // `r#type` happens later in `idents::rust_path_to_tokens`.
        let result = resolve_extern_prefix("google.type", &[(".".into(), "crate::proto".into())]);
        assert_eq!(result, Some("crate::proto::google::type".into()));
    }

    // ── resolve_extern_type — per-type FQN resolution (issue #111) ─────────

    #[test]
    fn test_resolve_extern_type_exact_match() {
        // An exact entry for the type FQN is the full Rust path verbatim.
        let result = resolve_extern_type(
            ".google.protobuf.Timestamp",
            &[(
                ".google.protobuf.Timestamp".into(),
                "::pbjson_types::Timestamp".into(),
            )],
        );
        assert_eq!(result, Some("::pbjson_types::Timestamp".into()));
    }

    #[test]
    fn test_resolve_extern_type_exact_wins_over_prefix() {
        // Exact-match-first: the per-type entry beats a covering package entry.
        let result = resolve_extern_type(
            ".my.pkg.Msg",
            &[
                (".my.pkg".into(), "::pkg_crate".into()),
                (".my.pkg.Msg".into(), "::ext_crate::Msg".into()),
            ],
        );
        assert_eq!(result, Some("::ext_crate::Msg".into()));
    }

    #[test]
    fn test_resolve_extern_type_package_prefix_appends_type() {
        // A package prefix yields `<rust_prefix>::<sub modules>::<TypeName>`,
        // matching what resolve_extern_prefix + the type name would build.
        let result = resolve_extern_type(
            ".my.common.sub.Msg",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, Some("::common_protos::sub::Msg".into()));
    }

    #[test]
    fn test_resolve_extern_type_catchall() {
        let result = resolve_extern_type(".greet.v1.Hello", &[(".".into(), "crate::proto".into())]);
        assert_eq!(result, Some("crate::proto::greet::v1::Hello".into()));
    }

    #[test]
    fn test_resolve_extern_type_no_match() {
        let result = resolve_extern_type(
            ".other.pkg.Msg",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_extern_type_partial_name_no_match() {
        // ".my.common" must not match ".my.commonext.Msg" (dot-boundary).
        let result = resolve_extern_type(
            ".my.commonext.Msg",
            &[(".my.common".into(), "::common_protos".into())],
        );
        assert_eq!(result, None);
    }

    // ── rust_type_relative_split — extern branch ────────────────────────

    #[test]
    fn test_split_extern_top_level() {
        let outer = msg_with_nested("Value", vec![msg("Inner")]);
        let files = [make_file(
            "struct.proto",
            "google.protobuf",
            vec![outer],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let extern_paths = vec![(
            ".google.protobuf".into(),
            "::buffa_types::google::protobuf".into(),
        )];
        let ctx = CodeGenContext::new(&files, &config, &extern_paths);

        let split = ctx
            .rust_type_relative_split(".google.protobuf.Value", "my.pkg", 3)
            .expect("type resolves");
        assert!(split.is_extern);
        // Extern path is absolute → nesting irrelevant.
        assert_eq!(split.to_package, "::buffa_types::google::protobuf");
        assert_eq!(split.within_package, "Value");
    }

    #[test]
    fn test_split_extern_nested_type() {
        // Nested `.google.protobuf.Value.Inner` →
        // extern path `::buffa_types::google::protobuf::value::Inner`.
        // Segment-count slice: 2 within-package segments → cut after the
        // extern module prefix.
        let outer = msg_with_nested("Value", vec![msg("Inner")]);
        let files = [make_file(
            "struct.proto",
            "google.protobuf",
            vec![outer],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let extern_paths = vec![(
            ".google.protobuf".into(),
            "::buffa_types::google::protobuf".into(),
        )];
        let ctx = CodeGenContext::new(&files, &config, &extern_paths);

        let split = ctx
            .rust_type_relative_split(".google.protobuf.Value.Inner", "my.pkg", 0)
            .expect("nested type resolves");
        assert!(split.is_extern);
        assert_eq!(split.to_package, "::buffa_types::google::protobuf");
        assert_eq!(split.within_package, "value::Inner");
    }

    #[test]
    fn test_split_per_type_extern_override() {
        // Per-type override (issue #111): the `__buffa::` boundary recovery must
        // still split the override path at the package/within-package seam, so
        // callers compose `<to_package>::__buffa::view::<within_package>`
        // correctly. Both the overridden type and its nested children are
        // exercised.
        let outer = msg_with_nested("Outer", vec![msg("Inner")]);
        let files = [make_file("custom.proto", "my.pkg", vec![outer], vec![])];
        let config = CodeGenConfig {
            extern_paths: vec![(".my.pkg.Outer".into(), "::ext::custom::Outer".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);

        let split = ctx
            .rust_type_relative_split(".my.pkg.Outer", "other.pkg", 2)
            .expect("overridden type resolves");
        assert!(split.is_extern);
        assert_eq!(split.to_package, "::ext::custom");
        assert_eq!(split.within_package, "Outer");

        // The nested type inherits the override's module, and the seam is
        // recovered the same way (2 within-package segments stripped).
        let nested = ctx
            .rust_type_relative_split(".my.pkg.Outer.Inner", "other.pkg", 0)
            .expect("nested type resolves");
        assert!(nested.is_extern);
        assert_eq!(nested.to_package, "::ext::custom");
        assert_eq!(nested.within_package, "outer::Inner");
    }

    #[test]
    fn test_extern_path_top_level_message() {
        let files = [make_file(
            "common.proto",
            "my.common",
            vec![msg("SharedMsg")],
            vec![],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".my.common".into(), "::common_protos".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type(".my.common.SharedMsg"),
            Some("::common_protos::SharedMsg")
        );
    }

    #[test]
    fn test_extern_path_nested_message() {
        let files = [make_file(
            "common.proto",
            "my.common",
            vec![msg_with_nested("Outer", vec![msg("Inner")])],
            vec![],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".my.common".into(), "::common_protos".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type(".my.common.Outer"),
            Some("::common_protos::Outer")
        );
        assert_eq!(
            ctx.rust_type(".my.common.Outer.Inner"),
            Some("::common_protos::outer::Inner")
        );
    }

    #[test]
    fn test_extern_path_nested_types_use_deconflicted_module() {
        // Mirrors pb.lyft.Money.{Currency,Breakdown} vs sub-package pb.lyft.money.
        let money =
            msg_with_nested_and_enums("Money", vec![msg("Breakdown")], vec![enum_desc("Currency")]);
        let files = [
            make_file("lyft_money.proto", "pb.lyft", vec![money], vec![]),
            make_file("money.proto", "pb.lyft.money", vec![msg("Money")], vec![]),
            make_file(
                "users.proto",
                "pb.lyft.users",
                vec![msg("DailyTotalFares")],
                vec![],
            ),
        ];
        let config = CodeGenConfig {
            extern_paths: vec![(".pb.lyft".into(), "::idl_pb_lyft::pb::lyft".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::for_generate(&files, &["users.proto".to_string()], &config);
        assert_eq!(
            ctx.rust_type(".pb.lyft.Money.Currency"),
            Some("::idl_pb_lyft::pb::lyft::money_::Currency")
        );
        assert_eq!(
            ctx.rust_type(".pb.lyft.Money.Breakdown"),
            Some("::idl_pb_lyft::pb::lyft::money_::Breakdown")
        );
    }

    #[test]
    fn test_extern_path_enum() {
        let files = [make_file(
            "common.proto",
            "my.common",
            vec![],
            vec![enum_desc("Status")],
        )];
        let config = CodeGenConfig {
            extern_paths: vec![(".my.common".into(), "::common_protos".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(
            ctx.rust_type(".my.common.Status"),
            Some("::common_protos::Status")
        );
    }

    #[test]
    fn test_extern_path_does_not_affect_other_packages() {
        let files = [
            make_file("common.proto", "my.common", vec![msg("SharedMsg")], vec![]),
            make_file(
                "service.proto",
                "my.service",
                vec![msg("MyService")],
                vec![],
            ),
        ];
        let config = CodeGenConfig {
            extern_paths: vec![(".my.common".into(), "::common_protos".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Extern type uses absolute path.
        assert_eq!(
            ctx.rust_type(".my.common.SharedMsg"),
            Some("::common_protos::SharedMsg")
        );
        // Non-extern type uses normal package-derived path.
        assert_eq!(
            ctx.rust_type(".my.service.MyService"),
            Some("my::service::MyService")
        );
    }

    #[test]
    fn test_extern_path_relative_returns_absolute() {
        // When an extern type is referenced from another package,
        // rust_type_relative should return the full absolute path.
        let files = [
            make_file("common.proto", "my.common", vec![msg("SharedMsg")], vec![]),
            make_file(
                "service.proto",
                "my.service",
                vec![msg("MyService")],
                vec![],
            ),
        ];
        let config = CodeGenConfig {
            extern_paths: vec![(".my.common".into(), "::common_protos".into())],
            ..Default::default()
        };
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Cross-package reference to extern type: absolute path.
        assert_eq!(
            ctx.rust_type_relative(".my.common.SharedMsg", "my.service", 0),
            Some("::common_protos::SharedMsg".into())
        );
    }

    // ── is_enum_closed tests ──────────────────────────────────────────────

    #[test]
    fn test_is_enum_closed_proto3_default_open() {
        let files = [make_file("a.proto", "p", vec![], vec![enum_desc("E")])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // proto3 default (make_file has no syntax = proto2/implicit)
        // actually make_file doesn't set syntax, so it's proto2 default...
        // proto2 default is CLOSED.
        assert_eq!(ctx.is_enum_closed(".p.E"), Some(true));
    }

    #[test]
    fn test_is_enum_closed_editions_default_open() {
        let files = [editions_file("a.proto", "p", vec![], vec![enum_desc("E")])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // Edition 2023 default is OPEN.
        assert_eq!(ctx.is_enum_closed(".p.E"), Some(false));
    }

    #[test]
    fn test_is_enum_closed_per_enum_override() {
        // This is THE bug: enum with `option features.enum_type = CLOSED`
        // in an otherwise-open editions file must be detected as closed.
        let files = [editions_file(
            "a.proto",
            "p",
            vec![],
            vec![enum_desc("Open"), enum_with_closed_feature("Closed")],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.is_enum_closed(".p.Open"), Some(false));
        assert_eq!(ctx.is_enum_closed(".p.Closed"), Some(true));
    }

    #[test]
    fn test_is_enum_closed_nested_per_enum_override() {
        // Feature resolution through file → message → enum.
        let files = [editions_file(
            "a.proto",
            "p",
            vec![msg_with_nested_and_enums(
                "M",
                vec![],
                vec![enum_with_closed_feature("Inner")],
            )],
            vec![],
        )];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        assert_eq!(ctx.is_enum_closed(".p.M.Inner"), Some(true));
    }

    #[test]
    fn test_is_enum_closed_unknown_enum_returns_none() {
        let files = [editions_file("a.proto", "p", vec![], vec![])];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::new(&files, &config, &config.extern_paths);
        // extern_path or missing enum → None (caller falls back).
        assert_eq!(ctx.is_enum_closed(".other.Unknown"), None);
    }

    #[test]
    fn test_for_generate_auto_injects_wkt_mapping() {
        // for_generate() must produce the same type_map as generate() uses
        // internally — including the auto-injected WKT extern_path.
        let ts_msg = DescriptorProto {
            name: Some("Timestamp".into()),
            ..Default::default()
        };
        let files = [FileDescriptorProto {
            name: Some("google/protobuf/timestamp.proto".into()),
            package: Some("google.protobuf".into()),
            syntax: Some("proto3".into()),
            message_type: vec![ts_msg],
            ..Default::default()
        }];
        let config = CodeGenConfig::default();
        // Not generating the WKT file itself → auto-mapping should kick in.
        let ctx = CodeGenContext::for_generate(&files, &["other.proto".into()], &config);
        assert_eq!(
            ctx.rust_type(".google.protobuf.Timestamp"),
            Some("::buffa_types::google::protobuf::Timestamp"),
            "WKT auto-mapping must be applied via for_generate"
        );
    }

    #[test]
    fn test_for_generate_suppresses_wkt_when_generating_wkt() {
        // When files_to_generate includes a google.protobuf file (building
        // buffa-types itself), the WKT auto-mapping must NOT be applied.
        let ts_msg = DescriptorProto {
            name: Some("Timestamp".into()),
            ..Default::default()
        };
        let files = [FileDescriptorProto {
            name: Some("google/protobuf/timestamp.proto".into()),
            package: Some("google.protobuf".into()),
            syntax: Some("proto3".into()),
            message_type: vec![ts_msg],
            ..Default::default()
        }];
        let config = CodeGenConfig::default();
        let ctx = CodeGenContext::for_generate(
            &files,
            &["google/protobuf/timestamp.proto".into()],
            &config,
        );
        // No extern mapping → local-package path.
        assert_eq!(
            ctx.rust_type(".google.protobuf.Timestamp"),
            Some("google::protobuf::Timestamp")
        );
    }

    // ── matching_attributes tests ──────────────────────────────────────

    #[test]
    fn test_matching_attributes_catchall() {
        // "." matches every type.
        let attrs = vec![(".".into(), "#[derive(Foo)]".into())];
        let result = CodeGenContext::matching_attributes(&attrs, "my.pkg.MyMessage").unwrap();
        assert!(result.to_string().contains("derive"));
    }

    #[test]
    fn test_matching_attributes_exact_match() {
        let attrs = vec![(".my.pkg.MyMessage".into(), "#[derive(Bar)]".into())];
        let result = CodeGenContext::matching_attributes(&attrs, "my.pkg.MyMessage").unwrap();
        assert!(result.to_string().contains("derive"));
    }

    #[test]
    fn test_matching_attributes_package_prefix() {
        let attrs = vec![(".my.pkg".into(), "#[derive(Baz)]".into())];
        let result = CodeGenContext::matching_attributes(&attrs, "my.pkg.MyMessage").unwrap();
        assert!(result.to_string().contains("derive"));
    }

    #[test]
    fn test_matching_attributes_no_partial_segment_match() {
        // ".my.pk" must not match ".my.pkg" (partial segment).
        let attrs = vec![(".my.pk".into(), "#[derive(Bad)]".into())];
        let result = CodeGenContext::matching_attributes(&attrs, "my.pkg.MyMessage").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_matching_attributes_no_match() {
        let attrs = vec![(".other.pkg".into(), "#[derive(Nope)]".into())];
        let result = CodeGenContext::matching_attributes(&attrs, "my.pkg.MyMessage").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_matching_attributes_multiple_accumulate() {
        // All matching entries are emitted, not just the first.
        let attrs = vec![
            (".".into(), "#[derive(A)]".into()),
            (".my.pkg".into(), "#[derive(B)]".into()),
        ];
        let result = CodeGenContext::matching_attributes(&attrs, "my.pkg.MyMessage").unwrap();
        let s = result.to_string();
        assert!(s.contains("A") && s.contains("B"));
    }

    #[test]
    fn test_matching_attributes_invalid_attr_errors() {
        // Unparseable attributes surface as a hard error so the user sees
        // the problem at build time rather than a silently-dropped attribute.
        let attrs = vec![(".".into(), "not valid {{{{".into())];
        let err = CodeGenContext::matching_attributes(&attrs, "my.pkg.Msg").unwrap_err();
        assert!(matches!(
            err,
            crate::CodeGenError::InvalidCustomAttribute { .. }
        ));
    }

    #[test]
    fn test_matches_proto_prefix_catchall() {
        assert!(matches_proto_prefix(".", ".anything.here"));
        assert!(matches_proto_prefix(".", "."));
    }

    #[test]
    fn test_matches_proto_prefix_segment_boundary() {
        // Segment-aware: ".my.pk" must not match ".my.pkg".
        assert!(!matches_proto_prefix(".my.pk", ".my.pkg.Msg"));
        // But full-segment prefix match does.
        assert!(matches_proto_prefix(".my.pkg", ".my.pkg.Msg"));
    }
}

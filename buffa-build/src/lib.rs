//! Build-time integration for buffa.
//!
//! Use this crate in your `build.rs` to compile `.proto` files into Rust code
//! at build time. Parses `.proto` files into a `FileDescriptorSet` (via
//! `protoc` or `buf`), then uses `buffa-codegen` to generate Rust source.
//!
//! # Example
//!
//! ```rust,ignore
//! // build.rs
//! fn main() {
//!     buffa_build::Config::new()
//!         .files(&["proto/my_service.proto"])
//!         .includes(&["proto/"])
//!         .compile()
//!         .unwrap();
//! }
//! ```
//!
//! # Requirements
//!
//! By default, requires `protoc` on the system PATH (or set via the `PROTOC`
//! environment variable) — the same as `prost-build` and `tonic-build`.
//!
//! If `protoc` is unavailable or outdated on your platform, `buf` can be
//! used instead — see [`Config::use_buf()`]. Alternatively, feed a
//! pre-compiled descriptor set via [`Config::descriptor_set()`].

use std::path::{Path, PathBuf};
use std::process::Command;

use buffa::Message;
use buffa_codegen::generated::descriptor::FileDescriptorSet;

use buffa_codegen::CodeGenConfig;

/// How to produce a `FileDescriptorSet` from `.proto` files.
#[derive(Debug, Clone, Default)]
enum DescriptorSource {
    /// Invoke `protoc` (default). Requires `protoc` on PATH or `PROTOC` env var.
    #[default]
    Protoc,
    /// Invoke `buf build --as-file-descriptor-set`. Requires `buf` on PATH.
    Buf,
    /// Read a pre-built `FileDescriptorSet` from a file.
    Precompiled(PathBuf),
}

/// Builder for configuring and running protobuf compilation.
pub struct Config {
    files: Vec<PathBuf>,
    includes: Vec<PathBuf>,
    out_dir: Option<PathBuf>,
    codegen_config: CodeGenConfig,
    descriptor_source: DescriptorSource,
    /// If set, generate a module-tree include file with this name in the
    /// output directory. Users can then `include!` this single file instead
    /// of manually setting up `pub mod` nesting.
    include_file: Option<String>,
}

impl Config {
    /// Create a new configuration with defaults.
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            includes: Vec::new(),
            out_dir: None,
            codegen_config: CodeGenConfig::default(),
            descriptor_source: DescriptorSource::default(),
            include_file: None,
        }
    }

    /// Add `.proto` files to compile.
    #[must_use]
    pub fn files(mut self, files: &[impl AsRef<Path>]) -> Self {
        self.files
            .extend(files.iter().map(|f| f.as_ref().to_path_buf()));
        self
    }

    /// Add include directories for protoc to search for imports.
    #[must_use]
    pub fn includes(mut self, includes: &[impl AsRef<Path>]) -> Self {
        self.includes
            .extend(includes.iter().map(|i| i.as_ref().to_path_buf()));
        self
    }

    /// Set the output directory for generated files.
    /// Defaults to `$OUT_DIR` if not set.
    #[must_use]
    pub fn out_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(dir.into());
        self
    }

    /// Enable or disable view type generation (default: true).
    #[must_use]
    pub fn generate_views(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_views = enabled;
        self
    }

    /// Enable or disable serde Serialize/Deserialize derive generation
    /// for generated message structs and enum types (default: false).
    ///
    /// When enabled, the downstream crate must depend on `serde` and enable
    /// the `buffa/json` feature for the runtime helpers.
    #[must_use]
    pub fn generate_json(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_json = enabled;
        self
    }

    /// Enable or disable `#[derive(arbitrary::Arbitrary)]` on generated
    /// types (default: false).
    ///
    /// The derive is gated behind `#[cfg_attr(feature = "arbitrary", ...)]`
    /// so the downstream crate compiles with or without the feature enabled.
    #[must_use]
    pub fn generate_arbitrary(mut self, enabled: bool) -> Self {
        self.codegen_config.generate_arbitrary = enabled;
        self
    }

    /// Enable or disable unknown field preservation (default: true).
    ///
    /// When enabled (the default), unrecognized fields encountered during
    /// decode are stored and re-emitted on encode — essential for proxy /
    /// middleware services and round-trip fidelity across schema versions.
    ///
    /// **Disabling is primarily a memory optimization** (24 bytes/message for
    /// the `UnknownFields` Vec header), not a throughput one. When no unknown
    /// fields appear on the wire — the common case for schema-aligned
    /// services — decode and encode costs are effectively identical in
    /// either mode. Consider disabling for embedded / `no_std` targets or
    /// large in-memory collections of small messages.
    #[must_use]
    pub fn preserve_unknown_fields(mut self, enabled: bool) -> Self {
        self.codegen_config.preserve_unknown_fields = enabled;
        self
    }

    /// Honor `features.utf8_validation = NONE` by emitting `Vec<u8>` / `&[u8]`
    /// for such string fields instead of `String` / `&str` (default: false).
    ///
    /// When disabled (the default), all string fields map to `String` and
    /// UTF-8 is validated on decode — stricter than proto2 requires, but
    /// ergonomic and safe.
    ///
    /// When enabled, string fields with `utf8_validation = NONE` become
    /// `Vec<u8>` / `&[u8]`. Decode skips validation; the caller chooses
    /// whether to `std::str::from_utf8` (checked) or `from_utf8_unchecked`
    /// (trusted-input fast path). This is the only sound Rust mapping when
    /// strings may actually contain non-UTF-8 bytes.
    ///
    /// **Note for proto2 users**: proto2's default is `utf8_validation = NONE`,
    /// so enabling this turns ALL proto2 string fields into `Vec<u8>`. Use
    /// only for new code or when profiling identifies UTF-8 validation as a
    /// bottleneck (it can be 10%+ of decode CPU for string-heavy messages).
    ///
    /// **JSON note**: fields normalized to bytes serialize as base64 in JSON
    /// (the proto3 JSON encoding for `bytes`). Keep strict mapping disabled
    /// for fields that need JSON string interop with other implementations.
    #[must_use]
    pub fn strict_utf8_mapping(mut self, enabled: bool) -> Self {
        self.codegen_config.strict_utf8_mapping = enabled;
        self
    }

    /// Permit `option message_set_wire_format = true` on input messages.
    ///
    /// MessageSet is a legacy Google-internal wire format. Default: `false`
    /// (such messages produce a codegen error). Set to `true` only when
    /// compiling protos that interoperate with old Google-internal services.
    #[must_use]
    pub fn allow_message_set(mut self, enabled: bool) -> Self {
        self.codegen_config.allow_message_set = enabled;
        self
    }

    /// Declare an external type path mapping.
    ///
    /// Types under the given protobuf path prefix will reference the specified
    /// Rust module path instead of being generated. This allows shared proto
    /// packages to be compiled once in a dedicated crate and referenced from
    /// others.
    ///
    /// `proto_path` is a fully-qualified protobuf package path, e.g.,
    /// `".my.common"` or `"my.common"` (the leading dot is optional and will
    /// be added automatically). `rust_path` is the Rust module path where
    /// those types are accessible (e.g., `"::common_protos"`).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .extern_path(".my.common", "::common_protos")
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn extern_path(
        mut self,
        proto_path: impl Into<String>,
        rust_path: impl Into<String>,
    ) -> Self {
        let mut proto_path = proto_path.into();
        // Normalize: ensure the proto path is fully-qualified (leading dot).
        // Accept both ".my.package" and "my.package" for convenience.
        if !proto_path.starts_with('.') {
            proto_path.insert(0, '.');
        }
        self.codegen_config
            .extern_paths
            .push((proto_path, rust_path.into()));
        self
    }

    /// Configure `bytes` fields to use `bytes::Bytes` instead of `Vec<u8>`.
    ///
    /// Each path is a fully-qualified proto path prefix. Use `"."` to apply
    /// to all bytes fields, or specify individual field paths like
    /// `".my.pkg.MyMessage.data"`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// buffa_build::Config::new()
    ///     .bytes(&["."])  // all bytes fields use Bytes
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn use_bytes_type_in(mut self, paths: &[impl AsRef<str>]) -> Self {
        self.codegen_config
            .bytes_fields
            .extend(paths.iter().map(|p| p.as_ref().to_string()));
        self
    }

    /// Use `bytes::Bytes` for all `bytes` fields in all messages.
    ///
    /// This is a convenience for `.use_bytes_type_in(&["."])`. Use `.use_bytes_type_in(&[...])` with
    /// specific proto paths if you only want `Bytes` for certain fields.
    #[must_use]
    pub fn use_bytes_type(mut self) -> Self {
        self.codegen_config.bytes_fields.push(".".to_string());
        self
    }

    /// Use `buf build` instead of `protoc` for descriptor generation.
    ///
    /// `buf` is often easier to install and keep current than `protoc`
    /// (which many distros pin to old versions). This mode is intended for
    /// the **single-crate case**: a `buf.yaml` at the crate root defining
    /// the module layout.
    ///
    /// Requires `buf` on PATH and a `buf.yaml` at the crate root. The
    /// [`includes()`](Self::includes) setting is ignored — buf resolves
    /// imports via its own module configuration.
    ///
    /// Each path given to [`files()`](Self::files) must be **relative to its
    /// owning module's directory** (the `path:` value inside `buf.yaml`), not
    /// the crate root where `buf.yaml` itself lives. buf strips the module
    /// path when producing `FileDescriptorProto.name`, so for
    /// `modules: [{path: proto}]` and a file on disk at
    /// `proto/api/v1/service.proto`, the descriptor name is
    /// `api/v1/service.proto` — that is what `.files()` must contain.
    /// Multiple modules in one `buf.yaml` work fine; buf enforces that
    /// module-relative names are unique across the workspace.
    ///
    /// # Monorepo / multi-module setups
    ///
    /// For a workspace-root `buf.yaml` with many modules, this mode is a
    /// poor fit. Prefer running `buf generate` with the `protoc-gen-buffa`
    /// plugin and checking in the generated code, or use
    /// [`descriptor_set()`](Self::descriptor_set) with the output of
    /// `buf build --as-file-descriptor-set -o fds.binpb <module-path>`
    /// run as a pre-build step.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // buf.yaml (at crate root):
    /// //   version: v2
    /// //   modules:
    /// //     - path: proto
    /// //
    /// // build.rs:
    /// buffa_build::Config::new()
    ///     .use_buf()
    ///     .files(&["api/v1/service.proto"])  // relative to module root
    ///     .compile()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn use_buf(mut self) -> Self {
        self.descriptor_source = DescriptorSource::Buf;
        self
    }

    /// Use a pre-compiled `FileDescriptorSet` binary file as input.
    ///
    /// Skips invoking `protoc` or `buf` entirely. The file must contain a
    /// serialized `google.protobuf.FileDescriptorSet` (as produced by
    /// `protoc --descriptor_set_out` or `buf build --as-file-descriptor-set`).
    ///
    /// When using this, `.files()` specifies which proto files in the
    /// descriptor set to generate code for (matching by proto file name).
    #[must_use]
    pub fn descriptor_set(mut self, path: impl Into<PathBuf>) -> Self {
        self.descriptor_source = DescriptorSource::Precompiled(path.into());
        self
    }

    /// Generate a module-tree include file alongside the per-package `.rs`
    /// files.
    ///
    /// The include file contains nested `pub mod` declarations with
    /// `include!()` directives that assemble the generated code into a
    /// module hierarchy matching the protobuf package structure. Users can
    /// then include this single file instead of manually creating the
    /// module tree.
    ///
    /// The form of the emitted `include!` directives depends on whether
    /// [`out_dir`](Self::out_dir) was set:
    ///
    /// - **Default (`$OUT_DIR`)**: emits
    ///   `include!(concat!(env!("OUT_DIR"), "/foo.rs"))`, for use from
    ///   `build.rs` via `include!(concat!(env!("OUT_DIR"), "/<name>"))`.
    /// - **Explicit `out_dir`**: emits sibling-relative `include!("foo.rs")`,
    ///   for checking the generated code into the source tree and referencing
    ///   it as a module (e.g. `mod gen;`).
    ///
    /// # Example — `build.rs` / `$OUT_DIR`
    ///
    /// ```rust,ignore
    /// // build.rs
    /// buffa_build::Config::new()
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .include_file("_include.rs")
    ///     .compile()
    ///     .unwrap();
    ///
    /// // lib.rs
    /// include!(concat!(env!("OUT_DIR"), "/_include.rs"));
    /// ```
    ///
    /// # Example — checked-in source
    ///
    /// ```rust,ignore
    /// // codegen.rs (run manually, not from build.rs)
    /// buffa_build::Config::new()
    ///     .files(&["proto/my_service.proto"])
    ///     .includes(&["proto/"])
    ///     .out_dir("src/gen")
    ///     .include_file("mod.rs")
    ///     .compile()
    ///     .unwrap();
    ///
    /// // lib.rs
    /// mod gen;
    /// ```
    #[must_use]
    pub fn include_file(mut self, name: impl Into<String>) -> Self {
        self.include_file = Some(name.into());
        self
    }

    /// Compile proto files and generate Rust source.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `OUT_DIR` is not set and no `out_dir` was configured
    /// - `protoc` or `buf` cannot be found on `PATH` (when using those sources)
    /// - the proto compiler exits with a non-zero status (syntax errors,
    ///   missing imports, etc.)
    /// - a precompiled descriptor set file cannot be read
    /// - the descriptor set bytes cannot be decoded as a `FileDescriptorSet`
    /// - code generation fails (e.g. unsupported proto feature)
    /// - the output directory cannot be created or written to
    pub fn compile(self) -> Result<(), Box<dyn std::error::Error>> {
        // When out_dir is explicitly set, the include file should use
        // relative `include!("foo.rs")` paths (the index is a sibling of the
        // generated files). When defaulted to $OUT_DIR, keep the
        // `concat!(env!("OUT_DIR"), ...)` form so that
        // `include!(concat!(env!("OUT_DIR"), "/_include.rs"))` from src/
        // still resolves to absolute paths.
        let relative_includes = self.out_dir.is_some();
        let out_dir = self
            .out_dir
            .or_else(|| std::env::var("OUT_DIR").ok().map(PathBuf::from))
            .ok_or("OUT_DIR not set and no out_dir configured")?;

        // Produce a FileDescriptorSet from the configured source.
        let descriptor_bytes = match &self.descriptor_source {
            DescriptorSource::Protoc => invoke_protoc(&self.files, &self.includes)?,
            DescriptorSource::Buf => invoke_buf()?,
            DescriptorSource::Precompiled(path) => std::fs::read(path).map_err(|e| {
                format!("failed to read descriptor set '{}': {}", path.display(), e)
            })?,
        };
        let fds = FileDescriptorSet::decode_from_slice(&descriptor_bytes)
            .map_err(|e| format!("failed to decode FileDescriptorSet: {}", e))?;

        // Determine which files were explicitly requested.
        //
        // `FileDescriptorProto.name` contains the path relative to the proto
        // source root (protoc: `--proto_path`; buf: the module root). For
        // Precompiled and Buf mode, `.files()` are expected to already be
        // proto-relative names. For Protoc mode, strip the longest matching
        // include prefix.
        let files_to_generate: Vec<String> = if matches!(
            self.descriptor_source,
            DescriptorSource::Precompiled(_) | DescriptorSource::Buf
        ) {
            self.files
                .iter()
                .filter_map(|f| f.to_str().map(str::to_string))
                .collect()
        } else {
            self.files
                .iter()
                .map(|f| proto_relative_name(f, &self.includes))
                .filter(|s| !s.is_empty())
                .collect()
        };

        // Generate Rust source.
        let generated =
            buffa_codegen::generate(&fds.file, &files_to_generate, &self.codegen_config)?;

        // Build a map from generated file name to proto package for the
        // module tree generator.
        let file_to_package: std::collections::HashMap<String, String> = fds
            .file
            .iter()
            .map(|fd| {
                let proto_name = fd.name.as_deref().unwrap_or("");
                let rs_name = buffa_codegen::proto_path_to_rust_module(proto_name);
                let package = fd.package.as_deref().unwrap_or("").to_string();
                (rs_name, package)
            })
            .collect();

        // Write output files and collect (name, package) pairs.
        let mut output_entries: Vec<(String, String)> = Vec::new();
        for file in generated {
            let path = out_dir.join(&file.name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_if_changed(&path, file.content.as_bytes())?;
            let package = file_to_package.get(&file.name).cloned().unwrap_or_default();
            output_entries.push((file.name, package));
        }

        // Generate the include file if requested.
        if let Some(ref include_name) = self.include_file {
            let include_content = generate_include_file(&output_entries, relative_includes);
            let include_path = out_dir.join(include_name);
            write_if_changed(&include_path, include_content.as_bytes())?;
        }

        // Tell cargo to re-run if any proto file changes.
        //
        // For Buf mode, `self.files` are module-root-relative and cargo can't
        // stat them — use `buf ls-files` instead, which lists all workspace
        // protos with workspace-relative paths. This also catches changes to
        // transitively-imported protos (a gap in the Protoc mode, which only
        // watches explicitly-listed files).
        match self.descriptor_source {
            DescriptorSource::Buf => emit_buf_rerun_if_changed(),
            DescriptorSource::Protoc => {
                // Rerun if PROTOC changes (different binary may accept
                // protos the previous one rejected, e.g. newer editions).
                println!("cargo:rerun-if-env-changed=PROTOC");
                for proto_file in &self.files {
                    println!("cargo:rerun-if-changed={}", proto_file.display());
                }
            }
            DescriptorSource::Precompiled(ref path) => {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Write `content` to `path` only if the file doesn't already exist with
/// identical content. Avoids bumping timestamps on unchanged files, which
/// prevents unnecessary downstream recompilation.
fn write_if_changed(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Ok(existing) = std::fs::read(path) {
        if existing == content {
            return Ok(());
        }
    }
    std::fs::write(path, content)
}

/// Invoke `protoc` to produce a `FileDescriptorSet` (serialized bytes).
fn invoke_protoc(
    files: &[PathBuf],
    includes: &[PathBuf],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());

    let descriptor_file =
        tempfile::NamedTempFile::new().map_err(|e| format!("failed to create temp file: {}", e))?;
    let descriptor_path = descriptor_file.path().to_path_buf();

    let mut cmd = Command::new(&protoc);
    cmd.arg("--include_imports");
    cmd.arg("--include_source_info");
    cmd.arg(format!(
        "--descriptor_set_out={}",
        descriptor_path.display()
    ));

    for include in includes {
        cmd.arg(format!("--proto_path={}", include.display()));
    }

    for file in files {
        cmd.arg(file.as_os_str());
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run protoc ({}): {}", protoc, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("protoc failed: {}", stderr).into());
    }

    let bytes = std::fs::read(&descriptor_path)
        .map_err(|e| format!("failed to read descriptor set: {}", e))?;

    Ok(bytes)
}

/// Invoke `buf build` to produce a `FileDescriptorSet` (serialized bytes).
///
/// Requires a `buf.yaml` discoverable from the build script's cwd. Builds
/// the entire workspace — no `--path` filtering, because buf's `--path` flag
/// expects workspace-relative paths while `FileDescriptorProto.name` is
/// module-root-relative; passing user paths to both would be a contradiction.
/// Codegen filtering happens on our side via `files_to_generate` matching.
fn invoke_buf() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // buf build includes SourceCodeInfo by default (there's an
    // --exclude-source-info flag to disable it), so proto comments
    // propagate to generated code without an explicit opt-in here.
    let output = Command::new("buf")
        .arg("build")
        .arg("--as-file-descriptor-set")
        .arg("-o")
        .arg("-")
        .output()
        .map_err(|e| format!("failed to run buf (is it installed and on PATH?): {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(
            format!("buf build failed (is buf.yaml present at crate root?): {stderr}").into(),
        );
    }

    Ok(output.stdout)
}

/// Emit `cargo:rerun-if-changed` directives for a buf workspace.
///
/// Runs `buf ls-files` to discover all proto files with workspace-relative
/// paths (which cargo can stat). Also watches `buf.yaml` and `buf.lock`
/// (the latter only if it exists — cargo treats a missing rerun-if-changed
/// path as always-dirty). Failure is non-fatal: worst case cargo reruns
/// every build.
fn emit_buf_rerun_if_changed() {
    println!("cargo:rerun-if-changed=buf.yaml");
    if Path::new("buf.lock").exists() {
        println!("cargo:rerun-if-changed=buf.lock");
    }
    match Command::new("buf").arg("ls-files").output() {
        Ok(out) if out.status.success() => {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let path = line.trim();
                if !path.is_empty() {
                    println!("cargo:rerun-if-changed={path}");
                }
            }
        }
        _ => {
            // ls-files failed; cargo already knows about buf.yaml above.
            // If buf itself is missing, invoke_buf() will error clearly.
        }
    }
}

/// Convert a filesystem proto path to the name protoc uses in the descriptor.
///
/// `FileDescriptorProto.name` is relative to the `--proto_path` include
/// directory. This strips the longest matching include prefix; if no include
/// matches, returns the path as-is (not just file_name — that would break
/// nested proto directories).
fn proto_relative_name(file: &Path, includes: &[PathBuf]) -> String {
    // Longest prefix wins: a file under both "proto/" and "proto/vendor/"
    // should strip "proto/vendor/" for a correct relative name.
    let mut best: Option<&Path> = None;
    for include in includes {
        if let Ok(rel) = file.strip_prefix(include) {
            match best {
                Some(prev) if prev.as_os_str().len() <= rel.as_os_str().len() => {}
                _ => best = Some(rel),
            }
        }
    }
    best.unwrap_or(file).to_str().unwrap_or("").to_string()
}

/// Generate the content of an include file that assembles generated `.rs`
/// files into a nested module tree matching the protobuf package hierarchy.
///
/// Each generated file is named like `my.package.file_name.rs`. The package
/// segments become `pub mod` wrappers, and the file is `include!`d inside
/// the innermost module.
///
/// For example, files `["foo.bar.rs", "foo.baz.rs"]` produce:
/// ```text
/// pub mod foo {
///     #[allow(unused_imports)]
///     use super::*;
///     include!(concat!(env!("OUT_DIR"), "/foo.bar.rs"));
///     include!(concat!(env!("OUT_DIR"), "/foo.baz.rs"));
/// }
/// ```
///
/// When `relative` is true (the caller set [`Config::out_dir`] explicitly),
/// `include!` directives use bare sibling paths (`include!("foo.bar.rs")`)
/// instead of the `env!("OUT_DIR")` prefix, so the include file works when
/// checked into the source tree and referenced via `mod`.
fn generate_include_file(entries: &[(String, String)], relative: bool) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;

    fn escape_mod_name(name: &str) -> String {
        const KEYWORDS: &[&str] = &[
            "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
            "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
            "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
            "unsafe", "use", "where", "while", "async", "await", "dyn", "gen", "abstract",
            "become", "box", "do", "final", "macro", "override", "priv", "try", "typeof",
            "unsized", "virtual", "yield",
        ];
        if KEYWORDS.contains(&name) {
            if matches!(name, "self" | "super" | "Self" | "crate") {
                format!("{name}_")
            } else {
                format!("r#{name}")
            }
        } else {
            name.to_string()
        }
    }

    #[derive(Default)]
    struct ModNode {
        files: Vec<String>,
        children: BTreeMap<String, Self>,
    }

    let mut root = ModNode::default();
    for (file_name, package) in entries {
        let pkg_parts: Vec<&str> = if package.is_empty() {
            vec![]
        } else {
            package.split('.').collect()
        };
        let mut node = &mut root;
        for seg in &pkg_parts {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node.files.push(file_name.clone());
    }

    let mut out = String::new();
    writeln!(out, "// @generated by buffa-build. DO NOT EDIT.").unwrap();
    writeln!(out).unwrap();

    fn emit(out: &mut String, node: &ModNode, depth: usize, relative: bool) {
        let indent = "    ".repeat(depth);
        for file in &node.files {
            if relative {
                writeln!(out, r#"{indent}include!("{file}");"#).unwrap();
            } else {
                writeln!(
                    out,
                    r#"{indent}include!(concat!(env!("OUT_DIR"), "/{file}"));"#
                )
                .unwrap();
            }
        }
        for (name, child) in &node.children {
            let escaped = escape_mod_name(name);
            writeln!(
                out,
                "{indent}#[allow(non_camel_case_types, dead_code, unused_imports, \
                 clippy::derivable_impls, clippy::match_single_binding)]"
            )
            .unwrap();
            writeln!(out, "{indent}pub mod {escaped} {{").unwrap();
            writeln!(out, "{indent}    use super::*;").unwrap();
            emit(out, child, depth + 1, relative);
            writeln!(out, "{indent}}}").unwrap();
        }
    }

    emit(&mut out, &root, 0, relative);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_relative_name_strips_include() {
        let got = proto_relative_name(
            Path::new("proto/my/service.proto"),
            &[PathBuf::from("proto/")],
        );
        assert_eq!(got, "my/service.proto");
    }

    #[test]
    fn proto_relative_name_longest_prefix_wins() {
        // Overlapping includes: file under both proto/ and proto/vendor/.
        // Must strip the LONGER prefix for the correct relative name.
        let got = proto_relative_name(
            Path::new("proto/vendor/ext.proto"),
            &[PathBuf::from("proto/"), PathBuf::from("proto/vendor/")],
        );
        assert_eq!(got, "ext.proto");
        // Same with reversed include order.
        let got = proto_relative_name(
            Path::new("proto/vendor/ext.proto"),
            &[PathBuf::from("proto/vendor/"), PathBuf::from("proto/")],
        );
        assert_eq!(got, "ext.proto");
    }

    #[test]
    fn proto_relative_name_no_match_returns_full_path() {
        // Regression: previously fell back to file_name(), which stripped
        // directory components and broke descriptor_set() mode with nested
        // proto packages. Now returns the full path as-is.
        let got = proto_relative_name(Path::new("my/pkg/service.proto"), &[]);
        assert_eq!(got, "my/pkg/service.proto");
    }

    #[test]
    fn proto_relative_name_no_match_with_unrelated_includes() {
        let got = proto_relative_name(
            Path::new("src/my.proto"),
            &[PathBuf::from("other/"), PathBuf::from("third/")],
        );
        assert_eq!(got, "src/my.proto");
    }

    #[test]
    fn include_file_out_dir_mode_uses_env_var() {
        let entries = vec![
            ("foo.bar.rs".to_string(), "foo".to_string()),
            ("root.rs".to_string(), String::new()),
        ];
        let out = generate_include_file(&entries, false);
        assert!(
            out.contains(r#"include!(concat!(env!("OUT_DIR"), "/foo.bar.rs"));"#),
            "nested-package file should use env!(OUT_DIR): {out}"
        );
        assert!(
            out.contains(r#"include!(concat!(env!("OUT_DIR"), "/root.rs"));"#),
            "empty-package file should use env!(OUT_DIR): {out}"
        );
        assert!(!out.contains(r#"include!("foo.bar.rs")"#));
    }

    #[test]
    fn include_file_relative_mode_uses_sibling_paths() {
        let entries = vec![
            ("foo.bar.rs".to_string(), "foo".to_string()),
            ("root.rs".to_string(), String::new()),
        ];
        let out = generate_include_file(&entries, true);
        assert!(
            out.contains(r#"include!("foo.bar.rs");"#),
            "nested-package file should use relative path: {out}"
        );
        assert!(
            out.contains(r#"include!("root.rs");"#),
            "empty-package file should use relative path: {out}"
        );
        assert!(
            !out.contains("OUT_DIR"),
            "relative mode must not reference OUT_DIR: {out}"
        );
    }

    #[test]
    fn include_file_relative_mode_nested_packages() {
        // Two files in the same depth-2 package: verifies the relative flag
        // propagates through recursive emit() calls and both files land in
        // the same innermost mod.
        let entries = vec![
            ("a.b.one.rs".to_string(), "a.b".to_string()),
            ("a.b.two.rs".to_string(), "a.b".to_string()),
        ];
        let out = generate_include_file(&entries, true);
        // Both includes should appear once, at the same depth-2 indent,
        // inside a single `pub mod b { ... }`.
        let indent = "        "; // depth 2 = 8 spaces
        assert!(
            out.contains(&format!(r#"{indent}include!("a.b.one.rs");"#)),
            "first file at depth 2: {out}"
        );
        assert!(
            out.contains(&format!(r#"{indent}include!("a.b.two.rs");"#)),
            "second file at depth 2: {out}"
        );
        assert_eq!(
            out.matches("pub mod b {").count(),
            1,
            "both files share one `mod b`: {out}"
        );
        assert!(!out.contains("OUT_DIR"));
    }

    #[test]
    fn write_if_changed_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.rs");
        write_if_changed(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn write_if_changed_skips_identical_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.rs");
        std::fs::write(&path, b"content").unwrap();
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Sleep briefly so any write would produce a different mtime.
        std::thread::sleep(std::time::Duration::from_millis(50));

        write_if_changed(&path, b"content").unwrap();
        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn write_if_changed_overwrites_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.rs");
        std::fs::write(&path, b"old").unwrap();

        write_if_changed(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }
}

# Contributing to buffa

See [DESIGN.md](DESIGN.md) for the architectural overview and [README.md](README.md) for the workspace layout.

## Prerequisites

- **protoc v27+** — the test suite includes editions-syntax protos (`edition = "2023"`). The `editions_2024.proto` test requires protoc v30+ (where edition 2024 was stabilized) — it's skipped with a cargo warning on older versions. Ubuntu's apt `protobuf-compiler` (v21.12) is too old. Run `task install-protoc` to download the CI-matched version into `.local/` (requires `gh` authenticated), then use `task test-local`.

## Change Size

Keep each change to **≤ 250 lines net** (additions minus deletions, excluding test files) wherever possible. If a task naturally exceeds that, split it into focused, self-contained PRs or commits.

## Test Coverage

Every change must include unit tests. Target **≥ 80% line coverage** for new code. Reaching 100% is not required when the remaining paths would require artificial or contrived tests, but coverage gaps must be intentional and justified. Tests live in `#[cfg(test)]` modules colocated with the code they test; integration tests go under `tests/`.

## Rust Conventions

- Run `task lint` and `task test` before every commit (or `task verify` to also run the conformance suite).
- Prefer `thiserror` for library error types; avoid `.unwrap()` in non-test, non-provably-safe code.
- Every `unsafe` block requires a `// SAFETY:` comment explaining the invariant.
- Public API items require doc comments (`///`); include `# Errors` / `# Panics` / `# Safety` sections where applicable.
- Keep dependencies minimal; use Cargo feature flags to avoid pulling in unnecessary transitive deps.

## Code Generation (`buffa-codegen`)

All code generation uses `quote!` blocks rather than string manipulation. `prettyplease` formats the final `TokenStream` into readable Rust source.

Key rules:

- **Regular `//` comments** are not tokens and are silently dropped by `quote!`. Only use `quote!` for code structure. Inject comments as raw strings before or after the formatted output (e.g. the `// @generated` file header in `lib.rs`).
- **Doc comments** (`///`) survive because `quote!` treats them as `#[doc = "..."]` attributes. Use `let doc = format!("..."); quote! { #[doc = #doc] ... }` when the content is dynamic.
- **Identifiers** must go through `format_ident!` (or `Ident::new_raw` for Rust keywords). Never interpolate raw strings as identifiers.
- **Type paths** from the descriptor context (e.g. `"my_pkg::MyMessage"`) must be parsed with `syn::parse_str::<syn::Type>` before interpolation into `quote!`.
- The pipeline in `lib.rs` is: accumulate `TokenStream` → `syn::parse2::<syn::File>` → `prettyplease::unparse` → prepend file-header string.
- **`no_std`-safe type paths**: Generated code must compile in both `std` and `no_std` contexts. `String`, `Vec`, and `Box` are **not** in the `no_std` prelude (even with `extern crate alloc`) — they must always be emitted as `::buffa::alloc::string::String`, `::buffa::alloc::vec::Vec`, `::buffa::alloc::boxed::Box`. Use the `ImportResolver` methods (`resolver.string()`, `resolver.vec()`, `resolver.boxed()`) which handle this. Only `core` prelude types (`Option`, `Result`, `Default`, etc.) can be emitted as bare names. See `imports.rs` for details.

## Conformance Tests

The protobuf conformance suite runs via Docker. Use `task conformance` (or `task verify` to run lint + tests + conformance). It requires Docker and uses the pre-built tools image `ghcr.io/anthropics/buffa/tools:v33.5` which bundles `conformance_test_runner` (from protobuf v33.5) and the test `.proto` files.

**If the tools image pull fails** (403 Forbidden from GHCR), build it locally first:

```bash
task tools-image-local   # builds for local platform only, ~5 min
task conformance         # now uses the locally-built image
```

**Understanding the output**: The conformance runner executes three runs
(std, no_std, via-view), each producing two suites:

1. Binary + JSON suite — expects thousands of successes (~5500 std, ~5500 no_std, ~2800 via-view — view mode skips JSON)
2. Text format suite — always `0 successes, 883 skipped` (text format is not supported)

So a healthy run shows **6 `CONFORMANCE SUITE PASSED` lines**. The `883 skipped` in the text format suites is expected and correct.

The Dockerfile builds **two binaries**: one with default features (std) and one with `--no-default-features` (no_std). The via-view run reuses the std binary with `BUFFA_VIA_VIEW=1` set, routing binary input through `decode_view → to_owned_message → encode` to verify owned/view decoder parity.

**Expected failures** are listed in `conformance/known_failures.txt` (std), `conformance/known_failures_nostd.txt` (no_std), and `conformance/known_failures_view.txt` (via-view). When a previously-failing test starts passing, remove it from the relevant file; when a new test is expected to fail, add it.

**Capturing output**: To save per-run logs for analysis, mount a directory and set `CONFORMANCE_OUT`:

```bash
docker run --rm -v /tmp/conf:/out -e CONFORMANCE_OUT=/out buffa-conformance
# logs: /tmp/conf/conformance-{std,nostd,view}.log
```

**Upgrading the protobuf version**: bump `TOOLS_IMAGE` in `Taskfile.yml` and `PROTOC_VERSION` in `.github/workflows/ci.yml`, then:

```bash
task tools-image               # rebuild and push the multi-arch tools image
task vendor-bootstrap-protos   # re-fetch buffa-descriptor/protos/ from the new release tag
task gen-bootstrap-types       # regenerate checked-in descriptor types
```

Commit the refreshed `buffa-descriptor/protos/` and `buffa-descriptor/src/generated/` alongside the version bump.

## Checked-In Generated Code

Three sets of generated code are checked into the repo and **must be regenerated** whenever codegen output changes (e.g. changes to `imports.rs`, `message.rs`, `oneof.rs`, etc.):

1. **Bootstrap descriptor types** (`buffa-descriptor/src/generated/`): Used by codegen itself to parse `.proto` descriptors. Regenerate with `task gen-bootstrap-types`. The source protos are vendored in `buffa-descriptor/protos/` (pinned; refresh with `task vendor-bootstrap-protos` when bumping the protobuf version), so output is independent of your local protoc's bundled includes — only a protoc binary ≥ v27 is needed. Only needs regeneration when a codegen change affects the descriptor types themselves — most changes don't.

2. **Well-known types** (`buffa-types/src/generated/`): `Timestamp`, `Duration`, `Any`, `Struct`/`Value`, `FieldMask`, `Empty`, wrappers. Checked in (rather than generated at build time) so that consumers of `buffa-types` don't need `protoc` or the `buffa-build`/`buffa-codegen` toolchain. Regenerate with `task gen-wkt-types`. The WKT `.proto` sources are vendored in `buffa-types/protos/` (not read from the protoc installation) so the output is pinned. **This is the one most likely to need regeneration** — WKTs use views, unknown-field preservation, and the `arbitrary` derive, so almost any codegen output-format change touches them. If in doubt, run it and check `git status`.

3. **Logging example** (`examples/logging/src/gen/`): Regenerate with `task gen-logging-example` (requires `buf` on PATH).

CI (`check-generated-code` job) will fail if checked-in generated code is stale.

## Cross-Target Checks

Run `task install-targets` to install the additional rustup targets needed by cross-target tasks. The targets are:

- `i686-unknown-linux-gnu` — 32-bit x86 Linux (for `task check-32bit` / `task test-32bit`; `test-32bit` also needs `gcc-multilib`)
- `thumbv7em-none-eabihf` — bare-metal ARM Cortex-M4 (for the second step of `task check-nostd`)

The tasks have preconditions that print a clear error if the targets are missing.

## Continuous Integration

GitHub Actions CI (`.github/workflows/ci.yml`) runs on every push to `main` and on all pull requests. Jobs:

- **lint-and-test** — clippy + `cargo test --workspace` on stable
- **lint-markdown** — markdownlint over all `*.md` (config: `.markdownlint.json`)
- **msrv-check** — `cargo check --workspace` on Rust 1.85
- **check-nostd** — no_std (host + bare-metal ARM) and 32-bit compilation checks
- **check-generated-code** — regenerates bootstrap descriptor types and fails if the checked-in code is stale
- **conformance** — builds the tools and conformance Docker images, runs the full protobuf conformance suite

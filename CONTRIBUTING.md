# Contributing to buffa

See [DESIGN.md](DESIGN.md) for the architectural overview and [README.md](README.md) for the workspace layout.

## Prerequisites

- **Rust toolchain** — pinned by `rust-toolchain.toml`; rustup installs it automatically on the first `cargo` invocation. This keeps local `cargo fmt`/`cargo clippy` output identical to the CI lint job (rustfmt formatting drifts across releases, so an unpinned toolchain can disagree with CI). Bump the pin together with the `lint-and-test` toolchain in `.github/workflows/ci.yml`.
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
- **Re-exports for codegen**: `buffa` re-exports `alloc`, `bytes`, and (under the `json` feature) `serde_json` as `#[doc(hidden)]` items so generated code can reference `::buffa::alloc::*`, `::buffa::bytes::*`, and `::buffa::serde_json::*` without the consumer crate declaring those deps. These re-exports are load-bearing for every downstream crate's build — do not remove them or change their visibility. `serde` is the deliberate exception: the `#[derive(::serde::Serialize)]` macro emits `extern crate serde as _serde;`, so consumers of `json=true` codegen must depend on `serde` directly regardless.

## Conformance Tests

The protobuf conformance suite runs via Docker. Use `task conformance` (or `task verify` to run lint + tests + conformance). It requires Docker and uses the pre-built tools image `ghcr.io/anthropics/buffa/tools:v33.5` which bundles `conformance_test_runner` (from protobuf v33.5) and the test `.proto` files.

**If the tools image pull fails** (403 Forbidden from GHCR), build it locally first:

```bash
task tools-image-local   # builds for local platform only, ~5 min
task conformance         # now uses the locally-built image
```

**Without Docker**: the suite can also run natively. `conformance_test_runner`
talks to the testee over stdin/stdout pipes, so no container plumbing is
needed — the runner just has to be built from protobuf source once:

```bash
task conformance-tools-local   # one-time: cmake-builds the runner into .local/bin,
                               # populates conformance/protos/ (~10-20 min)
task conformance-local         # seven runs, same failure lists as the Docker path
```

Requires cmake, a C++ toolchain, and protoc v30+ on PATH or `$PROTOC`
(`task install-protoc`). Set `CONFORMANCE_OUT=<dir>` to tee per-run logs.

**Understanding the output**: The conformance runner executes seven runs
(std, no_std, via-view, via-lazy, view-json, via-reflect, via-vtable), each
producing two suites:

1. Binary + JSON suite — expects thousands of successes (~5500 std, ~5500 no_std). The via-view and via-lazy runs only handle binary→binary (~2800); the view-json, via-reflect, and via-vtable runs handle binary→JSON (and via-reflect also binary→binary).
2. Text format suite — 883 successes for std and no_std (the full suite); via-view, via-lazy, view-json, via-reflect, and via-vtable show `0 successes, 883 skipped` (those modes have no `TextFormat` path).

So a healthy run shows **14 `CONFORMANCE SUITE PASSED` lines**.

The Dockerfile builds **two binaries**: one with default features (std) and one with `--no-default-features` (no_std). The std binary is reused for the view/reflect runs by setting an env var:

- **via-view** (`BUFFA_VIA_VIEW=1`) — binary input through `decode_view → to_owned_message → encode`, verifying owned/view decoder parity.
- **via-lazy** (`BUFFA_VIA_LAZY=1`) — binary input through `decode_lazy → to_owned_message → encode` on the lazy view family (`lazy_views(true)`), verifying the lazy decoder (record arms, fragment merge, budget capture) against the corpus.
- **view-json** (`BUFFA_VIEW_JSON=1`) — binary→JSON through `decode_view → serde_json::to_string(&view)`, verifying the generated view `Serialize` impls (and the hand-written WKT view `Serialize` impls in `buffa-types`).
- **via-reflect** (`BUFFA_VIA_REFLECT=1`) — binary/JSON I/O through `DynamicMessage`'s descriptor-driven codec and reflective serde, verifying the runtime reflection codec independently of any generated type.
- **via-vtable** (`BUFFA_VIA_VTABLE=1`) — binary→JSON: decode the view, walk its vtable `ReflectMessage` surface to rebuild a `DynamicMessage`, then serialize to JSON. Verifies the generated `impl ReflectMessage for FooView`. It reuses `DynamicMessage`'s JSON serializer (which passes the corpus cleanly under via-reflect), so any failure isolates a bug in the vtable `get`/`has`/`for_each_set` surface. Requires the conformance crate's `reflect` feature, so it is absent from the no_std binary.

**Expected failures** are listed in `conformance/known_failures.txt` (std binary+JSON), `conformance/known_failures_nostd.txt` (no_std binary+JSON), `conformance/known_failures_view.txt` (via-view), `conformance/known_failures_lazy.txt` (via-lazy), `conformance/known_failures_view_json.txt` (view-json), `conformance/known_failures_reflect.txt` (via-reflect), `conformance/known_failures_view_vtable.txt` (via-vtable), and `conformance/known_failures_text.txt` (text format — shared between std and no_std; currently empty). The text list is passed via `--text_format_failure_list` since the runner validates each suite's list independently. When a previously-failing test starts passing, remove it from the relevant file; when a new test is expected to fail, add it.

**Capturing output**: To save per-run logs for analysis, mount a directory and set `CONFORMANCE_OUT`:

```bash
docker run --rm -v /tmp/conf:/out -e CONFORMANCE_OUT=/out buffa-conformance
# logs: /tmp/conf/conformance-{std,nostd,view,lazy,view-json,reflect,vtable}.log
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

`task check-nostd` adds the bare-metal `thumbv7em-none-eabihf` target on demand (it runs `rustup target add` itself, which is idempotent), so it needs no separate setup.

For the 32-bit tasks, run `task install-targets` first to install the additional rustup target:

- `i686-unknown-linux-gnu` — 32-bit x86 Linux (for `task check-32bit` / `task test-32bit`; `test-32bit` also needs `gcc-multilib`)

`task install-targets` also installs `thumbv7em-none-eabihf` for convenience; the `check-32bit` / `test-32bit` tasks have preconditions that print a clear error if the 32-bit target is missing.

## Continuous Integration

GitHub Actions CI (`.github/workflows/ci.yml`) runs on every push to `main` and on all pull requests. Jobs:

- **lint-and-test** — clippy + `cargo test --workspace` on stable
- **lint-markdown** — markdownlint over all `*.md` (config: `.markdownlint.json`)
- **msrv-check** — `cargo check --workspace` on Rust 1.75 (the declared `rust-version`)
- **check-nostd** — no_std (host + bare-metal ARM) and 32-bit compilation checks
- **check-generated-code** — regenerates bootstrap descriptor types and fails if the checked-in code is stale
- **conformance** — builds the tools and conformance Docker images, runs the full protobuf conformance suite

## Benchmarks

`task bench` runs the combined benchmark suite for the dev loop — all message shapes in one binary, with criterion comparing against your previous run. `task bench-prost` / `task bench-prost-bytes` run the prost equivalents on the same machine for buffa-vs-prost comparison.

For the per-shape performance history (`benchmarks/history/`), each message is measured **in isolation** — built with only its own decoder compiled — so that adding or removing a benchmark message cannot perturb another's numbers through the compiler's global inlining decisions. Run one shape isolated with `task bench-iso -- <message>` (for example `task bench-iso -- media_frame`); it builds only that message at the reproducible, layout-normalized profile (`lto=true, codegen-units=1`, plus 64-byte block alignment via `-align-all-nofallthru-blocks=6`, which removes the build-time code-layout lottery — see `benchmarks/history/annotations.md`). The available shapes are the per-message features in `benchmarks/buffa/Cargo.toml`.

Isolation needs one proto file per message, so each shape is also defined under `benchmarks/proto/iso/`. These mirror the message blocks in `bench_messages.proto`, which stays the single shape source for every other consumer (prost, the cross-implementation harnesses, dataset generation). `task check-iso-protos` asserts the two stay field-identical — run it after editing any benchmark message.

### Reproducible benchmark roots per release

The history is reproducible because every release has a committed branch carrying the exact harness used to measure it. **When you tag a release `vX.Y.Z`, create and push `historical-benchmark/vX.Y.Z` from that tag:**

```sh
git branch historical-benchmark/vX.Y.Z vX.Y.Z
git push origin historical-benchmark/vX.Y.Z
```

Releases cut from `main` already carry the isolated harness, so this branch is just a stable pointer to measure from. The pre-isolation back-catalogue (v0.1.0–v0.7.1) has the harness retrofitted onto each of these branches instead. Either way, `historical-benchmark/*` is the source of truth for rebuilding any cell of the per-shape history — see `benchmarks/history/DESIGN.md`.

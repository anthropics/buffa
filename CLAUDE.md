# buffa — Claude Code Instructions

@CONTRIBUTING.md

See [DESIGN.md](DESIGN.md) for the architectural overview.

## After Changing Codegen Output

If a change to `buffa-codegen` (notably `message.rs`, `impl_message.rs`, `view.rs`, `oneof.rs`, `enumeration.rs`, `imports.rs`) affects generated output, you **must** regenerate checked-in code before committing, or CI (`check-generated-code`) will fail:

```bash
task gen-wkt-types          # buffa-types/src/generated/ — WKTs for consumer use
task gen-bootstrap-types    # buffa-descriptor/src/generated/ — only if the change affects descriptor types
```

Most codegen changes don't touch descriptor-specific paths, so `gen-wkt-types` is usually sufficient.

A quick check: `git status` after `task lint` — if `buffa-types/src/generated/` shows as modified, you forgot to commit the regen.

## Pre-Commit Code Review

Before producing a commit, run the `rust-code-reviewer` agent (defined in `.claude/agents/rust-code-reviewer.md`) and address all **Critical**, **High**, and **Medium** findings. For **Low** / advisory findings, flag these to the user to decide if they are worth addressing or can be ignored.

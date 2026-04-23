#!/usr/bin/env python3
"""Generate a lib.rs module tree from buffa-generated .rs files.

Each generated file is named like `google.api.expr.v1alpha1.checked.rs`.
The dots represent the package + file structure. We need to build a nested
module tree where each package segment is a `pub mod` and the leaf
includes the generated file.

For example, `google.api.expr.v1alpha1.checked.rs` becomes:
    pub mod google {
        pub mod api {
            pub mod expr {
                pub mod v1alpha1 {
                    include!("gen/google.api.expr.v1alpha1.checked.rs");
                    // ... other files in this package
                }
            }
        }
    }

Multiple files in the same package are included in the same module.
"""

import sys
import os
from collections import defaultdict
from pathlib import Path

RUST_KEYWORDS = {
    "as", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
    "super", "trait", "true", "type", "unsafe", "use", "where", "while",
    "async", "await", "dyn", "gen",
    "abstract", "become", "box", "do", "final", "macro", "override", "priv",
    "try", "typeof", "unsized", "virtual", "yield",
}

# Keywords that can't use r# — use _ suffix instead.
NON_RAW_KEYWORDS = {"self", "super", "Self", "crate"}


def escape_ident(name: str) -> str:
    """Escape a Rust keyword for use as a module name."""
    if name in RUST_KEYWORDS:
        if name in NON_RAW_KEYWORDS:
            return f"{name}_"
        return f"r#{name}"
    return name


def main():
    if len(sys.argv) < 2:
        print("Usage: gen_lib_rs.py <gen_dir> [include_prefix]", file=sys.stderr)
        print("  include_prefix: path prefix for include! directives (default: 'gen/')", file=sys.stderr)
        sys.exit(1)

    gen_dir = Path(sys.argv[1])
    include_prefix = sys.argv[2] if len(sys.argv) > 2 else "gen/"

    # Files to exclude from compilation (known codegen limitations).
    # These are still generated successfully but produce Rust that doesn't
    # compile due to recursive type boxing not being implemented yet.
    exclude_files = set()
    exclude_path = gen_dir.parent / "exclude_from_compile.txt"
    if exclude_path.exists():
        for line in exclude_path.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                exclude_files.add(line)
    # Only the per-package `<pkg>.mod.rs` stitchers need wiring; the
    # per-proto content files (`*.rs`, `*.__view.rs`, …) are reached via
    # `include!` from the stitcher.
    rs_files = sorted(gen_dir.glob("*.mod.rs"))

    if not rs_files:
        print("No .mod.rs files found in", gen_dir, file=sys.stderr)
        sys.exit(1)

    # `<pkg>.mod.rs` → stem `<pkg>.mod` → parts[:-1] = package segments.
    packages = defaultdict(list)
    excluded = []
    for rs_file in rs_files:
        if rs_file.name in exclude_files:
            excluded.append(rs_file.name)
            continue
        stem = rs_file.stem  # e.g. "google.api.expr.v1alpha1.mod"
        parts = stem.split(".")
        pkg = tuple(parts[:-1])
        packages[pkg].append(rs_file.name)

    if excluded:
        print(f"Excluded {len(excluded)} files: {', '.join(excluded)}",
              file=sys.stderr)

    # Build a tree structure.
    tree = {}
    for pkg, files in packages.items():
        node = tree
        for seg in pkg:
            if seg not in node:
                node[seg] = {"__children": {}, "__files": []}
            node = node[seg]["__children"]
        # We're past all segments; go back to the last node.
        # Actually, let me restructure: the files go on the package node.
        pass

    # Simpler approach: build the tree directly.
    tree = {"__files": [], "__children": {}}

    for pkg, files in packages.items():
        node = tree
        for seg in pkg:
            if seg not in node["__children"]:
                node["__children"][seg] = {"__files": [], "__children": {}}
            node = node["__children"][seg]
        node["__files"].extend(files)

    # Generate lib.rs.
    lines = [
        "// @generated — do not edit.",
        "// Module tree for googleapis stress test compilation.",
        "",
        "#![allow(non_camel_case_types, dead_code, unused_imports)]",
        "",
    ]

    def emit(node, indent=0):
        prefix = "    " * indent
        for filename in sorted(node["__files"]):
            lines.append(f'{prefix}include!("{include_prefix}{filename}");')
        for seg in sorted(node["__children"].keys()):
            child = node["__children"][seg]
            escaped = escape_ident(seg)
            lines.append(f"{prefix}pub mod {escaped} {{")
            lines.append(f"{prefix}    #[allow(unused_imports)]")
            lines.append(f"{prefix}    use super::*;")
            emit(child, indent + 1)
            lines.append(f"{prefix}}}")

    emit(tree)
    print("\n".join(lines))


if __name__ == "__main__":
    main()

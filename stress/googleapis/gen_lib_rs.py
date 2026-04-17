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
    rs_files = sorted(gen_dir.glob("*.rs"))

    if not rs_files:
        print("No .rs files found in", gen_dir, file=sys.stderr)
        sys.exit(1)

    # Each proto file produces five output siblings:
    #   - `<stem>.rs`                 (owned tree)
    #   - `<stem>.__view.rs`          (view-tree contents, inside `pub mod view`)
    #   - `<stem>.__ext.rs`           (ext-tree contents, inside `pub mod ext`)
    #   - `<stem>.__oneofs.rs`        (owned oneofs, inside `pub mod oneofs`)
    #   - `<stem>.__view_oneofs.rs`   (view-of-oneofs, inside `pub mod view { pub mod oneofs {} }`)
    # where `<stem>` is the dotted proto path, e.g.
    # `google.api.expr.v1alpha1.checked`.
    #
    # Group them per package and per kind. Inside the module tree we emit
    # a single `pub mod view { include!(a.__view.rs); include!(b.__view.rs); }`
    # per package (and likewise `pub mod ext { … }`, `pub mod oneofs { … }`)
    # so sibling files share one wrapper module instead of colliding on
    # per-file wrappers. The `view_oneofs` kind nests inside the `view`
    # wrapper: `pub mod view { … pub mod oneofs { <view_oneofs files> } }`.
    packages = defaultdict(lambda: {"owned": [], "view": [], "ext": [], "oneofs": [], "view_oneofs": []})
    excluded = []
    # Check the longer suffix before the shorter one — `.__view_oneofs`
    # must match before `.__view`.
    for rs_file in rs_files:
        if rs_file.name in exclude_files:
            excluded.append(rs_file.name)
            continue
        stem = rs_file.stem  # e.g. "google.api.expr.v1alpha1.checked"
        kind = "owned"
        if stem.endswith(".__view_oneofs"):
            kind = "view_oneofs"
            stem = stem[: -len(".__view_oneofs")]
        elif stem.endswith(".__view"):
            kind = "view"
            stem = stem[: -len(".__view")]
        elif stem.endswith(".__ext"):
            kind = "ext"
            stem = stem[: -len(".__ext")]
        elif stem.endswith(".__oneofs"):
            kind = "oneofs"
            stem = stem[: -len(".__oneofs")]
        parts = stem.split(".")
        # The package is everything except the last segment.
        pkg = tuple(parts[:-1])
        packages[pkg][kind].append(rs_file.name)

    if excluded:
        print(f"Excluded {len(excluded)} files: {', '.join(excluded)}",
              file=sys.stderr)

    # Build the tree directly. Each node tracks per-kind file lists so we
    # can emit a single wrapper per kind per package.
    def new_node():
        return {
            "__owned": [],
            "__view": [],
            "__ext": [],
            "__oneofs": [],
            "__view_oneofs": [],
            "__children": {},
        }

    tree = new_node()

    for pkg, kinds in packages.items():
        node = tree
        for seg in pkg:
            if seg not in node["__children"]:
                node["__children"][seg] = new_node()
            node = node["__children"][seg]
        node["__owned"].extend(kinds["owned"])
        node["__view"].extend(kinds["view"])
        node["__ext"].extend(kinds["ext"])
        node["__oneofs"].extend(kinds["oneofs"])
        node["__view_oneofs"].extend(kinds["view_oneofs"])

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
        for filename in sorted(node["__owned"]):
            lines.append(f'{prefix}include!("{include_prefix}{filename}");')
        if node["__view"] or node["__view_oneofs"]:
            lines.append(f"{prefix}#[allow(non_camel_case_types, dead_code, unused_imports, clippy::derivable_impls, clippy::match_single_binding)]")
            lines.append(f"{prefix}pub mod view {{")
            for filename in sorted(node["__view"]):
                lines.append(f'{prefix}    include!("{include_prefix}{filename}");')
            if node["__view_oneofs"]:
                lines.append(f"{prefix}    #[allow(non_camel_case_types, dead_code, unused_imports, clippy::derivable_impls, clippy::match_single_binding)]")
                lines.append(f"{prefix}    pub mod oneofs {{")
                for filename in sorted(node["__view_oneofs"]):
                    lines.append(f'{prefix}        include!("{include_prefix}{filename}");')
                lines.append(f"{prefix}    }}")
            lines.append(f"{prefix}}}")
        if node["__ext"]:
            lines.append(f"{prefix}#[allow(non_camel_case_types, dead_code, unused_imports, clippy::derivable_impls, clippy::match_single_binding)]")
            lines.append(f"{prefix}pub mod ext {{")
            for filename in sorted(node["__ext"]):
                lines.append(f'{prefix}    include!("{include_prefix}{filename}");')
            lines.append(f"{prefix}}}")
        if node["__oneofs"]:
            lines.append(f"{prefix}#[allow(non_camel_case_types, dead_code, unused_imports, clippy::derivable_impls, clippy::match_single_binding)]")
            lines.append(f"{prefix}pub mod oneofs {{")
            for filename in sorted(node["__oneofs"]):
                lines.append(f'{prefix}    include!("{include_prefix}{filename}");')
            lines.append(f"{prefix}}}")
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

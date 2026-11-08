"""Update reviewed Rust runtime notices from the matching Rust source distribution.

Pass --distribution (containing LICENSE-MIT, LICENSE-APACHE and
share/doc/rustc/COPYRIGHT-library.html) and --source (containing library/ and
src/llvm-project/libunwind/). Review the resulting notices before committing.
The source must match the Rust release specified in rust-toolchain.toml.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "licenses/RUST_STDLIB_NOTICES.txt"
RUNTIME_TREES = ("vendor", "compiler-builtins", "backtrace", "stdarch/crates/core_arch", "portable-simd/crates/core_simd", "core", "alloc", "std", "std_detect", "unwind", "panic_abort", "panic_unwind", "windows-sys", "windows_link")


def generate(distribution, source):
    version = subprocess.check_output(["rustc", "--version"], text=True).strip()
    channel = re.search(r'channel\s*=\s*"([^"]+)"', (ROOT / "rust-toolchain.toml").read_text())[1]
    if version.split()[1] != channel:
        raise ValueError(f"expected Rust {channel}, got {version}")
    files = {
        "LICENSE-MIT": distribution / "LICENSE-MIT",
        "LICENSE-APACHE": distribution / "LICENSE-APACHE",
        "COPYRIGHT-library.html": distribution / "share/doc/rustc/COPYRIGHT-library.html",
        "src/llvm-project/libunwind/LICENSE.TXT": source / "src/llvm-project/libunwind/LICENSE.TXT",
    }
    comments = {}
    # Preserve additional per-file notices (notably musl/CORE-MATH in libm).
    comment_pattern = re.compile(r"/\*.*?\*/|(?:^[ \t]*//[^\n]*(?:\n|$))+", re.MULTILINE | re.DOTALL)
    notice_pattern = re.compile(r"copyright|SPDX-License-Identifier|permission is hereby granted|redistribution and use", re.IGNORECASE)
    for name in RUNTIME_TREES:
        tree = source / "library" / name
        if not tree.is_dir():
            raise ValueError(f"missing Rust source tree: {tree}")
        for path in sorted(tree.rglob("*")):
            if not path.is_file():
                continue
            relative = str(path.relative_to(source))
            if any(word in path.name.upper() for word in ("LICENSE", "LICENCE", "COPYRIGHT", "NOTICE")):
                files[relative] = path
            elif path.suffix in {".rs", ".c", ".h", ".cpp", ".S"}:
                for match in comment_pattern.finditer(path.read_text()):
                    text = match[0].strip()
                    if notice_pattern.search(text):
                        comments.setdefault(text, []).append(relative)

    sections = [
        f"Monika — Rust standard-library and runtime notices\n\n{version}\n"
        "Source: the corresponding Rust source distribution, https://github.com/rust-lang/rust\n"
        "This inventory includes runtime source notices for multiple targets and build\n"
        "dependencies; not every component is linked into every Monika executable.\n"
        "The Rust Project's MIT option is selected where available. Additional licenses,\n"
        "including Unicode-3.0 and Apache-2.0 WITH LLVM-exception, are retained below.\n"
        "Upstream files and per-file attribution comments are reproduced unchanged.\n",
    ]
    sources = {}
    for relative, path in sorted(files.items()):
        body = path.read_bytes()
        sources[relative] = hashlib.sha256(body).hexdigest()
        sections.append(relative + "\n\n" + body.decode().rstrip())
    # The Unicode data in core uses the same Unicode-3.0 terms, with its own
    # copyright year retained in COPYRIGHT-library.html and the source comments.
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True))
    unicode_crate = next(p for p in metadata["packages"] if p["name"] == "icu_collections")
    unicode_license = Path(unicode_crate["manifest_path"]).parent / "LICENSE"
    sections.append("Unicode-3.0 license text\n\n" + unicode_license.read_text().rstrip())
    sources["Unicode-3.0 (icu_collections " + unicode_crate["version"] + ")"] = hashlib.sha256(unicode_license.read_bytes()).hexdigest()
    for text, paths in sorted(comments.items(), key=lambda item: (sorted(item[1]), item[0])):
        sections.append("Per-file notice: " + ", ".join(sorted(set(paths))) + "\n\n" + text)
    OUTPUT.parent.mkdir(exist_ok=True)
    OUTPUT.write_text(("\n\n" + "=" * 78 + "\n\n").join(sections) + "\n")
    tracked = ("rust-toolchain.toml", "scripts/update_rust_notices.py", str(OUTPUT.relative_to(ROOT)))
    (ROOT / "licenses/rust-review.json").write_text(json.dumps({
        "rustc": version,
        "files": {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in tracked},
        "upstream_files": sources,
    }, indent=2) + "\n")
    print(f"Recorded {len(files)} runtime license files and {len(comments)} distinct per-file notices")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--distribution", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    generate(args.distribution, args.source)

"""Stage the public documentation with its relative links intact."""

from pathlib import Path
import shutil

from license_notices import verify_snapshot


ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = (
    "README.md",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "licenses/CARGO_DEPENDENCIES.txt",
    "licenses/RUST_STDLIB_NOTICES.txt",
    "licenses/BUILD_TOOLS.txt",
    "EXTENSIONS.md",
    "BUNDLED_EXTENSIONS.md",
    "docs/extensions/documents.md",
    "docs/extensions/data.md",
    "docs/extensions/code.md",
    "docs/AGENT_GUIDE.md",
    "examples/demo.py",
    "examples/bundled_demo.py",
    "examples/extension_api.py",
    "tests/fixtures/sets_extension.py",
) + tuple(str(path.relative_to(ROOT)) for path in sorted((ROOT / "licenses/sources").glob("*.crate")))


def prepare():
    verify_snapshot()
    destination = ROOT / "target/release-assets/documentation"
    if destination.exists():
        shutil.rmtree(destination)
    for relative in DOCUMENTS:
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, target)
    print(destination)


if __name__ == "__main__":
    prepare()

"""Stage the public documentation with its relative links intact."""

from pathlib import Path
import shutil


ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = (
    "README.md",
    "EXTENSIONS.md",
    "docs/AGENT_GUIDE.md",
    "examples/demo.py",
    "examples/extension_api.py",
    "tests/fixtures/sets_extension.py",
)


def prepare():
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

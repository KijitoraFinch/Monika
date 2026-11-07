"""Verify an archive's checksum, contents, documentation, and executable demo."""

import argparse
import hashlib
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile

from prepare_release import DOCUMENTS


def verify(archive, version):
    archive = archive.resolve()
    expected_hash = Path(str(archive) + ".sha256").read_text().split()[0]
    actual_hash = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual_hash != expected_hash:
        raise ValueError(f"checksum mismatch: {archive.name}")

    directory = archive.name.removesuffix(".tar.gz")
    expected = {f"{directory}/monika", f"{directory}/README.md"}
    expected.update(f"{directory}/documentation/{name}" for name in DOCUMENTS)
    with tempfile.TemporaryDirectory(prefix="monika-release-check-") as temporary:
        root = Path(temporary).resolve()
        with tarfile.open(archive) as package:
            members = package.getmembers()
            names = [member.name for member in members if member.isfile()]
            if len(names) != len(set(names)) or set(names) != expected:
                raise ValueError(f"unexpected archive contents: {sorted(set(names) ^ expected)}")
            for member in members:
                if not (member.isfile() or member.isdir()):
                    raise ValueError(f"unexpected archive entry: {member.name}")
                (root / member.name).resolve().relative_to(root)
            package.extractall(root)

        bundle = root / directory
        binary = bundle / "monika"
        output = subprocess.check_output([str(binary), "--version"], text=True).strip()
        if output != f"monika {version}":
            raise ValueError(f"unexpected executable version: {output}")

        documentation = bundle / "documentation"
        for name in DOCUMENTS:
            if not name.endswith(".md"):
                continue
            source = documentation / name
            for link in re.findall(r"\]\(([^)]+)\)", source.read_text()):
                if "://" in link or link.startswith("#"):
                    continue
                destination = (source.parent / link.split("#")[0]).resolve()
                destination.relative_to(documentation)
                if not destination.exists():
                    raise ValueError(f"broken documentation link: {name}: {link}")

        # Run the copy in the archive, so the checkout cannot supply missing files.
        subprocess.run(
            ["python3", str(documentation / "examples/demo.py"), "--monika", str(binary)],
            cwd=root,
            check=True,
        )
    print(f"Verified {archive.name}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    verify(args.archive, args.version)

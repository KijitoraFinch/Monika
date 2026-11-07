"""Test the generated Formula against the actual release archives."""

import argparse
import hashlib
from pathlib import Path
import re
import subprocess

from prepare_release import DOCUMENTS, ROOT


TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
)


def prepare(destination, targets):
    artifacts = ROOT / "target/distrib"
    formula = (artifacts / "monika.rb").read_text()
    pattern = r'url "https://github.com/KijitoraFinch/Monika/releases/download/[^/]+/(monika-([a-z0-9_-]+)\.tar\.gz)"\s+sha256 "([a-f0-9]{64})"'
    matches = list(re.finditer(pattern, formula))
    if sorted(match[2] for match in matches) != sorted(targets):
        raise ValueError("Formula must contain a URL and checksum for every selected target")
    for match in matches:
        archive = artifacts / match[1]
        if hashlib.sha256(archive.read_bytes()).hexdigest() != match[3]:
            raise ValueError(f"Formula checksum mismatch: {archive.name}")

    def local_url(match):
        archive = artifacts / match[1]
        return f'url "{archive.as_uri()}"\n      sha256 "{match[3]}"'

    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(re.sub(pattern, local_url, formula))
    print(f"Verified Formula URLs and checksums; local test copy: {destination}")


def verify(prefix, version):
    binary = prefix / "bin/monika"
    documentation = prefix / "share/monika/documentation"
    output = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if output != f"monika {version}":
        raise ValueError(f"unexpected installed version: {output}")
    for relative in DOCUMENTS:
        if not (documentation / relative).is_file():
            raise ValueError(f"missing installed document: {relative}")
    subprocess.run(
        ["python3", str(documentation / "examples/demo.py"), "--monika", str(binary)],
        cwd=prefix,
        check=True,
    )
    print(f"Verified Homebrew installation: {prefix}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("destination", type=Path)
    prepare_parser.add_argument("--target", action="append", choices=TARGETS)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("prefix", type=Path)
    verify_parser.add_argument("--version", required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.destination, args.target or TARGETS)
    else:
        verify(args.prefix, args.version)

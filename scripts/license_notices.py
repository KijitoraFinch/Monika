"""Generate/check Cargo notices with cargo-about 0.9.3; verify the reviewed snapshot.

Install cargo-about 0.9.3, then run this script with --generate after reviewing
dependency changes. --check regenerates and compares in CI. Without either flag,
only the snapshot is checked (no cargo-about installation or network required).
Rust notices are updated separately with update_rust_notices.py.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
NOTICES = ROOT / "licenses/CARGO_DEPENDENCIES.txt"
SNAPSHOT = ROOT / "licenses/cargo-review.json"
INPUTS = (
    "Cargo.toml", "Cargo.lock", "about.toml", "LICENSE", "scripts/license_notices.py",
    "licenses/BUILD_TOOLS.txt", "assets/logo/requirements.txt", "licenses/clarifications.json",
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def locked_checksums():
    # Cargo writes package names, versions and checksums as simple quoted ASCII
    # strings. Read those generated fields without requiring Python 3.11 tomllib.
    checksums = {}
    for block in (ROOT / "Cargo.lock").read_text().split("[[package]]")[1:]:
        fields = {}
        for line in block.splitlines():
            key, separator, value = line.partition(" = ")
            if separator and key in {"name", "version", "checksum"}:
                fields[key] = json.loads(value)
        if "checksum" in fields:
            checksums[fields["name"], fields["version"]] = fields["checksum"]
    return checksums


def cargo_review_files():
    fonts = sorted(
        str(path.relative_to(ROOT))
        for path in (ROOT / "assets/logo/fonts").iterdir() if path.is_file()
    )
    source_files = sorted(str(path.relative_to(ROOT)) for directory in ("licenses/upstream", "licenses/sources") for path in (ROOT / directory).iterdir() if path.is_file())
    return (*INPUTS, *fonts, *source_files, "licenses/CARGO_DEPENDENCIES.txt")


def cargo_notices(cargo_about):
    version = subprocess.check_output([cargo_about, "--version"], text=True).strip()
    if version != "cargo-about 0.9.3":
        raise ValueError(f"expected cargo-about 0.9.3, got {version}")
    subprocess.run(["cargo", "fetch", "--locked"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="monika-licenses-") as temporary:
        report = Path(temporary) / "report.json"
        subprocess.run(
            [cargo_about, "generate", "--locked", "--offline", "--fail", "--format", "json", "-o", str(report)],
            cwd=ROOT, check=True,
        )
        data = json.loads(report.read_text())
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True,
    ))
    packages = {
        (p["name"], p["version"]): p for p in metadata["packages"]
        if p["id"] not in metadata["workspace_members"]
    }
    covered = set()
    groups = []
    clarifications = json.loads((ROOT / "licenses/clarifications.json").read_text())
    checksums = locked_checksums()

    def clarified(name, version, identifier):
        key = f"{name} {version}"
        source = clarifications.get(key)
        if not source or source["license"] != identifier:
            raise ValueError(f"license text lacks a reviewed source: {key}")
        package = packages[name, version]
        if checksums[name, version] != source["package_checksum"] or package["license"] != source["declared_license"]:
            raise ValueError(f"upstream clarification is stale: {key}")
        path = ROOT / source["file"]
        if digest(path) != source["sha256"]:
            raise ValueError(f"upstream license text changed: {key}")
        return f"License source: {source['source']}\n\n" + path.read_text()
    for license in data["licenses"]:
        users = sorted(
            (entry["crate"]["name"], entry["crate"]["version"])
            for entry in license["used_by"]
            if (entry["crate"]["name"], entry["crate"]["version"]) in packages
        )
        if not users:
            continue
        covered.update(users)
        if not license["source_path"]:
            for name, version in users:
                body = clarified(name, version, license["id"])
                groups.append((license["id"], [(name, version)], body))
            continue
        # Include every applicable license group, including both parts of AND.
        groups.append((license["id"], users, license["text"].replace("\r\n", "\n").replace("\r", "\n")))
    # Cargo.lock also includes optional dependency branches (e.g. HTTP/3) that
    # cargo-about filters from the active graph. Keep their reviewed texts too.
    for name, version in sorted(packages.keys() - covered):
        source = clarifications.get(f"{name} {version}")
        if not source or not source.get("locked_only"):
            raise ValueError(f"unreviewed optional locked dependency: {name} {version}")
        groups.append((source["license"], [(name, version)], clarified(name, version, source["license"])))

    sections = [
        "Monika — Cargo dependency notices\n\n"
        "Generated with cargo-about 0.9.3 from Cargo.lock.\n"
        "Includes all locked direct, transitive, build, development, and target-specific\n"
        "dependencies, even when not linked into a particular release.\n"
        "MIT is selected where available; all additionally required terms are retained.\n"
        "Unmodified MPL-2.0 package sources are included in licenses/sources/*.crate\n"
        "(gzip-compressed tar archives), alongside their original source-file notices.\n"
        "Original alternative-license declarations below do not change that selection.\n"
        "Rust standard-library notices are in RUST_STDLIB_NOTICES.txt.\n",
        "Dependency inventory\n\n" + "\n".join(
            f"{name} {version} | {p['license']} | {p['repository'] or p['homepage'] or 'https://crates.io/crates/' + name}"
            for (name, version), p in sorted(packages.items())
        ),
    ]
    for (name, version), package in sorted(packages.items()):
        if "MPL-2.0" not in package["license"]:
            continue
        source = ROOT / "licenses/sources" / f"{name}-{version}.crate"
        checksum = checksums[name, version]
        if digest(source) != checksum:
            raise ValueError(f"bundled MPL source does not match Cargo.lock: {name} {version}")
        sections.append(f"MPL-2.0 Source Code Form: {name} {version}\n"
                        f"Unmodified source included at licenses/sources/{source.name}\n"
                        f"Upstream: https://crates.io/api/v1/crates/{name}/{version}/download\n"
                        f"SHA-256: {checksum}\nAvailable under MPL-2.0; see the license and original notices below and in the source archive.")
    for identifier, users, body in sorted(groups):
        sections.append(
            f"{identifier}\nUsed by: " + ", ".join(f"{n} {v}" for n, v in users)
            + "\n\n" + body.rstrip()
        )
    for (name, version), package in sorted(packages.items()):
        root = Path(package["manifest_path"]).parent
        for path in sorted(root.rglob("*")):
            if path.is_file() and path.name.upper().split(".")[0] in {"COPYRIGHT", "NOTICE", "NOTICES", "AUTHORS"}:
                sections.append(f"Supplement: {name} {version} / {path.relative_to(root)}\n\n{path.read_text().rstrip()}")
    return ("\n\n" + "=" * 78 + "\n\n").join(sections) + "\n", len(packages)


def verify_snapshot():
    for relative in ("licenses/cargo-review.json", "licenses/rust-review.json"):
        review = json.loads((ROOT / relative).read_text())
        if relative == "licenses/cargo-review.json" and set(review["files"]) != set(cargo_review_files()):
            raise ValueError("license review file inventory changed; review and run --generate")
        for name, expected in review["files"].items():
            if digest(ROOT / name) != expected:
                raise ValueError(f"license review is stale: {name}; update {relative}")
    print("License review snapshots match the dependencies and Rust toolchain")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--generate", action="store_true")
    mode.add_argument("--check", action="store_true")
    parser.add_argument("--cargo-about", default="cargo-about")
    args = parser.parse_args()
    if args.generate or args.check:
        text, count = cargo_notices(args.cargo_about)
        if args.generate:
            NOTICES.parent.mkdir(exist_ok=True)
            NOTICES.write_text(text)
            SNAPSHOT.write_text(json.dumps({
                "cargo_about": "0.9.3", "external_packages": count,
                "files": {name: digest(ROOT / name) for name in cargo_review_files()},
            }, indent=2) + "\n")
        elif NOTICES.read_text() != text:
            raise ValueError("Cargo notices differ; review and run --generate")
        print(f"Verified license texts for {count} external Cargo packages")
    verify_snapshot()


if __name__ == "__main__":
    main()

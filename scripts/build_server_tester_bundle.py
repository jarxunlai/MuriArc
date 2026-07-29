#!/usr/bin/env python3
"""Build a deterministic, digest-pinned MuriArc Server Tester Compose ZIP.

The CLI is deliberately release-only: it accepts only a clean canonical checkout
whose HEAD, origin/main and explicit expected commit are identical. Unit tests
exercise ``build_bundle`` directly without weakening that gate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TEMPLATE_ROOT = ROOT / "deploy" / "server-tester"
CANONICAL_ORIGIN = "https://github.com/jarxunlai/MuriArc"
SERVER_REPOSITORY = "ghcr.io/jarxunlai/muriarc-server-tester"
POSTGRES_REPOSITORY = "postgres:17-bookworm"
POSTGRES_IMAGE = (
    "postgres:17-bookworm@sha256:"
    "4f736ae292687621d4dbe0d499ffd024a36bd2ee7d8ca6f2ccd4c800f047b394"
)
HEX40 = re.compile(r"^[0-9a-f]{40}$")
DIGEST_IMAGE = re.compile(r"^(?P<name>[^@\s]+)@sha256:(?P<digest>[0-9a-f]{64})$")
PLACEHOLDERS = ("@@SOURCE_COMMIT@@", "@@SERVER_IMAGE@@", "@@POSTGRES_IMAGE@@", "@@RELEASE_TAG@@")
TEMPLATE_FILES = {
    "compose.yaml.in": "compose.yaml",
    "compose.bootstrap.yaml": "compose.bootstrap.yaml",
    ".env.empty.example.in": ".env.empty.example",
    ".env.demo.example.in": ".env.demo.example",
    "muriarc-tester.sh": "muriarc-tester.sh",
    "muriarc-tester.ps1": "muriarc-tester.ps1",
    "README.md.in": "README.md",
    "README_cn.md.in": "README_cn.md",
}


class BuildError(RuntimeError):
    pass


@dataclass(frozen=True)
class BuiltArtifact:
    archive: Path
    checksum: Path
    manifest: Path
    release_tag: str


def run_git(*args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(ROOT), *args],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip()


def normalized_origin(value: str) -> str:
    value = value.strip().rstrip("/")
    if value.endswith(".git"):
        value = value[:-4]
    return value


def enforce_source_gate(expected_commit: str) -> None:
    if not HEX40.fullmatch(expected_commit):
        raise BuildError("expected commit must be 40 lowercase hexadecimal characters")
    origin = normalized_origin(run_git("remote", "get-url", "origin"))
    if origin != CANONICAL_ORIGIN:
        raise BuildError(f"origin is not canonical: {origin}")
    head = run_git("rev-parse", "HEAD")
    remote_main = run_git("rev-parse", "origin/main")
    if head != expected_commit or remote_main != expected_commit:
        raise BuildError(
            "release source gate requires HEAD == origin/main == expected commit; "
            f"HEAD={head}, origin/main={remote_main}, expected={expected_commit}"
        )
    dirty = run_git("status", "--porcelain", "--untracked-files=all")
    if dirty:
        raise BuildError("release source gate requires a clean worktree")


def require_digest_image(value: str, repository: str) -> str:
    match = DIGEST_IMAGE.fullmatch(value)
    if not match or match.group("name") != repository:
        raise BuildError(f"image must be {repository}@sha256:<64 lowercase hex>")
    return match.group("digest")


def sha256_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            size += len(chunk)
            digest.update(chunk)
    return size, digest.hexdigest()


def render_text(text: str, values: dict[str, str], label: str) -> str:
    for key, value in values.items():
        text = text.replace(key, value)
    remaining = sorted(token for token in PLACEHOLDERS if token in text)
    if remaining:
        raise BuildError(f"unrendered placeholders in {label}: {', '.join(remaining)}")
    return text


def write_regular(path: Path, data: bytes, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    path.chmod(0o755 if executable else 0o644)


def safe_inventory(root: Path) -> list[Path]:
    files: list[Path] = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root)
        if path.is_symlink():
            raise BuildError(f"bundle contains a symlink: {relative.as_posix()}")
        if path.is_dir():
            continue
        if not path.is_file():
            raise BuildError(f"bundle contains a non-regular entry: {relative.as_posix()}")
        if relative.is_absolute() or ".." in relative.parts or "\\" in relative.as_posix():
            raise BuildError(f"unsafe bundle path: {relative.as_posix()}")
        files.append(path)
    if len(files) != len({path.relative_to(root).as_posix() for path in files}):
        raise BuildError("bundle contains duplicate paths")
    return files


def deterministic_zip(source: Path, destination: Path, root_name: str) -> None:
    seen: set[str] = set()
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in safe_inventory(source):
            relative = path.relative_to(source).as_posix()
            member = f"{root_name}/{relative}"
            if member in seen:
                raise BuildError(f"duplicate ZIP member: {member}")
            seen.add(member)
            info = zipfile.ZipInfo(member, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            mode = stat.S_IFREG | (0o755 if os.access(path, os.X_OK) else 0o644)
            info.external_attr = mode << 16
            info.create_system = 3
            archive.writestr(info, path.read_bytes())


def verify_zip_roundtrip(archive: Path, expected_files: dict[str, str], root_name: str) -> None:
    with zipfile.ZipFile(archive) as handle:
        names = handle.namelist()
        if len(names) != len(set(names)):
            raise BuildError("ZIP contains duplicate members")
        actual: dict[str, str] = {}
        for info in handle.infolist():
            name = info.filename
            prefix = f"{root_name}/"
            if not name.startswith(prefix):
                raise BuildError(f"ZIP member escaped bundle root: {name}")
            relative = name[len(prefix) :]
            parts = Path(relative).parts
            if not relative or relative.startswith("/") or ".." in parts or "\\" in relative:
                raise BuildError(f"unsafe ZIP member: {name}")
            unix_mode = (info.external_attr >> 16) & 0o170000
            if unix_mode not in (0, stat.S_IFREG):
                raise BuildError(f"ZIP member is not a regular file: {name}")
            actual[relative] = hashlib.sha256(handle.read(info)).hexdigest()
        if actual != expected_files:
            raise BuildError("ZIP round-trip inventory differs from the manifest")


def build_bundle(
    *,
    expected_commit: str,
    server_image: str,
    output_directory: Path,
    postgres_image: str = POSTGRES_IMAGE,
    enforce_git: bool = True,
) -> BuiltArtifact:
    if enforce_git:
        enforce_source_gate(expected_commit)
    elif not HEX40.fullmatch(expected_commit):
        raise BuildError("expected commit must be 40 lowercase hexadecimal characters")
    server_digest = require_digest_image(server_image, SERVER_REPOSITORY)
    postgres_digest = require_digest_image(postgres_image, POSTGRES_REPOSITORY)
    short = expected_commit[:12]
    release_tag = f"server-tester-v1.0.0-standard-v1-{short}"
    root_name = f"MuriArc-server-tester-v1.0.0-standard-v1-{short}-linux-amd64"
    values = {
        "@@SOURCE_COMMIT@@": expected_commit,
        "@@SERVER_IMAGE@@": server_image,
        "@@POSTGRES_IMAGE@@": postgres_image,
        "@@RELEASE_TAG@@": release_tag,
    }

    output_directory.mkdir(parents=True, exist_ok=True)
    if any(output_directory.iterdir()):
        raise BuildError(f"output directory must be empty: {output_directory}")
    with tempfile.TemporaryDirectory(prefix="muriarc-server-tester-") as temporary:
        bundle = Path(temporary) / root_name
        bundle.mkdir()
        for source_name, destination_name in TEMPLATE_FILES.items():
            source = TEMPLATE_ROOT / source_name
            if not source.is_file() or source.is_symlink():
                raise BuildError(f"missing regular source template: {source}")
            rendered = render_text(source.read_text(encoding="utf-8"), values, source_name)
            executable = destination_name == "muriarc-tester.sh"
            write_regular(bundle / destination_name, rendered.encode("utf-8"), executable)

        checksums: list[str] = []
        for path in safe_inventory(bundle):
            relative = path.relative_to(bundle).as_posix()
            _, digest = sha256_file(path)
            checksums.append(f"{digest}  {relative}")
        write_regular(bundle / "CHECKSUMS.sha256", ("\n".join(checksums) + "\n").encode())

        inventory: dict[str, dict[str, int | str]] = {}
        expected_zip: dict[str, str] = {}
        for path in safe_inventory(bundle):
            relative = path.relative_to(bundle).as_posix()
            size, digest = sha256_file(path)
            inventory[relative] = {"size": size, "sha256": digest}
            expected_zip[relative] = digest

        archive = output_directory / f"{root_name}.zip"
        deterministic_zip(bundle, archive, root_name)
        verify_zip_roundtrip(archive, expected_zip, root_name)

    archive_size, archive_digest = sha256_file(archive)
    checksum = archive.with_suffix(archive.suffix + ".sha256")
    checksum.write_text(f"{archive_digest}  {archive.name}\n", encoding="utf-8")
    manifest = archive.with_suffix(archive.suffix + ".manifest.json")
    manifest_value = {
        "schemaVersion": 1,
        "artifactType": "muriarc-server-docker-tester",
        "releaseTag": release_tag,
        "source": {"repository": CANONICAL_ORIGIN, "commit": expected_commit},
        "platform": ["linux/amd64"],
        "classification": [
            "unsigned",
            "synthetic-capable",
            "not-for-production",
            "not-formal-rc-evidence",
        ],
        "formalRelease": False,
        "formalRcEvidence": False,
        "defaultDataset": "empty",
        "optionalDataset": {
            "id": "muriarc-standard-v1",
            "version": "standard-v1",
            "synthetic": True,
            "datasetSha256": hashlib.sha256(
                (ROOT / "fixtures/standard-v1/dataset.json").read_bytes()
            ).hexdigest(),
            "manifestSha256": hashlib.sha256(
                (ROOT / "fixtures/standard-v1/manifest.json").read_bytes()
            ).hexdigest(),
        },
        "images": {
            "server": {"reference": server_image, "digest": f"sha256:{server_digest}"},
            "postgresql": {"reference": postgres_image, "digest": f"sha256:{postgres_digest}"},
        },
        "archive": {"name": archive.name, "size": archive_size, "sha256": archive_digest},
        "bundleFiles": inventory,
        "excludedFrom": ["artifact-lock.json", "Fixture Catalog", "formal RC evidence"],
    }
    manifest.write_text(json.dumps(manifest_value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return BuiltArtifact(archive=archive, checksum=checksum, manifest=manifest, release_tag=release_tag)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--server-image", required=True)
    parser.add_argument("--postgres-image", default=POSTGRES_IMAGE)
    parser.add_argument("--output-directory", required=True, type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        artifact = build_bundle(
            expected_commit=args.expected_commit,
            server_image=args.server_image,
            postgres_image=args.postgres_image,
            output_directory=args.output_directory.resolve(),
        )
    except (BuildError, OSError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"server tester bundle error: {error}", file=sys.stderr)
        return 2
    print(json.dumps({
        "archive": str(artifact.archive),
        "checksum": str(artifact.checksum),
        "manifest": str(artifact.manifest),
        "releaseTag": artifact.release_tag,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

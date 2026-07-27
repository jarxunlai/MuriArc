#!/usr/bin/env python3
"""Assemble the external 1.0 Release Manifest from signed artifact descriptors.

The manifest is deliberately created only after Native, Compose, and Desktop
artifacts have final digests. It is never embedded into those artifacts, which
would create a self-referential digest. This command does not sign anything.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any

from check_release_readiness import (
    MANDATORY_ARTIFACTS,
    ReadinessError,
    exact_keys,
    read_json,
    require_digest,
    require_int,
    sha256,
    validate_definition,
    validate_release_manifest,
    validate_source_identity,
)


IDENTITY_KEYS = {
    "format_version",
    "application_version",
    "data_epoch",
    "gateway_contract_revision",
    "backend_states",
    "postgres_major",
    "bootstrap_protocol_revision",
    "controller_protocol_min",
    "controller_protocol_max",
    "migration_class",
}
DESCRIPTOR_KEYS = {
    "format_version",
    "artifact_name",
    "media_type",
    "digest",
    "size_bytes",
    "provenance_digest",
    "signature_evidence_digest",
}


class AssemblyError(ValueError):
    pass


def parse_artifact_argument(value: str) -> tuple[str, Path]:
    name, separator, path = value.partition("=")
    if not separator or not name or not path:
        raise argparse.ArgumentTypeError("--artifact must use name=/absolute/descriptor.json")
    return name, Path(path)


def assemble(
    *,
    source_root: Path,
    identity_path: Path,
    artifact_arguments: list[tuple[str, Path]],
    release_provenance_digest: str,
    rc_definition_path: Path,
) -> tuple[dict[str, Any], dict[str, Any]]:
    identity, _ = read_json(identity_path)
    definition, _ = read_json(rc_definition_path)
    validate_definition(definition)
    exact_keys(identity, IDENTITY_KEYS, "release identity input")
    if require_int(identity["format_version"], "release identity format_version") != 1:
        raise AssemblyError("release identity format_version must be 1")
    require_digest(release_provenance_digest, "release provenance digest")

    descriptors: dict[str, dict[str, Any]] = {}
    for declared_name, path in artifact_arguments:
        descriptor, _ = read_json(path)
        exact_keys(descriptor, DESCRIPTOR_KEYS, f"artifact descriptor {declared_name}")
        if (
            require_int(
                descriptor["format_version"],
                f"artifact descriptor {declared_name}.format_version",
            )
            != 1
            or descriptor["artifact_name"] != declared_name
            or declared_name in descriptors
        ):
            raise AssemblyError("artifact descriptors must be format 1, name-matched, and unique")
        require_digest(descriptor["digest"], f"artifact descriptor {declared_name}.digest")
        require_digest(
            descriptor["provenance_digest"],
            f"artifact descriptor {declared_name}.provenance_digest",
        )
        require_digest(
            descriptor["signature_evidence_digest"],
            f"artifact descriptor {declared_name}.signature_evidence_digest",
        )
        require_int(
            descriptor["size_bytes"],
            f"artifact descriptor {declared_name}.size_bytes",
            minimum=1,
        )
        if not isinstance(descriptor["media_type"], str) or not descriptor["media_type"].strip():
            raise AssemblyError(f"artifact descriptor {declared_name}.media_type is invalid")
        descriptors[declared_name] = descriptor
    if not MANDATORY_ARTIFACTS.issubset(descriptors):
        raise AssemblyError("Native, Managed Compose, and Desktop descriptors are required")

    manifest = {
        **identity,
        "artifacts": {
            name: {
                "media_type": descriptor["media_type"],
                "digest": descriptor["digest"],
                "size_bytes": descriptor["size_bytes"],
            }
            for name, descriptor in sorted(descriptors.items())
        },
    }
    validate_release_manifest(manifest, definition)
    validate_source_identity(source_root, manifest)
    manifest_raw = (json.dumps(manifest, ensure_ascii=False, indent=2) + "\n").encode()
    artifact_lock = {
        "format_version": 1,
        "release_manifest_digest": sha256(manifest_raw),
        "release_provenance_digest": release_provenance_digest,
        "artifacts": {
            name: {
                "digest": descriptor["digest"],
                "size_bytes": descriptor["size_bytes"],
                "provenance_digest": descriptor["provenance_digest"],
                "signature_evidence_digest": descriptor["signature_evidence_digest"],
            }
            for name, descriptor in sorted(descriptors.items())
        },
    }
    return manifest, artifact_lock


def write_outputs(output_directory: Path, source_root: Path, manifest: dict[str, Any], lock: dict[str, Any]) -> None:
    if not output_directory.is_absolute():
        raise AssemblyError("--output-directory must be absolute")
    resolved = output_directory.resolve(strict=False)
    try:
        resolved.relative_to(source_root.resolve(strict=True))
    except ValueError:
        pass
    else:
        raise AssemblyError("Release Manifest outputs must remain outside the Git worktree")
    if output_directory.exists() or output_directory.is_symlink():
        raise AssemblyError("--output-directory must not already exist")
    output_directory.mkdir(parents=False)
    for name, value in (
        ("release-manifest.json", manifest),
        ("artifact-lock.json", lock),
    ):
        temporary = output_directory / f".{name}.tmp-{os.getpid()}"
        temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        os.replace(temporary, output_directory / name)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--source-root", type=Path, default=Path("."))
    result.add_argument("--identity", type=Path, required=True)
    result.add_argument("--artifact", action="append", type=parse_artifact_argument, required=True)
    result.add_argument("--release-provenance-digest", required=True)
    result.add_argument("--rc-definition", type=Path, default=Path("release-fixtures/rc-gate.json"))
    result.add_argument("--output-directory", type=Path, required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        manifest, lock = assemble(
            source_root=args.source_root,
            identity_path=args.identity,
            artifact_arguments=args.artifact,
            release_provenance_digest=args.release_provenance_digest,
            rc_definition_path=args.rc_definition,
        )
        write_outputs(args.output_directory, args.source_root, manifest, lock)
    except (AssemblyError, ReadinessError, OSError) as exc:
        print(f"Release Manifest assembly failed: {exc}", file=sys.stderr)
        return 2
    print(
        json.dumps(
            {
                "release_manifest": str(args.output_directory / "release-manifest.json"),
                "artifact_lock": str(args.output_directory / "artifact-lock.json"),
                "release_manifest_digest": lock["release_manifest_digest"],
            },
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

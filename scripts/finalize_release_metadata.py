#!/usr/bin/env python3
"""Assemble and sign final MuriArc Release Manifest metadata.

The three artifact descriptors must already be produced by
finalize_release_artifact.py. This command creates a non-self-referential
release provenance statement, uses its digest to assemble artifact-lock.json,
and signs/verifies the provenance, Release Manifest, and artifact lock.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import assemble_release_manifest as assembler
import finalize_release_artifact as artifact_finalizer


class MetadataError(ValueError):
    pass


def parse_descriptor(value: str) -> tuple[str, Path]:
    name, separator, path = value.partition("=")
    if not separator or name not in artifact_finalizer.ARTIFACT_NAMES or not path:
        raise argparse.ArgumentTypeError(
            "--descriptor must use native-system|managed-compose|desktop-windows=/absolute/descriptor.json"
        )
    return name, Path(path)


def read_descriptor_evidence(
    name: str, descriptor_path: Path
) -> tuple[dict[str, Any], dict[str, str]]:
    descriptor_path = artifact_finalizer.regular_file(
        descriptor_path.resolve(strict=True), f"{name} descriptor"
    )
    try:
        descriptor = json.loads(descriptor_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise MetadataError(f"{name} descriptor is invalid: {exc}") from exc
    assembler.exact_keys(descriptor, assembler.DESCRIPTOR_KEYS, f"{name} descriptor")
    if descriptor["artifact_name"] != name or descriptor["format_version"] != 1:
        raise MetadataError(f"{name} descriptor identity is invalid")

    evidence_root = descriptor_path.parent
    evidence = {}
    for filename, key in (
        ("provenance.intoto.json", "provenance_digest"),
        ("signature-evidence.json", "signature_evidence_digest"),
        ("scan-evidence.json", None),
        ("sbom.cdx.json", None),
        ("grype.json", None),
        ("artifact.cosign.bundle.json", None),
    ):
        path = artifact_finalizer.regular_file(
            evidence_root / filename, f"{name} {filename}"
        )
        _, digest = artifact_finalizer.sha256_file(path)
        evidence[filename] = digest
        if key is not None and descriptor[key] != digest:
            raise MetadataError(f"{name} descriptor differs from {filename}")
    return descriptor, evidence


def release_provenance(
    *,
    expected_commit: str,
    identity_digest: str,
    descriptors: dict[str, dict[str, Any]],
    evidence: dict[str, dict[str, str]],
    invocation_id: str,
    source_date_epoch: int,
) -> dict[str, Any]:
    if set(descriptors) != artifact_finalizer.ARTIFACT_NAMES:
        raise MetadataError("release provenance requires exactly all three artifacts")
    return {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [
            {
                "name": name,
                "digest": {"sha256": descriptor["digest"].removeprefix("sha256:")},
            }
            for name, descriptor in sorted(descriptors.items())
        ],
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {
                "buildType": "https://github.com/jarxunlai/MuriArc/release-set/v1",
                "externalParameters": {
                    "applicationVersion": "1.0.0",
                    "dataEpoch": "E0001",
                    "artifactNames": sorted(descriptors),
                },
                "internalParameters": {
                    "checksSkipped": False,
                    "singleDigestSet": True,
                },
                "resolvedDependencies": [
                    {
                        "uri": "git+https://github.com/jarxunlai/MuriArc",
                        "digest": {"gitCommit": expected_commit},
                    },
                    {
                        "uri": "file:release-identity.json",
                        "digest": {"sha256": identity_digest.removeprefix("sha256:")},
                    },
                    *[
                        {
                            "uri": f"file:{name}/provenance.intoto.json",
                            "digest": {
                                "sha256": evidence[name]["provenance.intoto.json"].removeprefix(
                                    "sha256:"
                                )
                            },
                        }
                        for name in sorted(evidence)
                    ],
                ],
            },
            "runDetails": {
                "builder": {
                    "id": "https://github.com/jarxunlai/MuriArc/release-environment/1.0.0"
                },
                "metadata": {
                    "invocationId": invocation_id,
                    "startedOn": datetime.fromtimestamp(
                        source_date_epoch, timezone.utc
                    ).isoformat().replace("+00:00", "Z"),
                    "finishedOn": datetime.now(timezone.utc)
                    .isoformat()
                    .replace("+00:00", "Z"),
                },
            },
        },
    }


def sign_and_verify(
    *,
    path: Path,
    bundle: Path,
    cosign: Path,
    key: Path,
    public_key: Path,
    env: dict[str, str],
    source_root: Path,
) -> str:
    artifact_finalizer.run_checked(
        (cosign, "sign-blob", "--yes", "--key", key, "--bundle", bundle, path),
        cwd=source_root,
        env=env,
    )
    artifact_finalizer.regular_file(bundle, f"{path.name} Cosign bundle")
    artifact_finalizer.run_checked(
        (cosign, "verify-blob", "--key", public_key, "--bundle", bundle, path),
        cwd=source_root,
        env=env,
    )
    return artifact_finalizer.sha256_file(bundle)[1]


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    source_root = args.source_root.resolve(strict=True)
    source_date_epoch = artifact_finalizer.validate_source(
        source_root, args.expected_commit
    )
    identity = artifact_finalizer.regular_file(
        args.identity.resolve(strict=True), "release identity"
    )
    identity_size, identity_digest = artifact_finalizer.sha256_file(identity)
    if identity_size <= 0:
        raise MetadataError("release identity is empty")

    descriptors: dict[str, dict[str, Any]] = {}
    evidence: dict[str, dict[str, str]] = {}
    descriptor_arguments: list[tuple[str, Path]] = []
    for name, path in args.descriptor:
        if name in descriptors:
            raise MetadataError("artifact descriptor names must be unique")
        descriptor, item_evidence = read_descriptor_evidence(name, path)
        descriptors[name] = descriptor
        evidence[name] = item_evidence
        descriptor_arguments.append((name, path.resolve(strict=True)))
    if set(descriptors) != artifact_finalizer.ARTIFACT_NAMES:
        raise MetadataError("exactly three mandatory artifact descriptors are required")

    output = args.output_directory
    if not output.is_absolute():
        raise MetadataError("--output-directory must be absolute")
    if output.exists() or output.is_symlink():
        raise MetadataError("--output-directory must not already exist")
    try:
        output.resolve(strict=False).relative_to(source_root)
    except ValueError:
        pass
    else:
        raise MetadataError("release metadata must remain outside the Git worktree")

    cosign = artifact_finalizer.executable_file(
        args.cosign.resolve(strict=True), "Cosign"
    )
    key = artifact_finalizer.regular_file(
        args.cosign_key.resolve(strict=True), "Cosign private key"
    )
    public_key = artifact_finalizer.regular_file(
        args.cosign_public_key.resolve(strict=True), "Cosign public key"
    )
    password_file = artifact_finalizer.regular_file(
        args.cosign_password_file.resolve(strict=True), "Cosign password"
    )
    password = password_file.read_text(encoding="utf-8").rstrip("\r\n")
    if not password:
        raise MetadataError("Cosign password file is empty")
    env = artifact_finalizer.safe_tool_environment(password, source_date_epoch)

    output.mkdir(mode=0o700, parents=False)
    try:
        provenance = release_provenance(
            expected_commit=args.expected_commit,
            identity_digest=identity_digest,
            descriptors=descriptors,
            evidence=evidence,
            invocation_id=args.invocation_id,
            source_date_epoch=source_date_epoch,
        )
        provenance_path = output / "release-provenance.intoto.json"
        artifact_finalizer.write_json_new(provenance_path, provenance, canonical=True)
        _, provenance_digest = artifact_finalizer.sha256_file(provenance_path)

        manifest, lock = assembler.assemble(
            source_root=source_root,
            identity_path=identity,
            artifact_arguments=descriptor_arguments,
            release_provenance_digest=provenance_digest,
            rc_definition_path=args.rc_definition,
        )
        manifest_path = output / "release-manifest.json"
        lock_path = output / "artifact-lock.json"
        artifact_finalizer.write_json_new(manifest_path, manifest)
        artifact_finalizer.write_json_new(lock_path, lock)

        bundles = {}
        for path in (provenance_path, manifest_path, lock_path):
            bundle = output / f"{path.name}.cosign.bundle.json"
            bundles[path.name] = sign_and_verify(
                path=path,
                bundle=bundle,
                cosign=cosign,
                key=key,
                public_key=public_key,
                env=env,
                source_root=source_root,
            )

        if artifact_finalizer.git_text(
            source_root, "status", "--porcelain=v1", "--untracked-files=all"
        ):
            raise MetadataError("release metadata generation dirtied the source tree")
        metadata_evidence = {
            "format_version": 1,
            "expected_commit": args.expected_commit,
            "release_identity_digest": identity_digest,
            "release_provenance_digest": provenance_digest,
            "release_manifest_digest": artifact_finalizer.sha256_file(manifest_path)[1],
            "artifact_lock_digest": artifact_finalizer.sha256_file(lock_path)[1],
            "cosign_public_key_digest": artifact_finalizer.sha256_file(public_key)[1],
            "signature_bundle_digests": bundles,
            "verification": "pass",
        }
        evidence_path = output / "metadata-signature-evidence.json"
        artifact_finalizer.write_json_new(evidence_path, metadata_evidence)
        return {
            "release_provenance": str(provenance_path),
            "release_manifest": str(manifest_path),
            "artifact_lock": str(lock_path),
            "metadata_signature_evidence": str(evidence_path),
            "release_provenance_digest": provenance_digest,
        }
    except Exception:
        shutil.rmtree(output, ignore_errors=True)
        raise


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--source-root", type=Path, default=Path("."))
    result.add_argument("--expected-commit", required=True)
    result.add_argument("--identity", type=Path, required=True)
    result.add_argument("--descriptor", action="append", type=parse_descriptor, required=True)
    result.add_argument("--invocation-id", required=True)
    result.add_argument(
        "--rc-definition",
        type=Path,
        default=Path("release-fixtures/rc-gate.json"),
    )
    result.add_argument("--output-directory", type=Path, required=True)
    result.add_argument("--cosign", type=Path, required=True)
    result.add_argument("--cosign-key", type=Path, required=True)
    result.add_argument("--cosign-public-key", type=Path, required=True)
    result.add_argument("--cosign-password-file", type=Path, required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        value = finalize(parser().parse_args(sys.argv[1:] if argv is None else argv))
    except (
        MetadataError,
        artifact_finalizer.FinalizeError,
        assembler.AssemblyError,
        assembler.ReadinessError,
        OSError,
        UnicodeError,
    ) as exc:
        print(f"release metadata finalization failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(value, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Produce and publish immutable E0001 Fixtures from final MuriArc artifacts.

This driver is intentionally fail-closed. It verifies the signed Release Manifest,
artifact lock, release provenance, every final artifact and its per-artifact
provenance/signature evidence before executing the Desktop payload or Server
image. It writes only to the external fixture cache and candidate Catalog path.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import uuid
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any

SCRIPT_ROOT = Path(__file__).resolve().parent
REPOSITORY_ROOT = SCRIPT_ROOT.parent
sys.path.insert(0, str(SCRIPT_ROOT))

import assemble_release_manifest as assembler  # noqa: E402
import check_release_readiness as readiness  # noqa: E402
import finalize_release_artifact as finalizer  # noqa: E402

DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
ARTIFACT_INPUT_ROOT_KEYS = {"format_version", "artifacts"}
ARTIFACT_INPUT_KEYS = {"artifact", "descriptor"}
WINDOWS_INVENTORY_KEYS = {
    "format_version",
    "artifact_name",
    "application_version",
    "source_commit",
    "archive_prefix",
    "files",
}
WINDOWS_FILE_KEYS = {"path", "kind", "size_bytes", "sha256"}
IMAGE_LOCK_KEYS = {
    "format_version",
    "source_commit",
    "server_image",
    "postgres_source_image",
    "postgres_image",
    "server_image_archive_digest",
    "postgres_image_archive_digest",
    "server_signature_bundle_digest",
    "postgres_signature_bundle_digest",
}
SIGNATURE_EVIDENCE_KEYS = {
    "format_version",
    "artifact_name",
    "artifact_digest",
    "scheme",
    "cosign_bundle_digest",
    "cosign_public_key_digest",
    "verification",
    "verified_artifact_unchanged",
}
METADATA_SIGNATURE_EVIDENCE_KEYS = {
    "format_version",
    "expected_commit",
    "release_identity_digest",
    "release_provenance_digest",
    "release_manifest_digest",
    "artifact_lock_digest",
    "cosign_public_key_digest",
    "signature_bundle_digests",
    "verification",
}
METADATA_BUNDLES = {
    "release-provenance.intoto.json": "release-provenance.intoto.json.cosign.bundle.json",
    "release-manifest.json": "release-manifest.json.cosign.bundle.json",
    "artifact-lock.json": "artifact-lock.json.cosign.bundle.json",
}
CANONICAL_ORIGINS = {
    "https://github.com/jarxunlai/MuriArc",
    "https://github.com/jarxunlai/MuriArc.git",
}


class ProducerError(ValueError):
    pass


def exact_keys(value: dict[str, Any], expected: set[str], context: str) -> None:
    actual = set(value)
    if actual != expected:
        raise ProducerError(
            f"{context} keys differ; missing={sorted(expected - actual)}, "
            f"extra={sorted(actual - expected)}"
        )


def object_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ProducerError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def regular_file(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise ProducerError(f"{label} cannot be inspected: {exc}") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ProducerError(f"{label} must be a regular non-symlink file")
    if metadata.st_size <= 0:
        raise ProducerError(f"{label} must not be empty")
    return path


def real_directory(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise ProducerError(f"{label} cannot be inspected: {exc}") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise ProducerError(f"{label} must be a real non-symlink directory")
    return path


def load_json(path: Path, label: str) -> tuple[dict[str, Any], bytes]:
    regular_file(path, label)
    raw = path.read_bytes()
    try:
        value = json.loads(raw, object_pairs_hook=object_no_duplicates)
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise ProducerError(f"{label} is invalid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise ProducerError(f"{label} root must be an object")
    return value, raw


def sha256_file(path: Path) -> tuple[int, str]:
    size = 0
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            size += len(block)
            hasher.update(block)
    return size, f"sha256:{hasher.hexdigest()}"


def require_digest(value: Any, context: str) -> str:
    if not isinstance(value, str) or DIGEST_RE.fullmatch(value) is None:
        raise ProducerError(f"{context} must be a lowercase SHA-256 digest")
    return value


def outside_repository(path: Path, label: str) -> Path:
    resolved = path.resolve(strict=False)
    try:
        resolved.relative_to(REPOSITORY_ROOT.resolve(strict=True))
    except ValueError:
        return resolved
    raise ProducerError(f"{label} must remain outside the Git worktree")


def run(
    command: list[str | os.PathLike[str]],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    capture: bool = True,
) -> str:
    result = subprocess.run(
        [os.fspath(value) for value in command],
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
    )
    if result.returncode != 0:
        program = Path(os.fspath(command[0])).name
        raise ProducerError(f"{program} failed with exit code {result.returncode}")
    return result.stdout.strip() if capture and result.stdout is not None else ""


def git_text(*arguments: str) -> str:
    return run(["git", *arguments], cwd=REPOSITORY_ROOT)


def validate_current_checkout(source_commit: str) -> None:
    root = REPOSITORY_ROOT.resolve(strict=True)
    if git_text("rev-parse", "--show-toplevel") != str(root):
        raise ProducerError("Fixture producer must run from its exact Git worktree")
    origin = git_text("remote", "get-url", "origin").rstrip("/")
    if origin not in {value.rstrip("/") for value in CANONICAL_ORIGINS}:
        raise ProducerError("Fixture producer source origin is not canonical GitHub MuriArc")
    if git_text("rev-parse", "HEAD") != source_commit:
        raise ProducerError("Fixture producer checkout differs from release provenance")
    if git_text("rev-parse", "refs/remotes/origin/main") != source_commit:
        raise ProducerError("Fixture producer checkout is not the locally verified origin/main")
    if git_text("status", "--porcelain=v1", "--untracked-files=all"):
        raise ProducerError("Fixture producer checkout must remain clean")


def executable(value: str, label: str) -> str:
    path = shutil.which(value) if os.sep not in value else value
    if path is None:
        raise ProducerError(f"{label} is unavailable: {value}")
    candidate = Path(path)
    regular_file(candidate, label)
    if os.name != "nt" and candidate.stat().st_mode & 0o111 == 0:
        raise ProducerError(f"{label} is not executable")
    return str(candidate)


def cosign_verify_blob(cosign: str, public_key: Path, payload: Path, bundle: Path) -> None:
    regular_file(bundle, f"{payload.name} Cosign bundle")
    run(
        [cosign, "verify-blob", "--key", public_key, "--bundle", bundle, payload],
        cwd=REPOSITORY_ROOT,
    )


def provenance_source_commit(value: dict[str, Any], context: str) -> str:
    try:
        dependencies = value["predicate"]["buildDefinition"]["resolvedDependencies"]
    except (KeyError, TypeError) as exc:
        raise ProducerError(f"{context} has no resolved dependency set") from exc
    commits = {
        item.get("digest", {}).get("gitCommit")
        for item in dependencies
        if isinstance(item, dict)
        and item.get("uri") == "git+https://github.com/jarxunlai/MuriArc"
    }
    if len(commits) != 1:
        raise ProducerError(f"{context} must bind exactly one canonical Git commit")
    commit = commits.pop()
    if not isinstance(commit, str) or COMMIT_RE.fullmatch(commit) is None:
        raise ProducerError(f"{context} Git commit is invalid")
    return commit


def provenance_subject_digest(value: dict[str, Any], name: str) -> str:
    subjects = value.get("subject")
    if not isinstance(subjects, list):
        raise ProducerError("artifact provenance has no subject array")
    matches = [
        item
        for item in subjects
        if isinstance(item, dict) and item.get("name") == name
    ]
    if len(matches) != 1:
        raise ProducerError(f"artifact provenance must bind one {name} subject")
    digest = matches[0].get("digest", {}).get("sha256")
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise ProducerError(f"artifact provenance {name} digest is invalid")
    return f"sha256:{digest}"


def validate_release_provenance(
    value: dict[str, Any], release: dict[str, Any], source_commit: str
) -> None:
    if value.get("_type") != "https://in-toto.io/Statement/v1" or value.get(
        "predicateType"
    ) != "https://slsa.dev/provenance/v1":
        raise ProducerError("release provenance statement identity is invalid")
    try:
        definition = value["predicate"]["buildDefinition"]
        external = definition["externalParameters"]
        internal = definition["internalParameters"]
    except (KeyError, TypeError) as exc:
        raise ProducerError("release provenance build definition is incomplete") from exc
    if (
        definition.get("buildType")
        != "https://github.com/jarxunlai/MuriArc/release-set/v1"
        or external.get("applicationVersion") != release["application_version"]
        or external.get("dataEpoch") != release["data_epoch"]
        or external.get("artifactNames") != sorted(release["artifacts"])
        or internal.get("checksSkipped") is not False
        or internal.get("singleDigestSet") is not True
    ):
        raise ProducerError("release provenance build parameters are invalid")
    subjects = value.get("subject")
    if not isinstance(subjects, list) or len(subjects) != len(release["artifacts"]):
        raise ProducerError("release provenance must bind exactly the signed artifact set")
    names = [item.get("name") for item in subjects if isinstance(item, dict)]
    if len(names) != len(subjects) or len(set(names)) != len(names):
        raise ProducerError("release provenance artifact subjects must be unique objects")
    if set(names) != set(release["artifacts"]):
        raise ProducerError("release provenance artifact subjects differ from Release Manifest")
    for name, record in release["artifacts"].items():
        if provenance_subject_digest(value, name) != record["digest"]:
            raise ProducerError(f"release provenance {name} subject differs from artifact")
    if provenance_source_commit(value, "release provenance") != source_commit:
        raise ProducerError("release provenance source commit is inconsistent")


def validate_artifact_provenance(
    value: dict[str, Any], name: str, media_type: str, digest: str, source_commit: str
) -> None:
    if value.get("_type") != "https://in-toto.io/Statement/v1" or value.get(
        "predicateType"
    ) != "https://slsa.dev/provenance/v1":
        raise ProducerError(f"{name} provenance statement identity is invalid")
    subjects = value.get("subject")
    if not isinstance(subjects, list) or len(subjects) != 1:
        raise ProducerError(f"{name} provenance must bind exactly one artifact subject")
    if provenance_subject_digest(value, f"MuriArc-{name}") != digest:
        raise ProducerError(f"{name} provenance subject differs from artifact")
    if provenance_source_commit(value, f"{name} provenance") != source_commit:
        raise ProducerError(f"{name} provenance source commit differs from release")
    try:
        definition = value["predicate"]["buildDefinition"]
        external = definition["externalParameters"]
        internal = definition["internalParameters"]
    except (KeyError, TypeError) as exc:
        raise ProducerError(f"{name} provenance build definition is incomplete") from exc
    if (
        definition.get("buildType")
        != "https://github.com/jarxunlai/MuriArc/release-build/v1"
        or external != {"artifactName": name, "mediaType": media_type}
        or internal.get("checksSkipped") is not False
        or internal.get("sourceTreeCleanBeforeAndAfter") is not True
    ):
        raise ProducerError(f"{name} provenance build parameters are invalid")


def validate_signature_evidence(
    value: dict[str, Any],
    name: str,
    digest: str,
    bundle_digest: str,
    public_key_digest: str,
) -> None:
    exact_keys(value, SIGNATURE_EVIDENCE_KEYS, f"{name} signature evidence")
    if (
        value["format_version"] != 1
        or value["artifact_name"] != name
        or value["artifact_digest"] != digest
        or value["scheme"] != "sigstore-cosign-key-pair-bundle-v3"
        or value["cosign_bundle_digest"] != bundle_digest
        or value["cosign_public_key_digest"] != public_key_digest
        or value["verification"] != "pass"
        or value["verified_artifact_unchanged"] is not True
    ):
        raise ProducerError(f"{name} signature evidence does not bind the final artifact")


def validate_metadata_signature_evidence(
    metadata_root: Path,
    value: dict[str, Any],
    source_commit: str,
    provenance_digest: str,
    release_manifest_path: Path,
    artifact_lock_path: Path,
    public_key_digest: str,
) -> None:
    exact_keys(value, METADATA_SIGNATURE_EVIDENCE_KEYS, "metadata signature evidence")
    bundle_digests = value.get("signature_bundle_digests")
    if not isinstance(bundle_digests, dict) or set(bundle_digests) != set(METADATA_BUNDLES):
        raise ProducerError("metadata signature bundle set is incomplete")
    expected_bundles = {
        payload: sha256_file(
            regular_file(metadata_root / bundle, f"{payload} Cosign bundle")
        )[1]
        for payload, bundle in METADATA_BUNDLES.items()
    }
    require_digest(value["release_identity_digest"], "release identity digest")
    if (
        value["format_version"] != 1
        or value["expected_commit"] != source_commit
        or value["release_provenance_digest"] != provenance_digest
        or value["release_manifest_digest"] != sha256_file(release_manifest_path)[1]
        or value["artifact_lock_digest"] != sha256_file(artifact_lock_path)[1]
        or value["cosign_public_key_digest"] != public_key_digest
        or value["signature_bundle_digests"] != expected_bundles
        or value["verification"] != "pass"
    ):
        raise ProducerError("metadata signature evidence differs from signed release metadata")


def validate_release_provenance_dependencies(
    value: dict[str, Any], locked: dict[str, dict[str, Any]]
) -> None:
    try:
        dependencies = value["predicate"]["buildDefinition"]["resolvedDependencies"]
    except (KeyError, TypeError) as exc:
        raise ProducerError("release provenance dependency set is incomplete") from exc
    if not isinstance(dependencies, list):
        raise ProducerError("release provenance dependency set must be an array")
    observed: dict[str, str] = {}
    for item in dependencies:
        if not isinstance(item, dict):
            continue
        uri = item.get("uri")
        if isinstance(uri, str) and uri.startswith("file:") and uri.endswith(
            "/provenance.intoto.json"
        ):
            digest = item.get("digest", {}).get("sha256")
            if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
                raise ProducerError("release provenance contains an invalid artifact dependency")
            if uri in observed:
                raise ProducerError("release provenance contains a duplicate artifact dependency")
            observed[uri] = f"sha256:{digest}"
    expected = {
        f"file:{name}/provenance.intoto.json": record["provenance_digest"]
        for name, record in locked.items()
    }
    if observed != expected:
        raise ProducerError("release provenance artifact dependencies differ from artifact lock")


def validate_signed_release(
    release_manifest_path: Path,
    artifact_lock_path: Path,
    artifact_inputs_path: Path,
    cosign: str,
    public_key: Path,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, dict[str, Path]], str]:
    release, release_raw = load_json(release_manifest_path, "Release Manifest")
    lock, _ = load_json(artifact_lock_path, "artifact lock")
    definition, _ = load_json(REPOSITORY_ROOT / "release-fixtures/rc-gate.json", "RC definition")
    readiness.validate_definition(definition)
    readiness.validate_release_manifest(release, definition)
    readiness.validate_source_identity(REPOSITORY_ROOT, release)
    locked = readiness.validate_artifact_lock(lock, release, release_raw)

    metadata_root = release_manifest_path.parent
    if artifact_lock_path.parent.resolve(strict=True) != metadata_root.resolve(strict=True):
        raise ProducerError("Release Manifest and artifact lock must share one metadata directory")
    provenance_path = regular_file(
        metadata_root / "release-provenance.intoto.json", "release provenance"
    )
    provenance, _ = load_json(provenance_path, "release provenance")
    provenance_digest = sha256_file(provenance_path)[1]
    if provenance_digest != lock["release_provenance_digest"]:
        raise ProducerError("release provenance digest differs from artifact lock")
    source_commit = provenance_source_commit(provenance, "release provenance")
    validate_release_provenance(provenance, release, source_commit)
    validate_release_provenance_dependencies(provenance, locked)
    validate_current_checkout(source_commit)

    public_key_digest = sha256_file(public_key)[1]
    metadata_evidence, _ = load_json(
        metadata_root / "metadata-signature-evidence.json", "metadata signature evidence"
    )
    validate_metadata_signature_evidence(
        metadata_root,
        metadata_evidence,
        source_commit,
        provenance_digest,
        release_manifest_path,
        artifact_lock_path,
        public_key_digest,
    )

    cosign_verify_blob(
        cosign,
        public_key,
        release_manifest_path,
        metadata_root / "release-manifest.json.cosign.bundle.json",
    )
    cosign_verify_blob(
        cosign,
        public_key,
        artifact_lock_path,
        metadata_root / "artifact-lock.json.cosign.bundle.json",
    )
    cosign_verify_blob(
        cosign,
        public_key,
        provenance_path,
        metadata_root / "release-provenance.intoto.json.cosign.bundle.json",
    )

    inputs, _ = load_json(artifact_inputs_path, "artifact input map")
    exact_keys(inputs, ARTIFACT_INPUT_ROOT_KEYS, "artifact input map")
    if inputs["format_version"] != 1 or not isinstance(inputs["artifacts"], dict):
        raise ProducerError("artifact input map format is invalid")
    if set(inputs["artifacts"]) != set(release["artifacts"]):
        raise ProducerError("artifact input map must cover exactly the signed artifact set")

    resolved: dict[str, dict[str, Path]] = {}
    for name, record in inputs["artifacts"].items():
        if not isinstance(record, dict):
            raise ProducerError(f"artifact input {name} must be an object")
        exact_keys(record, ARTIFACT_INPUT_KEYS, f"artifact input {name}")
        artifact = regular_file(
            outside_repository(
                Path(record["artifact"]).resolve(strict=True), f"{name} artifact"
            ),
            f"{name} artifact",
        )
        descriptor_path = regular_file(
            outside_repository(
                Path(record["descriptor"]).resolve(strict=True), f"{name} descriptor"
            ),
            f"{name} descriptor",
        )
        descriptor, _ = load_json(descriptor_path, f"{name} descriptor")
        assembler.exact_keys(descriptor, assembler.DESCRIPTOR_KEYS, f"{name} descriptor")
        expected = locked[name]
        size, digest = sha256_file(artifact)
        if (
            descriptor["artifact_name"] != name
            or descriptor["format_version"] != 1
            or descriptor["digest"] != digest
            or descriptor["size_bytes"] != size
            or descriptor["digest"] != expected["digest"]
            or descriptor["size_bytes"] != expected["size_bytes"]
            or descriptor["media_type"] != release["artifacts"][name]["media_type"]
            or descriptor["provenance_digest"] != expected["provenance_digest"]
            or descriptor["signature_evidence_digest"]
            != expected["signature_evidence_digest"]
        ):
            raise ProducerError(f"{name} artifact/descriptor differs from signed artifact lock")
        evidence_root = descriptor_path.parent
        artifact_provenance = regular_file(
            evidence_root / "provenance.intoto.json", f"{name} provenance"
        )
        signature_evidence = regular_file(
            evidence_root / "signature-evidence.json", f"{name} signature evidence"
        )
        artifact_bundle = regular_file(
            evidence_root / "artifact.cosign.bundle.json", f"{name} artifact Cosign bundle"
        )
        if sha256_file(artifact_provenance)[1] != expected["provenance_digest"]:
            raise ProducerError(f"{name} provenance digest differs from artifact lock")
        if sha256_file(signature_evidence)[1] != expected["signature_evidence_digest"]:
            raise ProducerError(f"{name} signature evidence differs from artifact lock")
        artifact_provenance_value, _ = load_json(artifact_provenance, f"{name} provenance")
        validate_artifact_provenance(
            artifact_provenance_value,
            name,
            descriptor["media_type"],
            digest,
            source_commit,
        )
        signature_evidence_value, _ = load_json(
            signature_evidence, f"{name} signature evidence"
        )
        validate_signature_evidence(
            signature_evidence_value,
            name,
            digest,
            sha256_file(artifact_bundle)[1],
            public_key_digest,
        )
        cosign_verify_blob(
            cosign,
            public_key,
            artifact,
            artifact_bundle,
        )
        resolved[name] = {
            "artifact": artifact,
            "descriptor": descriptor_path,
            "evidence_root": evidence_root,
        }
    return release, lock, resolved, source_commit


def safe_zip_extract(archive: Path, destination: Path) -> Path:
    with zipfile.ZipFile(archive) as bundle:
        names: set[str] = set()
        folded_names: set[str] = set()
        for entry in bundle.infolist():
            path = PurePosixPath(entry.filename)
            normalized = path.as_posix()
            mode = (entry.external_attr >> 16) & 0o170000
            folded = normalized.casefold()
            if (
                not entry.filename
                or "\\" in entry.filename
                or "\x00" in entry.filename
                or path.is_absolute()
                or path == PurePosixPath(".")
                or ".." in path.parts
                or normalized in names
                or folded in folded_names
                or mode not in {0, stat.S_IFREG, stat.S_IFDIR}
            ):
                raise ProducerError(f"unsafe Windows ZIP entry: {entry.filename}")
            names.add(normalized)
            folded_names.add(folded)
        for entry in bundle.infolist():
            if entry.is_dir():
                continue
            path = PurePosixPath(entry.filename)
            target = destination.joinpath(*path.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(entry) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
    inventories = list(destination.rglob("artifact-inventory.json"))
    if len(inventories) != 1:
        raise ProducerError("Windows artifact must contain exactly one inventory")
    inventory_path = inventories[0]
    inventory, _ = load_json(inventory_path, "Windows artifact inventory")
    exact_keys(inventory, WINDOWS_INVENTORY_KEYS, "Windows artifact inventory")
    root = inventory_path.parent
    if (
        inventory["format_version"] != 1
        or inventory["artifact_name"] != "desktop-windows"
        or inventory["application_version"] != "1.0.0"
        or inventory["archive_prefix"] != root.name
        or not isinstance(inventory["files"], list)
    ):
        raise ProducerError("Windows artifact inventory identity is invalid")
    expected = {"artifact-inventory.json"}
    folded_expected = {"artifact-inventory.json"}
    producer_paths: list[Path] = []
    for index, record in enumerate(inventory["files"]):
        if not isinstance(record, dict):
            raise ProducerError(f"Windows inventory file {index} must be an object")
        exact_keys(record, WINDOWS_FILE_KEYS, f"Windows inventory file {index}")
        raw_path = record["path"]
        if not isinstance(raw_path, str) or "\\" in raw_path or "\x00" in raw_path:
            raise ProducerError("Windows inventory path must be a portable string")
        relative = PurePosixPath(raw_path)
        normalized = relative.as_posix()
        folded = normalized.casefold()
        if (
            relative.is_absolute()
            or relative == PurePosixPath(".")
            or ".." in relative.parts
            or normalized in expected
            or folded in folded_expected
        ):
            raise ProducerError("Windows inventory contains an unsafe or duplicate path")
        if (
            not isinstance(record["kind"], str)
            or not record["kind"]
            or isinstance(record["size_bytes"], bool)
            or not isinstance(record["size_bytes"], int)
            or record["size_bytes"] <= 0
        ):
            raise ProducerError("Windows inventory payload metadata is invalid")
        require_digest(record["sha256"], "Windows inventory payload digest")
        payload = regular_file(root.joinpath(*relative.parts), "Windows inventory payload")
        size, digest = sha256_file(payload)
        if size != record["size_bytes"] or digest != record["sha256"]:
            raise ProducerError(f"Windows inventory payload differs: {relative}")
        expected.add(normalized)
        folded_expected.add(folded)
        if record["kind"] == "fixture-producer-executable":
            producer_paths.append(payload)
    actual = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
    }
    if actual != expected or len(producer_paths) != 1:
        raise ProducerError("Windows artifact inventory is not closed or lacks one producer")
    producer = producer_paths[0]
    if os.name != "nt":
        producer.chmod(producer.stat().st_mode | 0o500)
    return producer


def safe_tar_extract(archive: Path, destination: Path) -> Path:
    with tarfile.open(archive, "r:*") as bundle:
        members = bundle.getmembers()
        names: set[str] = set()
        for member in members:
            path = PurePosixPath(member.name)
            normalized = path.as_posix()
            if (
                not member.name
                or "\\" in member.name
                or "\x00" in member.name
                or path.is_absolute()
                or path == PurePosixPath(".")
                or ".." in path.parts
                or normalized in names
                or not (member.isdir() or member.isfile())
            ):
                raise ProducerError(f"unsafe Server artifact entry: {member.name}")
            names.add(normalized)
        for member in members:
            path = PurePosixPath(member.name)
            target = destination.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            source = bundle.extractfile(member)
            if source is None:
                raise ProducerError(f"cannot read Server artifact entry: {member.name}")
            with source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
    manifests = list(destination.rglob("bundle-manifest.json"))
    if len(manifests) != 1:
        raise ProducerError("Server artifact must contain exactly one bundle manifest")
    manifest_path = manifests[0]
    manifest, _ = load_json(manifest_path, "Server bundle manifest")
    if (
        set(manifest) != {"format_version", "application_version", "profile", "files"}
        or manifest["format_version"] != 1
        or manifest["application_version"] != "1.0.0"
        or manifest["profile"] != "managed_compose"
        or not isinstance(manifest["files"], list)
    ):
        raise ProducerError("Managed Compose bundle manifest identity is invalid")
    root = manifest_path.parent
    expected = {"bundle-manifest.json"}
    for index, record in enumerate(manifest["files"]):
        if not isinstance(record, dict) or set(record) != {"path", "role", "size_bytes", "sha256"}:
            raise ProducerError(f"Server bundle file {index} has invalid schema")
        raw_path = record["path"]
        if not isinstance(raw_path, str) or "\\" in raw_path or "\x00" in raw_path:
            raise ProducerError("Server bundle path must be a portable string")
        relative = PurePosixPath(raw_path)
        normalized = relative.as_posix()
        if (
            relative.is_absolute()
            or relative == PurePosixPath(".")
            or ".." in relative.parts
            or normalized in expected
            or not isinstance(record["role"], str)
            or not record["role"]
            or isinstance(record["size_bytes"], bool)
            or not isinstance(record["size_bytes"], int)
            or record["size_bytes"] <= 0
        ):
            raise ProducerError("Server bundle contains unsafe or invalid payload metadata")
        require_digest(record["sha256"], "Server bundle payload digest")
        payload = regular_file(root.joinpath(*relative.parts), "Server bundle payload")
        size, digest = sha256_file(payload)
        if size != record["size_bytes"] or digest != record["sha256"]:
            raise ProducerError(f"Server bundle payload differs: {relative}")
        expected.add(normalized)
    actual = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
    }
    if actual != expected:
        raise ProducerError("Managed Compose bundle file set differs from its manifest")
    return root


def windows_path(path: Path, wslpath: str | None) -> str:
    if wslpath is None:
        return str(path)
    converted = run([wslpath, "-w", path])
    if not converted or "\n" in converted or "\r" in converted:
        raise ProducerError("wslpath returned an invalid Windows path")
    return converted


def run_desktop_fixture(
    executable_path: Path,
    fixture_root: Path,
    output: Path,
    source_commit: str,
    artifact_digest: str,
    provenance_digest: str,
) -> None:
    wslpath = None
    if os.name != "nt" and executable_path.suffix.lower() == ".exe":
        wslpath = shutil.which("wslpath")
        if wslpath is None:
            raise ProducerError("Windows Fixture production from Linux requires WSL interop")
    executable_arg = str(executable_path)
    fixture_arg = windows_path(fixture_root, wslpath)
    output_arg = windows_path(output, wslpath)
    run(
        [
            executable_arg,
            "--muriarc-release-fixture",
            "prepare-sqlite",
            "--fixture",
            fixture_arg,
            "--output",
            output_arg,
            "--source-commit",
            source_commit,
        ]
    )
    run(
        [
            executable_arg,
            "--muriarc-release-fixture",
            "finalize",
            "--root",
            output_arg,
            "--backend",
            "sqlite",
            "--source-artifact-digest",
            artifact_digest,
            "--source-provenance-digest",
            provenance_digest,
        ]
    )


def tar_member_bytes(archive: tarfile.TarFile, name: str, label: str) -> bytes:
    try:
        member = archive.getmember(name)
    except KeyError as exc:
        raise ProducerError(f"Docker image archive has no {label}") from exc
    if not member.isfile():
        raise ProducerError(f"Docker image archive {label} must be a regular file")
    source = archive.extractfile(member)
    if source is None:
        raise ProducerError(f"Docker image archive {label} cannot be read")
    with source:
        return source.read()


def archive_json(raw: bytes, label: str) -> Any:
    try:
        return json.loads(raw, object_pairs_hook=object_no_duplicates)
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise ProducerError(f"Docker image archive {label} is invalid JSON") from exc


def digest_tar_blob(archive: tarfile.TarFile, digest: str, label: str) -> bytes:
    require_digest(digest, label)
    algorithm, hexadecimal = digest.split(":", 1)
    raw = tar_member_bytes(archive, f"blobs/{algorithm}/{hexadecimal}", label)
    observed = f"sha256:{hashlib.sha256(raw).hexdigest()}"
    if observed != digest:
        raise ProducerError(f"Docker image archive {label} digest differs")
    return raw


def docker_archive_image_id(path: Path) -> str:
    with tarfile.open(path, "r:*") as archive:
        names = {member.name for member in archive.getmembers()}
        if "manifest.json" in names:
            value = archive_json(
                tar_member_bytes(archive, "manifest.json", "manifest.json"),
                "manifest.json",
            )
            if not isinstance(value, list) or len(value) != 1 or not isinstance(value[0], dict):
                raise ProducerError("Docker image archive must contain exactly one image")
            config_name = value[0].get("Config")
            if not isinstance(config_name, str) or "\\" in config_name:
                raise ProducerError("Docker image archive config path is invalid")
            config = PurePosixPath(config_name)
            if config.is_absolute() or ".." in config.parts:
                raise ProducerError("Docker image archive config path is unsafe")
            candidate = config.name.removesuffix(".json")
            if re.fullmatch(r"[0-9a-f]{64}", candidate) is None:
                raise ProducerError("Docker image archive config digest is invalid")
            raw = tar_member_bytes(archive, config.as_posix(), "image config")
            if hashlib.sha256(raw).hexdigest() != candidate:
                raise ProducerError("Docker image archive config content digest differs")
            return f"sha256:{candidate}"

        if "index.json" in names and "oci-layout" in names:
            index = archive_json(
                tar_member_bytes(archive, "index.json", "OCI index"), "OCI index"
            )
            manifests = index.get("manifests") if isinstance(index, dict) else None
            if not isinstance(manifests, list) or len(manifests) != 1:
                raise ProducerError("OCI image archive must contain exactly one manifest")
            descriptor = manifests[0]
            if not isinstance(descriptor, dict):
                raise ProducerError("OCI image archive manifest descriptor is invalid")
            manifest_digest = require_digest(
                descriptor.get("digest"), "OCI image manifest digest"
            )
            manifest = archive_json(
                digest_tar_blob(archive, manifest_digest, "OCI image manifest"),
                "OCI image manifest",
            )
            config = manifest.get("config") if isinstance(manifest, dict) else None
            if not isinstance(config, dict):
                raise ProducerError("OCI image manifest config descriptor is invalid")
            config_digest = require_digest(config.get("digest"), "OCI image config digest")
            digest_tar_blob(archive, config_digest, "OCI image config")
            return config_digest

    raise ProducerError("Docker image archive is neither Docker save nor OCI format")


def verify_image_lock(root: Path, source_commit: str) -> tuple[Path, Path, dict[str, Any]]:
    image_lock_path = regular_file(root / "images/image-lock.json", "image lock")
    lock, _ = load_json(image_lock_path, "image lock")
    exact_keys(lock, IMAGE_LOCK_KEYS, "image lock")
    if lock["format_version"] != 1 or lock["source_commit"] != source_commit:
        raise ProducerError("image lock source identity differs from release provenance")
    for key in IMAGE_LOCK_KEYS - {
        "format_version",
        "source_commit",
        "server_image",
        "postgres_source_image",
        "postgres_image",
    }:
        require_digest(lock[key], f"image lock {key}")
    for key in ("server_image", "postgres_image"):
        reference = lock[key]
        if (
            not isinstance(reference, str)
            or re.fullmatch(r"ghcr\.io/jarxunlai/[A-Za-z0-9._/-]+@sha256:[0-9a-f]{64}", reference)
            is None
        ):
            raise ProducerError(f"image lock {key} must be a digest-pinned MuriArc GHCR image")
    source_reference = lock["postgres_source_image"]
    if (
        not isinstance(source_reference, str)
        or re.fullmatch(r"[A-Za-z0-9._/:+-]+@sha256:[0-9a-f]{64}", source_reference)
        is None
    ):
        raise ProducerError("image lock PostgreSQL source image must be digest-pinned")
    server = regular_file(root / "images/muriarc-server.docker.tar", "Server image archive")
    postgres = regular_file(root / "images/postgres-17.docker.tar", "PostgreSQL image archive")
    if sha256_file(server)[1] != lock["server_image_archive_digest"]:
        raise ProducerError("Server image archive differs from image lock")
    if sha256_file(postgres)[1] != lock["postgres_image_archive_digest"]:
        raise ProducerError("PostgreSQL image archive differs from image lock")
    return server, postgres, lock


def run_postgres_fixture(
    managed_root: Path,
    fixture_root: Path,
    output: Path,
    source_commit: str,
    artifact_digest: str,
    provenance_digest: str,
    docker: str,
    cosign: str,
    public_key: Path,
) -> None:
    server_archive, postgres_archive, image_lock = verify_image_lock(managed_root, source_commit)
    server_signature = regular_file(
        managed_root / "images/muriarc-server.cosign.bundle.json", "Server image signature"
    )
    postgres_signature = regular_file(
        managed_root / "images/postgres-17.cosign.bundle.json", "PostgreSQL image signature"
    )
    if sha256_file(server_signature)[1] != image_lock["server_signature_bundle_digest"]:
        raise ProducerError("Server image signature bundle differs from image lock")
    if sha256_file(postgres_signature)[1] != image_lock["postgres_signature_bundle_digest"]:
        raise ProducerError("PostgreSQL image signature bundle differs from image lock")
    run([cosign, "verify", "--key", public_key, "--bundle", server_signature, image_lock["server_image"]])
    run([cosign, "verify", "--key", public_key, "--bundle", postgres_signature, image_lock["postgres_image"]])

    server_id = docker_archive_image_id(server_archive)
    postgres_id = docker_archive_image_id(postgres_archive)
    run([docker, "load", "--input", server_archive])
    run([docker, "load", "--input", postgres_archive])
    run([docker, "image", "inspect", server_id])
    run([docker, "image", "inspect", postgres_id])

    suffix = uuid.uuid4().hex[:12]
    network = f"muriarc-fixture-{suffix}"
    database = f"muriarc-fixture-db-{suffix}"
    run([docker, "network", "create", network])
    try:
        run(
            [
                docker,
                "run",
                "--detach",
                "--name",
                database,
                "--network",
                network,
                "--network-alias",
                "database",
                "--env",
                "POSTGRES_HOST_AUTH_METHOD=trust",
                postgres_id,
            ]
        )
        for _ in range(90):
            probe = subprocess.run(
                [docker, "exec", database, "pg_isready", "-U", "postgres"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            if probe.returncode == 0:
                break
            import time

            time.sleep(1)
        else:
            raise ProducerError("temporary PostgreSQL 17 container did not become ready")

        output.parent.mkdir(parents=True, exist_ok=True)
        uid_gid = f"{os.getuid()}:{os.getgid()}" if hasattr(os, "getuid") else "10001:10001"
        base = [
            docker,
            "run",
            "--rm",
            "--network",
            network,
            "--user",
            uid_gid,
            "--mount",
            f"type=bind,src={fixture_root},dst=/fixture-definition,readonly",
            "--mount",
            f"type=bind,src={output.parent},dst=/fixture-output",
            "--env",
            "MURIARC_FIXTURE_DATABASE_URL=postgresql://postgres@database/postgres",
            "--entrypoint",
            "/usr/local/bin/muriarc-release-fixture",
            server_id,
        ]
        run(
            base
            + [
                "prepare-postgres",
                "--fixture",
                "/fixture-definition",
                "--output",
                f"/fixture-output/{output.name}",
                "--source-commit",
                source_commit,
            ]
        )
        (output / "database").mkdir()
        run(
            [
                docker,
                "exec",
                database,
                "pg_dump",
                "--username",
                "postgres",
                "--format=custom",
                "--file=/tmp/postgres.dump",
                "postgres",
            ]
        )
        run([docker, "cp", f"{database}:/tmp/postgres.dump", output / "database/postgres.dump"])
        run(
            base
            + [
                "finalize",
                "--root",
                f"/fixture-output/{output.name}",
                "--backend",
                "postgres",
                "--source-artifact-digest",
                artifact_digest,
                "--source-provenance-digest",
                provenance_digest,
            ]
        )
    finally:
        subprocess.run(
            [docker, "rm", "--force", database],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        subprocess.run(
            [docker, "network", "rm", network],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )


def parse_asset_verification(raw: str) -> dict[str, Any]:
    try:
        value = json.loads(raw, object_pairs_hook=object_no_duplicates)
    except json.JSONDecodeError as exc:
        raise ProducerError("final verifier returned invalid JSON") from exc
    if not isinstance(value, dict):
        raise ProducerError("final verifier response must be an object")
    exact_keys(value, {"ok", "code", "message", "data"}, "final verifier response")
    data = value.get("data")
    expected = {
        "fixture_id",
        "fixture_manifest_digest",
        "expected_facts_digest",
        "fixture_content_digest",
        "verified_file_count",
        "verified_bytes",
    }
    if value["ok"] is not True or value["code"] != "ok" or not isinstance(data, dict):
        raise ProducerError("final verifier rejected generated Fixture")
    exact_keys(data, expected, "final verifier asset data")
    for key in (
        "fixture_manifest_digest",
        "expected_facts_digest",
        "fixture_content_digest",
    ):
        require_digest(data[key], f"final verifier {key}")
    for key in ("verified_file_count", "verified_bytes"):
        if isinstance(data[key], bool) or not isinstance(data[key], int) or data[key] <= 0:
            raise ProducerError(f"final verifier {key} must be a positive integer")
    if not isinstance(data["fixture_id"], str):
        raise ProducerError("final verifier fixture_id must be a string")
    return data


def verify_fixture(verifier: str, root: Path) -> dict[str, Any]:
    real_directory(root, "Fixture root")
    result = run([verifier, "asset", "--root", root, "--output", "json"])
    verification = parse_asset_verification(result)
    manifest, _ = load_json(root / "fixture-manifest.json", "Fixture manifest")
    if manifest.get("fixture_id") != verification["fixture_id"]:
        raise ProducerError("final verifier Fixture identity differs from manifest")
    return manifest


def publish_fixture(
    root: Path,
    backend: str,
    manifest: dict[str, Any],
    artifact_digest: str,
    verifier: str,
    repository: str,
    environment: dict[str, str],
) -> dict[str, Any]:
    manifest_digest = sha256_file(root / "fixture-manifest.json")[1]
    # The typed manifest digest is canonical JSON rather than the pretty file digest.
    verification = run([verifier, "asset", "--root", root, "--output", "json"])
    typed = parse_asset_verification(verification)["fixture_manifest_digest"]
    tag = (
        f"v1.0.0-e0001-{backend}-"
        f"{artifact_digest.removeprefix('sha256:')[:12]}-{typed.removeprefix('sha256:')[:12]}"
    )
    output = run(
        [
            REPOSITORY_ROOT / "scripts/publish-release-fixture.sh",
            "--fixture",
            root,
            "--repository",
            repository,
            "--tag",
            tag,
            "--manifest-digest",
            typed,
        ],
        cwd=REPOSITORY_ROOT,
        env=environment,
    )
    try:
        value = json.loads(output, object_pairs_hook=object_no_duplicates)
    except json.JSONDecodeError as exc:
        raise ProducerError("Fixture publisher returned invalid JSON") from exc
    if not isinstance(value, dict):
        raise ProducerError("Fixture publisher response must be an object")
    exact_keys(
        value,
        {
            "oci_reference",
            "fixture_artifact_digest",
            "fixture_tar_digest",
            "fixture_manifest_digest",
        },
        "Fixture publisher response",
    )
    if value["fixture_manifest_digest"] != typed:
        raise ProducerError("Fixture publisher changed the manifest digest")
    artifact_oci_digest = require_digest(
        value["fixture_artifact_digest"], "published Fixture OCI digest"
    )
    require_digest(value["fixture_tar_digest"], "published Fixture tar digest")
    reference = value["oci_reference"]
    if reference != f"{repository}@{artifact_oci_digest}":
        raise ProducerError("Fixture publisher returned a mutable or mismatched OCI reference")
    # Retain the pretty-file digest only as a local diagnostic; Catalog uses the typed digest.
    value["fixture_manifest_file_digest"] = manifest_digest
    value["fixture_id"] = manifest["fixture_id"]
    return value


def append_catalog(
    verifier: str,
    baseline: Path,
    fixtures: list[tuple[Path, dict[str, Any]]],
    output: Path,
    temporary: Path,
) -> None:
    previous = baseline
    for index, (fixture_root, published) in enumerate(fixtures):
        candidate = output if index == len(fixtures) - 1 else temporary / f"catalog-{index}.json"
        response = run(
            [
                verifier,
                "catalog-append",
                "--catalog",
                previous,
                "--fixture-root",
                fixture_root,
                "--fixture-artifact-digest",
                published["fixture_artifact_digest"],
                "--oci-reference",
                published["oci_reference"],
                "--candidate-output",
                candidate,
                "--output",
                "json",
            ]
        )
        try:
            wrapper = json.loads(response, object_pairs_hook=object_no_duplicates)
        except json.JSONDecodeError as exc:
            raise ProducerError("Catalog append verifier returned invalid JSON") from exc
        if (
            not isinstance(wrapper, dict)
            or set(wrapper) != {"ok", "code", "message", "data"}
            or wrapper.get("ok") is not True
            or wrapper.get("code") != "ok"
            or not isinstance(wrapper.get("data"), dict)
            or set(wrapper["data"])
            != {"entryCount", "fixtureId", "fixtureContentDigest"}
        ):
            raise ProducerError("Catalog append verifier returned an invalid response")
        require_digest(
            wrapper["data"]["fixtureContentDigest"], "Catalog append content digest"
        )
        if (
            isinstance(wrapper["data"]["entryCount"], bool)
            or not isinstance(wrapper["data"]["entryCount"], int)
            or wrapper["data"]["entryCount"] <= 0
            or not isinstance(wrapper["data"]["fixtureId"], str)
            or not wrapper["data"]["fixtureId"]
        ):
            raise ProducerError("Catalog append verifier returned invalid result fields")
        previous = candidate


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--release-manifest", type=Path, required=True)
    result.add_argument("--artifact-lock", type=Path, required=True)
    result.add_argument("--catalog-baseline", type=Path, required=True)
    result.add_argument("--fixture-cache", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    return result


def produce(args: argparse.Namespace) -> dict[str, Any]:
    verifier = executable(os.environ.get("MURIARC_VERIFIER", ""), "final verifier")
    cosign = executable(os.environ.get("MURIARC_COSIGN", "cosign"), "Cosign")
    docker = executable(os.environ.get("MURIARC_DOCKER", "docker"), "Docker")
    public_key_value = os.environ.get("COSIGN_PUBLIC_KEY", "")
    if not public_key_value:
        raise ProducerError("COSIGN_PUBLIC_KEY is required")
    public_key = regular_file(Path(public_key_value).resolve(strict=True), "Cosign public key")
    inputs_value = os.environ.get("MURIARC_RELEASE_ARTIFACT_INPUTS", "")
    if not inputs_value:
        raise ProducerError("MURIARC_RELEASE_ARTIFACT_INPUTS is required")
    artifact_inputs = outside_repository(
        Path(inputs_value).resolve(strict=True), "artifact input map"
    )
    repository = os.environ.get(
        "MURIARC_RELEASE_FIXTURE_REPOSITORY",
        "ghcr.io/jarxunlai/muriarc-release-fixtures",
    )
    if not re.fullmatch(r"ghcr\.io/[A-Za-z0-9._/-]+", repository) or ":" in repository or "@" in repository:
        raise ProducerError("MURIARC_RELEASE_FIXTURE_REPOSITORY must be an untagged GHCR repository")

    release_manifest = outside_repository(
        args.release_manifest.resolve(strict=True), "Release Manifest"
    )
    artifact_lock = outside_repository(args.artifact_lock.resolve(strict=True), "artifact lock")
    baseline = regular_file(args.catalog_baseline.resolve(strict=True), "Catalog baseline")
    output = outside_repository(args.output, "candidate Catalog output")
    if output.exists() or output.is_symlink():
        raise ProducerError("candidate Catalog output must be a new path")
    fixture_cache = outside_repository(args.fixture_cache, "Fixture cache")
    fixture_cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    real_directory(fixture_cache, "Fixture cache")

    release, lock, artifacts, source_commit = validate_signed_release(
        release_manifest,
        artifact_lock,
        artifact_inputs,
        cosign,
        public_key,
    )
    working = Path(tempfile.mkdtemp(prefix=".release-fixture-producer-", dir=fixture_cache))
    try:
        extracted_windows = working / "desktop"
        extracted_windows.mkdir()
        desktop_executable = safe_zip_extract(
            artifacts["desktop-windows"]["artifact"], extracted_windows
        )
        inventory, _ = load_json(
            desktop_executable.parents[1] / "artifact-inventory.json",
            "Windows artifact inventory",
        )
        if inventory["source_commit"] != source_commit:
            raise ProducerError("Windows artifact source commit differs from release provenance")

        extracted_managed = working / "managed"
        extracted_managed.mkdir()
        managed_root = safe_tar_extract(
            artifacts["managed-compose"]["artifact"], extracted_managed
        )
        fixture_definition = real_directory(
            REPOSITORY_ROOT / "fixtures/standard-v1", "standard-v1 definition"
        )
        fixtures_root = fixture_cache / "v1.0.0-e0001"
        fixtures_root.mkdir(exist_ok=True, mode=0o700)
        real_directory(fixtures_root, "versioned Fixture cache")
        generated: list[tuple[str, Path]] = []
        for backend, artifact_name in (
            ("sqlite", "desktop-windows"),
            ("postgres", "managed-compose"),
        ):
            artifact_digest = lock["artifacts"][artifact_name]["digest"]
            provenance_digest = lock["artifacts"][artifact_name]["provenance_digest"]
            final_root = fixtures_root / (
                f"{backend}-{artifact_digest.removeprefix('sha256:')[:16]}"
            )
            if final_root.exists() or final_root.is_symlink():
                real_directory(final_root, f"cached {backend} Fixture")
                manifest = verify_fixture(verifier, final_root)
                producer = manifest.get("producer", {})
                if (
                    manifest.get("backend") != backend
                    or producer.get("source_release_artifact_digest") != artifact_digest
                    or producer.get("source_release_provenance_digest") != provenance_digest
                ):
                    raise ProducerError(f"cached {backend} Fixture differs from final artifact")
            else:
                staging = working / f"{backend}-fixture"
                if backend == "sqlite":
                    run_desktop_fixture(
                        desktop_executable,
                        fixture_definition,
                        staging,
                        source_commit,
                        artifact_digest,
                        provenance_digest,
                    )
                else:
                    run_postgres_fixture(
                        managed_root,
                        fixture_definition,
                        staging,
                        source_commit,
                        artifact_digest,
                        provenance_digest,
                        docker,
                        cosign,
                        public_key,
                    )
                verify_fixture(verifier, staging)
                os.replace(staging, final_root)
            generated.append((backend, final_root))

        environment = os.environ.copy()
        environment["MURIARC_VERIFIER"] = verifier
        environment["MURIARC_COSIGN"] = cosign
        published: list[tuple[Path, dict[str, Any]]] = []
        publication_summary = []
        for backend, root in generated:
            manifest = verify_fixture(verifier, root)
            artifact_name = "desktop-windows" if backend == "sqlite" else "managed-compose"
            result = publish_fixture(
                root,
                backend,
                manifest,
                release["artifacts"][artifact_name]["digest"],
                verifier,
                repository,
                environment,
            )
            published.append((root, result))
            publication_summary.append(
                {
                    "backend": backend,
                    "fixture_id": result["fixture_id"],
                    "oci_reference": result["oci_reference"],
                    "fixture_artifact_digest": result["fixture_artifact_digest"],
                    "fixture_manifest_digest": result["fixture_manifest_digest"],
                }
            )
        output.parent.mkdir(parents=True, exist_ok=True)
        append_catalog(verifier, baseline, published, output, working)
        run(
            [
                sys.executable,
                REPOSITORY_ROOT / "scripts/check_fixture_catalog.py",
                "--catalog",
                output,
                "--previous",
                baseline,
                "--require-non-empty",
            ]
        )
        return {
            "format_version": 1,
            "application_version": release["application_version"],
            "data_epoch": release["data_epoch"],
            "source_commit": source_commit,
            "candidate_catalog": str(output),
            "fixtures": publication_summary,
        }
    finally:
        shutil.rmtree(working, ignore_errors=True)


def main(argv: list[str] | None = None) -> int:
    try:
        result = produce(parser().parse_args(sys.argv[1:] if argv is None else argv))
    except (
        ProducerError,
        readiness.ReadinessError,
        finalizer.FinalizeError,
        OSError,
        ValueError,
        subprocess.SubprocessError,
    ) as exc:
        print(f"release Fixture producer failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Dispatch one real final-package compatibility run and close seven-layer evidence."""

from __future__ import annotations

import argparse
import json
import os
import sys
import uuid
from pathlib import Path
from typing import Any

import release_driver_common as common


PROFILE_RUNNERS = {
    "native-system": "MURIARC_COMPATIBILITY_NATIVE_RUNNER",
    "managed-compose": "MURIARC_COMPATIBILITY_COMPOSE_RUNNER",
    "desktop-windows": "MURIARC_COMPATIBILITY_WINDOWS_RUNNER",
}
BACKEND_PROFILES = {
    "sqlite": {"desktop-windows"},
    "postgres": {"native-system", "managed-compose"},
}
LAYER_FILES = {
    "storage": "storage.json",
    "store_application": "store_application.json",
    "api": "api.json",
    "remote_ui": "remote_ui.json",
    "continue_write": "continue_write.json",
    "read_only_no_side_effects": "read_only_no_side_effects.json",
}
RESULT_KEYS = {
    "format_version",
    "fixture_id",
    "backend",
    "profile",
    "release_manifest_digest",
    "artifact_lock_digest",
    "target_artifact_digest",
    "target_artifact_size_bytes",
    "execution_kind",
    "status",
    "fail_count",
    "skip_count",
    "evidence_files",
    "started_at",
    "completed_at",
}


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--mode", choices=("pr", "nightly", "rc"), required=True)
    result.add_argument("--fixture-id", required=True)
    result.add_argument("--fixture-manifest-digest", required=True)
    result.add_argument("--fixture-root", type=Path, required=True)
    result.add_argument("--profile", choices=tuple(PROFILE_RUNNERS), required=True)
    result.add_argument("--target-artifacts", type=Path, required=True)
    result.add_argument("--report", type=Path, required=True)
    return result


def require_backend_profile(backend: Any, profile: str) -> str:
    if not isinstance(backend, str) or backend not in BACKEND_PROFILES:
        raise common.DriverError("Fixture Manifest backend is unsupported")
    if profile not in BACKEND_PROFILES[backend]:
        raise common.DriverError(
            "Fixture backend is incompatible with the requested delivery profile"
        )
    return backend


def validate_runner_result(
    value: dict[str, Any],
    *,
    raw_release: bytes,
    raw_lock: bytes,
    fixture_id: str,
    backend: str,
    profile: str,
    artifact_digest: str,
    artifact_size: int,
    evidence_directory: Path,
) -> None:
    common.exact_keys(value, RESULT_KEYS, "compatibility runner result")
    if value["format_version"] != 1:
        raise common.DriverError("compatibility runner format is unsupported")
    if (
        value["fixture_id"] != fixture_id
        or value["backend"] != backend
        or value["profile"] != profile
        or value["release_manifest_digest"] != common.sha256_bytes(raw_release)
        or value["artifact_lock_digest"] != common.sha256_bytes(raw_lock)
        or value["target_artifact_digest"] != artifact_digest
        or value["target_artifact_size_bytes"] != artifact_size
    ):
        raise common.DriverError("compatibility runner identity differs from pinned inputs")
    if (
        value["execution_kind"] != "final_package"
        or value["status"] != "pass"
        or common.require_count(value["fail_count"], "compatibility fail_count") != 0
        or common.require_count(value["skip_count"], "compatibility skip_count") != 0
    ):
        raise common.DriverError("compatibility runner contains FAIL, SKIP, or non-final execution")
    started = common.parse_time(value["started_at"], "compatibility started_at")
    completed = common.parse_time(value["completed_at"], "compatibility completed_at")
    if completed < started:
        raise common.DriverError("compatibility completion precedes start")
    evidence = value["evidence_files"]
    if not isinstance(evidence, dict) or set(evidence) != set(LAYER_FILES):
        raise common.DriverError("compatibility runner did not cover six physical layers")
    common.real_directory(evidence_directory, "compatibility evidence directory")
    observed_files = set()
    for entry in evidence_directory.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise common.DriverError("compatibility evidence directory contains an unsafe entry")
        observed_files.add(entry.name)
    if observed_files != set(LAYER_FILES.values()):
        raise common.DriverError("compatibility evidence directory is not closed")
    for layer, filename in LAYER_FILES.items():
        declared = common.require_digest(evidence[layer], f"{layer} evidence digest")
        path = common.regular_file(evidence_directory / filename, f"{layer} evidence")
        _, observed = common.sha256_file(path)
        if observed != declared:
            raise common.DriverError(f"{layer} evidence changed after the physical runner")


def execute(args: argparse.Namespace) -> None:
    try:
        uuid.UUID(args.fixture_id)
    except ValueError as exc:
        raise common.DriverError("fixture ID is invalid") from exc
    fixture_manifest_digest = common.require_digest(
        args.fixture_manifest_digest, "Fixture Manifest digest"
    )
    fixture_root = common.real_directory(args.fixture_root.absolute(), "Fixture root")
    report = common.outside_repository(args.report, "compatibility report")
    if report.exists() or report.is_symlink():
        raise common.DriverError("compatibility report output must be new")
    release_path = common.regular_file(
        args.target_artifacts.absolute(), "Release Manifest"
    )
    lock_value = os.environ.get("MURIARC_ARTIFACT_LOCK", "")
    if not lock_value:
        raise common.DriverError("MURIARC_ARTIFACT_LOCK is required")
    lock_path = common.regular_file(Path(lock_value).absolute(), "artifact lock")
    release, lock, artifacts, _source_commit = common.validate_signed_release_inputs(
        release_path, lock_path
    )
    _release_value, raw_release = common.read_json(release_path, "Release Manifest")
    _lock_value, raw_lock = common.read_json(lock_path, "artifact lock")

    verifier = common.executable(os.environ.get("MURIARC_VERIFIER", ""), "final verifier")
    common.validate_final_verifier(verifier, release)
    fixture_manifest, _ = common.read_json(
        fixture_root / "fixture-manifest.json", "Fixture Manifest"
    )
    if fixture_manifest.get("fixture_id") != args.fixture_id:
        raise common.DriverError("Fixture Manifest identity is invalid")
    backend = require_backend_profile(fixture_manifest.get("backend"), args.profile)

    artifact = artifacts[args.profile]["artifact"]
    descriptor = artifacts[args.profile]["descriptor"]
    artifact_size, artifact_digest = common.sha256_file(artifact)
    release_artifact = release["artifacts"][args.profile]
    if (
        artifact_size != release_artifact["size_bytes"]
        or artifact_digest != release_artifact["digest"]
        or lock["artifacts"][args.profile]["digest"] != artifact_digest
    ):
        raise common.DriverError("physical runner artifact differs from the signed release")

    runner = common.executable(
        os.environ.get(PROFILE_RUNNERS[args.profile], ""),
        f"{args.profile} compatibility runner",
    )
    evidence_root = common.new_directory(
        report.parent / f".{report.name}.physical-evidence", "compatibility physical evidence root"
    )
    evidence_directory = common.new_directory(
        evidence_root / "layers", "compatibility layer evidence directory"
    )
    runner_result = evidence_root / "runner-result.json"
    common.run_suppressed(
        [
            runner,
            "--mode",
            args.mode,
            "--fixture-id",
            args.fixture_id,
            "--fixture-root",
            fixture_root,
            "--fixture-manifest-digest",
            fixture_manifest_digest,
            "--backend",
            backend,
            "--profile",
            args.profile,
            "--release-manifest",
            release_path,
            "--artifact-lock",
            lock_path,
            "--artifact",
            artifact,
            "--descriptor",
            descriptor,
            "--evidence-directory",
            evidence_directory,
            "--result",
            runner_result,
        ],
        f"{args.profile} physical compatibility runner",
    )
    result, _ = common.read_json(runner_result, "compatibility runner result")
    validate_runner_result(
        result,
        raw_release=raw_release,
        raw_lock=raw_lock,
        fixture_id=args.fixture_id,
        backend=backend,
        profile=args.profile,
        artifact_digest=artifact_digest,
        artifact_size=artifact_size,
        evidence_directory=evidence_directory,
    )

    target_identity = {
        "application_version": release["application_version"],
        "data_epoch": release["data_epoch"],
        "backend_state_digest": release["backend_states"][backend],
        "gateway_contract_revision": release["gateway_contract_revision"],
    }
    request = evidence_root / "verifier-request.json"
    common.write_json_new(
        request,
        {
            "fixture_root": os.fspath(fixture_root),
            "expected_manifest_digest": fixture_manifest_digest,
            "target_identity": target_identity,
            "target_artifact_digest": artifact_digest,
            "mode": args.mode,
            "profile": args.profile,
            "execution_kind": "final_package",
            "evidence_directory": os.fspath(evidence_directory),
            "report_output": os.fspath(report),
        },
    )
    common.run_suppressed(
        [verifier, "run", "--request", request, "--output", "json"],
        "final seven-layer verifier",
    )
    report_value, _ = common.read_json(report, "compatibility report")
    if (
        report_value.get("fixture_id") != args.fixture_id
        or report_value.get("profile") != args.profile
        or report_value.get("mode") != args.mode
        or report_value.get("target_artifact_digest") != artifact_digest
        or report_value.get("execution_kind") != "final_package"
    ):
        raise common.DriverError("final compatibility report differs from the physical run")


def main(argv: list[str] | None = None) -> int:
    os.umask(0o077)
    args = parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        execute(args)
    except (common.DriverError, OSError, KeyError, TypeError, ValueError) as exc:
        print(f"release compatibility driver: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

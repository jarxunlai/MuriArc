#!/usr/bin/env python3
"""Validate the complete, digest-bound MuriArc 1.0 RC evidence set.

This command never builds, signs, downloads, or invents evidence. It only
accepts final release inputs produced by the platform drivers and fails closed
when the repository is still preview, the Fixture Catalog is incomplete, or
any required run is FAIL/SKIP/non-final.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from check_fixture_catalog import CatalogError, assert_append_only, validate_catalog
from compatibility_matrix import MatrixError, validate_definition as validate_matrix_definition


DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
TOKEN_RE = re.compile(r"^[A-Za-z0-9._+-]{1,64}$")
SEMVER_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
EPOCH_RE = re.compile(r"^E[0-9]{4}$")

RELEASE_KEYS = {
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
    "artifacts",
}
RELEASE_ARTIFACT_KEYS = {"media_type", "digest", "size_bytes"}
ARTIFACT_LOCK_KEYS = {
    "format_version",
    "release_manifest_digest",
    "release_provenance_digest",
    "artifacts",
}
DEFINITION_KEYS = {
    "format_version",
    "initial_release",
    "required_artifacts",
    "required_scenarios",
    "requires_non_empty_catalog",
    "requires_current_backend_fixtures",
    "requires_complete_compatibility_matrix",
    "requires_final_packages",
}
INITIAL_RELEASE_KEYS = {"application_version", "data_epoch", "postgres_major"}
SCENARIO_DEFINITION_KEYS = {"scenario_id", "artifact_name", "environment"}
EVIDENCE_KEYS = {
    "format_version",
    "release_manifest_digest",
    "artifact_lock_digest",
    "release_provenance_digest",
    "compatibility_matrix_digest",
    "artifacts",
    "scenarios",
    "completed_at",
}
ARTIFACT_EVIDENCE_KEYS = {
    "digest",
    "size_bytes",
    "provenance_digest",
    "signature_evidence_digest",
}
SCENARIO_EVIDENCE_KEYS = {
    "scenario_id",
    "artifact_name",
    "environment",
    "target_artifact_digest",
    "status",
    "execution_kind",
    "fail_count",
    "skip_count",
    "evidence_digest",
    "started_at",
    "completed_at",
}
MATRIX_REPORT_KEYS = {
    "format_version",
    "mode",
    "selected_fixture_ids",
    "runs",
}
MATRIX_RUN_KEYS = {
    "fixture_id",
    "profile",
    "report_digest",
    "status",
    "execution_kind",
}
MANDATORY_ARTIFACTS = {"native-system", "managed-compose", "desktop-windows"}
MANDATORY_BACKENDS = {"sqlite", "postgres"}
MANDATORY_SCENARIOS = {
    "native-system-install-upgrade-restore": ("native-system", "linux-systemd"),
    "managed-compose-install-upgrade-restore": ("managed-compose", "linux-docker"),
    "desktop-windows-install-upgrade-restore": ("desktop-windows", "windows-installer"),
    "cloudflare-public-native": ("native-system", "cloudflare-staging"),
    "cloudflare-public-compose": ("managed-compose", "cloudflare-staging"),
    "native-system-fault-injection": ("native-system", "linux-systemd"),
    "managed-compose-fault-injection": ("managed-compose", "linux-docker"),
    "desktop-windows-fault-injection": ("desktop-windows", "windows-installer"),
    "native-system-first-write-rollback-guard": ("native-system", "linux-systemd"),
    "managed-compose-first-write-rollback-guard": ("managed-compose", "linux-docker"),
    "desktop-windows-first-write-rollback-guard": ("desktop-windows", "windows-installer"),
    "native-system-tuf-attacks": ("native-system", "linux-systemd"),
    "managed-compose-tuf-sigstore-attacks": ("managed-compose", "linux-docker"),
    "desktop-windows-tauri-signature-attacks": ("desktop-windows", "windows-installer"),
}


class ReadinessError(ValueError):
    pass


def _object_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ReadinessError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_json(path: Path) -> tuple[dict[str, Any], bytes]:
    try:
        path.lstat()
        if path.is_symlink() or not path.is_file():
            raise ReadinessError(f"input must be a regular non-symlink file: {path}")
        raw = path.read_bytes()
        value = json.loads(raw, object_pairs_hook=_object_no_duplicates)
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ReadinessError(f"cannot read JSON input {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise ReadinessError(f"JSON input must contain an object: {path}")
    return value, raw


def sha256(raw: bytes) -> str:
    return f"sha256:{hashlib.sha256(raw).hexdigest()}"


def exact_keys(value: dict[str, Any], expected: set[str], context: str) -> None:
    if set(value) != expected:
        raise ReadinessError(
            f"{context} keys differ; missing={sorted(expected - set(value))}, "
            f"extra={sorted(set(value) - expected)}"
        )


def require_digest(value: Any, context: str) -> str:
    if not isinstance(value, str) or not DIGEST_RE.fullmatch(value):
        raise ReadinessError(f"{context} must be lowercase SHA-256")
    return value


def require_int(value: Any, context: str, *, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        raise ReadinessError(f"{context} must be an integer >= {minimum}")
    return value


def parse_time(value: Any, context: str) -> datetime:
    if not isinstance(value, str) or not value.endswith("Z"):
        raise ReadinessError(f"{context} must be an RFC3339 UTC timestamp")
    try:
        parsed = datetime.fromisoformat(value[:-1] + "+00:00")
    except ValueError as exc:
        raise ReadinessError(f"{context} must be an RFC3339 UTC timestamp") from exc
    if parsed.tzinfo != timezone.utc:
        raise ReadinessError(f"{context} must use UTC")
    return parsed


def validate_definition(definition: dict[str, Any]) -> None:
    exact_keys(definition, DEFINITION_KEYS, "RC gate definition")
    if require_int(definition["format_version"], "RC gate definition format_version") != 1:
        raise ReadinessError("RC gate definition format_version must be 1")
    initial = definition["initial_release"]
    if not isinstance(initial, dict):
        raise ReadinessError("initial_release must be an object")
    exact_keys(initial, INITIAL_RELEASE_KEYS, "initial_release")
    require_int(initial["postgres_major"], "initial_release.postgres_major", minimum=1)
    if initial != {
        "application_version": "1.0.0",
        "data_epoch": "E0001",
        "postgres_major": 17,
    }:
        raise ReadinessError("the initial permanent contract must remain 1.0.0 / E0001 / PostgreSQL 17")
    artifacts = definition["required_artifacts"]
    if (
        not isinstance(artifacts, list)
        or not all(isinstance(item, str) for item in artifacts)
        or set(artifacts) != MANDATORY_ARTIFACTS
        or len(artifacts) != 3
    ):
        raise ReadinessError("RC must require Native, Managed Compose, and Desktop artifacts")
    scenarios = definition["required_scenarios"]
    if not isinstance(scenarios, list):
        raise ReadinessError("required_scenarios must be an array")
    observed: dict[str, tuple[str, str]] = {}
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict):
            raise ReadinessError(f"required_scenarios[{index}] must be an object")
        exact_keys(scenario, SCENARIO_DEFINITION_KEYS, f"required_scenarios[{index}]")
        scenario_id = scenario["scenario_id"]
        pair = (scenario["artifact_name"], scenario["environment"])
        if not isinstance(scenario_id, str) or scenario_id in observed:
            raise ReadinessError("required scenario identifiers must be unique strings")
        observed[scenario_id] = pair
    if observed != MANDATORY_SCENARIOS:
        raise ReadinessError("RC scenario definition weakens or changes mandatory coverage")
    current_backends = definition["requires_current_backend_fixtures"]
    if (
        not isinstance(current_backends, list)
        or not all(isinstance(item, str) for item in current_backends)
        or len(current_backends) != len(MANDATORY_BACKENDS)
        or set(current_backends) != MANDATORY_BACKENDS
    ):
        raise ReadinessError("RC must require current SQLite and PostgreSQL Fixtures")
    for key in (
        "requires_non_empty_catalog",
        "requires_complete_compatibility_matrix",
        "requires_final_packages",
    ):
        if definition[key] is not True:
            raise ReadinessError(f"{key} cannot be disabled")


def validate_release_manifest(release: dict[str, Any], definition: dict[str, Any]) -> None:
    exact_keys(release, RELEASE_KEYS, "Release Manifest")
    initial = definition["initial_release"]
    if require_int(release["format_version"], "Release Manifest format_version") != 1:
        raise ReadinessError("Release Manifest format_version must be 1")
    if release["application_version"] != initial["application_version"] or not SEMVER_RE.fullmatch(
        str(release["application_version"])
    ):
        raise ReadinessError("initial RC application_version must be exactly 1.0.0")
    if release["data_epoch"] != initial["data_epoch"] or not EPOCH_RE.fullmatch(
        str(release["data_epoch"])
    ):
        raise ReadinessError("initial RC data_epoch must be exactly E0001")
    if not isinstance(release["gateway_contract_revision"], str) or not TOKEN_RE.fullmatch(
        release["gateway_contract_revision"]
    ):
        raise ReadinessError("Gateway contract revision is invalid")
    if require_int(release["postgres_major"], "postgres_major", minimum=1) != initial[
        "postgres_major"
    ]:
        raise ReadinessError("initial RC must use PostgreSQL 17")
    bootstrap = require_int(
        release["bootstrap_protocol_revision"], "bootstrap_protocol_revision", minimum=1
    )
    controller_min = require_int(
        release["controller_protocol_min"], "controller_protocol_min", minimum=1
    )
    controller_max = require_int(
        release["controller_protocol_max"], "controller_protocol_max", minimum=1
    )
    if bootstrap <= 0 or controller_min > controller_max:
        raise ReadinessError("Release Manifest control protocol range is invalid")
    if release["migration_class"] not in {"M0", "M1", "M2", "M3"}:
        raise ReadinessError("Release Manifest migration_class is invalid")
    backend_states = release["backend_states"]
    if not isinstance(backend_states, dict) or set(backend_states) != MANDATORY_BACKENDS:
        raise ReadinessError("Release Manifest must pin exactly SQLite and PostgreSQL states")
    for backend, digest in backend_states.items():
        require_digest(digest, f"backend_states.{backend}")
    artifacts = release["artifacts"]
    if not isinstance(artifacts, dict) or not MANDATORY_ARTIFACTS.issubset(artifacts):
        raise ReadinessError("Release Manifest is missing a required final artifact")
    for name, artifact in artifacts.items():
        if not isinstance(name, str) or not TOKEN_RE.fullmatch(name) or not isinstance(artifact, dict):
            raise ReadinessError("Release Manifest artifact name or record is invalid")
        exact_keys(artifact, RELEASE_ARTIFACT_KEYS, f"artifacts.{name}")
        require_digest(artifact["digest"], f"artifacts.{name}.digest")
        if not isinstance(artifact["media_type"], str) or not artifact["media_type"].strip():
            raise ReadinessError(f"artifacts.{name}.media_type is invalid")
        require_int(artifact["size_bytes"], f"artifacts.{name}.size_bytes", minimum=1)


def _source_constant(source: str, name: str) -> str:
    match = re.search(
        rf'^pub const {re.escape(name)}: &str = "([^"]+)";$', source, flags=re.MULTILINE
    )
    if match is None:
        raise ReadinessError(f"cannot find exact source constant {name}")
    return match.group(1)


def validate_source_identity(source_root: Path, release: dict[str, Any]) -> None:
    try:
        root = source_root.resolve(strict=True)
        cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
        compatibility = (root / "crates/core/src/compatibility.rs").read_text(encoding="utf-8")
    except (OSError, UnicodeError, tomllib.TOMLDecodeError, KeyError) as exc:
        raise ReadinessError(f"cannot inspect release source identity: {exc}") from exc
    version = cargo.get("workspace", {}).get("package", {}).get("version")
    epoch = _source_constant(compatibility, "CURRENT_DATA_EPOCH")
    gateway = _source_constant(compatibility, "CURRENT_GATEWAY_CONTRACT_REVISION")
    support = _source_constant(compatibility, "CURRENT_RELEASE_SUPPORT")
    if (
        version != release["application_version"]
        or epoch != release["data_epoch"]
        or gateway != release["gateway_contract_revision"]
        or support != "permanent-upgrade"
    ):
        raise ReadinessError(
            "source is not the formal permanent release identity pinned by the Release Manifest"
        )


def validate_artifact_records(
    artifacts: Any, release: dict[str, Any], context: str
) -> dict[str, dict[str, Any]]:
    release_artifacts = release["artifacts"]
    if not isinstance(artifacts, dict) or set(artifacts) != set(release_artifacts):
        raise ReadinessError(f"{context} must cover exactly every Release Manifest artifact")
    for name, record in artifacts.items():
        if not isinstance(record, dict):
            raise ReadinessError(f"{context} {name} must be an object")
        exact_keys(record, ARTIFACT_EVIDENCE_KEYS, f"{context} {name}")
        size_bytes = require_int(
            record["size_bytes"], f"{context} {name}.size_bytes", minimum=1
        )
        if (
            require_digest(record["digest"], f"{context} {name}.digest")
            != release_artifacts[name]["digest"]
            or size_bytes != release_artifacts[name]["size_bytes"]
        ):
            raise ReadinessError(f"{context} {name} differs from Release Manifest")
        require_digest(record["provenance_digest"], f"{context} {name}.provenance")
        require_digest(
            record["signature_evidence_digest"], f"{context} {name}.signature"
        )
    return artifacts


def validate_artifact_lock(
    lock: dict[str, Any], release: dict[str, Any], release_raw: bytes
) -> dict[str, dict[str, Any]]:
    exact_keys(lock, ARTIFACT_LOCK_KEYS, "artifact lock")
    if require_int(lock["format_version"], "artifact lock format_version") != 1:
        raise ReadinessError("artifact lock format_version must be 1")
    if lock["release_manifest_digest"] != sha256(release_raw):
        raise ReadinessError("artifact lock references a different Release Manifest")
    require_digest(lock["release_provenance_digest"], "artifact lock release provenance")
    return validate_artifact_records(lock["artifacts"], release, "artifact lock record")


def validate_artifact_evidence(
    evidence: dict[str, Any], release: dict[str, Any]
) -> dict[str, dict[str, Any]]:
    return validate_artifact_records(evidence["artifacts"], release, "RC artifact evidence")


def validate_current_fixtures(
    catalog: dict[str, Any], release: dict[str, Any], artifact_evidence: dict[str, dict[str, Any]]
) -> None:
    try:
        validate_catalog(catalog, require_non_empty=True)
    except CatalogError as exc:
        raise ReadinessError(str(exc)) from exc
    current: dict[str, dict[str, Any]] = {}
    for entry in catalog["entries"]:
        backend = entry["backend"]
        if entry["backend_state_digest"] == release["backend_states"][backend]:
            if backend in current:
                raise ReadinessError(f"current {backend} state has multiple Fixture entries")
            current[backend] = entry
    if set(current) != MANDATORY_BACKENDS:
        raise ReadinessError("Catalog does not contain both current SQLite and PostgreSQL Fixtures")
    for backend, entry in current.items():
        if (
            entry["application_version"] != release["application_version"]
            or entry["data_epoch"] != release["data_epoch"]
            or entry["gateway_contract_revision"] != release["gateway_contract_revision"]
        ):
            raise ReadinessError(f"initial {backend} Fixture was not generated by the 1.0/E0001 release")
        allowed = {"desktop-windows"} if backend == "sqlite" else {"native-system", "managed-compose"}
        producers = [
            name
            for name in allowed
            if release["artifacts"][name]["digest"] == entry["source_release_artifact_digest"]
        ]
        if len(producers) != 1:
            raise ReadinessError(f"{backend} Fixture source is not the matching final release artifact")
        producer = producers[0]
        if entry["source_release_provenance_digest"] != artifact_evidence[producer]["provenance_digest"]:
            raise ReadinessError(f"{backend} Fixture provenance is not bound to its final artifact")


def validate_matrix(
    definition: dict[str, Any], report: dict[str, Any], catalog: dict[str, Any]
) -> None:
    try:
        validate_matrix_definition(definition)
    except (MatrixError, KeyError, TypeError) as exc:
        raise ReadinessError(str(exc)) from exc
    if isinstance(definition["format_version"], bool) or not isinstance(
        definition["format_version"], int
    ):
        raise ReadinessError("matrix definition format_version must be an integer")
    exact_keys(report, MATRIX_REPORT_KEYS, "compatibility matrix report")
    if require_int(report["format_version"], "compatibility matrix format_version") != 1 or report[
        "mode"
    ] != "rc":
        raise ReadinessError("compatibility matrix must be an RC report")
    selected = report["selected_fixture_ids"]
    catalog_ids = [entry["fixture_id"] for entry in catalog["entries"]]
    if (
        not isinstance(selected, list)
        or not all(isinstance(item, str) for item in selected)
        or len(selected) != len(set(selected))
        or set(selected) != set(catalog_ids)
    ):
        raise ReadinessError("RC compatibility matrix must select the complete Catalog")
    profiles = set(definition["rc_profiles"])
    expected_pairs = {(fixture_id, profile) for fixture_id in catalog_ids for profile in profiles}
    runs = report["runs"]
    if not isinstance(runs, list):
        raise ReadinessError("compatibility matrix runs must be an array")
    observed_pairs: set[tuple[str, str]] = set()
    for index, run in enumerate(runs):
        if not isinstance(run, dict):
            raise ReadinessError(f"compatibility run {index} must be an object")
        exact_keys(run, MATRIX_RUN_KEYS, f"compatibility run {index}")
        if not isinstance(run["fixture_id"], str) or not isinstance(run["profile"], str):
            raise ReadinessError("compatibility run identity must use strings")
        pair = (run["fixture_id"], run["profile"])
        if pair in observed_pairs:
            raise ReadinessError("compatibility matrix contains a duplicate run")
        observed_pairs.add(pair)
        require_digest(run["report_digest"], f"compatibility run {index}.report_digest")
        if run["status"] != "pass" or run["execution_kind"] != "final_package":
            raise ReadinessError("compatibility matrix contains FAIL/SKIP/non-final execution")
    if observed_pairs != expected_pairs or len(runs) != len(expected_pairs):
        raise ReadinessError("compatibility matrix is missing a required fixture/profile run")


def validate_scenarios(
    evidence: dict[str, Any], definition: dict[str, Any], release: dict[str, Any]
) -> dict[str, str]:
    scenarios = evidence["scenarios"]
    if not isinstance(scenarios, list):
        raise ReadinessError("RC scenarios must be an array")
    required = {
        item["scenario_id"]: (item["artifact_name"], item["environment"])
        for item in definition["required_scenarios"]
    }
    observed: dict[str, str] = {}
    evidence_digests: set[str] = set()
    latest_completion: datetime | None = None
    for index, record in enumerate(scenarios):
        if not isinstance(record, dict):
            raise ReadinessError(f"RC scenario {index} must be an object")
        exact_keys(record, SCENARIO_EVIDENCE_KEYS, f"RC scenario {index}")
        scenario_id = record["scenario_id"]
        if not isinstance(scenario_id, str) or scenario_id not in required or scenario_id in observed:
            raise ReadinessError("RC scenarios contain an unknown or duplicate identifier")
        artifact_name, environment = required[scenario_id]
        if record["artifact_name"] != artifact_name or record["environment"] != environment:
            raise ReadinessError(f"RC scenario {scenario_id} changed its required environment/artifact")
        if record["target_artifact_digest"] != release["artifacts"][artifact_name]["digest"]:
            raise ReadinessError(f"RC scenario {scenario_id} did not execute the pinned artifact")
        if (
            record["status"] != "pass"
            or record["execution_kind"] != "final_package"
            or require_int(record["fail_count"], f"RC scenario {scenario_id}.fail_count") != 0
            or require_int(record["skip_count"], f"RC scenario {scenario_id}.skip_count") != 0
        ):
            raise ReadinessError(f"RC scenario {scenario_id} contains FAIL/SKIP/non-final execution")
        digest = require_digest(record["evidence_digest"], f"RC scenario {scenario_id}.evidence")
        if digest in evidence_digests:
            raise ReadinessError("distinct RC scenarios may not reuse one evidence digest")
        evidence_digests.add(digest)
        started = parse_time(record["started_at"], f"RC scenario {scenario_id}.started_at")
        completed = parse_time(record["completed_at"], f"RC scenario {scenario_id}.completed_at")
        if completed < started:
            raise ReadinessError(f"RC scenario {scenario_id} completion precedes start")
        latest_completion = max(latest_completion or completed, completed)
        observed[scenario_id] = digest
    if set(observed) != set(required) or len(scenarios) != len(required):
        raise ReadinessError("RC evidence is missing a mandatory scenario")
    overall_completion = parse_time(evidence["completed_at"], "RC evidence completed_at")
    if latest_completion is not None and overall_completion < latest_completion:
        raise ReadinessError("RC evidence completed_at precedes a scenario completion")
    return observed


def validate_readiness(
    *,
    source_root: Path,
    release_manifest_path: Path,
    artifact_lock_path: Path,
    catalog_path: Path,
    catalog_baseline_path: Path,
    matrix_definition_path: Path,
    matrix_report_path: Path,
    rc_definition_path: Path,
    rc_evidence_path: Path,
) -> dict[str, Any]:
    release, release_raw = read_json(release_manifest_path)
    artifact_lock, artifact_lock_raw = read_json(artifact_lock_path)
    catalog, catalog_raw = read_json(catalog_path)
    catalog_baseline, catalog_baseline_raw = read_json(catalog_baseline_path)
    matrix_definition, matrix_definition_raw = read_json(matrix_definition_path)
    matrix_report, matrix_report_raw = read_json(matrix_report_path)
    rc_definition, rc_definition_raw = read_json(rc_definition_path)
    rc_evidence, rc_evidence_raw = read_json(rc_evidence_path)

    validate_definition(rc_definition)
    validate_release_manifest(release, rc_definition)
    validate_source_identity(source_root, release)
    exact_keys(rc_evidence, EVIDENCE_KEYS, "RC evidence index")
    if require_int(rc_evidence["format_version"], "RC evidence format_version") != 1:
        raise ReadinessError("RC evidence format_version must be 1")
    if rc_evidence["release_manifest_digest"] != sha256(release_raw):
        raise ReadinessError("RC evidence references a different Release Manifest")
    if rc_evidence["artifact_lock_digest"] != sha256(artifact_lock_raw):
        raise ReadinessError("RC evidence references a different artifact lock")
    if rc_evidence["compatibility_matrix_digest"] != sha256(matrix_report_raw):
        raise ReadinessError("RC evidence references a different compatibility matrix")

    locked_artifacts = validate_artifact_lock(artifact_lock, release, release_raw)
    artifact_evidence = validate_artifact_evidence(rc_evidence, release)
    if rc_evidence["release_provenance_digest"] != artifact_lock["release_provenance_digest"]:
        raise ReadinessError("RC evidence release provenance differs from the artifact lock")
    if artifact_evidence != locked_artifacts:
        raise ReadinessError("RC artifact evidence differs from the signed artifact lock")
    try:
        assert_append_only(catalog, catalog_baseline)
    except CatalogError as exc:
        raise ReadinessError(f"candidate Catalog is not append-only from baseline: {exc}") from exc
    validate_current_fixtures(catalog, release, artifact_evidence)
    validate_matrix(matrix_definition, matrix_report, catalog)
    scenario_digests = validate_scenarios(rc_evidence, rc_definition, release)

    return {
        "format_version": 1,
        "status": "pass",
        "application_version": release["application_version"],
        "data_epoch": release["data_epoch"],
        "release_manifest_digest": sha256(release_raw),
        "artifact_lock_digest": sha256(artifact_lock_raw),
        "release_provenance_digest": artifact_lock["release_provenance_digest"],
        "fixture_catalog_baseline_digest": sha256(catalog_baseline_raw),
        "fixture_catalog_digest": sha256(catalog_raw),
        "matrix_definition_digest": sha256(matrix_definition_raw),
        "compatibility_matrix_digest": sha256(matrix_report_raw),
        "rc_definition_digest": sha256(rc_definition_raw),
        "rc_evidence_digest": sha256(rc_evidence_raw),
        "artifact_digests": {
            name: record["digest"] for name, record in sorted(artifact_evidence.items())
        },
        "scenario_evidence_digests": dict(sorted(scenario_digests.items())),
        "verified_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    }


def write_json_atomic(path: Path, value: dict[str, Any], source_root: Path) -> None:
    resolved_root = source_root.resolve(strict=True)
    resolved_output = path.resolve(strict=False)
    try:
        resolved_output.relative_to(resolved_root)
    except ValueError:
        pass
    else:
        raise ReadinessError("release readiness reports must remain outside the Git worktree")
    if path.exists() or path.is_symlink():
        raise ReadinessError("release readiness report output must not already exist")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--source-root", type=Path, default=Path("."))
    result.add_argument("--release-manifest", type=Path, required=True)
    result.add_argument("--artifact-lock", type=Path, required=True)
    result.add_argument("--catalog", type=Path, default=Path("release-fixtures/catalog.json"))
    result.add_argument(
        "--catalog-baseline", type=Path, default=Path("release-fixtures/catalog.json")
    )
    result.add_argument("--matrix-definition", type=Path, default=Path("release-fixtures/matrix.json"))
    result.add_argument("--matrix-report", type=Path, required=True)
    result.add_argument("--rc-definition", type=Path, default=Path("release-fixtures/rc-gate.json"))
    result.add_argument("--rc-evidence", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        report = validate_readiness(
            source_root=args.source_root,
            release_manifest_path=args.release_manifest,
            artifact_lock_path=args.artifact_lock,
            catalog_path=args.catalog,
            catalog_baseline_path=args.catalog_baseline,
            matrix_definition_path=args.matrix_definition,
            matrix_report_path=args.matrix_report,
            rc_definition_path=args.rc_definition,
            rc_evidence_path=args.rc_evidence,
        )
        write_json_atomic(args.output, report, args.source_root)
    except (ReadinessError, CatalogError, MatrixError, OSError) as exc:
        print(f"release readiness failed: {exc}", file=sys.stderr)
        return 2
    print(
        "release readiness PASS: "
        f"{report['application_version']} / {report['data_epoch']} / "
        f"{len(report['scenario_evidence_digests'])} required scenarios"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

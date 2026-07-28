#!/usr/bin/env python3
"""Execute and close all mandatory physical MuriArc 1.0 RC scenarios."""

from __future__ import annotations

import argparse
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping

import check_release_readiness as readiness
import release_driver_common as common


ENVIRONMENT_RUNNERS = {
    "linux-systemd": "MURIARC_RC_NATIVE_RUNNER",
    "linux-docker": "MURIARC_RC_COMPOSE_RUNNER",
    "windows-installer": "MURIARC_RC_WINDOWS_RUNNER",
    "cloudflare-staging": "MURIARC_RC_CLOUDFLARE_RUNNER",
}

REQUIRED_CHECKS = {
    "native-system-install-upgrade-restore": {
        "artifact_signature_verified",
        "clean_install_verified",
        "e0001_fixture_restored",
        "upgrade_completed",
        "joint_backup_verified",
        "isolated_restore_verified",
        "readiness_verified",
        "continued_write_verified",
        "explicit_restore_verified",
        "post_restore_readiness_verified",
        "no_residual_isolated_database",
    },
    "managed-compose-install-upgrade-restore": {
        "artifact_signature_verified",
        "image_digest_pins_verified",
        "clean_install_verified",
        "e0001_fixture_restored",
        "upgrade_completed",
        "joint_backup_verified",
        "isolated_restore_verified",
        "readiness_verified",
        "continued_write_verified",
        "explicit_restore_verified",
        "volumes_preserved",
        "no_residual_isolated_database",
    },
    "desktop-windows-install-upgrade-restore": {
        "artifact_signature_verified",
        "msi_install_verified",
        "nsis_install_verified",
        "e0001_fixture_restored",
        "desktop_started",
        "upgrade_completed",
        "joint_backup_verified",
        "isolated_restore_verified",
        "continued_write_verified",
        "explicit_restore_verified",
        "post_restore_start_verified",
    },
    "cloudflare-public-native": {
        "pinned_native_artifact_deployed",
        "tunnel_origin_loopback_verified",
        "tls_verified",
        "access_policy_verified",
        "service_token_exact_match_verified",
        "browser_session_verified",
        "unauthenticated_access_rejected",
        "api_bypass_rejected",
        "audit_redaction_verified",
    },
    "cloudflare-public-compose": {
        "pinned_compose_artifact_deployed",
        "pinned_images_verified",
        "tunnel_origin_loopback_verified",
        "tls_verified",
        "access_policy_verified",
        "service_token_exact_match_verified",
        "browser_session_verified",
        "unauthenticated_access_rejected",
        "api_bypass_rejected",
        "audit_redaction_verified",
    },
    "native-system-fault-injection": {
        "preflight_failure_preserved_source",
        "drain_failure_preserved_source",
        "backup_failure_recovered_source",
        "restore_failure_cleaned_candidate",
        "migration_failure_cleaned_candidate",
        "verification_failure_blocked_switch",
        "activation_failure_kept_gate_closed",
        "resume_from_verified_boundary",
        "no_unverified_traffic",
        "no_residual_isolated_database",
    },
    "managed-compose-fault-injection": {
        "preflight_failure_preserved_source",
        "drain_failure_preserved_source",
        "backup_failure_recovered_source",
        "restore_failure_cleaned_candidate",
        "migration_failure_cleaned_candidate",
        "verification_failure_blocked_switch",
        "activation_failure_kept_gate_closed",
        "resume_from_verified_boundary",
        "volumes_preserved",
        "no_unverified_traffic",
        "no_residual_isolated_database",
    },
    "desktop-windows-fault-injection": {
        "interrupted_download_rejected",
        "interrupted_backup_recovered_source",
        "restore_failure_preserved_source",
        "migration_failure_blocked_switch",
        "verification_failure_blocked_switch",
        "activation_crash_resumed_safely",
        "no_unverified_user_write",
        "source_data_preserved",
    },
    "native-system-first-write-rollback-guard": {
        "rollback_before_first_write_succeeded",
        "target_first_write_recorded",
        "rollback_after_first_write_rejected",
        "source_not_auto_reactivated",
        "explicit_recovery_remained_available",
        "audit_and_provenance_preserved",
    },
    "managed-compose-first-write-rollback-guard": {
        "rollback_before_first_write_succeeded",
        "target_first_write_recorded",
        "rollback_after_first_write_rejected",
        "source_not_auto_reactivated",
        "explicit_recovery_remained_available",
        "volumes_preserved",
        "audit_and_provenance_preserved",
    },
    "desktop-windows-first-write-rollback-guard": {
        "rollback_before_first_write_succeeded",
        "target_first_write_recorded",
        "rollback_after_first_write_rejected",
        "source_not_auto_reactivated",
        "explicit_recovery_remained_available",
        "audit_and_provenance_preserved",
    },
    "native-system-tuf-attacks": {
        "expired_timestamp_rejected",
        "metadata_rollback_rejected",
        "mix_and_match_metadata_rejected",
        "wrong_target_length_rejected",
        "wrong_target_digest_rejected",
        "untrusted_root_rotation_rejected",
        "source_service_preserved",
    },
    "managed-compose-tuf-sigstore-attacks": {
        "expired_timestamp_rejected",
        "metadata_rollback_rejected",
        "mix_and_match_metadata_rejected",
        "wrong_target_digest_rejected",
        "unsigned_server_image_rejected",
        "wrong_cosign_key_rejected",
        "tampered_signature_bundle_rejected",
        "mutable_tag_substitution_rejected",
        "source_service_preserved",
        "volumes_preserved",
    },
    "desktop-windows-tauri-signature-attacks": {
        "invalid_updater_signature_rejected",
        "wrong_updater_key_rejected",
        "truncated_updater_archive_rejected",
        "manifest_version_rollback_rejected",
        "manifest_digest_mismatch_rejected",
        "source_desktop_preserved",
        "source_data_preserved",
    },
}

RESULT_KEYS = {
    "format_version",
    "scenario_id",
    "artifact_name",
    "environment",
    "target_artifact_digest",
    "release_manifest_digest",
    "artifact_lock_digest",
    "compatibility_matrix_digest",
    "status",
    "execution_kind",
    "fail_count",
    "skip_count",
    "checks",
    "started_at",
    "completed_at",
}
CHECK_KEYS = {"check_id", "status", "evidence_digest", "started_at", "completed_at"}


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--release-manifest", type=Path, required=True)
    result.add_argument("--artifact-lock", type=Path, required=True)
    result.add_argument("--definition", type=Path, required=True)
    result.add_argument("--matrix-report", type=Path, required=True)
    result.add_argument("--run-root", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    return result


def validate_runner_result(
    value: dict[str, Any],
    *,
    scenario: Mapping[str, Any],
    release_digest: str,
    lock_digest: str,
    matrix_digest: str,
    artifact_digest: str,
    evidence_directory: Path,
) -> tuple[str, str]:
    common.exact_keys(value, RESULT_KEYS, "RC scenario result")
    if value["format_version"] != 1:
        raise common.DriverError("RC scenario result format is unsupported")
    if (
        value["scenario_id"] != scenario["scenario_id"]
        or value["artifact_name"] != scenario["artifact_name"]
        or value["environment"] != scenario["environment"]
        or value["target_artifact_digest"] != artifact_digest
        or value["release_manifest_digest"] != release_digest
        or value["artifact_lock_digest"] != lock_digest
        or value["compatibility_matrix_digest"] != matrix_digest
    ):
        raise common.DriverError("RC scenario result differs from pinned inputs")
    if (
        value["status"] != "pass"
        or value["execution_kind"] != "final_package"
        or common.require_count(value["fail_count"], "RC fail_count") != 0
        or common.require_count(value["skip_count"], "RC skip_count") != 0
    ):
        raise common.DriverError("RC scenario contains FAIL, SKIP, or non-final execution")
    started = common.parse_time(value["started_at"], "RC scenario started_at")
    completed = common.parse_time(value["completed_at"], "RC scenario completed_at")
    if completed < started:
        raise common.DriverError("RC scenario completion precedes start")

    checks = value["checks"]
    if not isinstance(checks, list):
        raise common.DriverError("RC scenario checks must be an array")
    expected = REQUIRED_CHECKS[scenario["scenario_id"]]
    common.real_directory(evidence_directory, "RC check evidence directory")
    observed_files = set()
    for entry in evidence_directory.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise common.DriverError("RC check evidence directory contains an unsafe entry")
        observed_files.add(entry.name)
    expected_files = {f"{check_id}.json" for check_id in expected}
    if observed_files != expected_files:
        raise common.DriverError("RC check evidence directory is not closed")

    observed: set[str] = set()
    digests: set[str] = set()
    for index, record in enumerate(checks):
        if not isinstance(record, dict):
            raise common.DriverError(f"RC scenario check {index} must be an object")
        common.exact_keys(record, CHECK_KEYS, f"RC scenario check {index}")
        check_id = record["check_id"]
        if not isinstance(check_id, str) or check_id not in expected or check_id in observed:
            raise common.DriverError("RC scenario contains an unknown or duplicate check")
        if record["status"] != "pass":
            raise common.DriverError("RC scenario check did not pass")
        digest = common.require_digest(record["evidence_digest"], "RC check evidence")
        if digest in digests:
            raise common.DriverError("RC scenario checks reused an evidence digest")
        evidence_path = common.regular_file(
            evidence_directory / f"{check_id}.json", f"{check_id} RC evidence"
        )
        _, observed_digest = common.sha256_file(evidence_path)
        if observed_digest != digest:
            raise common.DriverError("RC check evidence changed after the physical runner")
        check_started = common.parse_time(record["started_at"], "RC check started_at")
        check_completed = common.parse_time(record["completed_at"], "RC check completed_at")
        if check_completed < check_started or check_started < started or check_completed > completed:
            raise common.DriverError("RC check timestamps escape the scenario interval")
        observed.add(check_id)
        digests.add(digest)
    if observed != expected or len(checks) != len(expected):
        raise common.DriverError("RC scenario is missing a mandatory physical check")
    return value["started_at"], value["completed_at"]


def validate_matrix_report(value: dict[str, Any]) -> None:
    common.exact_keys(value, readiness.MATRIX_REPORT_KEYS, "compatibility matrix report")
    runs = value.get("runs")
    if value.get("format_version") != 1 or value.get("mode") != "rc" or not isinstance(runs, list) or not runs:
        raise common.DriverError("RC requires a non-empty compatibility matrix")
    for index, run in enumerate(runs):
        if not isinstance(run, dict):
            raise common.DriverError(f"compatibility run {index} must be an object")
        common.exact_keys(run, readiness.MATRIX_RUN_KEYS, f"compatibility run {index}")
        common.require_digest(run["report_digest"], f"compatibility run {index} digest")
        if run["status"] != "pass" or run["execution_kind"] != "final_package":
            raise common.DriverError("compatibility matrix contains non-final or failed evidence")


def execute(args: argparse.Namespace) -> None:
    release_path = common.regular_file(args.release_manifest.absolute(), "Release Manifest")
    lock_path = common.regular_file(args.artifact_lock.absolute(), "artifact lock")
    definition_path = common.regular_file(args.definition.absolute(), "RC definition")
    matrix_path = common.regular_file(args.matrix_report.absolute(), "matrix report")
    output = common.outside_repository(args.output, "RC evidence index")
    if output.exists() or output.is_symlink():
        raise common.DriverError("RC evidence index must be a new path")
    run_root = common.outside_repository(args.run_root.absolute(), "RC scenario root")
    common.real_directory(run_root, "RC scenario root")
    if any(run_root.iterdir()):
        raise common.DriverError("RC scenario root must be empty")

    definition, _ = common.read_json(definition_path, "RC definition")
    matrix, raw_matrix = common.read_json(matrix_path, "compatibility matrix report")
    release, lock, artifacts, _source_commit = common.validate_signed_release_inputs(
        release_path, lock_path
    )
    release_value, raw_release = common.read_json(release_path, "Release Manifest")
    lock_value, raw_lock = common.read_json(lock_path, "artifact lock")
    try:
        readiness.validate_definition(definition)
        readiness.validate_release_manifest(release_value, definition)
        readiness.validate_artifact_lock(lock_value, release_value, raw_release)
    except readiness.ReadinessError as exc:
        raise common.DriverError("RC definition or signed release binding is invalid") from exc
    validate_matrix_report(matrix)
    if set(REQUIRED_CHECKS) != {item["scenario_id"] for item in definition["required_scenarios"]}:
        raise common.DriverError("RC physical check policy differs from the mandatory scenario set")

    release_digest = common.sha256_bytes(raw_release)
    lock_digest = common.sha256_bytes(raw_lock)
    matrix_digest = common.sha256_bytes(raw_matrix)
    scenario_records = []
    evidence_digests: set[str] = set()
    for scenario in definition["required_scenarios"]:
        scenario_id = scenario["scenario_id"]
        artifact_name = scenario["artifact_name"]
        environment = scenario["environment"]
        runner_env = ENVIRONMENT_RUNNERS.get(environment)
        if runner_env is None:
            raise common.DriverError("RC scenario environment has no physical runner class")
        runner = common.executable(
            os.environ.get(runner_env, ""), f"{environment} RC runner"
        )
        artifact = artifacts[artifact_name]["artifact"]
        descriptor = artifacts[artifact_name]["descriptor"]
        artifact_size, artifact_digest = common.sha256_file(artifact)
        if (
            artifact_digest != release["artifacts"][artifact_name]["digest"]
            or artifact_size != release["artifacts"][artifact_name]["size_bytes"]
            or lock["artifacts"][artifact_name]["digest"] != artifact_digest
        ):
            raise common.DriverError("RC runner artifact differs from the signed release")
        scenario_root = common.new_directory(run_root / scenario_id, f"{scenario_id} run root")
        evidence_directory = common.new_directory(
            scenario_root / "evidence", f"{scenario_id} check evidence directory"
        )
        result_path = scenario_root / "scenario-evidence.json"
        common.run_suppressed(
            [
                runner,
                "--scenario-id",
                scenario_id,
                "--artifact-name",
                artifact_name,
                "--environment",
                environment,
                "--release-manifest",
                release_path,
                "--artifact-lock",
                lock_path,
                "--matrix-report",
                matrix_path,
                "--artifact",
                artifact,
                "--descriptor",
                descriptor,
                "--run-root",
                scenario_root,
                "--evidence-directory",
                evidence_directory,
                "--output",
                result_path,
            ],
            f"{scenario_id} physical RC runner",
        )
        common.real_directory(scenario_root, f"{scenario_id} run root")
        scenario_entries = {entry.name: entry for entry in scenario_root.iterdir()}
        if set(scenario_entries) != {"evidence", "scenario-evidence.json"}:
            raise common.DriverError("RC scenario run root is not closed")
        common.real_directory(scenario_entries["evidence"], f"{scenario_id} evidence root")
        common.regular_file(
            scenario_entries["scenario-evidence.json"], f"{scenario_id} evidence"
        )
        result, raw_result = common.read_json(result_path, f"{scenario_id} evidence")
        started_at, completed_at = validate_runner_result(
            result,
            scenario=scenario,
            release_digest=release_digest,
            lock_digest=lock_digest,
            matrix_digest=matrix_digest,
            artifact_digest=artifact_digest,
            evidence_directory=evidence_directory,
        )
        evidence_digest = common.sha256_bytes(raw_result)
        if evidence_digest in evidence_digests:
            raise common.DriverError("distinct RC scenarios reused an evidence digest")
        evidence_digests.add(evidence_digest)
        scenario_records.append(
            {
                "scenario_id": scenario_id,
                "artifact_name": artifact_name,
                "environment": environment,
                "target_artifact_digest": artifact_digest,
                "status": "pass",
                "execution_kind": "final_package",
                "fail_count": 0,
                "skip_count": 0,
                "evidence_digest": evidence_digest,
                "started_at": started_at,
                "completed_at": completed_at,
            }
        )

    completed_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    common.write_json_new(
        output,
        {
            "format_version": 1,
            "release_manifest_digest": release_digest,
            "artifact_lock_digest": lock_digest,
            "release_provenance_digest": lock["release_provenance_digest"],
            "compatibility_matrix_digest": matrix_digest,
            "artifacts": lock["artifacts"],
            "scenarios": scenario_records,
            "completed_at": completed_at,
        },
    )


def main(argv: list[str] | None = None) -> int:
    os.umask(0o077)
    args = parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        execute(args)
    except (common.DriverError, OSError, KeyError, TypeError, ValueError) as exc:
        print(f"release RC driver: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

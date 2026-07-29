#!/usr/bin/env python3
"""Plan and collect fail-closed MuriArc compatibility matrix runs."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path
from typing import Any

from check_fixture_catalog import CatalogError, load_json, validate_catalog


PROFILES = {"native-system", "managed-compose", "desktop-windows"}
BACKEND_PROFILES = {
    "sqlite": {"desktop-windows"},
    "postgres": {"native-system", "managed-compose"},
}
LAYERS = {
    "asset_restore",
    "storage",
    "store_application",
    "api",
    "remote_ui",
    "continue_write",
    "read_only_no_side_effects",
}
DEFINITION_KEYS = {
    "format_version",
    "pr_profiles",
    "nightly_profiles",
    "rc_profiles",
    "rc_requires_all_catalog_entries",
    "rc_requires_final_artifacts",
}
PERSISTENCE_PREFIXES = (
    "migrations/",
    "crates/core/",
    "crates/application/",
    "crates/store-sqlite/",
    "crates/store-postgres/",
    "crates/server/",
    "crates/ai/",
    "crates/data/",
    "crates/upgrade/",
    "crates/muriarcctl/",
    "crates/release-evidence/",
    "crates/muriarc-verifier/",
    "src-tauri/",
    "release-fixtures/",
    "scripts/check_fixture_catalog.py",
    "scripts/compatibility_matrix.py",
    "scripts/release_driver_common.py",
    "scripts/release_compatibility_driver.py",
    "scripts/release_rc_driver.py",
    "scripts/run-release-compatibility.sh",
)


class MatrixError(ValueError):
    pass


def validate_definition(definition: dict[str, Any]) -> None:
    if set(definition) != DEFINITION_KEYS or definition.get("format_version") != 1:
        raise MatrixError("matrix definition schema is invalid")
    for key in ("pr_profiles", "nightly_profiles", "rc_profiles"):
        values = definition[key]
        if (
            not isinstance(values, list)
            or not values
            or len(values) != len(set(values))
            or not set(values).issubset(PROFILES)
        ):
            raise MatrixError(f"{key} must be a non-empty unique profile list")
    pr_profiles = set(definition["pr_profiles"])
    if "desktop-windows" not in pr_profiles or not pr_profiles.intersection(
        {"native-system", "managed-compose"}
    ):
        raise MatrixError("PR must cover both SQLite/Desktop and PostgreSQL/Server")
    if set(definition["nightly_profiles"]) != PROFILES:
        raise MatrixError("Nightly must cover Native, Managed Compose, and Desktop")
    if set(definition["rc_profiles"]) != PROFILES:
        raise MatrixError("RC must cover Native, Managed Compose, and Desktop")
    if definition["rc_requires_all_catalog_entries"] is not True:
        raise MatrixError("RC all-history policy cannot be disabled")
    if definition["rc_requires_final_artifacts"] is not True:
        raise MatrixError("RC final-artifact policy cannot be disabled")


def changed_paths(args: argparse.Namespace) -> list[str]:
    paths = list(args.changed_file or [])
    if args.changed_files_file:
        try:
            paths.extend(args.changed_files_file.read_text(encoding="utf-8").splitlines())
        except OSError as exc:
            raise MatrixError(f"cannot read changed-files list: {exc}") from exc
    normalized = []
    for value in paths:
        path = value.strip().removeprefix("./")
        if path:
            normalized.append(path)
    return normalized


def select_fixture_ids(
    mode: str, catalog: dict[str, Any], paths: list[str]
) -> list[str]:
    entries = catalog["entries"]
    if mode in {"nightly", "rc"}:
        return [entry["fixture_id"] for entry in entries]
    if any(path.startswith(PERSISTENCE_PREFIXES) for path in paths):
        return [entry["fixture_id"] for entry in entries]

    # Catalog order is append-only. For presentation-only changes, keep at least
    # the latest state of each backend in the full seven-layer PR gate.
    latest_by_backend: dict[str, str] = {}
    for entry in entries:
        latest_by_backend[entry["backend"]] = entry["fixture_id"]
    selected = set(latest_by_backend.values())
    return [entry["fixture_id"] for entry in entries if entry["fixture_id"] in selected]


def build_plan(args: argparse.Namespace) -> dict[str, Any]:
    try:
        catalog = load_json(args.catalog)
        definition = load_json(args.definition)
        validate_catalog(catalog)
        validate_definition(definition)
    except CatalogError as exc:
        raise MatrixError(str(exc)) from exc

    if args.mode == "rc" and not catalog["entries"]:
        raise MatrixError("RC cannot run with an empty stable Fixture Catalog")
    selected = select_fixture_ids(args.mode, catalog, changed_paths(args))
    profiles = definition[f"{args.mode}_profiles"]
    entries = {entry["fixture_id"]: entry for entry in catalog["entries"]}
    runs = [
        {"fixture_id": fixture_id, "profile": profile}
        for fixture_id in selected
        for profile in profiles
        if profile in BACKEND_PROFILES[entries[fixture_id]["backend"]]
    ]
    missing = [
        fixture_id
        for fixture_id in selected
        if not any(run["fixture_id"] == fixture_id for run in runs)
    ]
    if missing:
        raise MatrixError("selected Fixture has no backend-compatible delivery profile")
    return {
        "format_version": 1,
        "mode": args.mode,
        "selected_fixture_ids": selected,
        "profiles": profiles,
        "runs": runs,
    }


def _load_report(path: Path) -> tuple[dict[str, Any], bytes]:
    try:
        raw = path.read_bytes()
        report = load_json(path)
    except (OSError, CatalogError, UnicodeError) as exc:
        raise MatrixError(f"cannot read verification report {path}: {exc}") from exc
    return report, raw


def collect_report(args: argparse.Namespace) -> dict[str, Any]:
    plan = load_json(args.plan)
    required_plan_keys = {
        "format_version",
        "mode",
        "selected_fixture_ids",
        "profiles",
        "runs",
    }
    if set(plan) != required_plan_keys or plan.get("format_version") != 1:
        raise MatrixError("execution plan schema is invalid")
    mode = plan.get("mode")
    if mode not in {"pr", "nightly", "rc"}:
        raise MatrixError("execution plan mode is invalid")

    matrix_runs = []
    for run in plan["runs"]:
        if not isinstance(run, dict) or set(run) != {"fixture_id", "profile"}:
            raise MatrixError("execution plan run schema is invalid")
        fixture_id = run["fixture_id"]
        profile = run["profile"]
        if profile not in PROFILES:
            raise MatrixError("execution plan profile is invalid")
        path = args.report_directory / f"{fixture_id}--{profile}.json"
        report, raw = _load_report(path)
        if (
            report.get("format_version") != 1
            or report.get("fixture_id") != fixture_id
            or report.get("mode") != mode
            or report.get("profile") != profile
        ):
            raise MatrixError(f"verification report identity differs: {path}")
        layers = report.get("layers")
        if not isinstance(layers, dict) or set(layers) != LAYERS:
            raise MatrixError(f"verification report does not contain seven layers: {path}")
        if any(
            not isinstance(record, dict)
            or record.get("status") != "pass"
            or not record.get("evidence_digest")
            for record in layers.values()
        ):
            raise MatrixError(f"verification report contains FAIL/SKIP/missing evidence: {path}")
        execution_kind = report.get("execution_kind")
        if execution_kind not in {"final_package", "source_run", "demo_gateway"}:
            raise MatrixError(f"verification execution kind is invalid: {path}")
        if mode == "rc" and execution_kind != "final_package":
            raise MatrixError(f"RC report did not execute a final package/image: {path}")
        matrix_runs.append(
            {
                "fixture_id": fixture_id,
                "profile": profile,
                "report_digest": f"sha256:{hashlib.sha256(raw).hexdigest()}",
                "status": "pass",
                "execution_kind": execution_kind,
            }
        )
    return {
        "format_version": 1,
        "mode": mode,
        "selected_fixture_ids": plan["selected_fixture_ids"],
        "runs": matrix_runs,
    }


def write_json_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    os.replace(temporary, path)


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    plan = commands.add_parser("plan", help="select immutable Fixture/profile pairs")
    plan.add_argument("--mode", choices=("pr", "nightly", "rc"), required=True)
    plan.add_argument("--catalog", type=Path, required=True)
    plan.add_argument("--definition", type=Path, required=True)
    plan.add_argument("--changed-file", action="append")
    plan.add_argument("--changed-files-file", type=Path)
    plan.add_argument("--output", type=Path, required=True)

    collect = commands.add_parser("collect", help="build a fail-closed matrix report")
    collect.add_argument("--plan", type=Path, required=True)
    collect.add_argument("--report-directory", type=Path, required=True)
    collect.add_argument("--output", type=Path, required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        value = build_plan(args) if args.command == "plan" else collect_report(args)
        write_json_atomic(args.output, value)
    except (CatalogError, MatrixError, OSError) as exc:
        print(f"compatibility matrix failed: {exc}", file=sys.stderr)
        return 1
    print(
        f"compatibility {args.command} OK: {len(value.get('runs', []))} fixture/profile runs"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

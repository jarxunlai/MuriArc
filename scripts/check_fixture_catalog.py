#!/usr/bin/env python3
"""Validate the immutable MuriArc release-fixture Catalog.

This deliberately uses only the Python standard library so it can run before
the Rust workspace is compiled. The Rust verifier remains the authoritative
typed validator used when a Fixture is restored.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import uuid
from pathlib import Path
from typing import Any


DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
VERSION_RE = re.compile(r"^[A-Za-z0-9.+-]{1,64}$")
EPOCH_RE = re.compile(r"^(?:preview_epoch_0|E[0-9]{4})$")
TOKEN_RE = re.compile(r"^[A-Za-z0-9._+-]{1,64}$")

ROOT_KEYS = {"format_version", "entries"}
ENTRY_KEYS = {
    "fixture_id",
    "application_version",
    "data_epoch",
    "gateway_contract_revision",
    "backend",
    "backend_state_digest",
    "source_release_artifact_digest",
    "source_release_provenance_digest",
    "fixture_artifact_digest",
    "fixture_manifest_digest",
    "expected_facts_digest",
    "oci_reference",
    "created_at",
    "immutable_entry_digest",
}
ENTRY_DIGEST_KEYS = (
    "fixture_id",
    "application_version",
    "data_epoch",
    "gateway_contract_revision",
    "backend",
    "backend_state_digest",
    "source_release_artifact_digest",
    "source_release_provenance_digest",
    "fixture_artifact_digest",
    "fixture_manifest_digest",
    "expected_facts_digest",
    "oci_reference",
    "created_at",
)
DIGEST_KEYS = {
    "backend_state_digest",
    "source_release_artifact_digest",
    "source_release_provenance_digest",
    "fixture_artifact_digest",
    "fixture_manifest_digest",
    "expected_facts_digest",
    "immutable_entry_digest",
}


class CatalogError(ValueError):
    pass


def _object_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise CatalogError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_json(path: Path) -> dict[str, Any]:
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_object_no_duplicates)
    except (OSError, json.JSONDecodeError, UnicodeError) as exc:
        raise CatalogError(f"cannot read {path}: {exc}") from exc
    if not isinstance(loaded, dict):
        raise CatalogError("Catalog root must be an object")
    return loaded


def canonical_digest(value: dict[str, Any]) -> str:
    encoded = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return f"sha256:{hashlib.sha256(encoded).hexdigest()}"


def _require_exact_keys(value: dict[str, Any], expected: set[str], context: str) -> None:
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise CatalogError(f"{context} keys differ; missing={missing}, extra={extra}")


def _require_string(entry: dict[str, Any], key: str) -> str:
    value = entry[key]
    if not isinstance(value, str):
        raise CatalogError(f"{key} must be a string")
    return value


def validate_catalog(catalog: dict[str, Any], *, require_non_empty: bool = False) -> None:
    _require_exact_keys(catalog, ROOT_KEYS, "Catalog")
    if catalog["format_version"] != 1:
        raise CatalogError("Catalog format_version must be 1")
    entries = catalog["entries"]
    if not isinstance(entries, list):
        raise CatalogError("Catalog entries must be an array")
    if require_non_empty and not entries:
        raise CatalogError("the requested gate requires a non-empty stable Fixture Catalog")

    fixture_ids: set[str] = set()
    backend_states: set[tuple[str, str]] = set()
    for index, entry in enumerate(entries):
        context = f"entries[{index}]"
        if not isinstance(entry, dict):
            raise CatalogError(f"{context} must be an object")
        _require_exact_keys(entry, ENTRY_KEYS, context)

        fixture_id = _require_string(entry, "fixture_id")
        try:
            parsed_id = uuid.UUID(fixture_id)
        except ValueError as exc:
            raise CatalogError(f"{context}.fixture_id is invalid") from exc
        if parsed_id.int == 0 or str(parsed_id) != fixture_id:
            raise CatalogError(f"{context}.fixture_id must be canonical lowercase and non-nil")
        if fixture_id in fixture_ids:
            raise CatalogError(f"duplicate fixture_id: {fixture_id}")
        fixture_ids.add(fixture_id)

        version = _require_string(entry, "application_version")
        epoch = _require_string(entry, "data_epoch")
        gateway = _require_string(entry, "gateway_contract_revision")
        backend = _require_string(entry, "backend")
        if not VERSION_RE.fullmatch(version):
            raise CatalogError(f"{context}.application_version is invalid")
        if not EPOCH_RE.fullmatch(epoch):
            raise CatalogError(f"{context}.data_epoch is invalid")
        if not TOKEN_RE.fullmatch(gateway):
            raise CatalogError(f"{context}.gateway_contract_revision is invalid")
        if backend not in {"sqlite", "postgres"}:
            raise CatalogError(f"{context}.backend must be sqlite or postgres")

        for key in DIGEST_KEYS:
            if not DIGEST_RE.fullmatch(_require_string(entry, key)):
                raise CatalogError(f"{context}.{key} must be lowercase sha256")

        state_key = (backend, entry["backend_state_digest"])
        if state_key in backend_states:
            raise CatalogError(f"backend state is already cataloged: {state_key}")
        backend_states.add(state_key)

        reference = _require_string(entry, "oci_reference")
        artifact_hex = entry["fixture_artifact_digest"].removeprefix("sha256:")
        if (
            not reference.startswith("ghcr.io/")
            or ":latest" in reference
            or "@sha256:" not in reference
            or not reference.endswith(artifact_hex)
        ):
            raise CatalogError(f"{context}.oci_reference must be GHCR digest-pinned")

        created_at = _require_string(entry, "created_at")
        if not created_at.endswith("Z") or "T" not in created_at:
            raise CatalogError(f"{context}.created_at must be an RFC3339 UTC timestamp")

        digest_view = {key: entry[key] for key in ENTRY_DIGEST_KEYS}
        if canonical_digest(digest_view) != entry["immutable_entry_digest"]:
            raise CatalogError(f"{context}.immutable_entry_digest differs")


def assert_append_only(current: dict[str, Any], previous: dict[str, Any]) -> None:
    validate_catalog(previous)
    validate_catalog(current)
    old_entries = previous["entries"]
    new_entries = current["entries"]
    if len(new_entries) < len(old_entries) or new_entries[: len(old_entries)] != old_entries:
        raise CatalogError("Catalog entries are not an exact append-only extension")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=Path("release-fixtures/catalog.json"))
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--require-non-empty", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        catalog = load_json(args.catalog)
        validate_catalog(catalog, require_non_empty=args.require_non_empty)
        if args.previous is not None:
            assert_append_only(catalog, load_json(args.previous))
    except CatalogError as exc:
        print(f"fixture catalog check failed: {exc}", file=sys.stderr)
        return 1
    print(f"fixture catalog OK: {len(catalog['entries'])} immutable entries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Verify that every database migration is append-only and checksum locked."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "migrations" / "checksums.json"


def migration_files() -> list[Path]:
    return sorted((ROOT / "migrations").glob("*/*.sql"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def current_entries() -> dict[str, str]:
    return {
        path.relative_to(ROOT).as_posix(): digest(path)
        for path in migration_files()
    }


def write_manifest() -> None:
    document = {
        "formatVersion": 1,
        "compatibilityFloor": "preview_epoch_0",
        "algorithm": "sha256",
        "files": current_entries(),
    }
    MANIFEST.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def verify_manifest() -> list[str]:
    failures: list[str] = []
    try:
        document = json.loads(MANIFEST.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"cannot read migration checksum manifest: {error}"]

    if document.get("formatVersion") != 1:
        failures.append("unsupported migration checksum manifest format")
    if document.get("algorithm") != "sha256":
        failures.append("migration checksum algorithm must be sha256")
    locked = document.get("files")
    if not isinstance(locked, dict):
        return failures + ["migration checksum manifest files must be an object"]

    current = current_entries()
    for path in sorted(set(locked) - set(current)):
        failures.append(f"locked migration was removed: {path}")
    for path in sorted(set(current) - set(locked)):
        failures.append(f"new migration is not registered: {path}")
    for path in sorted(set(current) & set(locked)):
        if locked[path] != current[path]:
            failures.append(f"locked migration was modified: {path}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--write",
        action="store_true",
        help="regenerate the manifest after intentionally appending migrations",
    )
    args = parser.parse_args()
    if args.write:
        write_manifest()
        print(f"wrote {MANIFEST.relative_to(ROOT)}")
        return 0

    failures = verify_manifest()
    if failures:
        for failure in failures:
            print(f"migration checksum error: {failure}")
        return 1
    print(f"verified {len(current_entries())} immutable migration files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())


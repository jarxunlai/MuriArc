from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace


SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import check_fixture_catalog as catalog_check  # noqa: E402
import compatibility_matrix as matrix  # noqa: E402


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def definition() -> dict[str, object]:
    return {
        "format_version": 1,
        "pr_profiles": ["managed-compose", "desktop-windows"],
        "nightly_profiles": [
            "native-system",
            "managed-compose",
            "desktop-windows",
        ],
        "rc_profiles": [
            "native-system",
            "managed-compose",
            "desktop-windows",
        ],
        "rc_requires_all_catalog_entries": True,
        "rc_requires_final_artifacts": True,
    }


def entry() -> dict[str, object]:
    value: dict[str, object] = {
        "fixture_id": "11111111-1111-4111-8111-111111111111",
        "application_version": "1.0.0",
        "data_epoch": "E0001",
        "gateway_contract_revision": "gateway-v1",
        "backend": "postgres",
        "backend_state_digest": "sha256:" + "a" * 64,
        "source_release_artifact_digest": "sha256:" + "b" * 64,
        "source_release_provenance_digest": "sha256:" + "c" * 64,
        "fixture_artifact_digest": "sha256:" + "d" * 64,
        "fixture_manifest_digest": "sha256:" + "e" * 64,
        "expected_facts_digest": "sha256:" + "f" * 64,
        "oci_reference": "ghcr.io/jarxunlai/muriarc-fixtures@sha256:" + "d" * 64,
        "created_at": "2026-07-26T12:00:00Z",
    }
    digest_view = {key: value[key] for key in catalog_check.ENTRY_DIGEST_KEYS}
    value["immutable_entry_digest"] = catalog_check.canonical_digest(digest_view)
    return value


class CatalogTests(unittest.TestCase):
    def test_append_only_catalog_and_self_digest(self) -> None:
        previous = {"format_version": 1, "entries": []}
        current = {"format_version": 1, "entries": [entry()]}
        catalog_check.validate_catalog(current, require_non_empty=True)
        catalog_check.assert_append_only(current, previous)

        modified = json.loads(json.dumps(current))
        modified["entries"][0]["application_version"] = "1.0.1"
        with self.assertRaises(catalog_check.CatalogError):
            catalog_check.validate_catalog(modified)

    def test_catalog_removal_is_rejected(self) -> None:
        previous = {"format_version": 1, "entries": [entry()]}
        current = {"format_version": 1, "entries": []}
        with self.assertRaises(catalog_check.CatalogError):
            catalog_check.assert_append_only(current, previous)


class MatrixTests(unittest.TestCase):
    def test_nightly_selects_complete_catalog(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            catalog = root / "catalog.json"
            definition_path = root / "matrix.json"
            write_json(catalog, {"format_version": 1, "entries": [entry()]})
            write_json(definition_path, definition())
            args = SimpleNamespace(
                mode="nightly",
                catalog=catalog,
                definition=definition_path,
                changed_file=None,
                changed_files_file=None,
            )
            plan = matrix.build_plan(args)
            self.assertEqual(len(plan["runs"]), 2)
            self.assertEqual(
                {run["profile"] for run in plan["runs"]},
                {"native-system", "managed-compose"},
            )
            self.assertEqual(
                plan["selected_fixture_ids"],
                ["11111111-1111-4111-8111-111111111111"],
            )

    def test_rc_rejects_empty_catalog_and_weakened_policy(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            catalog = root / "catalog.json"
            definition_path = root / "matrix.json"
            write_json(catalog, {"format_version": 1, "entries": []})
            write_json(definition_path, definition())
            args = SimpleNamespace(
                mode="rc",
                catalog=catalog,
                definition=definition_path,
                changed_file=None,
                changed_files_file=None,
            )
            with self.assertRaises(matrix.MatrixError):
                matrix.build_plan(args)

            weakened = definition()
            weakened["rc_requires_final_artifacts"] = False
            with self.assertRaises(matrix.MatrixError):
                matrix.validate_definition(weakened)

            missing_sqlite = definition()
            missing_sqlite["pr_profiles"] = ["managed-compose"]
            with self.assertRaisesRegex(matrix.MatrixError, "SQLite/Desktop"):
                matrix.validate_definition(missing_sqlite)

            missing_native = definition()
            missing_native["nightly_profiles"] = [
                "managed-compose",
                "desktop-windows",
            ]
            with self.assertRaisesRegex(matrix.MatrixError, "Nightly"):
                matrix.validate_definition(missing_native)


if __name__ == "__main__":
    unittest.main()

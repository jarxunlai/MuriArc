from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

import check_fixture_catalog as catalog_check  # noqa: E402
import check_release_readiness as readiness  # noqa: E402
import assemble_release_manifest as assembler  # noqa: E402


def digest(character: str) -> str:
    return "sha256:" + character * 64


def write_json(path: Path, value: object) -> bytes:
    raw = (json.dumps(value, indent=2) + "\n").encode()
    path.write_bytes(raw)
    return raw


class ReleaseReadinessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        (self.source / "crates/core/src").mkdir(parents=True)
        (self.source / "Cargo.toml").write_text(
            '[workspace]\n[workspace.package]\nversion = "1.0.0"\n', encoding="utf-8"
        )
        (self.source / "crates/core/src/compatibility.rs").write_text(
            '\n'.join(
                [
                    'pub const CURRENT_DATA_EPOCH: &str = "E0001";',
                    'pub const CURRENT_GATEWAY_CONTRACT_REVISION: &str = "gateway-v1";',
                    'pub const CURRENT_RELEASE_SUPPORT: &str = "permanent-upgrade";',
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        self.release = {
            "format_version": 1,
            "application_version": "1.0.0",
            "data_epoch": "E0001",
            "gateway_contract_revision": "gateway-v1",
            "backend_states": {"sqlite": digest("1"), "postgres": digest("2")},
            "postgres_major": 17,
            "bootstrap_protocol_revision": 1,
            "controller_protocol_min": 1,
            "controller_protocol_max": 1,
            "migration_class": "M3",
            "artifacts": {
                "native-system": {
                    "media_type": "application/vnd.muriarc.native.v1+tar",
                    "digest": digest("3"),
                    "size_bytes": 300,
                },
                "managed-compose": {
                    "media_type": "application/vnd.muriarc.compose.v1+tar",
                    "digest": digest("4"),
                    "size_bytes": 400,
                },
                "desktop-windows": {
                    "media_type": "application/vnd.microsoft.portable-executable",
                    "digest": digest("5"),
                    "size_bytes": 500,
                },
            },
        }
        self.artifact_evidence = {
            name: {
                "digest": artifact["digest"],
                "size_bytes": artifact["size_bytes"],
                "provenance_digest": digest(str(index + 6)),
                "signature_evidence_digest": digest(chr(ord("a") + index)),
            }
            for index, (name, artifact) in enumerate(self.release["artifacts"].items())
        }
        self.catalog = {
            "format_version": 1,
            "entries": [
                self.catalog_entry(
                    "11111111-1111-4111-8111-111111111111",
                    "sqlite",
                    digest("1"),
                    "desktop-windows",
                    digest("d"),
                ),
                self.catalog_entry(
                    "22222222-2222-4222-8222-222222222222",
                    "postgres",
                    digest("2"),
                    "native-system",
                    digest("e"),
                ),
            ],
        }
        self.matrix_definition = {
            "format_version": 1,
            "pr_profiles": ["managed-compose"],
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
        fixture_ids = [entry["fixture_id"] for entry in self.catalog["entries"]]
        self.matrix_report = {
            "format_version": 1,
            "mode": "rc",
            "selected_fixture_ids": fixture_ids,
            "runs": [
                {
                    "fixture_id": fixture_id,
                    "profile": profile,
                    "report_digest": "sha256:" + f"{index + 16:064x}",
                    "status": "pass",
                    "execution_kind": "final_package",
                }
                for index, (fixture_id, profile) in enumerate(
                    (fixture_id, profile)
                    for fixture_id in fixture_ids
                    for profile in self.matrix_definition["rc_profiles"]
                )
            ],
        }
        self.rc_definition = json.loads(
            (ROOT / "release-fixtures/rc-gate.json").read_text(encoding="utf-8")
        )
        self.paths = {
            "release_manifest_path": self.root / "release-manifest.json",
            "artifact_lock_path": self.root / "artifact-lock.json",
            "catalog_path": self.root / "catalog.json",
            "catalog_baseline_path": self.root / "catalog-baseline.json",
            "matrix_definition_path": self.root / "matrix.json",
            "matrix_report_path": self.root / "matrix-report.json",
            "rc_definition_path": self.root / "rc-gate.json",
            "rc_evidence_path": self.root / "rc-evidence.json",
        }
        self.write_inputs()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def catalog_entry(
        self,
        fixture_id: str,
        backend: str,
        backend_digest: str,
        source_artifact: str,
        fixture_digest: str,
    ) -> dict[str, object]:
        entry: dict[str, object] = {
            "fixture_id": fixture_id,
            "application_version": "1.0.0",
            "data_epoch": "E0001",
            "gateway_contract_revision": "gateway-v1",
            "backend": backend,
            "backend_state_digest": backend_digest,
            "source_release_artifact_digest": self.release["artifacts"][source_artifact]["digest"],
            "source_release_provenance_digest": self.artifact_evidence[source_artifact][
                "provenance_digest"
            ],
            "fixture_artifact_digest": fixture_digest,
            "fixture_manifest_digest": digest("b"),
            "expected_facts_digest": digest("c"),
            "oci_reference": f"ghcr.io/jarxunlai/muriarc-fixtures@{fixture_digest}",
            "created_at": "2026-08-01T00:00:00Z",
        }
        entry["immutable_entry_digest"] = catalog_check.canonical_digest(
            {key: entry[key] for key in catalog_check.ENTRY_DIGEST_KEYS}
        )
        return entry

    def rc_evidence(
        self, release_raw: bytes, artifact_lock_raw: bytes, matrix_raw: bytes
    ) -> dict[str, object]:
        scenarios = []
        for index, scenario in enumerate(self.rc_definition["required_scenarios"]):
            artifact = scenario["artifact_name"]
            scenarios.append(
                {
                    **scenario,
                    "target_artifact_digest": self.release["artifacts"][artifact]["digest"],
                    "status": "pass",
                    "execution_kind": "final_package",
                    "fail_count": 0,
                    "skip_count": 0,
                    "evidence_digest": "sha256:" + f"{index + 32:064x}",
                    "started_at": "2026-08-01T01:00:00Z",
                    "completed_at": "2026-08-01T01:05:00Z",
                }
            )
        return {
            "format_version": 1,
            "release_manifest_digest": readiness.sha256(release_raw),
            "artifact_lock_digest": readiness.sha256(artifact_lock_raw),
            "release_provenance_digest": digest("9"),
            "compatibility_matrix_digest": readiness.sha256(matrix_raw),
            "artifacts": self.artifact_evidence,
            "scenarios": scenarios,
            "completed_at": "2026-08-01T01:06:00Z",
        }

    def write_inputs(self) -> None:
        release_raw = write_json(self.paths["release_manifest_path"], self.release)
        self.artifact_lock = {
            "format_version": 1,
            "release_manifest_digest": readiness.sha256(release_raw),
            "release_provenance_digest": digest("9"),
            "artifacts": self.artifact_evidence,
        }
        artifact_lock_raw = write_json(
            self.paths["artifact_lock_path"], self.artifact_lock
        )
        write_json(self.paths["catalog_baseline_path"], {"format_version": 1, "entries": []})
        write_json(self.paths["catalog_path"], self.catalog)
        write_json(self.paths["matrix_definition_path"], self.matrix_definition)
        matrix_raw = write_json(self.paths["matrix_report_path"], self.matrix_report)
        write_json(self.paths["rc_definition_path"], self.rc_definition)
        self.evidence = self.rc_evidence(release_raw, artifact_lock_raw, matrix_raw)
        write_json(self.paths["rc_evidence_path"], self.evidence)

    def validate(self) -> dict[str, object]:
        return readiness.validate_readiness(source_root=self.source, **self.paths)

    def test_complete_digest_bound_final_rc_passes(self) -> None:
        report = self.validate()
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["application_version"], "1.0.0")
        self.assertEqual(len(report["scenario_evidence_digests"]), len(readiness.MANDATORY_SCENARIOS))

    def test_checked_out_preview_repository_cannot_claim_formal_readiness(self) -> None:
        with self.assertRaisesRegex(readiness.ReadinessError, "formal permanent release identity"):
            readiness.validate_source_identity(ROOT, self.release)

    def test_workflow_routes_rc_through_the_complete_orchestrator(self) -> None:
        workflow = (ROOT / ".github/workflows/compatibility-evidence.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("Execute complete final-artifact RC gate", workflow)
        self.assertIn("MURIARC_RC_DRIVER", workflow)
        self.assertIn("MURIARC_ARTIFACT_LOCK", workflow)
        self.assertIn('--artifact-lock "${MURIARC_ARTIFACT_LOCK}"', workflow)
        self.assertIn("scripts/run-release-candidate.sh", workflow)

    def test_release_manifest_is_assembled_after_final_artifact_digests(self) -> None:
        identity = {key: value for key, value in self.release.items() if key != "artifacts"}
        identity_path = self.root / "release-identity.json"
        write_json(identity_path, identity)
        arguments = []
        for name, artifact in self.release["artifacts"].items():
            descriptor = {
                "format_version": 1,
                "artifact_name": name,
                **artifact,
                "provenance_digest": self.artifact_evidence[name]["provenance_digest"],
                "signature_evidence_digest": self.artifact_evidence[name][
                    "signature_evidence_digest"
                ],
            }
            path = self.root / f"{name}-descriptor.json"
            write_json(path, descriptor)
            arguments.append((name, path))
        manifest, lock = assembler.assemble(
            source_root=self.source,
            identity_path=identity_path,
            artifact_arguments=arguments,
            release_provenance_digest=digest("9"),
            rc_definition_path=self.paths["rc_definition_path"],
        )
        self.assertEqual(manifest, self.release)
        self.assertEqual(lock["artifacts"], self.artifact_evidence)
        self.assertEqual(
            lock["release_manifest_digest"],
            readiness.sha256((json.dumps(manifest, indent=2) + "\n").encode()),
        )

        with self.assertRaisesRegex(assembler.AssemblyError, "required"):
            assembler.assemble(
                source_root=self.source,
                identity_path=identity_path,
                artifact_arguments=arguments[:2],
                release_provenance_digest=digest("9"),
                rc_definition_path=self.paths["rc_definition_path"],
            )

    def test_preview_source_and_empty_catalog_fail_closed(self) -> None:
        compatibility = self.source / "crates/core/src/compatibility.rs"
        compatibility.write_text(
            compatibility.read_text(encoding="utf-8")
            .replace('"E0001"', '"preview_epoch_0"')
            .replace('"permanent-upgrade"', '"preview-only-adoption"'),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(readiness.ReadinessError, "formal permanent release identity"):
            self.validate()

        compatibility.write_text(
            compatibility.read_text(encoding="utf-8")
            .replace('"preview_epoch_0"', '"E0001"')
            .replace('"preview-only-adoption"', '"permanent-upgrade"'),
            encoding="utf-8",
        )
        self.catalog["entries"] = []
        self.write_inputs()
        with self.assertRaisesRegex(readiness.ReadinessError, "non-empty"):
            self.validate()

    def test_fail_skip_non_final_and_digest_tampering_are_rejected(self) -> None:
        mutations = [
            ("status", "fail", "FAIL/SKIP"),
            ("skip_count", 1, "FAIL/SKIP"),
            ("execution_kind", "source_run", "FAIL/SKIP"),
            ("target_artifact_digest", digest("0"), "pinned artifact"),
        ]
        for field, value, message in mutations:
            with self.subTest(field=field):
                evidence = copy.deepcopy(self.evidence)
                evidence["scenarios"][0][field] = value
                write_json(self.paths["rc_evidence_path"], evidence)
                with self.assertRaisesRegex(readiness.ReadinessError, message):
                    self.validate()
        write_json(self.paths["rc_evidence_path"], self.evidence)
        self.matrix_report["runs"][0]["execution_kind"] = "demo_gateway"
        self.write_inputs()
        with self.assertRaisesRegex(readiness.ReadinessError, "non-final"):
            self.validate()

    def test_signed_artifact_lock_cannot_be_replaced_or_diverge_from_rc_evidence(self) -> None:
        lock = copy.deepcopy(self.artifact_lock)
        lock["artifacts"]["native-system"]["provenance_digest"] = digest("0")
        lock_raw = write_json(self.paths["artifact_lock_path"], lock)
        evidence = copy.deepcopy(self.evidence)
        evidence["artifact_lock_digest"] = readiness.sha256(lock_raw)
        write_json(self.paths["rc_evidence_path"], evidence)
        with self.assertRaisesRegex(readiness.ReadinessError, "signed artifact lock"):
            self.validate()

        write_json(self.paths["artifact_lock_path"], self.artifact_lock)
        evidence = copy.deepcopy(self.evidence)
        evidence["scenarios"][0]["fail_count"] = False
        write_json(self.paths["rc_evidence_path"], evidence)
        with self.assertRaisesRegex(readiness.ReadinessError, "must be an integer"):
            self.validate()

    def test_rc_orchestrator_requires_the_signed_artifact_lock(self) -> None:
        run_root = self.root / "new-run-root"
        result = subprocess.run(
            [
                str(ROOT / "scripts/run-release-candidate.sh"),
                "--release-manifest",
                str(self.paths["release_manifest_path"]),
                "--run-root",
                str(run_root),
            ],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--artifact-lock", result.stderr)

    def test_fixture_must_come_from_matching_final_artifact_and_provenance(self) -> None:
        self.catalog["entries"][0]["source_release_artifact_digest"] = self.release["artifacts"][
            "native-system"
        ]["digest"]
        self.catalog["entries"][0]["immutable_entry_digest"] = catalog_check.canonical_digest(
            {
                key: self.catalog["entries"][0][key]
                for key in catalog_check.ENTRY_DIGEST_KEYS
            }
        )
        self.write_inputs()
        with self.assertRaisesRegex(readiness.ReadinessError, "matching final release artifact"):
            self.validate()

        self.setUp_fresh_catalog()
        self.catalog["entries"][1]["source_release_provenance_digest"] = digest("0")
        self.catalog["entries"][1]["immutable_entry_digest"] = catalog_check.canonical_digest(
            {
                key: self.catalog["entries"][1][key]
                for key in catalog_check.ENTRY_DIGEST_KEYS
            }
        )
        self.write_inputs()
        with self.assertRaisesRegex(readiness.ReadinessError, "provenance"):
            self.validate()

    def setUp_fresh_catalog(self) -> None:
        self.catalog["entries"] = [
            self.catalog_entry(
                "11111111-1111-4111-8111-111111111111",
                "sqlite",
                digest("1"),
                "desktop-windows",
                digest("d"),
            ),
            self.catalog_entry(
                "22222222-2222-4222-8222-222222222222",
                "postgres",
                digest("2"),
                "native-system",
                digest("e"),
            ),
        ]

    def test_scenarios_cannot_be_removed_duplicated_or_reuse_evidence(self) -> None:
        evidence = copy.deepcopy(self.evidence)
        evidence["scenarios"].pop()
        write_json(self.paths["rc_evidence_path"], evidence)
        with self.assertRaisesRegex(readiness.ReadinessError, "missing"):
            self.validate()

        evidence = copy.deepcopy(self.evidence)
        evidence["scenarios"][1]["evidence_digest"] = evidence["scenarios"][0][
            "evidence_digest"
        ]
        write_json(self.paths["rc_evidence_path"], evidence)
        with self.assertRaisesRegex(readiness.ReadinessError, "reuse"):
            self.validate()


if __name__ == "__main__":
    unittest.main()

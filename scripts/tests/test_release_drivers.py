from __future__ import annotations

import contextlib
import copy
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

import compatibility_matrix as matrix  # noqa: E402
import release_compatibility_driver as compatibility  # noqa: E402
import release_driver_common as common  # noqa: E402
import release_rc_driver as rc  # noqa: E402


def digest(character: str) -> str:
    return "sha256:" + character * 64


class CompatibilityDriverContractTests(unittest.TestCase):
    fixture_id = "11111111-1111-4111-8111-111111111111"
    profile = "managed-compose"
    backend = "postgres"
    artifact_digest = digest("a")
    artifact_size = 123
    raw_release = b'{"release":"pinned"}\n'
    raw_lock = b'{"lock":"pinned"}\n'

    def make_evidence(self, root: Path) -> tuple[dict[str, str], Path]:
        directory = root / "layers"
        directory.mkdir()
        declared: dict[str, str] = {}
        for layer, filename in compatibility.LAYER_FILES.items():
            path = directory / filename
            path.write_text(f"{layer} physical evidence\n", encoding="utf-8")
            _, declared[layer] = common.sha256_file(path)
        return declared, directory

    def result(self, evidence: dict[str, str]) -> dict[str, object]:
        return {
            "format_version": 1,
            "fixture_id": self.fixture_id,
            "backend": self.backend,
            "profile": self.profile,
            "release_manifest_digest": common.sha256_bytes(self.raw_release),
            "artifact_lock_digest": common.sha256_bytes(self.raw_lock),
            "target_artifact_digest": self.artifact_digest,
            "target_artifact_size_bytes": self.artifact_size,
            "execution_kind": "final_package",
            "status": "pass",
            "fail_count": 0,
            "skip_count": 0,
            "evidence_files": evidence,
            "started_at": "2026-08-01T00:00:00Z",
            "completed_at": "2026-08-01T00:01:00Z",
        }

    def validate(self, value: dict[str, object], directory: Path) -> None:
        compatibility.validate_runner_result(
            value,
            raw_release=self.raw_release,
            raw_lock=self.raw_lock,
            fixture_id=self.fixture_id,
            backend=self.backend,
            profile=self.profile,
            artifact_digest=self.artifact_digest,
            artifact_size=self.artifact_size,
            evidence_directory=directory,
        )

    def test_complete_six_layer_directory_passes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            evidence, directory = self.make_evidence(Path(raw))
            self.validate(self.result(evidence), directory)

    def test_missing_extra_and_symlink_evidence_fail_closed(self) -> None:
        cases = ("missing", "extra", "symlink")
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as raw:
                evidence, directory = self.make_evidence(Path(raw))
                if case == "missing":
                    (directory / "api.json").unlink()
                elif case == "extra":
                    (directory / "unexpected.json").write_text("extra\n", encoding="utf-8")
                else:
                    target = directory / "api.json"
                    target.unlink()
                    target.symlink_to(directory / "storage.json")
                with self.assertRaises(common.DriverError):
                    self.validate(self.result(evidence), directory)

    def test_digest_drift_fail_and_skip_are_rejected(self) -> None:
        mutations = (
            ("target_artifact_digest", digest("0"), "identity"),
            ("status", "fail", "FAIL, SKIP"),
            ("fail_count", 1, "FAIL, SKIP"),
            ("skip_count", 1, "FAIL, SKIP"),
            ("execution_kind", "source_run", "FAIL, SKIP"),
        )
        for field, replacement, message in mutations:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as raw:
                evidence, directory = self.make_evidence(Path(raw))
                value = self.result(evidence)
                value[field] = replacement
                with self.assertRaisesRegex(common.DriverError, message):
                    self.validate(value, directory)

        with tempfile.TemporaryDirectory() as raw:
            evidence, directory = self.make_evidence(Path(raw))
            evidence["storage"] = digest("0")
            with self.assertRaisesRegex(common.DriverError, "changed"):
                self.validate(self.result(evidence), directory)

    def test_backend_profile_mapping_is_physical_not_cartesian(self) -> None:
        self.assertEqual(
            compatibility.require_backend_profile("sqlite", "desktop-windows"),
            "sqlite",
        )
        self.assertEqual(
            compatibility.require_backend_profile("postgres", "native-system"),
            "postgres",
        )
        with self.assertRaisesRegex(common.DriverError, "incompatible"):
            compatibility.require_backend_profile("sqlite", "managed-compose")
        with self.assertRaisesRegex(common.DriverError, "incompatible"):
            compatibility.require_backend_profile("postgres", "desktop-windows")
        with self.assertRaisesRegex(common.DriverError, "unsupported"):
            compatibility.require_backend_profile("mysql", "native-system")

    def test_manifest_digest_is_mandatory_and_driver_files_are_persistent(self) -> None:
        base = [
            "--mode",
            "rc",
            "--fixture-id",
            self.fixture_id,
            "--fixture-root",
            "/tmp/fixture",
            "--profile",
            self.profile,
            "--target-artifacts",
            "/tmp/release.json",
            "--report",
            "/tmp/report.json",
        ]
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            compatibility.parser().parse_args(base)
        parsed = compatibility.parser().parse_args(
            base + ["--fixture-manifest-digest", digest("b")]
        )
        self.assertEqual(parsed.fixture_manifest_digest, digest("b"))
        for path in (
            "scripts/release_driver_common.py",
            "scripts/release_compatibility_driver.py",
            "scripts/release_rc_driver.py",
        ):
            self.assertIn(path, matrix.PERSISTENCE_PREFIXES)


class RcDriverContractTests(unittest.TestCase):
    release_digest = digest("1")
    lock_digest = digest("2")
    matrix_digest = digest("3")
    artifact_digest = digest("4")

    def scenario(self) -> dict[str, str]:
        definition = json.loads(
            (ROOT / "release-fixtures/rc-gate.json").read_text(encoding="utf-8")
        )
        return definition["required_scenarios"][0]

    def make_evidence(self, root: Path, scenario: dict[str, str]) -> Path:
        directory = root / "evidence"
        directory.mkdir()
        for check_id in sorted(rc.REQUIRED_CHECKS[scenario["scenario_id"]]):
            (directory / f"{check_id}.json").write_text(
                f"{check_id} physical evidence\n", encoding="utf-8"
            )
        return directory

    def result(
        self, scenario: dict[str, str], evidence_directory: Path
    ) -> dict[str, object]:
        checks = []
        for check_id in sorted(rc.REQUIRED_CHECKS[scenario["scenario_id"]]):
            _, evidence_digest = common.sha256_file(
                evidence_directory / f"{check_id}.json"
            )
            checks.append(
                {
                    "check_id": check_id,
                    "status": "pass",
                    "evidence_digest": evidence_digest,
                    "started_at": "2026-08-01T00:00:10Z",
                    "completed_at": "2026-08-01T00:00:20Z",
                }
            )
        return {
            "format_version": 1,
            "scenario_id": scenario["scenario_id"],
            "artifact_name": scenario["artifact_name"],
            "environment": scenario["environment"],
            "target_artifact_digest": self.artifact_digest,
            "release_manifest_digest": self.release_digest,
            "artifact_lock_digest": self.lock_digest,
            "compatibility_matrix_digest": self.matrix_digest,
            "status": "pass",
            "execution_kind": "final_package",
            "fail_count": 0,
            "skip_count": 0,
            "checks": checks,
            "started_at": "2026-08-01T00:00:00Z",
            "completed_at": "2026-08-01T00:01:00Z",
        }

    def validate(
        self,
        value: dict[str, object],
        scenario: dict[str, str],
        evidence_directory: Path,
    ) -> None:
        rc.validate_runner_result(
            value,
            scenario=scenario,
            release_digest=self.release_digest,
            lock_digest=self.lock_digest,
            matrix_digest=self.matrix_digest,
            artifact_digest=self.artifact_digest,
            evidence_directory=evidence_directory,
        )

    def test_complete_required_check_set_passes(self) -> None:
        scenario = self.scenario()
        with tempfile.TemporaryDirectory() as raw:
            directory = self.make_evidence(Path(raw), scenario)
            self.validate(self.result(scenario, directory), scenario, directory)

    def test_fail_skip_digest_drift_and_missing_check_are_rejected(self) -> None:
        scenario = self.scenario()
        mutations = (
            ("status", "fail", "FAIL, SKIP"),
            ("fail_count", 1, "FAIL, SKIP"),
            ("skip_count", 1, "FAIL, SKIP"),
            ("execution_kind", "source_run", "FAIL, SKIP"),
            ("target_artifact_digest", digest("0"), "pinned inputs"),
        )
        for field, replacement, message in mutations:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as raw:
                directory = self.make_evidence(Path(raw), scenario)
                value = self.result(scenario, directory)
                value[field] = replacement
                with self.assertRaisesRegex(common.DriverError, message):
                    self.validate(value, scenario, directory)

        with tempfile.TemporaryDirectory() as raw:
            directory = self.make_evidence(Path(raw), scenario)
            value = self.result(scenario, directory)
            value["checks"].pop()
            with self.assertRaisesRegex(common.DriverError, "missing"):
                self.validate(value, scenario, directory)

    def test_duplicate_check_and_evidence_digest_are_rejected(self) -> None:
        scenario = self.scenario()
        with tempfile.TemporaryDirectory() as raw:
            directory = self.make_evidence(Path(raw), scenario)
            value = self.result(scenario, directory)
            value["checks"][1]["check_id"] = value["checks"][0]["check_id"]
            with self.assertRaisesRegex(common.DriverError, "duplicate check"):
                self.validate(value, scenario, directory)

        with tempfile.TemporaryDirectory() as raw:
            directory = self.make_evidence(Path(raw), scenario)
            value = self.result(scenario, directory)
            value["checks"][1]["evidence_digest"] = value["checks"][0][
                "evidence_digest"
            ]
            with self.assertRaisesRegex(common.DriverError, "reused"):
                self.validate(value, scenario, directory)

    def test_missing_extra_symlink_and_digest_drift_evidence_are_rejected(self) -> None:
        scenario = self.scenario()
        for case in ("missing", "extra", "symlink", "drift"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as raw:
                directory = self.make_evidence(Path(raw), scenario)
                value = self.result(scenario, directory)
                first = sorted(rc.REQUIRED_CHECKS[scenario["scenario_id"]])[0]
                target = directory / f"{first}.json"
                if case == "missing":
                    target.unlink()
                elif case == "extra":
                    (directory / "unexpected.json").write_text(
                        "extra\n", encoding="utf-8"
                    )
                elif case == "symlink":
                    target.unlink()
                    target.symlink_to(next(directory.iterdir()))
                else:
                    target.write_text("changed\n", encoding="utf-8")
                with self.assertRaises(common.DriverError):
                    self.validate(value, scenario, directory)

    def test_driver_policy_matches_all_fourteen_rc_gate_scenarios(self) -> None:
        definition = json.loads(
            (ROOT / "release-fixtures/rc-gate.json").read_text(encoding="utf-8")
        )
        scenarios = {
            record["scenario_id"]: record["environment"]
            for record in definition["required_scenarios"]
        }
        self.assertEqual(len(scenarios), 14)
        self.assertEqual(set(rc.REQUIRED_CHECKS), set(scenarios))
        self.assertTrue(set(scenarios.values()).issubset(rc.ENVIRONMENT_RUNNERS))


if __name__ == "__main__":
    unittest.main()

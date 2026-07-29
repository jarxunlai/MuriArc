from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

import finalize_release_artifact as finalize  # noqa: E402


class ReleaseArtifactFinalizerTests(unittest.TestCase):
    def test_sha256_and_canonical_json_are_stable(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            artifact = Path(raw) / "artifact.tar"
            artifact.write_bytes(b"muriarc-final-artifact")
            size, digest = finalize.sha256_file(artifact)
            self.assertEqual(size, 22)
            self.assertRegex(digest, r"^sha256:[0-9a-f]{64}$")
            self.assertEqual(
                finalize.canonical_bytes({"b": 2, "a": 1}),
                b'{"a":1,"b":2}',
            )

    def test_regular_file_rejects_symlink_and_empty_file(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            empty = root / "empty"
            empty.touch()
            with self.assertRaises(finalize.FinalizeError):
                finalize.regular_file(empty, "empty")
            target = root / "target"
            target.write_text("x", encoding="utf-8")
            link = root / "link"
            link.symlink_to(target)
            with self.assertRaises(finalize.FinalizeError):
                finalize.regular_file(link, "link")

    def test_grype_report_rejects_high_and_critical(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            report = Path(raw) / "grype.json"
            report.write_text(
                json.dumps(
                    {
                        "matches": [
                            {"vulnerability": {"severity": "Medium"}},
                            {"vulnerability": {"severity": "Low"}},
                        ]
                    }
                ),
                encoding="utf-8",
            )
            counts = finalize.validate_grype_report(report)
            self.assertEqual(counts["medium"], 1)
            report.write_text(
                json.dumps(
                    {"matches": [{"vulnerability": {"severity": "High"}}]}
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(finalize.FinalizeError, "High or Critical"):
                finalize.validate_grype_report(report)

    def test_sensitive_environment_is_not_forwarded(self) -> None:
        old = dict(finalize.os.environ)
        try:
            finalize.os.environ["MURIARC_ROOT_PASSWORD"] = "do-not-forward"
            finalize.os.environ["GITHUB_TOKEN"] = "do-not-forward"
            finalize.os.environ["PATH"] = "/safe/path"
            environment = finalize.safe_tool_environment("cosign-password", 123)
        finally:
            finalize.os.environ.clear()
            finalize.os.environ.update(old)
        self.assertNotIn("MURIARC_ROOT_PASSWORD", environment)
        self.assertNotIn("GITHUB_TOKEN", environment)
        self.assertEqual(environment["COSIGN_PASSWORD"], "cosign-password")
        self.assertEqual(environment["SOURCE_DATE_EPOCH"], "123")

    def test_formal_script_has_no_skip_or_overwrite_path(self) -> None:
        text = (ROOT / "scripts/finalize_release_artifact.py").read_text(encoding="utf-8")
        self.assertNotIn("--skip", text)
        for marker in (
            "refs/remotes/origin/main",
            "--untracked-files=all",
            "--fail-on",
            "verify-blob",
            "sourceTreeCleanBeforeAndAfter",
            "artifact changed while evidence was generated",
        ):
            self.assertIn(marker, text)


if __name__ == "__main__":
    unittest.main()

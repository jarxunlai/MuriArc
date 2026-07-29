from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import sys
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "build_server_tester_bundle", ROOT / "scripts/build_server_tester_bundle.py"
)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = module
SPEC.loader.exec_module(module)


class ServerTesterBundleTests(unittest.TestCase):
    def test_digest_inputs_are_fail_closed(self) -> None:
        with self.assertRaises(module.BuildError):
            module.require_digest_image("ghcr.io/jarxunlai/muriarc-server-tester:latest", module.SERVER_REPOSITORY)
        with self.assertRaises(module.BuildError):
            module.require_digest_image(
                f"ghcr.io/other/server@sha256:{'0' * 64}", module.SERVER_REPOSITORY
            )

    def test_bundle_is_deterministic_and_roundtrips(self) -> None:
        commit = "a" * 40
        server = f"{module.SERVER_REPOSITORY}@sha256:{'b' * 64}"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = module.build_bundle(
                expected_commit=commit,
                server_image=server,
                output_directory=root / "one",
                enforce_git=False,
            )
            second = module.build_bundle(
                expected_commit=commit,
                server_image=server,
                output_directory=root / "two",
                enforce_git=False,
            )
            self.assertEqual(first.archive.read_bytes(), second.archive.read_bytes())
            manifest = json.loads(first.manifest.read_text())
            self.assertFalse(manifest["formalRelease"])
            self.assertFalse(manifest["formalRcEvidence"])
            self.assertEqual(manifest["defaultDataset"], "empty")
            self.assertEqual(manifest["source"]["commit"], commit)
            self.assertIn("not-for-production", manifest["classification"])
            archive_digest = hashlib.sha256(first.archive.read_bytes()).hexdigest()
            self.assertEqual(manifest["archive"]["sha256"], archive_digest)
            with zipfile.ZipFile(first.archive) as archive:
                names = archive.namelist()
                self.assertEqual(len(names), len(set(names)))
                self.assertTrue(any(name.endswith("/.env.demo.example") for name in names))
                compose_name = next(name for name in names if name.endswith("/compose.yaml"))
                compose = archive.read(compose_name).decode()
                self.assertIn(server, compose)
                self.assertNotIn("@@", compose)
                self.assertNotIn(":latest", compose)

    def test_release_gate_rejects_non_hex_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(module.BuildError):
                module.build_bundle(
                    expected_commit="main",
                    server_image=f"{module.SERVER_REPOSITORY}@sha256:{'c' * 64}",
                    output_directory=Path(temporary),
                    enforce_git=False,
                )


if __name__ == "__main__":
    unittest.main()

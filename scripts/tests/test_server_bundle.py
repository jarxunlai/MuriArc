from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
SCRIPT = REPOSITORY / "scripts" / "build_server_bundle.py"


class ServerBundleBuilderTests(unittest.TestCase):
    def artifacts(self, root: Path) -> dict[str, Path]:
        values: dict[str, Path] = {}
        for name in ("server", "controller", "executor", "verifier"):
            path = root / name
            path.write_bytes(f"synthetic-{name}".encode())
            values[name] = path
        ui = root / "ui"
        ui.mkdir()
        (ui / "index.html").write_text("<main>MuriArc</main>\n", encoding="utf-8")
        values["ui"] = ui
        return values

    def run_builder(self, output: Path, values: dict[str, Path]) -> dict:
        result = subprocess.run(
            [
                "python3",
                str(SCRIPT),
                "--profile",
                "native-system",
                "--version",
                "1.0.0",
                "--output",
                str(output),
                "--server",
                str(values["server"]),
                "--controller",
                str(values["controller"]),
                "--upgrade-executor",
                str(values["executor"]),
                "--verifier",
                str(values["verifier"]),
                "--ui-dir",
                str(values["ui"]),
                "--deploy-root",
                str(REPOSITORY / "deploy"),
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        return json.loads(result.stdout)

    def test_native_bundle_is_deterministic_and_closed_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            values = self.artifacts(root)
            first = self.run_builder(root / "bundle-one", values)
            second = self.run_builder(root / "bundle-two", values)
            self.assertEqual(first["manifest_object_digest"], second["manifest_object_digest"])
            manifest = json.loads(
                (root / "bundle-one" / "bundle-manifest.json").read_text(encoding="utf-8")
            )
            paths = {item["path"] for item in manifest["files"]}
            self.assertIn("bin/muriarc-server", paths)
            self.assertIn("bin/muriarcctl", paths)
            self.assertNotIn("release/release-manifest.json", paths)
            self.assertIn("ui/index.html", paths)
            self.assertIn("deploy/cloudflare/cloudflared.service", paths)
            self.assertIn("deploy/cloudflare/muriarc.yml.example", paths)
            self.assertIn(
                "deploy/cloudflare/muriarc-cloudflare-public.conf.example", paths
            )

    def test_symlinked_ui_asset_is_rejected_without_partial_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            values = self.artifacts(root)
            (values["ui"] / "link").symlink_to(values["ui"] / "index.html")
            output = root / "bundle"
            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--profile",
                    "native-system",
                    "--version",
                    "1.0.0",
                    "--output",
                    str(output),
                    "--server",
                    str(values["server"]),
                    "--controller",
                    str(values["controller"]),
                    "--upgrade-executor",
                    str(values["executor"]),
                    "--verifier",
                    str(values["verifier"]),
                    "--ui-dir",
                    str(values["ui"]),
                    "--deploy-root",
                    str(REPOSITORY / "deploy"),
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()

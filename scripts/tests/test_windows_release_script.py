from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "build-windows-release.ps1"


class WindowsReleaseScriptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = SCRIPT.read_text(encoding="utf-8")

    def test_source_is_ascii_for_windows_powershell_51(self) -> None:
        self.text.encode("ascii")

    def test_formal_build_has_no_skip_checks_path(self) -> None:
        self.assertNotIn("SkipChecks", self.text)
        self.assertIn("checks_skipped=False", self.text)
        for marker in (
            "cargo clippy",
            "cargo test",
            "pnpm audit",
            "UI tests",
            "UI end-to-end tests",
            "local UI production build",
        ):
            self.assertIn(marker, self.text)

    def test_normalizes_pathext_and_requires_all_signing_inputs(self) -> None:
        self.assertIn("$env:PATHEXT = '.COM;.EXE;.BAT;.CMD", self.text)
        for marker in (
            "MURIARC_DESKTOP_UPDATER_PUBLIC_KEY",
            "TAURI_SIGNING_PRIVATE_KEY",
            "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
            "$AllowedSensitiveEnvironment",
        ):
            self.assertIn(marker, self.text)

    def test_requires_exact_clean_fresh_canonical_main(self) -> None:
        for marker in (
            "https://github.com/jarxunlai/MuriArc.git",
            "fetch canonical origin/main",
            "refs/remotes/origin/main",
            "$OriginMain -ne $ExpectedCommit",
            "$ActualCommit -ne $ExpectedCommit",
            "--porcelain=v1 --untracked-files=all",
            "post-build git status",
            "clean_tree_before_and_after=True",
        ):
            self.assertIn(marker, self.text)
        self.assertLess(
            self.text.index("git origin URL"),
            self.text.index("fetch canonical origin/main"),
        )

    def test_requires_installers_updater_archive_and_signature(self) -> None:
        for marker in (
            "No MSI or NSIS release artifact",
            "No MSI release artifact",
            "No NSIS release artifact",
            "No signed updater artifact",
            "No updater archive",
            "Duplicate Windows release artifact name",
        ):
            self.assertIn(marker, self.text)

    def test_creates_closed_deterministic_final_zip(self) -> None:
        for marker in (
            "artifact-inventory.json",
            "fixture-producer-executable",
            "release\\muriarc-desktop.exe",
            "exactly one fixture producer executable",
            "MuriArc-1.0.0-desktop-windows",
            "1980-01-01T00:00:00Z",
            "ZipArchiveMode]::Create",
            "Replace([char]92, [char]47)",
            "Compare-Object -ReferenceObject $ExpectedEntries",
            "release_artifact_sha256=",
        ):
            self.assertIn(marker, self.text)
        self.assertLess(
            self.text.index("artifact-inventory.json"),
            self.text.index("release_artifact_sha256="),
        )


if __name__ == "__main__":
    unittest.main()

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "build-windows-tester.ps1"


class WindowsTesterScriptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = SCRIPT.read_text(encoding="utf-8")

    def test_source_is_ascii_for_windows_powershell_51(self) -> None:
        self.text.encode("ascii")

    def test_requires_exact_clean_canonical_main_checkout(self) -> None:
        for marker in (
            "refs/remotes/origin/main",
            "+refs/heads/main:refs/remotes/origin/main",
            "fetch canonical origin/main",
            "freshly fetched origin/main tip",
            "https://github.com/jarxunlai/MuriArc.git",
            "status', '--porcelain=v1', '--untracked-files=all",
            "$OriginMain -ne $ExpectedCommit",
            "$ActualCommit -ne $ExpectedCommit",
        ):
            self.assertIn(marker, self.text)
        self.assertLess(
            self.text.index("git origin URL"),
            self.text.index("fetch canonical origin/main"),
        )
        self.assertLess(
            self.text.index("fetch canonical origin/main"),
            self.text.index("git origin/main identity"),
        )

    def test_uses_exact_desktop_binary_to_seed_and_verify_e0001(self) -> None:
        self.assertGreaterEqual(
            self.text.count("--muriarc-standard-fixture"), 2
        )
        for marker in (
            "'seed'",
            "'verify'",
            "sourceCommit -ne $ExpectedCommit",
            "applicationVersion -ne '1.0.0'",
            "dataEpoch -ne 'E0001'",
            "backend -ne 'sqlite'",
            "$FixtureDefinition",
            "Packaged synthetic baseline verification failed",
        ):
            self.assertIn(marker, self.text)

    def test_isolated_unsigned_tester_cannot_be_release_evidence(self) -> None:
        for marker in (
            "org.muriarc.desktop.tester.c$ShortCommit",
            "Remove-Item Env:MURIARC_DESKTOP_UPDATER_PUBLIC_KEY",
            "Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY",
            "createUpdaterArtifacts = $false",
            "formalRelease = $false",
            "formalRcEvidence = $false",
            "unsigned",
            "synthetic-data",
            "not-for-production",
        ):
            self.assertIn(marker, self.text)

    def test_scans_and_round_trip_verifies_package(self) -> None:
        for marker in (
            "Assert-NoReparsePoints",
            "Test-BytesForCredentialMaterial",
            "Test-FileForCredentialMaterial",
            "$ChunkSize = 4 * 1024 * 1024",
            "scannedBytes = $ScannedBytes",
            "aiSecretInventoryVerifiedEmpty = $true",
            "inventoryVerifier = 'muriarc-standard-fixture verify'",
            "CHECKSUMS.sha256",
            "Compress-Archive",
            "Expand-Archive",
            "Expanded Tester ZIP differs",
            "archiveSha256",
        ):
            self.assertIn(marker, self.text)
        self.assertRegex(
            self.text,
            re.compile(r"tester-v1\.0\.0-standard-v1-\$ShortCommit"),
        )

    def test_native_process_arguments_are_safe_on_windows_powershell_51(self) -> None:
        for marker in (
            "CommandLineToArgvW/MSVC escaping rules",
            "New-Object System.Text.StringBuilder",
            "[AllowEmptyCollection()][AllowEmptyString()][string[]]$Arguments",
            "[System.IO.File]::ReadAllText($StdoutPath)",
            "[System.IO.File]::ReadAllText($StderrPath)",
        ):
            self.assertIn(marker, self.text)

    def test_launcher_verifies_package_before_executing_it(self) -> None:
        for marker in (
            "CHECKSUMS.sha256 is missing",
            "Checksum contains an unsafe relative path",
            "Tester package root must not be a reparse point",
            "$CanonicalPackagePrefix",
            "$CanonicalChecksumFile",
            "Tester package checksum mismatch",
            "Tester package file inventory differs",
            "TESTER-MANIFEST.json identity or safety classification is invalid",
            "Tester executable does not match TESTER-MANIFEST.json",
        ):
            self.assertIn(marker, self.text)
        launcher = self.text.split("$LauncherTemplate = @'", 1)[1].split("\n'@", 1)[0]
        self.assertLess(
            launcher.index("$ExecutableHash = (Get-FileHash -LiteralPath $Executable"),
            launcher.index("Start-Process -FilePath $Executable"),
        )
        for marker in (
            "param([switch]$VerifyOnly)",
            "function ConvertTo-LauncherNativeArgument",
            "$FixtureVerifyProcess = Start-Process -FilePath $Executable",
            "$FixtureVerifyProcess.ExitCode",
            "MURIARC_TESTER_LAUNCHER_VERIFY=PASS",
            "packaged Tester launcher verification",
            "Packaged Tester launcher verification mutated the package",
            "desktop-standard-v1-seed-receipt.json",
            "desktop-standard-v1-verify-receipt.json",
            "packaged-launcher-verify.log",
        ):
            self.assertIn(marker, self.text)

    def test_build_sanitizes_environment_and_requires_x64_toolchains(self) -> None:
        for marker in (
            "$SensitiveEnvironment = @(Get-ChildItem Env:",
            "$_.Name -like 'VITE_*'",
            "x86_64-pc-windows-msvc",
            "The Tester requires x64 Node.js",
            "Windows PowerShell 5.1 is required",
            "sanitizedEnvironmentVariableCount",
        ):
            self.assertIn(marker, self.text)

    def test_smoke_proves_packaged_baseline_was_not_mutated(self) -> None:
        for marker in (
            "$PackagedDataBeforeSmoke = Get-FileDigestMap -Root $DataRoot",
            "$PackagedDataAfterSmoke = Get-FileDigestMap -Root $DataRoot",
            "Desktop startup smoke mutated the packaged synthetic baseline",
        ):
            self.assertIn(marker, self.text)


if __name__ == "__main__":
    unittest.main()

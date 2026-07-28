from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "build-linux-release.sh"


class LinuxReleaseScriptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = SCRIPT.read_text(encoding="utf-8")

    def test_release_script_has_no_skip_path(self) -> None:
        self.assertNotIn("--skip", self.text)
        for marker in (
            "cargo clippy --locked --workspace --all-targets --all-features",
            "cargo test --locked --workspace --all-targets --all-features",
            "pnpm --dir ui audit --audit-level=high",
            "pnpm --dir ui run test:e2e",
            "VITE_MURIARC_GATEWAY=remote",
        ):
            self.assertIn(marker, self.text)

    def test_source_and_account_are_locked_before_registry_writes(self) -> None:
        for marker in (
            "refs/remotes/origin/main",
            "git status --porcelain=v1 --untracked-files=all",
            "gh api user --jq .login",
            '"jarxunlai"',
            "docker login ghcr.io",
        ):
            self.assertIn(marker, self.text)
        self.assertLess(
            self.text.index("gh api user --jq .login"),
            self.text.index("docker login ghcr.io"),
        )

    def test_both_images_are_digest_pinned_signed_scanned_and_packaged(self) -> None:
        for marker in (
            "server_ref=",
            "postgres_ref=",
            '"$cosign" sign',
            '"$cosign" verify',
            '"$syft" scan "registry:',
            '"$grype" "sbom:',
            "--server-image-archive",
            "--postgres-image-archive",
            "--image-evidence-dir",
        ):
            self.assertIn(marker, self.text)

    def test_final_artifacts_have_descriptors_and_identity_from_shipped_verifier(self) -> None:
        for marker in (
            "package_release_tree.py",
            "finalize_release_artifact.py",
            "native-system",
            "managed-compose",
            '$native_root/bin/muriarc-verifier" identity',
            "release-identity.json",
        ):
            self.assertIn(marker, self.text)


if __name__ == "__main__":
    unittest.main()

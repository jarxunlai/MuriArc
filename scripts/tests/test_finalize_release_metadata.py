from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

import finalize_release_artifact as artifact_finalizer  # noqa: E402
import finalize_release_metadata as metadata  # noqa: E402


def digest(character: str) -> str:
    return "sha256:" + character * 64


class ReleaseMetadataFinalizerTests(unittest.TestCase):
    def test_release_provenance_covers_exact_digest_set(self) -> None:
        descriptors = {
            name: {
                "digest": digest(str(index + 1)),
            }
            for index, name in enumerate(sorted(artifact_finalizer.ARTIFACT_NAMES))
        }
        evidence = {
            name: {"provenance.intoto.json": digest(chr(ord("a") + index))}
            for index, name in enumerate(sorted(descriptors))
        }
        value = metadata.release_provenance(
            expected_commit="1" * 40,
            identity_digest=digest("f"),
            descriptors=descriptors,
            evidence=evidence,
            invocation_id="test-invocation",
            source_date_epoch=0,
        )
        subjects = value["subject"]
        self.assertEqual(
            {subject["name"] for subject in subjects},
            artifact_finalizer.ARTIFACT_NAMES,
        )
        self.assertTrue(
            value["predicate"]["buildDefinition"]["internalParameters"][
                "singleDigestSet"
            ]
        )

    def test_missing_artifact_is_rejected(self) -> None:
        with self.assertRaisesRegex(metadata.MetadataError, "exactly all three"):
            metadata.release_provenance(
                expected_commit="1" * 40,
                identity_digest=digest("f"),
                descriptors={"native-system": {"digest": digest("1")}},
                evidence={
                    "native-system": {"provenance.intoto.json": digest("a")}
                },
                invocation_id="test",
                source_date_epoch=0,
            )

    def test_metadata_script_signs_and_verifies_every_control_file(self) -> None:
        text = (ROOT / "scripts/finalize_release_metadata.py").read_text(
            encoding="utf-8"
        )
        self.assertNotIn("--skip", text)
        for marker in (
            "release-provenance.intoto.json",
            "release-manifest.json",
            "artifact-lock.json",
            "sign-blob",
            "verify-blob",
            "metadata-signature-evidence.json",
        ):
            self.assertIn(marker, text)


if __name__ == "__main__":
    unittest.main()

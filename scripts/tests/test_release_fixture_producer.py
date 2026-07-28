from __future__ import annotations

import hashlib
import io
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import release_fixture_producer as producer  # noqa: E402


PREFIX = "MuriArc-1.0.0-desktop-windows"
COMMIT = "a" * 40


def digest(raw: bytes) -> str:
    return f"sha256:{hashlib.sha256(raw).hexdigest()}"


def add_tar_bytes(bundle: tarfile.TarFile, name: str, raw: bytes) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(raw)
    bundle.addfile(info, io.BytesIO(raw))


class ReleaseFixtureProducerTests(unittest.TestCase):
    def write_windows_zip(
        self,
        path: Path,
        *,
        payload: bytes = b"fixture-producer",
        kind: str = "fixture-producer-executable",
        declared_digest: str | None = None,
    ) -> None:
        record = {
            "path": "payload/muriarc-desktop.exe",
            "kind": kind,
            "size_bytes": len(payload),
            "sha256": declared_digest or digest(payload),
        }
        inventory = {
            "format_version": 1,
            "artifact_name": "desktop-windows",
            "application_version": "1.0.0",
            "source_commit": COMMIT,
            "archive_prefix": PREFIX,
            "files": [record],
        }
        with zipfile.ZipFile(path, "w") as bundle:
            bundle.writestr(f"{PREFIX}/artifact-inventory.json", json.dumps(inventory))
            bundle.writestr(f"{PREFIX}/{record['path']}", payload)

    def test_zip_rejects_traversal_duplicate_and_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            traversal = root / "traversal.zip"
            with zipfile.ZipFile(traversal, "w") as bundle:
                bundle.writestr("../escape", b"x")
            with self.assertRaisesRegex(producer.ProducerError, "unsafe Windows ZIP"):
                producer.safe_zip_extract(traversal, root / "out-traversal")

            duplicate = root / "duplicate.zip"
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with zipfile.ZipFile(duplicate, "w") as bundle:
                    bundle.writestr("same", b"one")
                    bundle.writestr("same", b"two")
            with self.assertRaisesRegex(producer.ProducerError, "unsafe Windows ZIP"):
                producer.safe_zip_extract(duplicate, root / "out-duplicate")

            symlink = root / "symlink.zip"
            link = zipfile.ZipInfo("payload/link")
            link.create_system = 3
            link.external_attr = (stat.S_IFLNK | 0o777) << 16
            with zipfile.ZipFile(symlink, "w") as bundle:
                bundle.writestr(link, "target")
            with self.assertRaisesRegex(producer.ProducerError, "unsafe Windows ZIP"):
                producer.safe_zip_extract(symlink, root / "out-symlink")

    def test_zip_inventory_rejects_missing_producer_and_digest_drift(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            missing = root / "missing.zip"
            self.write_windows_zip(missing, kind="nsis")
            with self.assertRaisesRegex(producer.ProducerError, "lacks one producer"):
                producer.safe_zip_extract(missing, root / "out-missing")

            drift = root / "drift.zip"
            self.write_windows_zip(drift, declared_digest="sha256:" + "0" * 64)
            with self.assertRaisesRegex(producer.ProducerError, "payload differs"):
                producer.safe_zip_extract(drift, root / "out-drift")

    def test_tar_rejects_traversal_and_links(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            traversal = root / "traversal.tar"
            with tarfile.open(traversal, "w") as bundle:
                add_tar_bytes(bundle, "../escape", b"x")
            with self.assertRaisesRegex(producer.ProducerError, "unsafe Server artifact"):
                producer.safe_tar_extract(traversal, root / "out-traversal")

            linked = root / "linked.tar"
            with tarfile.open(linked, "w") as bundle:
                info = tarfile.TarInfo("bundle/link")
                info.type = tarfile.SYMTYPE
                info.linkname = "target"
                bundle.addfile(info)
            with self.assertRaisesRegex(producer.ProducerError, "unsafe Server artifact"):
                producer.safe_tar_extract(linked, root / "out-linked")

    def test_docker_archive_supports_legacy_and_oci_layouts(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            config = b'{"architecture":"amd64"}'
            config_hex = hashlib.sha256(config).hexdigest()

            legacy = root / "legacy.tar"
            manifest = json.dumps([{"Config": f"{config_hex}.json"}]).encode()
            with tarfile.open(legacy, "w") as bundle:
                add_tar_bytes(bundle, "manifest.json", manifest)
                add_tar_bytes(bundle, f"{config_hex}.json", config)
            self.assertEqual(producer.docker_archive_image_id(legacy), f"sha256:{config_hex}")

            oci = root / "oci.tar"
            config_digest = digest(config)
            image_manifest = json.dumps(
                {"schemaVersion": 2, "config": {"digest": config_digest}}
            ).encode()
            manifest_digest = digest(image_manifest)
            index = json.dumps(
                {"schemaVersion": 2, "manifests": [{"digest": manifest_digest}]}
            ).encode()
            with tarfile.open(oci, "w") as bundle:
                add_tar_bytes(bundle, "oci-layout", b'{"imageLayoutVersion":"1.0.0"}')
                add_tar_bytes(bundle, "index.json", index)
                add_tar_bytes(
                    bundle,
                    f"blobs/sha256/{manifest_digest.removeprefix('sha256:')}",
                    image_manifest,
                )
                add_tar_bytes(
                    bundle,
                    f"blobs/sha256/{config_digest.removeprefix('sha256:')}",
                    config,
                )
            self.assertEqual(producer.docker_archive_image_id(oci), config_digest)

    def test_signed_release_rejects_artifact_map_schema_drift(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            release = root / "release-manifest.json"
            lock = root / "artifact-lock.json"
            provenance = root / "release-provenance.intoto.json"
            metadata_evidence = root / "metadata-signature-evidence.json"
            inputs = root / "inputs.json"
            public_key = root / "cosign.pub"
            release.write_text('{"artifacts":{}}', encoding="utf-8")
            provenance_value = {
                "predicate": {
                    "buildDefinition": {
                        "resolvedDependencies": [
                            {
                                "uri": "git+https://github.com/jarxunlai/MuriArc",
                                "digest": {"gitCommit": COMMIT},
                            }
                        ]
                    }
                }
            }
            provenance.write_text(json.dumps(provenance_value), encoding="utf-8")
            lock.write_text(
                json.dumps({"release_provenance_digest": producer.sha256_file(provenance)[1]}),
                encoding="utf-8",
            )
            metadata_evidence.write_text("{}", encoding="utf-8")
            inputs.write_text(
                json.dumps({"format_version": 1, "artifacts": {}, "unexpected": True}),
                encoding="utf-8",
            )
            public_key.write_text("synthetic-public-key", encoding="utf-8")

            patches = (
                mock.patch.object(producer.readiness, "validate_definition"),
                mock.patch.object(producer.readiness, "validate_release_manifest"),
                mock.patch.object(producer.readiness, "validate_source_identity"),
                mock.patch.object(producer.readiness, "validate_artifact_lock", return_value={}),
                mock.patch.object(producer, "validate_release_provenance"),
                mock.patch.object(producer, "validate_release_provenance_dependencies"),
                mock.patch.object(producer, "validate_current_checkout"),
                mock.patch.object(producer, "validate_metadata_signature_evidence"),
                mock.patch.object(producer, "cosign_verify_blob"),
            )
            with patches[0], patches[1], patches[2], patches[3], patches[4], patches[5], patches[6], patches[7], patches[8]:
                with self.assertRaisesRegex(producer.ProducerError, "artifact input map keys differ"):
                    producer.validate_signed_release(
                        release, lock, inputs, "cosign", public_key
                    )

    def test_signature_evidence_rejects_schema_and_digest_drift(self) -> None:
        value = {
            "format_version": 1,
            "artifact_name": "desktop-windows",
            "artifact_digest": "sha256:" + "1" * 64,
            "scheme": "sigstore-cosign-key-pair-bundle-v3",
            "cosign_bundle_digest": "sha256:" + "2" * 64,
            "cosign_public_key_digest": "sha256:" + "3" * 64,
            "verification": "pass",
            "verified_artifact_unchanged": True,
        }
        producer.validate_signature_evidence(
            value,
            "desktop-windows",
            value["artifact_digest"],
            value["cosign_bundle_digest"],
            value["cosign_public_key_digest"],
        )
        with self.assertRaisesRegex(producer.ProducerError, "keys differ"):
            producer.validate_signature_evidence(
                {**value, "unexpected": True},
                "desktop-windows",
                value["artifact_digest"],
                value["cosign_bundle_digest"],
                value["cosign_public_key_digest"],
            )
        with self.assertRaisesRegex(producer.ProducerError, "does not bind"):
            producer.validate_signature_evidence(
                value,
                "desktop-windows",
                "sha256:" + "4" * 64,
                value["cosign_bundle_digest"],
                value["cosign_public_key_digest"],
            )

    def test_publisher_never_overwrites_and_reuses_only_identical_signed_tag(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            fixture = root / "fixture"
            fixture.mkdir()
            (fixture / "data.txt").write_text("synthetic fixture\n", encoding="utf-8")
            state = root / "oras-state"
            log = root / "cosign.log"
            public_key = root / "cosign.pub"
            public_key.write_text("synthetic public key", encoding="utf-8")

            verifier = root / "verifier"
            verifier.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            verifier.chmod(0o700)
            cargo = root / "cargo"
            cargo.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            cargo.chmod(0o700)
            oras = root / "oras"
            oras.write_text(
                """#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  resolve)
    if [[ -f "$MURIARC_FAKE_ORAS_STATE" ]]; then
      printf 'sha256:%064d\n' 7
    else
      printf 'manifest unknown: not found\n' >&2
      exit 1
    fi
    ;;
  push)
    : >"$MURIARC_FAKE_ORAS_STATE"
    ;;
  manifest)
    [[ "${2-}" == fetch ]]
    python3 - "$MURIARC_FAKE_LAYER_DIGEST" <<'INNER'
import json,sys
print(json.dumps({
  "artifactType": "application/vnd.muriarc.release-fixture.v1",
  "layers": [{
    "mediaType": "application/vnd.muriarc.release-fixture.layer.v1+tar",
    "digest": sys.argv[1],
  }],
}))
INNER
    ;;
  *) exit 2 ;;
esac
""",
                encoding="utf-8",
            )
            oras.chmod(0o700)
            cosign = root / "cosign"
            cosign.write_text(
                """#!/usr/bin/env bash
printf '%s\n' "$1" >>"$MURIARC_FAKE_COSIGN_LOG"
exit 0
""",
                encoding="utf-8",
            )
            cosign.chmod(0o700)

            environment = os.environ.copy()
            environment["PATH"] = f"{root}:{environment.get('PATH', '')}"
            environment.update(
                {
                    "MURIARC_VERIFIER": str(verifier),
                    "MURIARC_ORAS": str(oras),
                    "MURIARC_COSIGN": str(cosign),
                    "MURIARC_FAKE_ORAS_STATE": str(state),
                    "MURIARC_FAKE_COSIGN_LOG": str(log),
                    "MURIARC_FAKE_LAYER_DIGEST": "sha256:" + "0" * 64,
                    "COSIGN_KEY": "synthetic-key-reference",
                    "COSIGN_PUBLIC_KEY": str(public_key),
                }
            )
            command = [
                str(SCRIPTS / "publish-release-fixture.sh"),
                "--fixture",
                str(fixture),
                "--repository",
                "ghcr.io/jarxunlai/test-release-fixtures",
                "--tag",
                "immutable-test",
                "--manifest-digest",
                "sha256:" + "1" * 64,
            ]
            first = subprocess.run(
                command,
                cwd=SCRIPTS.parent,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            published = json.loads(first.stdout)
            environment["MURIARC_FAKE_LAYER_DIGEST"] = published["fixture_tar_digest"]
            second = subprocess.run(
                command,
                cwd=SCRIPTS.parent,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(
                json.loads(second.stdout)["fixture_artifact_digest"],
                published["fixture_artifact_digest"],
            )
            actions = log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(actions.count("sign"), 1)
            self.assertEqual(actions.count("verify"), 2)

            environment["MURIARC_FAKE_LAYER_DIGEST"] = "sha256:" + "8" * 64
            mismatch = subprocess.run(
                command,
                cwd=SCRIPTS.parent,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(mismatch.returncode, 0)
            self.assertIn("existing immutable Fixture tag differs", mismatch.stderr)

    def test_catalog_append_uses_digest_pinned_parameters(self) -> None:
        response = json.dumps(
            {
                "ok": True,
                "code": "ok",
                "message": "done",
                "data": {
                    "entryCount": 1,
                    "fixtureId": "fixture",
                    "fixtureContentDigest": "sha256:" + "9" * 64,
                },
            }
        )
        published = {
            "fixture_artifact_digest": "sha256:" + "5" * 64,
            "oci_reference": "ghcr.io/jarxunlai/muriarc-release-fixtures@sha256:"
            + "5" * 64,
        }
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with mock.patch.object(producer, "run", return_value=response) as invoked:
                producer.append_catalog(
                    "verifier",
                    root / "baseline.json",
                    [(root / "fixture", published)],
                    root / "candidate.json",
                    root / "temporary",
                )
        command = invoked.call_args.args[0]
        self.assertEqual(command[0:2], ["verifier", "catalog-append"])
        self.assertEqual(
            command[command.index("--fixture-artifact-digest") + 1],
            published["fixture_artifact_digest"],
        )
        self.assertEqual(
            command[command.index("--oci-reference") + 1], published["oci_reference"]
        )
        self.assertIn("--candidate-output", command)
        self.assertEqual(command[-2:], ["--output", "json"])


if __name__ == "__main__":
    unittest.main()

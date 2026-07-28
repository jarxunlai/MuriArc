#!/usr/bin/env python3
"""Create fail-closed SBOM, vulnerability, provenance, and signature evidence.

This command finalizes one already-built immutable artifact. It never builds an
artifact and never writes inside the source checkout. The source must be the
clean, freshly fetched canonical origin/main commit named by --expected-commit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
ARTIFACT_NAMES = {"native-system", "managed-compose", "desktop-windows"}
CANONICAL_ORIGINS = {
    "https://github.com/jarxunlai/MuriArc",
    "https://github.com/jarxunlai/MuriArc.git",
}
SENSITIVE_ENV_RE = re.compile(
    r"(?:^|_)(?:api_?key|credential|csrf|master_?key|password|passwd|private_?key|secret|session|token)(?:$|_)",
    re.IGNORECASE,
)


class FinalizeError(ValueError):
    pass


def regular_file(path: Path, label: str, *, non_empty: bool = True) -> Path:
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise FinalizeError(f"{label} cannot be inspected: {exc}") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise FinalizeError(f"{label} must be a regular non-symlink file")
    if non_empty and metadata.st_size <= 0:
        raise FinalizeError(f"{label} must not be empty")
    return path


def executable_file(path: Path, label: str) -> Path:
    regular_file(path, label)
    if os.name != "nt" and path.stat().st_mode & 0o111 == 0:
        raise FinalizeError(f"{label} must be executable")
    return path


def sha256_file(path: Path) -> tuple[int, str]:
    size = 0
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            size += len(block)
            hasher.update(block)
    return size, f"sha256:{hasher.hexdigest()}"


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def write_json_new(path: Path, value: Any, *, canonical: bool = False) -> None:
    if path.exists() or path.is_symlink():
        raise FinalizeError(f"refusing to overwrite evidence: {path}")
    raw = canonical_bytes(value) if canonical else (
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode()
    with path.open("xb") as handle:
        handle.write(raw)
        handle.flush()
        os.fsync(handle.fileno())
    path.chmod(0o600)


def run_checked(
    args: Iterable[str | os.PathLike[str]],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    stdout: int | None = subprocess.PIPE,
) -> subprocess.CompletedProcess[bytes]:
    command = [os.fspath(value) for value in args]
    result = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=stdout,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        program = Path(command[0]).name
        raise FinalizeError(f"{program} failed with exit code {result.returncode}")
    return result


def git_text(source_root: Path, *args: str) -> str:
    result = run_checked(("git", *args), cwd=source_root)
    return result.stdout.decode("utf-8", "strict").strip()


def validate_source(source_root: Path, expected_commit: str) -> int:
    if not COMMIT_RE.fullmatch(expected_commit):
        raise FinalizeError("--expected-commit must be a lowercase 40-hex commit")
    root = source_root.resolve(strict=True)
    if git_text(root, "rev-parse", "--show-toplevel") != str(root):
        raise FinalizeError("--source-root must be the exact Git worktree root")
    origin = git_text(root, "remote", "get-url", "origin").rstrip("/")
    if origin not in {value.rstrip("/") for value in CANONICAL_ORIGINS}:
        raise FinalizeError("release source origin is not canonical GitHub MuriArc")
    run_checked(
        (
            "git",
            "fetch",
            "--no-tags",
            "--prune",
            "origin",
            "+refs/heads/main:refs/remotes/origin/main",
        ),
        cwd=root,
    )
    head = git_text(root, "rev-parse", "HEAD")
    origin_main = git_text(root, "rev-parse", "refs/remotes/origin/main")
    if head != expected_commit or origin_main != expected_commit:
        raise FinalizeError("HEAD, expected commit, and freshly fetched origin/main must match")
    if git_text(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise FinalizeError("formal release source must remain clean")
    timestamp = git_text(root, "show", "-s", "--format=%ct", expected_commit)
    if not timestamp.isascii() or not timestamp.isdigit():
        raise FinalizeError("source commit timestamp is invalid")
    return int(timestamp)


def safe_tool_environment(password: str, source_date_epoch: int) -> dict[str, str]:
    allowed = {
        "HOME",
        "PATH",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
    }
    result = {
        key: value
        for key, value in os.environ.items()
        if key in allowed and not SENSITIVE_ENV_RE.search(key)
    }
    result.update(
        {
            "COSIGN_PASSWORD": password,
            "SOURCE_DATE_EPOCH": str(source_date_epoch),
            "TZ": "UTC",
            "LC_ALL": "C",
        }
    )
    return result


def tool_version(tool: Path, source_root: Path, env: dict[str, str]) -> str:
    result = run_checked((tool, "version"), cwd=source_root, env=env)
    text = result.stdout.decode("utf-8", "replace").strip()
    if not text:
        raise FinalizeError(f"{tool.name} returned no version")
    return text[:4096]


def validate_grype_report(path: Path) -> dict[str, int]:
    regular_file(path, "Grype report")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise FinalizeError(f"Grype report is invalid: {exc}") from exc
    matches = value.get("matches")
    if not isinstance(matches, list):
        raise FinalizeError("Grype report has no matches array")
    counts = {"critical": 0, "high": 0, "medium": 0, "low": 0, "negligible": 0, "unknown": 0}
    for match in matches:
        severity = (
            match.get("vulnerability", {}).get("severity", "unknown")
            if isinstance(match, dict)
            else "unknown"
        )
        key = str(severity).lower()
        counts[key if key in counts else "unknown"] += 1
    if counts["critical"] or counts["high"]:
        raise FinalizeError("artifact contains High or Critical vulnerabilities")
    return counts


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    source_root = args.source_root.resolve(strict=True)
    source_date_epoch = validate_source(source_root, args.expected_commit)
    if args.artifact_name not in ARTIFACT_NAMES:
        raise FinalizeError("artifact name is not a mandatory release profile")
    artifact = regular_file(args.artifact.resolve(strict=True), "release artifact")
    if not isinstance(args.media_type, str) or "/" not in args.media_type or "\n" in args.media_type:
        raise FinalizeError("--media-type is invalid")

    output = args.output_directory
    if not output.is_absolute():
        raise FinalizeError("--output-directory must be absolute")
    output_resolved = output.resolve(strict=False)
    if output.exists() or output.is_symlink():
        raise FinalizeError("--output-directory must not already exist")
    try:
        output_resolved.relative_to(source_root)
    except ValueError:
        pass
    else:
        raise FinalizeError("release evidence must remain outside the Git worktree")

    tools = {
        "cosign": executable_file(args.cosign.resolve(strict=True), "Cosign"),
        "syft": executable_file(args.syft.resolve(strict=True), "Syft"),
        "grype": executable_file(args.grype.resolve(strict=True), "Grype"),
    }
    cosign_key = regular_file(args.cosign_key.resolve(strict=True), "Cosign private key")
    cosign_public_key = regular_file(
        args.cosign_public_key.resolve(strict=True), "Cosign public key"
    )
    password_file = regular_file(args.cosign_password_file.resolve(strict=True), "Cosign password")
    password = password_file.read_text(encoding="utf-8").rstrip("\r\n")
    if not password:
        raise FinalizeError("Cosign password file is empty")

    output.mkdir(mode=0o700, parents=False)
    try:
        artifact_size, artifact_digest = sha256_file(artifact)
        environment = safe_tool_environment(password, source_date_epoch)
        versions = {
            name: tool_version(tool, source_root, environment)
            for name, tool in tools.items()
        }

        sbom = output / "sbom.cdx.json"
        run_checked(
            (
                tools["syft"],
                "scan",
                f"file:{artifact}",
                "-o",
                f"cyclonedx-json={sbom}",
            ),
            cwd=source_root,
            env=environment,
        )
        regular_file(sbom, "CycloneDX SBOM")

        grype = output / "grype.json"
        run_checked(
            (
                tools["grype"],
                f"sbom:{sbom}",
                "-o",
                "json",
                "--file",
                grype,
                "--fail-on",
                "high",
            ),
            cwd=source_root,
            env=environment,
        )
        vulnerability_counts = validate_grype_report(grype)

        provenance = {
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [
                {
                    "name": f"MuriArc-{args.artifact_name}",
                    "digest": {"sha256": artifact_digest.removeprefix("sha256:")},
                }
            ],
            "predicateType": "https://slsa.dev/provenance/v1",
            "predicate": {
                "buildDefinition": {
                    "buildType": "https://github.com/jarxunlai/MuriArc/release-build/v1",
                    "externalParameters": {
                        "artifactName": args.artifact_name,
                        "mediaType": args.media_type,
                    },
                    "internalParameters": {
                        "checksSkipped": False,
                        "sourceTreeCleanBeforeAndAfter": True,
                    },
                    "resolvedDependencies": [
                        {
                            "uri": "git+https://github.com/jarxunlai/MuriArc",
                            "digest": {"gitCommit": args.expected_commit},
                        },
                        {
                            "uri": "pkg:generic/syft",
                            "digest": {"sha256": sha256_file(tools["syft"])[1].removeprefix("sha256:")},
                        },
                        {
                            "uri": "pkg:generic/grype",
                            "digest": {"sha256": sha256_file(tools["grype"])[1].removeprefix("sha256:")},
                        },
                    ],
                },
                "runDetails": {
                    "builder": {
                        "id": "https://github.com/jarxunlai/MuriArc/release-environment/1.0.0"
                    },
                    "metadata": {
                        "invocationId": args.invocation_id,
                        "startedOn": datetime.fromtimestamp(
                            source_date_epoch, timezone.utc
                        ).isoformat().replace("+00:00", "Z"),
                        "finishedOn": datetime.now(timezone.utc)
                        .isoformat()
                        .replace("+00:00", "Z"),
                    },
                },
            },
        }
        provenance_path = output / "provenance.intoto.json"
        write_json_new(provenance_path, provenance, canonical=True)

        cosign_bundle = output / "artifact.cosign.bundle.json"
        run_checked(
            (
                tools["cosign"],
                "sign-blob",
                "--yes",
                "--key",
                cosign_key,
                "--bundle",
                cosign_bundle,
                artifact,
            ),
            cwd=source_root,
            env=environment,
        )
        regular_file(cosign_bundle, "Cosign bundle")
        run_checked(
            (
                tools["cosign"],
                "verify-blob",
                "--key",
                cosign_public_key,
                "--bundle",
                cosign_bundle,
                artifact,
            ),
            cwd=source_root,
            env=environment,
        )

        post_size, post_digest = sha256_file(artifact)
        if (post_size, post_digest) != (artifact_size, artifact_digest):
            raise FinalizeError("release artifact changed while evidence was generated")
        if git_text(source_root, "status", "--porcelain=v1", "--untracked-files=all"):
            raise FinalizeError("evidence generation dirtied the release source tree")

        _, sbom_digest = sha256_file(sbom)
        _, grype_digest = sha256_file(grype)
        _, provenance_digest = sha256_file(provenance_path)
        _, bundle_digest = sha256_file(cosign_bundle)
        _, public_key_digest = sha256_file(cosign_public_key)
        signature_evidence = {
            "format_version": 1,
            "artifact_name": args.artifact_name,
            "artifact_digest": artifact_digest,
            "scheme": "sigstore-cosign-key-pair-bundle-v3",
            "cosign_bundle_digest": bundle_digest,
            "cosign_public_key_digest": public_key_digest,
            "verification": "pass",
            "verified_artifact_unchanged": True,
        }
        signature_path = output / "signature-evidence.json"
        write_json_new(signature_path, signature_evidence, canonical=True)
        _, signature_digest = sha256_file(signature_path)

        scan_evidence = {
            "format_version": 1,
            "artifact_name": args.artifact_name,
            "artifact_digest": artifact_digest,
            "sbom_digest": sbom_digest,
            "grype_report_digest": grype_digest,
            "vulnerability_counts": vulnerability_counts,
            "policy": {"fail_on": "high", "critical": 0, "high": 0},
            "tool_versions": versions,
        }
        write_json_new(output / "scan-evidence.json", scan_evidence, canonical=True)

        descriptor = {
            "format_version": 1,
            "artifact_name": args.artifact_name,
            "media_type": args.media_type,
            "digest": artifact_digest,
            "size_bytes": artifact_size,
            "provenance_digest": provenance_digest,
            "signature_evidence_digest": signature_digest,
        }
        descriptor_path = output / "descriptor.json"
        write_json_new(descriptor_path, descriptor)
        return {
            "artifact": str(artifact),
            "descriptor": str(descriptor_path),
            "digest": artifact_digest,
            "size_bytes": artifact_size,
            "provenance_digest": provenance_digest,
            "signature_evidence_digest": signature_digest,
        }
    except Exception:
        shutil.rmtree(output, ignore_errors=True)
        raise


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--source-root", type=Path, default=Path("."))
    result.add_argument("--expected-commit", required=True)
    result.add_argument("--artifact-name", required=True, choices=sorted(ARTIFACT_NAMES))
    result.add_argument("--artifact", type=Path, required=True)
    result.add_argument("--media-type", required=True)
    result.add_argument("--output-directory", type=Path, required=True)
    result.add_argument("--invocation-id", required=True)
    result.add_argument("--cosign", type=Path, required=True)
    result.add_argument("--cosign-key", type=Path, required=True)
    result.add_argument("--cosign-public-key", type=Path, required=True)
    result.add_argument("--cosign-password-file", type=Path, required=True)
    result.add_argument("--syft", type=Path, required=True)
    result.add_argument("--grype", type=Path, required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        value = finalize(parser().parse_args(sys.argv[1:] if argv is None else argv))
    except (FinalizeError, OSError, UnicodeError) as exc:
        print(f"release artifact finalization failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(value, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

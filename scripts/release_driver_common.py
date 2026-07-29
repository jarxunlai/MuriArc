#!/usr/bin/env python3
"""Shared fail-closed helpers for physical release evidence drivers."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Mapping

import release_fixture_producer as fixture_producer


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


class DriverError(ValueError):
    pass


def _object_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise DriverError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_json(path: Path, label: str) -> tuple[dict[str, Any], bytes]:
    path = regular_file(path, label)
    try:
        raw = path.read_bytes()
        value = json.loads(raw, object_pairs_hook=_object_no_duplicates)
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise DriverError(f"{label} is not valid JSON") from exc
    if not isinstance(value, dict):
        raise DriverError(f"{label} must contain a JSON object")
    return value, raw


def exact_keys(value: Mapping[str, Any], expected: set[str], label: str) -> None:
    if set(value) != expected:
        raise DriverError(f"{label} keys differ from the closed driver contract")


def regular_file(path: Path, label: str, *, nonempty: bool = True) -> Path:
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise DriverError(f"{label} is unavailable") from exc
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode) or (nonempty and metadata.st_size == 0):
        raise DriverError(f"{label} must be a regular non-symlink file")
    return path


def real_directory(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise DriverError(f"{label} is unavailable") from exc
    if path.is_symlink() or not stat.S_ISDIR(metadata.st_mode):
        raise DriverError(f"{label} must be a real directory")
    return path


def new_directory(path: Path, label: str) -> Path:
    if path.exists() or path.is_symlink():
        raise DriverError(f"{label} must be a new path")
    path.mkdir(parents=True, mode=0o700)
    os.chmod(path, 0o700)
    return real_directory(path, label)


def executable(path_value: str, label: str) -> Path:
    if not path_value:
        raise DriverError(f"{label} is required")
    path = Path(path_value)
    if not path.is_absolute():
        raise DriverError(f"{label} must use an absolute path")
    path = regular_file(path, label)
    if path.stat().st_mode & 0o111 == 0:
        raise DriverError(f"{label} is not executable")
    return path


def outside_repository(path: Path, label: str) -> Path:
    candidate = path.resolve(strict=False)
    root = REPOSITORY_ROOT.resolve(strict=True)
    try:
        candidate.relative_to(root)
    except ValueError:
        return path
    raise DriverError(f"{label} must remain outside the Git worktree")


def require_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or DIGEST_RE.fullmatch(value) is None:
        raise DriverError(f"{label} must be a lowercase SHA-256 digest")
    return value


def sha256_bytes(raw: bytes) -> str:
    return f"sha256:{hashlib.sha256(raw).hexdigest()}"


def sha256_file(path: Path) -> tuple[int, str]:
    path = regular_file(path, "hashed input")
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            size += len(block)
            digest.update(block)
    return size, f"sha256:{digest.hexdigest()}"


def parse_time(value: Any, label: str) -> datetime:
    if not isinstance(value, str) or not value.endswith("Z"):
        raise DriverError(f"{label} must be an RFC3339 UTC timestamp")
    try:
        parsed = datetime.fromisoformat(value[:-1] + "+00:00")
    except ValueError as exc:
        raise DriverError(f"{label} must be an RFC3339 UTC timestamp") from exc
    if parsed.tzinfo != timezone.utc:
        raise DriverError(f"{label} must use UTC")
    return parsed


def require_count(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise DriverError(f"{label} must be a non-negative integer")
    return value


def write_json_new(path: Path, value: Mapping[str, Any]) -> None:
    outside_repository(path, "driver output")
    if path.exists() or path.is_symlink():
        raise DriverError("driver output must be a new path")
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    real_directory(path.parent, "driver output parent")
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    if temporary.exists() or temporary.is_symlink():
        raise DriverError("driver temporary output already exists")
    raw = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()
    try:
        with temporary.open("xb") as stream:
            os.chmod(temporary, 0o600)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def run_suppressed(command: Iterable[object], label: str) -> None:
    completed = subprocess.run(
        [os.fspath(value) for value in command],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if completed.returncode != 0:
        raise DriverError(f"{label} failed closed")


def validate_signed_release_inputs(
    release_manifest: Path,
    artifact_lock: Path,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, dict[str, Path]], str]:
    inputs_value = os.environ.get("MURIARC_RELEASE_ARTIFACT_INPUTS", "")
    if not inputs_value:
        raise DriverError("MURIARC_RELEASE_ARTIFACT_INPUTS is required")
    public_key_value = os.environ.get("COSIGN_PUBLIC_KEY", "")
    if not public_key_value:
        raise DriverError("COSIGN_PUBLIC_KEY is required")
    try:
        cosign = fixture_producer.executable(
            os.environ.get("MURIARC_COSIGN", "cosign"), "Cosign"
        )
        public_key_path = Path(public_key_value)
        inputs_path = Path(inputs_value)
        if not public_key_path.is_absolute() or not inputs_path.is_absolute():
            raise DriverError("Cosign public key and artifact input map must use absolute paths")
        public_key = regular_file(public_key_path, "Cosign public key")
        inputs = regular_file(
            outside_repository(inputs_path, "artifact input map"), "artifact input map"
        )
        return fixture_producer.validate_signed_release(
            release_manifest.resolve(strict=True),
            artifact_lock.resolve(strict=True),
            inputs,
            cosign,
            public_key,
        )
    except (fixture_producer.ProducerError, OSError, ValueError) as exc:
        raise DriverError("signed final release inputs did not verify") from exc


def validate_final_verifier(verifier: Path, release: Mapping[str, Any]) -> None:
    completed = subprocess.run(
        [os.fspath(verifier), "identity", "--output", "json"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if completed.returncode != 0 or len(completed.stdout) > 1024 * 1024:
        raise DriverError("final verifier identity command failed")
    try:
        wrapper = json.loads(completed.stdout, object_pairs_hook=_object_no_duplicates)
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise DriverError("final verifier identity is invalid") from exc
    if not isinstance(wrapper, dict) or set(wrapper) != {"ok", "code", "message", "data"}:
        raise DriverError("final verifier identity wrapper is invalid")
    identity = wrapper.get("data")
    if not isinstance(identity, dict):
        raise DriverError("final verifier identity payload is invalid")
    expected = {
        key: release[key]
        for key in (
            "format_version",
            "application_version",
            "data_epoch",
            "gateway_contract_revision",
            "backend_states",
            "postgres_major",
            "bootstrap_protocol_revision",
            "controller_protocol_min",
            "controller_protocol_max",
            "migration_class",
        )
    }
    if identity != expected:
        raise DriverError("final verifier was not built from the target release identity")

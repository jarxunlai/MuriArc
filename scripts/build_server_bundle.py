#!/usr/bin/env python3
"""Build a deterministic, closed-inventory MuriArc Server bundle.

The script only packages already-built, final artifacts. It never invokes Cargo,
pnpm, Docker, or a signer, and it refuses symlinks and an existing output path.
The resulting bundle-manifest.json is the object whose SHA-256 must be carried
by signed release metadata before muriarcctl may install the bundle.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath


VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


@dataclass(frozen=True)
class InputFile:
    source: Path
    target: PurePosixPath
    role: str
    executable: bool = False


def regular_file(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ValueError(f"{label} cannot be inspected: {error}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{label} must be a regular non-symlink file")
    if metadata.st_size <= 0:
        raise ValueError(f"{label} must not be empty")
    return path


def real_directory(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ValueError(f"{label} cannot be inspected: {error}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise ValueError(f"{label} must be a real non-symlink directory")
    return path


def tree_files(root: Path, target_root: PurePosixPath, role: str) -> list[InputFile]:
    result: list[InputFile] = []
    pending = [root]
    while pending:
        directory = pending.pop()
        for child in sorted(directory.iterdir(), key=lambda item: item.name, reverse=True):
            metadata = child.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                raise ValueError(f"input tree contains symlink: {child}")
            if stat.S_ISDIR(metadata.st_mode):
                pending.append(child)
            elif stat.S_ISREG(metadata.st_mode):
                if metadata.st_size <= 0:
                    raise ValueError(f"input tree contains empty file: {child}")
                relative = PurePosixPath(child.relative_to(root).as_posix())
                result.append(InputFile(child, target_root / relative, role))
            else:
                raise ValueError(f"input tree contains non-regular entry: {child}")
    return result


def digest(path: Path) -> tuple[int, str]:
    hasher = hashlib.sha256()
    length = 0
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            length += len(block)
            hasher.update(block)
    return length, f"sha256:{hasher.hexdigest()}"


def copy_one(item: InputFile, output: Path) -> dict[str, object]:
    target_text = item.target.as_posix()
    if (
        item.target.is_absolute()
        or not target_text
        or any(part in ("", ".", "..") for part in item.target.parts)
        or "\\" in target_text
    ):
        raise ValueError(f"unsafe bundle target: {target_text}")
    target = output / target_text
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o750)
    with item.source.open("rb") as source, target.open("xb") as destination:
        shutil.copyfileobj(source, destination, 1024 * 1024)
        destination.flush()
        os.fsync(destination.fileno())
    target.chmod(0o750 if item.executable else 0o640)
    size, sha256 = digest(target)
    return {
        "path": target_text,
        "role": item.role,
        "size_bytes": size,
        "sha256": sha256,
    }


def inputs(args: argparse.Namespace) -> list[InputFile]:
    cloudflare = real_directory(args.deploy_root / "cloudflare-public", "Cloudflare deploy directory")
    common = [
        InputFile(regular_file(args.controller, "controller"), PurePosixPath("bin/muriarcctl"), "controller", True),
        InputFile(
            regular_file(args.upgrade_executor, "upgrade executor"),
            PurePosixPath("bin/muriarc-upgrade-executor"),
            "upgrade_executor",
            True,
        ),
        InputFile(regular_file(args.verifier, "verifier"), PurePosixPath("bin/muriarc-verifier"), "verifier", True),
        InputFile(regular_file(cloudflare / "cloudflared.service", "cloudflared systemd unit"), PurePosixPath("deploy/cloudflare/cloudflared.service"), "systemd_service"),
        InputFile(regular_file(cloudflare / "cloudflared.sysusers", "cloudflared sysusers file"), PurePosixPath("deploy/cloudflare/cloudflared.sysusers"), "sysusers"),
        InputFile(regular_file(cloudflare / "cloudflared.tmpfiles", "cloudflared tmpfiles file"), PurePosixPath("deploy/cloudflare/cloudflared.tmpfiles"), "tmpfiles"),
        InputFile(regular_file(cloudflare / "muriarc.yml.example", "Cloudflare Tunnel config example"), PurePosixPath("deploy/cloudflare/muriarc.yml.example"), "environment_example"),
    ]
    if args.profile == "native-system":
        if args.server is None or args.ui_dir is None:
            raise ValueError("native-system requires --server and --ui-dir")
        deploy = real_directory(args.deploy_root / "native-system", "native deploy directory")
        common.extend(
            [
                InputFile(regular_file(args.server, "server"), PurePosixPath("bin/muriarc-server"), "server", True),
                InputFile(regular_file(deploy / "muriarc.service", "systemd unit"), PurePosixPath("deploy/muriarc.service"), "systemd_service"),
                InputFile(regular_file(deploy / "muriarc.sysusers", "sysusers file"), PurePosixPath("deploy/muriarc.sysusers"), "sysusers"),
                InputFile(regular_file(deploy / "muriarc.tmpfiles", "tmpfiles file"), PurePosixPath("deploy/muriarc.tmpfiles"), "tmpfiles"),
                InputFile(regular_file(deploy / "delivery.json", "delivery descriptor"), PurePosixPath("deploy/delivery.json"), "delivery_descriptor"),
                InputFile(regular_file(deploy / "server.env.example", "environment example"), PurePosixPath("deploy/server.env.example"), "environment_example"),
                InputFile(regular_file(deploy / "active.env.example", "activation example"), PurePosixPath("deploy/active.env.example"), "environment_example"),
                InputFile(regular_file(cloudflare / "muriarc-cloudflare-public.conf.example", "MuriArc public profile systemd drop-in"), PurePosixPath("deploy/cloudflare/muriarc-cloudflare-public.conf.example"), "environment_example"),
                InputFile(regular_file(cloudflare / "muriarc-external-api.conf.example", "MuriArc external API systemd drop-in"), PurePosixPath("deploy/cloudflare/muriarc-external-api.conf.example"), "environment_example"),
            ]
        )
        common.extend(tree_files(real_directory(args.ui_dir, "UI directory"), PurePosixPath("ui"), "ui_asset"))
    else:
        deploy = real_directory(args.deploy_root / "managed-compose", "Compose deploy directory")
        common.extend(
            [
                InputFile(regular_file(deploy / "compose.yaml", "Compose file"), PurePosixPath("deploy/compose.yaml"), "compose_file"),
                InputFile(regular_file(deploy / "descriptor.json", "Compose descriptor"), PurePosixPath("deploy/descriptor.json"), "compose_descriptor"),
                InputFile(regular_file(deploy / ".env.example", "environment example"), PurePosixPath("deploy/.env.example"), "environment_example"),
                InputFile(regular_file(deploy / "active.env.example", "activation example"), PurePosixPath("deploy/active.env.example"), "environment_example"),
                InputFile(regular_file(cloudflare / "compose.override.yaml", "Cloudflare Compose override"), PurePosixPath("deploy/cloudflare/compose.override.yaml"), "compose_file"),
                InputFile(regular_file(cloudflare / "compose.external-api.override.yaml", "Cloudflare external API Compose override"), PurePosixPath("deploy/cloudflare/compose.external-api.override.yaml"), "compose_file"),
            ]
        )
    targets = [item.target.as_posix() for item in common]
    if len(targets) != len(set(targets)):
        raise ValueError("bundle inputs contain duplicate target paths")
    return sorted(common, key=lambda item: item.target.as_posix())


def build(args: argparse.Namespace) -> dict[str, object]:
    if not VERSION_RE.fullmatch(args.version):
        raise ValueError("--version must be a stable x.y.z version")
    if not args.output.is_absolute():
        raise ValueError("--output must be absolute and outside the source checkout")
    repository_root = Path(__file__).resolve().parents[1]
    if args.output.resolve(strict=False).is_relative_to(repository_root):
        raise ValueError("--output must remain outside the source checkout")
    if args.output.exists() or args.output.is_symlink():
        raise ValueError("--output must not already exist")
    args.output.mkdir(mode=0o750, parents=False)
    try:
        files = [copy_one(item, args.output) for item in inputs(args)]
        manifest = {
            "format_version": 1,
            "application_version": args.version,
            "profile": args.profile,
            "files": files,
        }
        canonical_manifest = json.dumps(
            manifest, ensure_ascii=False, separators=(",", ":")
        ).encode("utf-8")
        manifest_object_digest = f"sha256:{hashlib.sha256(canonical_manifest).hexdigest()}"
        manifest_bytes = json.dumps(
            manifest, ensure_ascii=False, indent=2, separators=(",", ": ")
        ).encode("utf-8") + b"\n"
        manifest_path = args.output / "bundle-manifest.json"
        with manifest_path.open("xb") as handle:
            handle.write(manifest_bytes)
            handle.flush()
            os.fsync(handle.fileno())
        manifest_path.chmod(0o640)
        _, manifest_file_digest = digest(manifest_path)
        return {
            "bundle_root": str(args.output),
            "profile": args.profile,
            "application_version": args.version,
            "file_count": len(files),
            "manifest_object_digest": manifest_object_digest,
            "manifest_file_digest": manifest_file_digest,
            "note": "pin manifest_object_digest in signed metadata and pass it to muriarcctl",
        }
    except Exception:
        shutil.rmtree(args.output, ignore_errors=True)
        raise


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--profile", required=True, choices=("native-system", "managed-compose"))
    result.add_argument("--version", required=True)
    result.add_argument("--output", required=True, type=Path)
    result.add_argument("--controller", required=True, type=Path)
    result.add_argument("--upgrade-executor", required=True, type=Path)
    result.add_argument("--verifier", required=True, type=Path)
    result.add_argument("--deploy-root", type=Path, default=Path("deploy"))
    result.add_argument("--server", type=Path)
    result.add_argument("--ui-dir", type=Path)
    return result


def main() -> int:
    try:
        print(json.dumps(build(parser().parse_args()), ensure_ascii=False, indent=2))
        return 0
    except (OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

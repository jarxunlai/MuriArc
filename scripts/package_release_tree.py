#!/usr/bin/env python3
"""Create a deterministic gzip-compressed tar from a closed release tree."""

from __future__ import annotations

import argparse
import gzip
import os
import shutil
import stat
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


class PackageError(ValueError):
    pass


def inventory(root: Path) -> list[Path]:
    metadata = root.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise PackageError("input root must be a real non-symlink directory")
    files: list[Path] = []
    pending = [root]
    while pending:
        directory = pending.pop()
        for child in sorted(directory.iterdir(), key=lambda value: value.name, reverse=True):
            metadata = child.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                raise PackageError(f"release tree contains a symlink: {child}")
            if stat.S_ISDIR(metadata.st_mode):
                pending.append(child)
            elif stat.S_ISREG(metadata.st_mode):
                if metadata.st_size <= 0:
                    raise PackageError(f"release tree contains an empty file: {child}")
                files.append(child)
            else:
                raise PackageError(f"release tree contains a special file: {child}")
    return sorted(files, key=lambda value: value.relative_to(root).as_posix())


def archive_name(prefix: str, relative: Path) -> str:
    prefix_path = PurePosixPath(prefix)
    if (
        prefix_path.is_absolute()
        or not prefix
        or any(part in ("", ".", "..") for part in prefix_path.parts)
        or "\\" in prefix
    ):
        raise PackageError("--prefix must be one safe relative directory name")
    relative_path = PurePosixPath(relative.as_posix())
    if any(part in ("", ".", "..") for part in relative_path.parts):
        raise PackageError("release tree contains an unsafe path")
    return str(prefix_path / relative_path)


def package(root: Path, output: Path, prefix: str, epoch: int) -> None:
    root = root.resolve(strict=True)
    if epoch < 0:
        raise PackageError("--source-date-epoch must be non-negative")
    if not output.is_absolute():
        raise PackageError("--output must be absolute")
    if output.exists() or output.is_symlink():
        raise PackageError("--output must not already exist")
    output.parent.mkdir(parents=True, exist_ok=True)
    files = inventory(root)
    if not files:
        raise PackageError("release tree contains no files")
    with tempfile.TemporaryDirectory(dir=output.parent) as raw:
        temporary_tar = Path(raw) / "release.tar"
        with tarfile.open(temporary_tar, "x", format=tarfile.PAX_FORMAT) as archive:
            for path in files:
                relative = path.relative_to(root)
                info = archive.gettarinfo(str(path), arcname=archive_name(prefix, relative))
                info.uid = 0
                info.gid = 0
                info.uname = "root"
                info.gname = "root"
                info.mtime = epoch
                info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                info.pax_headers = {}
                with path.open("rb") as handle:
                    archive.addfile(info, handle)
        temporary_output = output.with_name(f".{output.name}.tmp-{os.getpid()}")
        try:
            with temporary_tar.open("rb") as source, temporary_output.open("xb") as raw_target:
                with gzip.GzipFile(
                    filename="",
                    mode="wb",
                    fileobj=raw_target,
                    mtime=epoch,
                    compresslevel=9,
                ) as target:
                    shutil.copyfileobj(source, target, 1024 * 1024)
                raw_target.flush()
                os.fsync(raw_target.fileno())
            os.replace(temporary_output, output)
        except Exception:
            temporary_output.unlink(missing_ok=True)
            raise
    output.chmod(0o600)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--root", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    result.add_argument("--prefix", required=True)
    result.add_argument("--source-date-epoch", type=int, required=True)
    return result


def main() -> int:
    try:
        args = parser().parse_args()
        package(args.root, args.output, args.prefix, args.source_date_epoch)
    except (OSError, PackageError) as exc:
        print(f"release tree packaging failed: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

from __future__ import annotations

import sys
import tarfile
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import package_release_tree as package  # noqa: E402


class PackageReleaseTreeTests(unittest.TestCase):
    def test_archive_is_deterministic_and_normalized(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            tree = root / "tree"
            tree.mkdir()
            (tree / "b.txt").write_text("b\n", encoding="utf-8")
            executable = tree / "bin"
            executable.write_text("#!/bin/sh\n", encoding="utf-8")
            executable.chmod(0o755)
            first = root / "one.tar.gz"
            second = root / "two.tar.gz"
            package.package(tree, first, "MuriArc-1.0.0", 42)
            package.package(tree, second, "MuriArc-1.0.0", 42)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with tarfile.open(first, "r:gz") as archive:
                members = archive.getmembers()
            self.assertEqual(
                [member.name for member in members],
                ["MuriArc-1.0.0/b.txt", "MuriArc-1.0.0/bin"],
            )
            self.assertTrue(all(member.uid == 0 and member.gid == 0 for member in members))
            self.assertTrue(all(member.mtime == 42 for member in members))
            self.assertEqual(members[1].mode, 0o755)

    def test_symlink_and_empty_file_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            tree = root / "tree"
            tree.mkdir()
            empty = tree / "empty"
            empty.touch()
            with self.assertRaises(package.PackageError):
                package.package(tree, root / "empty.tar.gz", "MuriArc", 1)
            empty.unlink()
            target = tree / "target"
            target.write_text("x", encoding="utf-8")
            (tree / "link").symlink_to(target)
            with self.assertRaises(package.PackageError):
                package.package(tree, root / "link.tar.gz", "MuriArc", 1)


if __name__ == "__main__":
    unittest.main()

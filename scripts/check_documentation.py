#!/usr/bin/env python3
"""Validate MuriArc bilingual public documentation and local Markdown links."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
PAIR_STEMS = (
    "README",
    "docs/README",
    "docs/ARCHITECTURE",
    "docs/SECURITY",
    "docs/ENVIRONMENTS",
    "docs/DEPLOYMENT",
    "docs/DESKTOP_DELIVERY",
    "docs/SERVER_DELIVERY",
    "docs/MIGRATION",
    "docs/UPGRADE_ENGINE",
    "docs/UPGRADE_COMPATIBILITY",
    "docs/CLOUDFLARE_PUBLIC_PROFILE",
    "docs/DELIVERY_ACCEPTANCE",
)
BANNED_TERMS = (
    "Muris" + "Pro",
    "Lantern" + "X",
    "lanternx" + "/animal_lab",
    "muriarc-legacy" + "-migrator",
    "source" + "Notice",
)
SKIP_DIRS = {".git", ".codegraph", "node_modules", "target", "dist", "playwright-report", "test-results"}
LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
CJK_RE = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff]")


def fail(message: str, failures: list[str]) -> None:
    failures.append(message)


def public_markdown() -> list[Path]:
    output = subprocess.check_output(
        ["git", "-C", str(ROOT), "ls-files", "-z", "*.md"],
        text=True,
    )
    paths = [ROOT / rel for rel in output.split("\0") if rel]
    return [p for p in paths if p.is_file() and not any(part in SKIP_DIRS for part in p.parts)]


def check_pairs(failures: list[str]) -> None:
    for stem in PAIR_STEMS:
        en = ROOT / f"{stem}.md"
        cn = ROOT / f"{stem}_cn.md"
        if not en.is_file():
            fail(f"missing English document: {en.relative_to(ROOT)}", failures)
            continue
        if not cn.is_file():
            fail(f"missing Chinese document: {cn.relative_to(ROOT)}", failures)
            continue

        en_text = en.read_text(encoding="utf-8-sig")
        cn_text = cn.read_text(encoding="utf-8-sig")
        en_top = "\n".join(en_text.splitlines()[:8])
        cn_top = "\n".join(cn_text.splitlines()[:8])
        if cn.name not in en_top or "简体中文" not in en_top:
            fail(f"English language switch is missing or late: {en.relative_to(ROOT)}", failures)
        if en.name not in cn_top or "English" not in cn_top:
            fail(f"Chinese language switch is missing or late: {cn.relative_to(ROOT)}", failures)

        english_without_switch = en_text.replace("简体中文", "")
        if CJK_RE.search(english_without_switch):
            fail(f"English document contains unlocalized CJK text: {en.relative_to(ROOT)}", failures)
        if len(CJK_RE.findall(cn_text)) < 40:
            fail(f"Chinese document has insufficient Chinese content: {cn.relative_to(ROOT)}", failures)

    for adr in (ROOT / "docs" / "adr").glob("*.md"):
        if not adr.name.endswith("_cn.md"):
            fail(f"Chinese ADR must use _cn.md: {adr.relative_to(ROOT)}", failures)
    if (ROOT / "docs" / "RELEASE_EVIDENCE.md").exists():
        fail("internal Chinese release evidence must use RELEASE_EVIDENCE_cn.md", failures)


def split_link_target(raw: str) -> str:
    raw = raw.strip()
    if raw.startswith("<") and ">" in raw:
        raw = raw[1 : raw.index(">")]
    elif " \"" in raw:
        raw = raw.split(" \"", 1)[0]
    elif " '" in raw:
        raw = raw.split(" '", 1)[0]
    return unquote(raw.split("#", 1)[0].split("?", 1)[0])


def check_links(paths: list[Path], failures: list[str]) -> None:
    for source in paths:
        text = source.read_text(encoding="utf-8-sig")
        for match in LINK_RE.finditer(text):
            raw = match.group(1).strip()
            lower = raw.lower()
            if (
                not raw
                or raw in {"...", "<...>"}
                or raw.startswith("#")
                or lower.startswith(("http://", "https://", "mailto:", "data:"))
            ):
                continue
            target_text = split_link_target(raw)
            if not target_text:
                continue
            target = (source.parent / target_text).resolve()
            try:
                target.relative_to(ROOT.resolve())
            except ValueError:
                fail(f"local link escapes repository: {source.relative_to(ROOT)} -> {raw}", failures)
                continue
            if not target.exists():
                fail(f"broken local link: {source.relative_to(ROOT)} -> {raw}", failures)


def check_status_and_attribution(failures: list[str]) -> None:
    for rel in ("README.md", "README_cn.md"):
        text = (ROOT / rel).read_text(encoding="utf-8-sig")
        for token in ("0.1.0", "preview_epoch_0", "1.0.0", "E0001"):
            if token not in text:
                fail(f"release status token {token!r} missing from {rel}", failures)
        if "RC" not in text or not any(word in text.lower() for word in ("not", "no official", "尚未")):
            fail(f"{rel} must explicitly state that formal RC has not passed", failures)

    text_suffixes = {".md", ".json", ".toml", ".yml", ".yaml", ".ts", ".vue", ".py"}
    for path in ROOT.rglob("*"):
        if not path.is_file() or path.suffix not in text_suffixes or any(part in SKIP_DIRS for part in path.parts):
            continue
        text = path.read_text(encoding="utf-8-sig", errors="replace")
        for term in BANNED_TERMS:
            if term in text:
                fail(f"obsolete source/interface term {term!r}: {path.relative_to(ROOT)}", failures)


def main() -> int:
    failures: list[str] = []
    paths = public_markdown()
    check_pairs(failures)
    check_links(paths, failures)
    check_status_and_attribution(failures)
    if failures:
        for item in failures:
            print(f"documentation error: {item}", file=sys.stderr)
        return 1
    print(f"Documentation contracts OK: {len(PAIR_STEMS)} bilingual pairs, {len(paths)} Markdown files.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

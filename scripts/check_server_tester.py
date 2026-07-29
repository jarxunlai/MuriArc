#!/usr/bin/env python3
"""Static and artifact security contracts for the Server Docker Tester."""

from __future__ import annotations

import argparse
import json
import re
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEPLOY = ROOT / "deploy/server-tester"
HEX64 = r"[0-9a-f]{64}"
SECRET_PATTERNS = {
    "private-key": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "github-token": re.compile(rb"gh[pousr]_[A-Za-z0-9_]{20,}"),
    "openai-key": re.compile(rb"sk-[A-Za-z0-9_-]{20,}"),
    "jwt": re.compile(rb"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"),
}


def require(text: str, tokens: list[str], label: str, errors: list[str]) -> None:
    for token in tokens:
        if token not in text:
            errors.append(f"{label}: missing {token!r}")


def forbid(text: str, tokens: list[str], label: str, errors: list[str]) -> None:
    lowered = text.lower()
    for token in tokens:
        if token.lower() in lowered:
            errors.append(f"{label}: forbidden {token!r}")


def static_errors() -> list[str]:
    errors: list[str] = []
    dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
    compose = (DEPLOY / "compose.yaml.in").read_text(encoding="utf-8")
    shell = (DEPLOY / "muriarc-tester.sh").read_text(encoding="utf-8")
    bootstrap = (DEPLOY / "compose.bootstrap.yaml").read_text(encoding="utf-8")
    powershell = (DEPLOY / "muriarc-tester.ps1").read_text(encoding="utf-8")
    env_empty = (DEPLOY / ".env.empty.example.in").read_text(encoding="utf-8")
    env_demo = (DEPLOY / ".env.demo.example.in").read_text(encoding="utf-8")

    require(dockerfile, [
        "FROM server-build AS tester-build",
        "--target x86_64-unknown-linux-musl",
        "FROM alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce AS tester-runtime",
        "-p muriarc-standard-fixture",
        "muriarc-standard-fixture/postgres",
        "/usr/local/bin/muriarc-standard-fixture",
        'CMD ["wget", "-q", "-T", "5", "-O", "/dev/null", "http://127.0.0.1:8787/readyz"]',
        "org.opencontainers.image.source",
        "org.opencontainers.image.revision",
        "org.opencontainers.image.licenses",
    ], "Dockerfile", errors)
    require(compose, [
        "@@SERVER_IMAGE@@", "@@POSTGRES_IMAGE@@", "platform: linux/amd64",
        '"127.0.0.1:${MURIARC_TESTER_SERVER_PORT:-8787}:8787"',
        "profiles: [demo-tools]", "MURIARC_PREVIEW_BOOTSTRAP: \"false\"",
        "read_only: true", "cap_drop: [ALL]", "no-new-privileges:true",
        'test: ["CMD", "wget", "-q", "-T", "5", "-O", "/dev/null", "http://127.0.0.1:8787/readyz"]',
    ], "Tester Compose", errors)
    forbid(compose, [
        ":latest", "5432:5432", "/var/run/docker.sock", "0.0.0.0:${MURIARC_TESTER_SERVER_PORT",
        "MURIARC_AI_MASTER_KEY:", "MURIARC_BOOTSTRAP_TOKEN: ${",
    ], "Tester Compose", errors)
    for label, text in (("Bash", shell), ("PowerShell", powershell)):
        require(text, ["verify", "init-empty", "init-demo", "up", "status", "logs", "down"], label, errors)
        forbid(text, ["destroy", "down --volumes", "down', '--volumes", "docker volume rm"], label, errors)
        require(text, ["volume already exists", "volumes are preserved"], label, errors)
    require(shell, ["verify-postgres", "deployment-generation.json"], "Bash", errors)
    require(bootstrap, ["MURIARC_PREVIEW_BOOTSTRAP: \"true\""], "bootstrap override", errors)
    require(powershell, ["MURIARC_LAB_ID", "verify-postgres", "deployment-generation.json"], "PowerShell", errors)
    for label, text in (("empty env", env_empty), ("demo env", env_demo)):
        forbid(text, ["MURIARC_AI_MASTER_KEY=", "MURIARC_BOOTSTRAP_TOKEN=", "MURIARC_BOOTSTRAP_MCP_TOKEN="], label, errors)
        require(text, ["REPLACE_WITH_ROOT_EMAIL", "REPLACE_WITH_LONG_UNIQUE_ROOT_PASSWORD"], label, errors)
    require(env_demo, [
        "4d555249-4152-4300-0000-000000000001",
        "4d555249-4152-4300-0000-000000000002",
        "MURIARC_TESTER_DATASET_MODE=demo",
    ], "demo env", errors)
    return errors


def scan_zip(path: Path) -> dict[str, object]:
    matches: list[dict[str, str]] = []
    unsafe: list[str] = []
    duplicates: list[str] = []
    with zipfile.ZipFile(path) as archive:
        seen: set[str] = set()
        for info in archive.infolist():
            name = info.filename
            if name in seen:
                duplicates.append(name)
            seen.add(name)
            parts = Path(name).parts
            unix_type = (info.external_attr >> 16) & 0o170000
            if name.startswith("/") or ".." in parts or "\\" in name or unix_type not in (0, 0o100000):
                unsafe.append(name)
            data = archive.read(info)
            for label, pattern in SECRET_PATTERNS.items():
                if pattern.search(data):
                    matches.append({"file": name, "pattern": label})
    return {
        "status": "PASS" if not (matches or unsafe or duplicates) else "FAIL",
        "sensitiveMatches": matches,
        "unsafeEntries": unsafe,
        "duplicateEntries": duplicates,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--report", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    errors = static_errors()
    report: dict[str, object] = {
        "schemaVersion": 1,
        "status": "PASS" if not errors else "FAIL",
        "staticErrors": errors,
    }
    if args.bundle:
        artifact = scan_zip(args.bundle)
        report["artifact"] = artifact
        if artifact["status"] != "PASS":
            errors.append("bundle archive security scan failed")
            report["status"] = "FAIL"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for error in errors:
        print(f"server tester error: {error}", file=sys.stderr)
    if errors:
        return 2
    print("Server Tester contracts OK; sensitive information matches: 0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

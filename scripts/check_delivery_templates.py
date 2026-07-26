#!/usr/bin/env python3
"""Fail closed when formal Server delivery templates lose their security boundary."""

from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def require(text: str, tokens: list[str], label: str) -> list[str]:
    return [f"{label}: missing required token {token!r}" for token in tokens if token not in text]


def forbid(text: str, tokens: list[str], label: str) -> list[str]:
    lowered = text.lower()
    return [f"{label}: contains forbidden token {token!r}" for token in tokens if token.lower() in lowered]


def check() -> list[str]:
    compose = (ROOT / "deploy/managed-compose/compose.yaml").read_text(encoding="utf-8")
    unit = (ROOT / "deploy/native-system/muriarc.service").read_text(encoding="utf-8")
    dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
    errors: list[str] = []
    errors += forbid(
        compose,
        ["build:", ":latest", "watchtower", "/var/run/docker.sock", "0.0.0.0:8787:8787", "5432:5432"],
        "managed Compose",
    )
    errors += require(
        compose,
        [
            "127.0.0.1:8787:8787",
            "MURIARC_SERVER_IMAGE",
            "MURIARC_POSTGRES_IMAGE",
            "MURIARC_ACTIVE_GENERATION",
            "muriarc-upgrade-executor",
            "profiles: [control]",
            "read_only: true",
            "cap_drop: [ALL]",
            "no-new-privileges:true",
        ],
        "managed Compose",
    )
    errors += require(
        unit,
        [
            "User=muriarc",
            "Group=muriarc",
            "EnvironmentFile=/etc/muriarc/server.env",
            "EnvironmentFile=/var/lib/muriarc/control/active.env",
            "ExecStart=/opt/muriarc/current/bin/muriarc-server",
            "NoNewPrivileges=yes",
            "ProtectSystem=strict",
            "ReadWritePaths=/var/lib/muriarc",
            "CapabilityBoundingSet=",
        ],
        "native systemd",
    )
    errors += forbid(unit, ["muriarcctl", "docker.sock", "AmbientCapabilities=CAP_"], "native systemd")
    errors += require(
        dockerfile,
        [
            "-p muriarc-upgrade-executor",
            "-p muriarc-verifier",
            "-p muriarcctl",
            "USER 10001:10001",
            'ENTRYPOINT ["/usr/local/bin/muriarc-server"]',
        ],
        "Dockerfile",
    )
    return errors


def main() -> int:
    errors = check()
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 2
    print("delivery templates: native-system and managed-compose policies verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Fail when generated product metadata drifts from branding/brand.json."""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def load_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def main() -> int:
    brand = load_json(ROOT / "branding" / "brand.json")
    tauri = load_json(ROOT / "src-tauri" / "tauri.conf.json")
    package = load_json(ROOT / "ui" / "package.json")
    manifest = load_json(ROOT / "ui" / "public" / "manifest.webmanifest")
    html = (ROOT / "ui" / "index.html").read_text(encoding="utf-8-sig")
    tokens = (ROOT / "ui" / "src" / "styles" / "tokens.css").read_text(encoding="utf-8-sig")
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8-sig")
    compose = (ROOT / "docker-compose.yml").read_text(encoding="utf-8-sig")
    rust_app = (ROOT / "src-tauri" / "src" / "lib.rs").read_text(encoding="utf-8-sig")
    settings = (ROOT / "ui" / "src" / "views" / "SettingsView.vue").read_text(encoding="utf-8-sig")
    desktop_settings = (ROOT / "src-tauri" / "src" / "settings.rs").read_text(encoding="utf-8-sig")
    desktop_build = (ROOT / "src-tauri" / "build.rs").read_text(encoding="utf-8-sig")

    failures: list[str] = []
    product_name = str(brand["productName"])
    short_name = str(brand["shortName"])
    tagline = str(brand["tagline"])
    identifier = str(brand["bundleIdentifier"])
    version = str(brand["version"])
    primary_color = str(brand["primaryColor"]).lower()
    master_hash = str(brand["logoMasterSha256"]).lower()

    require(bool(product_name.strip()), "branding.productName is empty", failures)
    require(bool(str(brand.get("releaseStage", "")).strip()), "branding.releaseStage is empty", failures)
    require(tauri.get("productName") == product_name, "Tauri productName drifted", failures)
    require(tauri.get("mainBinaryName") == product_name, "Tauri mainBinaryName drifted", failures)
    require(tauri.get("identifier") == identifier, "Tauri bundle identifier drifted", failures)
    require(tauri.get("version") == version, "Tauri version drifted", failures)
    windows = tauri.get("app", {}).get("windows", [])  # type: ignore[union-attr]
    require(bool(windows) and windows[0].get("title") == product_name, "Tauri window title drifted", failures)
    require(package.get("version") == version, "UI package version drifted", failures)

    workspace_version = re.search(
        r'\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"',
        cargo,
        flags=re.MULTILINE | re.DOTALL,
    )
    require(bool(workspace_version) and workspace_version.group(1) == version, "Cargo workspace version drifted", failures)
    require(f"image: muriarc/server:{version}" in compose, "Compose image version drifted", failures)

    require(f"<title>{product_name}</title>" in html, "Web title drifted", failures)
    require(
        f'<meta name="theme-color" content="{primary_color}"' in html.lower(),
        "Web theme color drifted",
        failures,
    )
    require(f"--muri-primary: {primary_color};" in tokens.lower(), "CSS primary token drifted", failures)
    require(manifest.get("name") == product_name, "PWA name drifted", failures)
    require(manifest.get("short_name") == short_name, "PWA short name drifted", failures)
    require(manifest.get("description") == tagline, "PWA tagline drifted", failures)
    require(str(manifest.get("theme_color", "")).lower() == primary_color, "PWA theme color drifted", failures)
    require('env!("MURIARC_PRODUCT_NAME")' in rust_app, "Rust desktop name is not injected from branding", failures)
    require("MURIARC_BUNDLE_IDENTIFIER" in desktop_build, "Rust build does not inject bundle identifier", failures)
    require("MURIARC_BUNDLE_IDENTIFIER" in desktop_settings, "Desktop keyring identity drifted from branding", failures)
    require("Rewrite scaffold" not in settings, "About page still exposes rewrite scaffold text", failures)

    master = ROOT / "branding" / "logo-master.png"
    mark = ROOT / "branding" / "logo-mark.png"
    public_mark = ROOT / "ui" / "public" / "logo-mark.png"
    require(master.is_file() and digest(master) == master_hash, "Logo master hash drifted", failures)
    require(mark.is_file() and public_mark.is_file() and digest(mark) == digest(public_mark), "Web logo mark drifted", failures)

    required_assets = [
        mark,
        ROOT / "ui" / "public" / "favicon-32.png",
        ROOT / "ui" / "public" / "apple-touch-icon.png",
        ROOT / "ui" / "public" / "pwa-192.png",
        ROOT / "ui" / "public" / "pwa-512.png",
        ROOT / "src-tauri" / "icons" / "icon.ico",
        ROOT / "src-tauri" / "icons" / "icon.icns",
    ]
    for asset in required_assets:
        require(asset.is_file() and asset.stat().st_size > 0, f"Missing brand asset: {asset.relative_to(ROOT)}", failures)

    if failures:
        for failure in failures:
            print(f"branding error: {failure}", file=sys.stderr)
        return 1
    print(f"Branding metadata is consistent for {product_name} {version} ({identifier}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

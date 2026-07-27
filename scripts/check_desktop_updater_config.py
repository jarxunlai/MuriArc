#!/usr/bin/env python3
"""Fail closed when the Desktop updater safety wiring drifts."""

from __future__ import annotations

import json
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    raise SystemExit(f"desktop updater policy failed: {message}")


def main() -> int:
    manifest = tomllib.loads((ROOT / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8"))
    updater = manifest.get("dependencies", {}).get("tauri-plugin-updater")
    if updater != "2.10.1":
        fail("tauri-plugin-updater must stay pinned to 2.10.1")

    config = json.loads((ROOT / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
    if config.get("bundle", {}).get("createUpdaterArtifacts") is not True:
        fail("bundle.createUpdaterArtifacts must be true")
    updater_config = config.get("plugins", {}).get("updater", {})
    endpoints = updater_config.get("endpoints", [])
    if not endpoints or any(not endpoint.startswith("https://") for endpoint in endpoints):
        fail("every updater endpoint must use HTTPS")
    if any(key.startswith("dangerous") and value for key, value in updater_config.items()):
        fail("dangerous updater transport settings are forbidden")

    capability = json.loads(
        (ROOT / "src-tauri" / "capabilities" / "default.json").read_text(encoding="utf-8")
    )
    permissions = capability.get("permissions", [])
    if any(str(permission).startswith("updater:") for permission in permissions):
        fail("the WebView must not receive direct updater permissions")

    build_rs = (ROOT / "src-tauri" / "build.rs").read_text(encoding="utf-8")
    for required in [
        'profile == "release"',
        "MURIARC_DESKTOP_UPDATER_PUBLIC_KEY is required",
        "PublicKey::decode",
    ]:
        if required not in build_rs:
            fail(f"build.rs is missing release gate: {required}")

    lib_rs = (ROOT / "src-tauri" / "src" / "lib.rs").read_text(encoding="utf-8")
    for required in [
        "check_desktop_update",
        "apply_desktop_update",
        "schedule_verified_update",
        "resume_pending_upgrade",
        "delegate_to_binary_fallback",
        "activate_binary_fallback_after_failure",
        "confirm_verified_recovery",
    ]:
        if required not in lib_rs:
            fail(f"Desktop updater wiring is missing: {required}")
    if "download_and_install" in lib_rs:
        fail("download_and_install bypasses the verified intent boundary")
    resume = lib_rs.find("desktop_upgrade::resume_pending_upgrade")
    storage = lib_rs.find("StorageRootState::initialize", resume)
    if resume < 0 or storage < 0 or resume > storage:
        fail("pending upgrade must resume before StorageRootState opens the active data root")

    desktop_driver = (ROOT / "src-tauri" / "src" / "desktop_upgrade.rs").read_text(
        encoding="utf-8"
    )
    for required in [
        "impl UpgradeDriver for DesktopUpgradeDriver",
        "UpgradeEngine::new",
        "run_with_operation_id",
        "checkpoint_and_verify_database",
        "prepare_verified_copy",
        "verify_continue_write_with_rollback",
        "verify_attachment_objects",
        "open_candidate_write_lease",
        "activate_root_for_upgrade",
        "FirstWriteBlocksRollback",
        "stage_binary_recovery",
        "validate_recovery_executable",
        "BinaryFallbackState::Fallback",
        "muriarc_release_manifest_signature",
        "verify_manifest_signature",
        "validate_intent_binding",
    ]:
        if required not in desktop_driver:
            fail(f"Desktop UpgradeDriver is missing: {required}")

    settings = (ROOT / "ui" / "src" / "views" / "SettingsView.vue").read_text(
        encoding="utf-8"
    )
    for required in [
        "confirmedDesktopRecovery",
        "maintenanceClass",
        "dataRequiredBytes",
        "首次新写入后禁止自动降级",
    ]:
        if required not in settings:
            fail(f"Desktop update confirmation UI is missing: {required}")

    print("desktop updater policy: PASS")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"desktop updater policy failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error

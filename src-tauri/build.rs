use std::{env, fs, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use minisign_verify::PublicKey;

const UNCONFIGURED_UPDATER_KEY: &str = "MURIARC_DESKTOP_UPDATER_PUBLIC_KEY_NOT_CONFIGURED";

fn main() {
    println!("cargo:rerun-if-env-changed=MURIARC_DESKTOP_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=PROFILE");
    let brand_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../branding/brand.json");
    println!("cargo:rerun-if-changed={}", brand_path.display());
    let brand: serde_json::Value =
        serde_json::from_slice(&fs::read(&brand_path).expect("failed to read branding/brand.json"))
            .expect("branding/brand.json must be valid JSON");
    let product_name = brand
        .get("productName")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .expect("branding.productName must be a non-empty string");
    let bundle_identifier = brand
        .get("bundleIdentifier")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .expect("branding.bundleIdentifier must be a non-empty string");
    println!("cargo:rustc-env=MURIARC_PRODUCT_NAME={product_name}");
    println!("cargo:rustc-env=MURIARC_BUNDLE_IDENTIFIER={bundle_identifier}");
    let profile = env::var("PROFILE").unwrap_or_default();
    let updater_key = env::var("MURIARC_DESKTOP_UPDATER_PUBLIC_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if profile == "release" && updater_key.is_none() {
        panic!("MURIARC_DESKTOP_UPDATER_PUBLIC_KEY is required for every release Desktop build");
    }
    let updater_key = updater_key.unwrap_or_else(|| UNCONFIGURED_UPDATER_KEY.to_owned());
    if updater_key != UNCONFIGURED_UPDATER_KEY {
        let decoded = STANDARD
            .decode(updater_key.as_bytes())
            .unwrap_or_else(|_| panic!("MURIARC_DESKTOP_UPDATER_PUBLIC_KEY must use Base64"));
        let decoded = std::str::from_utf8(&decoded).unwrap_or_else(|_| {
            panic!("MURIARC_DESKTOP_UPDATER_PUBLIC_KEY must wrap UTF-8 Minisign public-key text")
        });
        PublicKey::decode(decoded).unwrap_or_else(|_| {
            panic!("MURIARC_DESKTOP_UPDATER_PUBLIC_KEY is not a valid Tauri Minisign public key")
        });
    }
    println!("cargo:rustc-env=MURIARC_DESKTOP_UPDATER_PUBLIC_KEY={updater_key}");
    tauri_build::build()
}

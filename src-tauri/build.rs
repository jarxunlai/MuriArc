use std::{fs, path::PathBuf};

fn main() {
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
    tauri_build::build()
}

# Environments

## Rust workspace

- purpose: Domain, SQLite/PostgreSQL adapters, Axum Server, AI safety layer and Tauri backend
- runtime: Rust stable, minimum supported Rust 1.88
- state: active
- manifest: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`
- check: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
- used_by: Desktop and Server

## Vue UI

- purpose: Shared responsive Desktop/Web interface
- runtime: Node.js >=22.13 and pnpm 11.5.0 (managed through Corepack)
- state: active
- manifest: `ui/package.json`, `ui/pnpm-lock.yaml`, `ui/pnpm-workspace.yaml`
- check: `pnpm --dir ui run test && pnpm --dir ui run build`
- used_by: Tauri WebView and Axum-hosted Web

## Legacy Python/Vue application

- purpose: Behaviour and migration reference only
- runtime: Frozen historical environment
- state: legacy
- manifest: Preserved in Git history and the archive branch
- check: Not part of MuriArc CI
- used_by: Migration acceptance only

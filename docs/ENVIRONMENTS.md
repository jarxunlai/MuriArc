# Environments

> English | [简体中文](ENVIRONMENTS_cn.md)

## Supported toolchain

| Area | Required runtime | Primary checks |
| --- | --- | --- |
| Rust workspace | Rust `1.88` | format, locked Clippy, locked tests |
| Vue UI | Node.js `>=22.13`, pnpm `11.5.0` | unit tests, typecheck, remote/local builds, Playwright |
| PostgreSQL Store | PostgreSQL `17` | fresh, idempotent, incremental, contract, and fencing tests |
| Desktop | Windows WebView2 + Tauri prerequisites | Windows strict Clippy/tests and no-bundle smoke build |
| Documentation | Python 3 | bilingual naming, status, and local-link checks |

## Worktree isolation

Run build and tests from the absolute path of the worktree whose source is being validated. Do not validate one branch from the main checkout. Each worktree owns its `ui/node_modules`, `.env`, SQLite files, attachment roots, Playwright output, and runtime services.

Cargo registry and pnpm store may be machine-shared. Cargo build output is a rebuildable cache and may use the repository-external shared target when jobs are serial:

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
command -v cargo >/dev/null || exit 1

if [ -d /mnt/e/Muriarc ]; then
  export CARGO_TARGET_DIR=/mnt/e/Muriarc/builds/cargo-target/shared
else
  export CARGO_TARGET_DIR="$HOME/.cache/muriarc-cargo-target/shared"
fi
mkdir -p "$CARGO_TARGET_DIR"
```

Do not run concurrent writers against the shared target, run a bare cached binary as branch evidence, or use `cargo clean` without owner authorization.

## Rust verification

```bash
cargo fmt --all -- --check
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
```

On WSL/Linux without GTK/WebKit development packages, Desktop strict Clippy/tests run in the fixed Tauri Linux tool image or the Windows CI job. A Linux result does not replace Windows WebView acceptance.

## PostgreSQL verification

Set `MURIARC_TEST_DATABASE_URL` to a disposable PostgreSQL 17 instance that is not shared with a manual Server, another worktree, or real data. The test role must be able to create/drop isolated test databases. After tests, verify there are no leftover test databases. Missing configuration that causes a PostgreSQL suite to skip is not a pass.

Never point automated migration tests at an acceptance or production volume.

## UI verification

```bash
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install --frozen-lockfile
pnpm --dir ui audit --audit-level=high
pnpm --dir ui run test
pnpm --dir ui run typecheck
VITE_MURIARC_GATEWAY=remote pnpm --dir ui run build
VITE_MURIARC_GATEWAY=local pnpm --dir ui run build
pnpm --dir ui run test:e2e
```

Playwright covers Desktop, Tablet, and Mobile projects. A case that is explicitly inapplicable to a device may skip; missing browser/runtime dependencies that prevent the suite from running may not be reported as a pass.

## Provider and service testing

Default tests use in-process or local mock upstreams. They must not call a real Provider or read a personal API key. A reusable manual Server service must use the repository-external acceptance kit and worktree-specific runtime. Dirty-worktree service builds are development harnesses, not formal acceptance.

## Generated and sensitive data

Cargo targets, `node_modules`, build output, test reports, databases, attachments, environment files, credentials, private keys, and acceptance evidence remain outside Git. Only small synthetic fixtures and immutable public definitions belong in the repository.

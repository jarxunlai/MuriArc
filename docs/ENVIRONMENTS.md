# Environments

## Rust workspace

- purpose: Domain, SQLite/PostgreSQL adapters, Axum Server, AI safety layer and Tauri backend
- runtime: Rust stable, minimum supported Rust 1.88
- state: active
- manifest: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`
- check: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-targets --all-features`
- used_by: Desktop and Server

每个 feature 必须在它自己的 worktree 源码目录内验证，Cargo 产物写到仓库外并按分支隔离：

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
branch_slug="$(git branch --show-current | sed 's#[/ ]#-#g')"
if [ -d /mnt/e/Muriarc ]; then
  export CARGO_TARGET_DIR="/mnt/e/Muriarc/builds/cargo-target/${branch_slug:-detached}"
else
  export CARGO_TARGET_DIR="$HOME/.cache/muriarc-cargo-target/${branch_slug:-detached}"
fi
mkdir -p "$CARGO_TARGET_DIR"

cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
```

PostgreSQL Store 和迁移测试必须设置指向一次性 PostgreSQL 17 数据库的
`MURIARC_TEST_DATABASE_URL`，并验证空库、重复应用和旧 schema 增量升级。测试因变量缺失而
跳过不构成通过；测试库、附件目录和端口不得与主工作树或其他 worktree 共用。

## Vue UI

- purpose: Shared responsive Desktop/Web interface
- runtime: Node.js >=22.13 and pnpm 11.5.0 (managed through Corepack)
- state: active
- manifest: `ui/package.json`, `ui/pnpm-lock.yaml`, `ui/pnpm-workspace.yaml`
- check: `pnpm --dir ui run test && pnpm --dir ui run typecheck`，并分别设置
  `VITE_MURIARC_GATEWAY=remote` / `local` 执行生产构建，最后运行 `pnpm --dir ui run test:e2e`
- used_by: Tauri WebView and Axum-hosted Web

`ui/node_modules` 属于当前 worktree，不链接或复制其他工作树的目录：

```bash
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install --frozen-lockfile
pnpm --dir ui run test
pnpm --dir ui run typecheck
VITE_MURIARC_GATEWAY=remote pnpm --dir ui run build
VITE_MURIARC_GATEWAY=local pnpm --dir ui run build
pnpm --dir ui run test:e2e
```

Playwright 必须覆盖 Desktop、Tablet 和 Mobile 项目；条件不适用而明确标记的 skip 可以记录，
浏览器或依赖未安装导致整套未执行则不能作为通过。

## Provider and desktop validation

- 三种 Provider 协议只通过进程内或本机 Mock upstream 验证标准端点、headers、请求、响应、
  usage 和错误映射。CI 与默认交付验收不调用真实厂商 API，也不读取个人 Key。
- WSL/Linux 宿主缺少 GTK/WebKit 开发库时，Desktop strict Clippy 与测试在固定 Tauri Linux
  工具镜像中运行；镜像挂载的源码必须是当前 worktree，Cargo target 仍在仓库外。
- Linux 容器通过不能替代 Windows WebView 验收。最终交付使用固定 commit 的 Windows
  PowerShell 清单，检查原生窗口、Keyring 隔离、升级兼容、release bundle 和无敏感信息证据。
- Server 验收套件位于仓库外时必须把 `MURIARC_REPO_ROOT` 设置为当前 worktree 的绝对路径；
  隔离运行数据、报告和构建产物不得加入 Git。

## Legacy Python/Vue application

- purpose: Behaviour and migration reference only
- runtime: Frozen historical environment
- state: legacy
- manifest: Preserved in Git history and the archive branch
- check: Not part of MuriArc CI
- used_by: Migration acceptance only

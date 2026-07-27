# 环境

> [English](ENVIRONMENTS.md) | 简体中文

## 支持的工具链

| 范围 | 必需运行时 | 主要门禁 |
| --- | --- | --- |
| Rust workspace | Rust `1.88` | format、locked Clippy、locked tests |
| Vue UI | Node.js `>=22.13`、pnpm `11.5.0` | 单测、typecheck、remote/local build、Playwright |
| PostgreSQL Store | PostgreSQL `17` | fresh、幂等、增量、contract、fencing 测试 |
| Desktop | Windows WebView2 + Tauri 构建依赖 | Windows strict Clippy/tests 与 no-bundle smoke build |
| 文档 | Python 3 | 双语命名、发布状态与本地链接检查 |

## Worktree 隔离

编译和测试必须在被验证源码所属 worktree 的绝对路径执行，禁止回到 main 验证其他分支。每个 worktree 独立拥有 `ui/node_modules`、`.env`、SQLite、附件根、Playwright 输出和运行服务。

Cargo registry 与 pnpm store 可以机器级共享。Cargo 构建产物只是可重建缓存；所有任务串行时可使用仓库外共享 target：

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

禁止并行写共享 target、把缓存中的裸二进制当作当前分支证据，或未经所有者授权执行 `cargo clean`。

## Rust 门禁

```bash
cargo fmt --all -- --check
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
```

WSL/Linux 缺少 GTK/WebKit 开发库时，Desktop strict Clippy/tests 使用固定 Tauri Linux 工具镜像或 GitHub Windows job。Linux 结果不能替代 Windows WebView 验收。

## PostgreSQL 门禁

`MURIARC_TEST_DATABASE_URL` 必须指向一次性 PostgreSQL 17，不能与人工 Server、其他 worktree 或真实数据共用。测试角色需能创建/删除隔离测试数据库；测试结束后核对无残留。因配置缺失导致 PostgreSQL suite skip 不算通过。

自动 migration 测试不得指向验收或生产 volume。

## UI 门禁

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

Playwright 覆盖 Desktop、Tablet、Mobile。对某设备明确不适用的单个 case 可以 skip；因浏览器或运行依赖缺失导致整套未运行，不能报告为通过。

## Provider 与服务测试

默认测试只使用进程内或本机 mock upstream，不调用真实 Provider，也不读取个人 API Key。可复用人工 Server 必须使用仓库外固定验收套件和 worktree 专属 runtime；dirty worktree 构建只属于开发 harness，不构成正式验收。

## 生成物与敏感数据

Cargo target、`node_modules`、build、测试报告、数据库、附件、环境文件、凭据、私钥和验收证据都留在 Git 外。仓库只允许小型合成 fixture 和不可变公开定义。

# AGENTS.md

## 项目目标

MuriArc 以动物全生命周期管理为核心。实验、测量、样本、附件和 AI 必须通过明确领域关系关联到动物，不得退化为任意 SQL 或无约束 EAV。

## 开工前阅读

1. `README.md`
2. `docs/ARCHITECTURE.md`
3. `docs/SECURITY.md`
4. 涉及旧库时阅读 `docs/MIGRATION.md`

## 工程规则

- 默认使用中文说明，Rust/TypeScript 标识符和公共 API 使用英文。
- 保持入口薄、领域逻辑集中；不要在 Tauri command、Axum handler 或 Vue 页面中复制业务规则。
- `core` 不依赖 Tauri、Axum、SQLx 具体数据库或模型 Provider。
- SQLite 与 PostgreSQL adapter 必须满足同一 Store contract tests。
- 所有写入均要求 actor、source、revision 和 audit；核心记录默认软删除。
- AI 不得执行 raw SQL，不得绕过权限、预览、审批或草稿签署。
- 不因文件长度机械拆分；只提取稳定边界或真实重复。
- 变更公共行为必须添加测试；提交前运行相关 Rust、前端和端到端检查。

## Git Worktree 与测试环境

Worktree 是**独立工作树**，不是“缺编译环境的半成品”。对该 feature 的编译与测试必须在**该 worktree 目录内**完成，不得回到 `main` 工作树去编译/测试别的 worktree 的改动。

**边界：** 主仓与 worktree 负责源码检出（可用 `cw`，与主仓同侧即可，例如 `Github/.worktrees/...`）；Cargo 编译产物必须写到仓库外。本机有 `E:\Muriarc` 时，默认写到该盘的 `builds/`，便于集中管理。

拆分与执行流程见 `.agents/skills/git-worktree-design` 与 `.agents/workflows/exec-worktree-spec.md`。Git 相关 skills 也在 `.agents/`。

### 严禁的错误做法

- **禁止**为了“借用 main 的编译环境”而 `cd` 到 main 工作树跑 `cargo`/`pnpm` 来验证 worktree 改动：测到的是 main 的源码与产物，会污染主仓，并掩盖 worktree 分支的真实结果。
- **禁止**把主仓或其它 worktree 的 `./target`、`./ui/node_modules` 符号链接/复制进当前 worktree 当“环境”。
- **禁止**将 `CARGO_TARGET_DIR` 指到任一 git worktree（含主仓）内的 `./target`。
- **禁止**多个 worktree（含 main）共用同一可写 SQLite / 附件目录做写测试。

### 正确心智模型

| 层级 | 内容 | 是否每个 worktree 重装 |
|------|------|------------------------|
| 工具链 | `rustup`/`cargo`/`rustc`、Node、Corepack、pnpm、系统库 | 否，机器级共享 |
| 工作树源码 | 该 worktree 检出的分支文件（可与主仓同侧） | 是，本来就独立 |
| Cargo 产物 | `CARGO_TARGET_DIR`（仓库外，默认 E 盘 `builds/`） | 按分支分流，见下 |
| 前端依赖等 | `ui/node_modules`、本地 `.env`、测试库 | 在当前 worktree 内；勿链回主仓 |

缺少的是该 worktree 下的**依赖安装与外部编译目录**，不是“只能用 main 才能编译”。

### Worktree 内跑测试前的引导（AI 必须照做）

在**该 worktree 的绝对路径**下执行（Cursor/Codex 会话的 cwd 也必须是该路径）：

```bash
# 1) 确保 cargo 在 PATH（非登录 shell 常见缺失）
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
command -v cargo >/dev/null || { echo "cargo not found; install rustup first"; exit 1; }

# 2) 编译产物：仓库外；本机有 E:\Muriarc 时默认落在 E 盘，并按分支分流
branch_slug="$(git branch --show-current 2>/dev/null | sed 's#[/ ]#-#g')"
branch_slug="${branch_slug:-detached}"
if [ -d /mnt/e/Muriarc ]; then
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/mnt/e/Muriarc/builds/cargo-target/$branch_slug}"
else
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/muriarc-cargo-target/$branch_slug}"
fi
mkdir -p "$CARGO_TARGET_DIR"

# 3) 前端依赖（每个 worktree 执行一次；pnpm 全局 store 已去重）
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install

# 4) 再跑检查（命令与 docs/ENVIRONMENTS.md 一致）
cargo test --workspace --all-features
pnpm --dir ui run test
```

可选：已安装 `sccache` 时设 `RUSTC_WRAPPER=sccache`。

### 共享与不共享（补充）

- 不要把 `target/`、`node_modules` 移出 `.gitignore` 或提交构建产物。
- `CARGO_TARGET_DIR` 必须落在**仓库工作树以外**；有 `/mnt/e/Muriarc` 时默认 `/mnt/e/Muriarc/builds/cargo-target/<branch-slug>`，否则 `$HOME/.cache/muriarc-cargo-target/<branch-slug>`。
- 本地 `.env` 按需从 main 或 `.env.example` 复制到**当前** worktree，不要共享可写运行时数据文件。

### 本机 Windows/WSL 的 `E:\Muriarc` 隔离规则

本机存在 `/mnt/e/Muriarc`（Windows `E:\Muriarc`）时：

- **源码：** `/home/ljx/Github/animal_lab` 为主仓；日常 feature worktree 可用 `cw` 建在与主仓同侧（如 `Github/.worktrees/...`），**不要求**建到 E 盘。
- **编译产物：** 默认使用 `/mnt/e/Muriarc/builds/cargo-target/<branch-slug>`（见上）；主仓与各 worktree 均不得长期在树内堆积 `target/`。
- **验收：** 可复用 Server 验收套件固定在 `/mnt/e/Muriarc/acceptance/kit/server-acceptance`，不得复制回仓库或纳入 Git。每次验收的隔离数据与证据放在 `/mnt/e/Muriarc/acceptance/runs/<run-id>`；不得使用真实数据库、附件、账号或 AI key。
- 调用外部验收套件时，必须把 `MURIARC_REPO_ROOT` 设置为实际被测 worktree 的绝对路径；不得默认指向主工作树，也不得使用 `origin/main`、旧 clone 或旧构建产物代测。
- 前端 `node_modules`、`dist`、Playwright 报告不得为了复用而链接回主仓。
- 测试套件、fixture、运行数据、构建产物、密钥和报告一律不得 stage、commit 或 push。只有产品源码与确有必要的项目文档才能进入 Git。
- 其他机器没有 E 盘时：Cargo 回退到 `$HOME/.cache/muriarc-cargo-target/<branch-slug>`；若仍有仓库外验收根目录，保持 `acceptance/` 与 `builds/` 边界即可（不必强行迁移 worktree）。

## 数据与 Git 边界

- Git 跟踪源代码、迁移、测试、小型 fixture、文档和依赖锁文件。
- Git 不跟踪数据库、附件、快照、密钥、构建产物、缓存和真实动物数据。
- 旧数据库只读；迁移必须写入新目标并生成报告。
- 不在日志、审计、测试快照或前端状态中记录 API key。

## 品牌与上游

- 产品名称统一为 `MuriArc`，品牌源由 `branding/brand.json` 管理。
- 不重新引入旧 Logo、二维码、QQ群或宣传按钮。
- 不删除 LICENSE、NOTICE 和关于页中的必要上游归属。

## 所有者权限

项目所有者决定正式产品状态、发布范围和数据迁移结果。AI 可以实施、检查和提出建议，但不得自行宣告异常数据已被人工确认。

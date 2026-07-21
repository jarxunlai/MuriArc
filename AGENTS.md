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

Worktree 是完整可编译、可测试的工作副本；在对应目录内可正常跑与主仓相同的检查（`cargo test`、`pnpm --dir ui run test` 等）。拆分与执行流程见 `.agents/skills/git-worktree-design` 与 `.agents/workflows/exec-worktree-spec.md`。

### 共享与不共享

- **机器级共享（无需每个 worktree 重装）**：`rustup` / `rust-toolchain.toml`、Node、Corepack、pnpm 版本、系统依赖。
- **每个 worktree 各自一份（被 `.gitignore`）**：`target/`、`ui/node_modules`、本地 `.env`、测试用数据库与附件路径。
- 不要把 `target/`、`node_modules` 移出 `.gitignore` 或跨分支提交构建产物。
- 不要多个 worktree 共用同一可写 SQLite / 附件目录做写测试，以免互相污染。

### 降低冗余（AI 建 worktree 或跑测试时照做）

1. **Cargo 共享编译缓存**（优先）：在当前 shell 设置统一目录后再跑测试，例如
   `export CARGO_TARGET_DIR="$HOME/.cache/muriarc-cargo-target"`。
   多个 worktree **同时** `cargo build/test` 可能争用同一 `target` 锁；并行时改为按分支分流，例如
   `CARGO_TARGET_DIR="$HOME/.cache/muriarc-cargo-target/<branch-slug>"`，或串行跑测试。
2. **前端**：每个 worktree 在该目录执行一次 `pnpm --dir ui install`。pnpm 全局 store 已 content-addressable，安装主要是建链接，不要跨 worktree 硬链整个 `node_modules`（分支间 lock 可能不同）。
3. **可选加速**：已安装 `sccache` 时可设 `RUSTC_WRAPPER=sccache`，可与共享 `CARGO_TARGET_DIR` 叠加。
4. 建 worktree 后若需跑 Rust/前端检查，先确认上述缓存与 `pnpm install`，再执行 `docs/ENVIRONMENTS.md` 中的 check 命令。

## 数据与 Git 边界

- Git 跟踪源代码、迁移、测试、小型 fixture、文档和依赖锁文件。
- Git 不跟踪数据库、附件、快照、密钥、构建产物、缓存和真实动物数据。
- 旧数据库只读；迁移必须写入新目标并生成报告。
- 不在日志、审计、测试快照或前端状态中记录 API key。
- Git相关的skills请读取home\ljx\Github\animal_lab\.agents目录下的内容

## 品牌与上游

- 产品名称统一为 `MuriArc`，品牌源由 `branding/brand.json` 管理。
- 不重新引入旧 Logo、二维码、QQ群或宣传按钮。
- 不删除 LICENSE、NOTICE 和关于页中的必要上游归属。

## 所有者权限

项目所有者决定正式产品状态、发布范围和数据迁移结果。AI 可以实施、检查和提出建议，但不得自行宣告异常数据已被人工确认。

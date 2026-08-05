# AGENTS.md

## 项目目标

MuriArc 以动物全生命周期管理为核心。实验、测量、样本、附件和 AI 必须通过明确领域关系关联到动物，不得退化为任意 SQL 或无约束 EAV。

## 开工前上下文策略

- 本文件提供项目的基础领域与工程约束，不默认扫描整个仓库。
- 涉及产品范围、运行形态、部署或发布时，阅读 `README.md` 的相关章节。
- 涉及跨层边界、Application/Core/Store、公共行为或事务设计时，阅读
  `docs/ARCHITECTURE.md` 的相关章节。
- 涉及认证、权限、AI、密钥、审计、日志或敏感数据时，阅读
  `docs/SECURITY.md` 的相关章节。
- 涉及旧库、旧数据或迁移时，阅读 `docs/MIGRATION.md` 的相关章节。
- 除非任务要求全面审计，否则只读取与当前任务有关的文档章节。

## 工程规则

- 默认使用中文说明，Rust/TypeScript 标识符和公共 API 使用英文。
- 保持入口薄、领域逻辑集中；不要在 Tauri command、Axum handler 或 Vue 页面中复制业务规则。
- `core` 不依赖 Tauri、Axum、SQLx 具体数据库或模型 Provider。
- SQLite 与 PostgreSQL adapter 必须满足同一 Store contract tests。
- 所有写入均要求 actor、source、revision 和 audit；核心记录默认软删除。
- AI 不得执行 raw SQL，不得绕过权限、预览、审批或草稿签署。
- 不因文件长度机械拆分；只提取稳定边界或真实重复。
- 变更公共行为必须添加测试；提交前运行相关 Rust、前端和端到端检查。

## CodeGraph 上下文导航

当任务需要定位、理解或修改源码时：

- 以 `repo_root="$(git rev-parse --show-toplevel)"` 确认当前实际 worktree。main 与每个 linked
  worktree 各自使用根目录内的 `.codegraph/`；禁止复制、链接或借用其他 worktree 的索引。
- 本项目授权为当前 worktree 创建本机索引。若 CodeGraph 可用但尚未初始化，确认 `.codegraph/`
  已被 Git 本地 exclude 后执行 `codegraph init "$repo_root"`；首次查询前用 `codegraph status
  "$repo_root"` 检查，落后时执行 `codegraph sync "$repo_root"`。
- 首次源码探索优先调用 `codegraph_explore`，并显式传入当前 `repo_root` 作为 `projectPath`；
  不得依赖 MCP 默认目录，也不得先进行全仓 `grep`、`find` 或逐文件读取。
- MCP 临时不可用时先回退到 `codegraph explore -p "$repo_root" "<精确符号或问题>"`；初始化或
  查询失败时说明原因后才使用普通搜索。
- 跨层修改须覆盖 UI/Gateway、transport、Application、Core/Store port、SQLite、PostgreSQL 和
  contract tests。CodeGraph 只负责导航；权限、安全、事务、SQL、Audit、Provenance 和领域不变量
  仍以源码、文档和测试为准。Markdown、SQL、配置和构建文件可直接读取。
- `.codegraph/` 只作本机缓存，不得 stage、commit 或 push。最终回复报告
  `CodeGraph: 已使用（projectPath：...；查询：...）` 或 `CodeGraph: 未使用（原因：...）`。

## 可复用 Server 开发测试服务

当用户要求启动、更新或恢复可人工使用的 Server 测试服务时，必须使用仓库外固定套件：

```text
/mnt/e/Muriarc/acceptance/kit/server-acceptance/scripts/run.sh
```

- 套件只保存脚本、合成 fixture 和定义；数据库、附件、模型、秘密与证据位于
  `/mnt/e/Muriarc/acceptance/runs/` 和 Docker volumes，不得进入 Git。
- 调用前设置 `MURIARC_REPO_ROOT="$(git rev-parse --show-toplevel)"`。套件按绝对路径为每个
  worktree 生成独立 repo-key、runtime、Compose project 和可写 volumes；同一 worktree 后续复用
  PostgreSQL、`server_data` 与 `local_ai_models`。
- 源码更新后使用 `run.sh ready` 重建并幂等补齐数据；镜像未变化且仅恢复服务时使用 `run.sh up`。
  随后执行 `run.sh status`。
- `ready` 默认加载并验证 `standard-v1` 标准数据。完整数据矩阵、固定 fixture、基线/沙盒边界与恢复
  命令以 `/mnt/e/Muriarc/acceptance/kit/server-acceptance/STANDARD-DATASET.md` 为准；当前访问地址、
  generation 和本机合成账号只查看 active generation 内受保护的 `TEST-ACCESS.md`。
- 正式标准基线只用于读取和回归验证；人工增删改必须使用“标准数据｜人工测试沙盒”。基线漂移时
  禁止自动清库或用 SQL 修补，先执行 `run.sh snapshot-standard`，不兼容时执行
  `run.sh new-standard-generation "<reason>"`。AI 测试不属于 standard-v1，由项目所有者自行执行。
- dirty worktree 仅开发 harness 可设置 `MURIARC_ACCEPTANCE_ALLOW_DIRTY=1`，并须报告“不构成正式
  验收”。正式验收必须使用干净 worktree、独立 run-id，显式覆盖 `MURIARC_ACCEPTANCE_RUNTIME`
  和 `COMPOSE_PROJECT_NAME` 后执行 `run.sh all`，如实报告 FAIL/SKIP。
- 用户要求服务供人工使用时保持运行；仅纯自动测试结束或用户要求停止时执行保留 volumes 的
  `run.sh down`。除非用户明确要求永久销毁，禁止 `destroy`、`down --volumes` 或删除 runtime/
  volumes。
- 访问凭据只提示用户在本机运行 `run.sh show-access`；不得把 `.env`、`TEST-ACCESS.md`、密码、
  Session、CSRF 或 Token 复制到对话或日志。

## Git Worktree 与测试环境

Worktree 是**独立工作树**，不是“缺编译环境的半成品”。功能开发和缺陷修复默认在非 main
worktree 中完成；若任务从 main 工作树启动，除非用户明确要求直接修改 main，否则不得写业务源码，
应先切换或按流程创建 feature worktree。

- 编译、测试和服务启动必须在实际 worktree 目录执行，禁止回到 main 验证其他 worktree 的源码。
- 工具链、Cargo registry 和 pnpm store 为机器级共享；`ui/node_modules`、`.env`、SQLite、附件和
  运行中服务必须按 worktree 隔离，禁止从 main 复制或链接可写运行时数据。
- 不得把 `target/`、`node_modules`、数据库、报告或构建产物纳入 Git。
- 拆分与执行流程见 `.agents/skills/git-worktree-design` 和
  `.agents/workflows/exec-worktree-spec.md`。

### 共享 Cargo target

main 与所有 worktree 默认**串行共享**一个仓库外 `CARGO_TARGET_DIR`，用于复用依赖编译产物：

- 有 `/mnt/e/Muriarc`：`/mnt/e/Muriarc/builds/cargo-target/shared`
- 否则：`$HOME/.cache/muriarc-cargo-target/shared`

共享 target 只是可重建缓存，不是运行时隔离边界。禁止并行写入、禁止直接运行其中的裸二进制并假定
它来自当前分支、禁止未经用户授权执行 `cargo clean`。必须从当前 worktree 通过 `cargo run/test`
调用；确需并行时改用独立的仓库外 target 并在最终回复说明。

### Worktree 内跑测试前的引导（AI 必须照做）

在**该 worktree 的绝对路径**下执行（Cursor/Codex 会话的 cwd 也必须是该路径）：

```bash
# 1) 确保 cargo 在 PATH（非登录 shell 常见缺失）
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
command -v cargo >/dev/null || { echo "cargo not found; install rustup first"; exit 1; }

# 2) 编译产物：main 与所有 worktree 串行共享同一仓库外目录
if [ -d /mnt/e/Muriarc ]; then
  export CARGO_TARGET_DIR="/mnt/e/Muriarc/builds/cargo-target/shared"
else
  export CARGO_TARGET_DIR="$HOME/.cache/muriarc-cargo-target/shared"
fi
mkdir -p "$CARGO_TARGET_DIR"

# 3) 前端依赖（每个 worktree 首次或 lockfile 更新后执行）
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install

# 4) 在同一 shell 中从当前 worktree 运行任务相关 cargo/pnpm 命令
```

完整门禁以 `docs/ENVIRONMENTS.md` 为准；可用 `sccache` 时设置 `RUSTC_WRAPPER=sccache`。

### 本机 Windows/WSL 的 `E:\Muriarc` 隔离规则

本机存在 `/mnt/e/Muriarc`（Windows `E:\Muriarc`）时：

- `/home/ljx/Github/animal_lab` 为 main；feature worktree 可放在同侧目录，不要求放到 E 盘。
- E 盘只集中保存共享 Cargo target、仓库外验收套件及其隔离 runtime；不得使用真实数据库、附件、
  账号或 AI key，也不得用 `origin/main`、旧 clone 或旧构建产物代测。
- 其他机器保持“源码 worktree、构建缓存、验收运行数据”三者分离即可。

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

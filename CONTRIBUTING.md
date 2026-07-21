# Contributing to MuriArc

## 分支与 Worktree

`main` 是唯一集成分支，只用于同步、验收和发布，不直接承载功能开发。每项修改都从最新
`origin/main` 创建短期 `codex/*` 分支，并在独立 Worktree 中完成：

- `codex/common/*`：领域、Application、Store contract、共享类型和公共迁移；
- `codex/desktop/*`：Tauri、SQLite 和本地运行形态；
- `codex/server/*`：Axum、PostgreSQL、认证和共享服务端；
- `codex/integration/*`：Docker、合成 fixture、端到端测试、CI 和部署。

Worktree 目录可以复用，但功能分支不得长期复用。公共 contract、schema、迁移、锁文件和路由
入口由单一负责人修改；依赖公共变更的 Desktop/Server 工作默认等待公共 PR 合并后再更新
`main`。

## Pull Request

1. 从最新 `origin/main` 建立分支和 Worktree。
2. 完成第一个语义完整的 commit 后推送分支并创建 Draft PR。
3. 后续修改继续提交到同一分支；PR 会自动更新。
4. 填写依赖、迁移、验证和数据安全检查，完成后标记 Ready for review。
5. CI、审查和冲突门禁通过后，默认使用 squash merge。
6. 合并后同步主 Worktree；新的需求使用新的分支和 PR。

单人独占的分支优先 rebase 最新基线；多人共享的分支使用 merge，避免改写他人历史。冲突必须
在产生冲突的功能 Worktree 内解决，不得向 `main` 临时复制文件。

## 并行运行隔离

并行 Docker/端到端环境必须分别配置 Compose project、宿主机端口、volume、`.env` 和附件目录。
任何真实数据库、动物记录、附件、快照、密钥、运行时配置或验收报告都不得进入 Git 或 Docker
build context。

## 提交前检查

运行受影响范围的格式化、静态检查、单元/契约测试和构建；公共行为变化必须包含测试。涉及
迁移时同时验证 fresh 与 incremental 路径。准备公开发布前，还必须对完整可达 Git 历史和发布
资产执行个人信息、密钥、真实研究数据和构建产物审计。

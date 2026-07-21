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

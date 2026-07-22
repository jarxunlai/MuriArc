# Feature Spec: Genetics v2 记录生命周期与唯一真值

> 此文档记录当前隔离工作树中的已确认范围、实现状态和验收证据。

## 分支信息

| 项目 | 值 |
|------|-----|
| 分支名称 | `feature/genetics-record-lifecycle` |
| 基于提交 | `origin/main@dbef8a7b639f1cb8e705ef9841ff4c9f04bb20ed` |
| Worktree 路径 | `/home/ljx/Github/.worktrees/animal_lab/codex-animal_breeding` |
| 建立日期 | `2026-07-21` |

## 目标

将 Genetics v2 确立为动物登记、普通动物导入、当前基因型投影和业务导出的新写入/读取真值；为基因检测记录提供可追溯的作废与更正语义，为位点、allele 和基因型定义提供安全归档能力，同时保留旧式 `Genotype` 的兼容读取与显式人工转换边界。

## 实现范围

- [x] 为 `GenotypingRecord` 增加显式作废与替代关系，定义并验证 lifecycle invariant。
- [x] 增加原子“作废记录”和“更正记录”Store/Application 用例，携带原因、`expected_revision`、Audit 与 Provenance。
- [x] 保证历史 Participation snapshot 指向的已作废记录仍可按 ID 读取；新快照和当前投影忽略已作废记录。
- [x] 为 `GeneLocus`、`Allele`、`GenotypeDefinition` 增加归档、恢复、引用影响查询及并发 revision 检查。
- [x] 位点或 allele 被活动定义引用时阻止归档；归档定义不再进入新记录选择器，但历史引用可读取。
- [x] 定义共享的“每个定义最新有效检测记录”投影，并让动物概览、详情及繁育读取同一口径。
- [x] 将动物 `ImportPlan` 的基因型写入从旧式 `Genotype` 改为已解析的 Genetics v2 `GenotypingRecord`，停止普通导入产生新的旧式记录。
- [x] 贯通 SQLite/PostgreSQL 相同 Store contract、REST、Tauri、Gateway 和必要的基因型纠错/归档 UI。
- [x] 更新 Snapshot/DTO/测试 fixture，确保新增 lifecycle 字段和历史记录完整归档。
- [x] 将动物与零到多个初始 Genetics v2 记录放入同一原子登记边界，并在登记 UI 中执行状态与权限约束。
- [x] 提供单一动物导入 schema、CSV/XLSX 模板、计划级预览和重复“动物 × 已有定义”写入。
- [x] 提供不含 UUID 的动物业务导出、组合人类可读身份、字段/日期/目录过滤，以及 XLSX 动物与基因型明细分表。
- [ ] 完成所有环境级验证：代码与本地可运行测试已通过；仍需有 PostgreSQL 测试库的真实 contract run，以及具备 DBus 开发库环境的 Tauri workspace check。

## 验收标准

- `confirmed` / `rejected` 仍必须有 `assessed_at`；`expected` / `unknown` 不冒充检测事实。
- 作废必须填写非空原因并通过二次确认；revision 不一致时返回冲突，不覆盖新数据。
- 更正在一个事务内作废原记录并创建 `supersedes_record_id` 指向原记录的替代记录，任一步失败整体回滚。
- 当前投影按创建顺序选择每个定义最新的未作废记录；历史 Participation snapshot 不被回写。
- 已作废记录仍可按 ID 查询并进入 Snapshot，新的 Participation snapshot 不选择它。
- 归档目录实体不会出现在新建选择器；显示已归档及恢复入口可用；历史名称和组件仍可读取。
- 普通动物导入不再写旧式 `Genotype`，也不会根据旧记录或自由文本静默创建 Genetics v2 定义。
- SQLite 与 PostgreSQL 通过相同 contract tests；Desktop 与 Server 对相同操作保持同等语义。
- 所有正式写入包含 actor、source、revision、Audit 和 Provenance。

## 技术约束

- `core` 不依赖 Tauri、Axum、SQLx adapter 或模型 Provider。
- 跨多步写入必须使用 Store 原子方法/数据库事务，不由 transport 或 Vue 循环模拟事务。
- Genetics v2 新写入不得静默转换旧式 `Genotype`；旧到新只能走显式预览、人工确认和 provenance。
- 检测作废不得仅使用通用 `RecordMeta.deleted_at`，以免历史 snapshot 无法解析原记录。
- 核心记录默认保留历史，不物理级联删除检测、定义引用或 Participation snapshot。
- SQLite/PostgreSQL 迁移按各自最新编号确定并保持相同业务语义。
- 不记录 API key、真实动物数据、数据库、附件路径或其他敏感信息。
- 不机械拆文件；领域规则集中于 core/application/store contract，入口保持薄。

## 当前边界

- 原计划拆出的动物登记和动物数据 I/O 已在本工作树内一起完成，便于一次核对领域边界；尚未提交或推送。
- `GenotypingEvidence` / gel batch 仍按已确认决定延期，不在本次模型和迁移中引入附件目标关系。
- 旧式 `Genotype` 仅保留兼容读取；本次没有自动迁移、静默转换或新增普通写入口。
- 未安装任何系统或项目依赖；Tauri 的完整 workspace check 受当前机器缺少 `dbus-1`/`pkg-config` 开发环境阻塞。

## 已完成验证

- `muriarc-core`、`muriarc-application` 全套测试通过。
- `muriarc-importer` 34 项测试通过；`muriarc-data` 15 项测试通过。
- SQLite contract/research 测试与 Server 61 项测试通过。
- PostgreSQL adapter 编译和无数据库测试通过；需要配置 `MURIARC_TEST_DATABASE_URL` 后补真实数据库 contract run。
- UI 16 个测试文件、94 项测试通过，`vue-tsc` 与 Vite production build 通过。
- `cargo fmt --check` 与 `git diff --check` 作为最终交付检查执行。

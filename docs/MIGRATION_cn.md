# MuriArc 数据迁移与恢复

> [English](MIGRATION.md) | 简体中文

## 范围

本文只描述 MuriArc 自身的 schema、运行数据根、配置引用和 release generation 迁移，不承诺从无关第三方数据库导入；原第三方旧库导入 CLI 已在 1.0 前移除。

普通 CSV/XLSX Import、业务 Snapshot、数据库备份/恢复、数据根迁移和签名版本升级是不同操作，不能互相冒充。

## Source of truth

- SQL migration 文件是 SQLite/PostgreSQL 有序、追加式 schema source。
- `migrations/checksums.json` 锁定全部已登记 migration；修改/删除已锁定文件或新增未登记文件会使 CI 失败。
- SQLx migration ledger 及 description/checksum 生成 backend-specific state digest。
- Release Manifest、deployment state、Write Lease 与 `deployment-generation.json` 共同绑定 ApplicationVersion、DataEpoch、backend state、Gateway contract 和 active filesystem generation。
- SQLite/PostgreSQL adapter 必须通过同一 Store contract；平台专属表可导致 migration 编号不同，但共享语义保持一致。

## 普通启动与升级

Server/Desktop 普通启动只核对兼容性并打开已有效 generation，不静默运行稳定版 schema migration。

在 `preview_epoch_0`，显式 preview bootstrap 可采用预发布数据库并建立初始 generation/lease/manifest；它不是稳定升级机制。永久兼容下限从最终 `1.0.0 / E0001` 开始，且 E0001 fixture 必须由最终制品生成并通过完整 RC 矩阵。

稳定安装/升级由 `muriarcctl` 与共享 Upgrade Engine 执行：目标验签、freeze/drain、联合备份、实际隔离恢复、Candidate migration、七层验证、原子激活和 Write Lease。

## Schema 变更策略

新结构遵循 `Expand → Backfill → Switch → Contract`：

1. **Expand**：增加兼容 column/table/index 与 Write Lease fence。
2. **Backfill**：有边界、幂等、可观测、可重启。
3. **Switch**：两种表示都有效后才切换 Application 读写路径。
4. **Contract**：仅在后续版本且兼容下限允许时删除旧结构。

不确定或必须离线的结构变更属于 M3。已发布 SQL 永不编辑/回滚；修复只能追加新 migration。

持久化 enum/JSON 必要时使用 preserved/versioned wrapper。未知 raw value 保持可读并标记 `needs_review`，不得静默映射为正常科研状态。

## Application 数据演化

MuriArc migration 追加或演化明确领域关系，包括 genetics definition/record、繁育事实、Observation/value history、入组基因型快照、Attachment link、AI model profile、视觉候选、账号/Session 治理、技术日志保留、genotyping batch、兼容身份、operation state 和 Write Lease fence。

Schema migration 不得发明科研事实，不能从模糊历史文本/附件推断繁育配对、父母、观察含义、基因型定义、证据批次或审批。此类转换需要独立、由所有者复核并保留 Provenance 的计划。

Server credential migration 保留 Argon2id hash 和既有账号身份。AI profile migration 保留 owner、endpoint/protocol、模型 ID、参数、secret-version reference、default 与会话绑定。无效 default 可由前向修复清空，但禁止通过删除档案、版本、秘密或历史让 migration 成功。

## Desktop 数据根迁移

Desktop 迁移 MuriArc data root，不迁移 OS keyring。操作通过原生 selection token 排期，并在 SQLite 打开前执行：

1. integrity/FK 与 WAL checkpoint；
2. 复制到允许的本机固定磁盘 staging；
3. 文件树 size/SHA-256 验证；
4. 目标 SQLite integrity；
5. 原子 locator switch。

失败时 source 继续 active 并被保留，绝不创建替代空库。SQLite、附件、数据产物、非敏感 AI 配置和 generation manifest 联合迁移。

## 备份、Candidate 与回滚

Server/Desktop 恢复集合联合数据库、附件、数据产物、配置、generation manifest、Keyset/Master Key reference 和 AI 状态。备份只有在实际隔离恢复并通过验证后才有效。

Candidate 使用独立存储，关闭真实用户流量、外部 Provider 与后台任务。七层验证覆盖：

1. 恢复资产与 hash；
2. 数据库完整性/migration state；
3. Store/Application 不变量；
4. 真实 API 读取；
5. 真实 UI 读取；
6. 受控事务内继续写入；
7. 只读无副作用。

目标首次写入前可原子恢复已验证 source generation；首次写入后禁止自动降级，只能前向修复或执行带操作者数据损失确认的显式恢复。

## Import/Export 与 Snapshot 边界

普通 Import 当前只支持明确的 Animal Registry 与实验 Measurement；证据化 genotyping batch 使用专用流程。普通 Export 只输出按作用域 Animal Registry。Import/Export 不是通用实体同步、数据库恢复或 Desktop-to-Server migration。

业务 Snapshot 是带附件/checksum 的 typed JSONL，用于完整性和离线留存；它排除账号/Session/Token 秘密，也不是可启动数据库备份。只有全量预检、跨实体事务、apply ledger、canonical hash、Lab mapping 与 Audit/Provenance 语义冻结并测试后，才可开放通用 restore/apply。

## 验收

迁移验收必须覆盖 fresh schema、幂等 replay、所有支持的增量状态、中断恢复、真实恢复副本、附件字节、AI 历史/secret reference、Audit/Provenance、业务不变量和首次写入回滚边界，并同时覆盖 SQLite 与 PostgreSQL 17。

数据库 suite skip、手改 SQL、清空数据或“文件仍存在”都不构成 PASS。

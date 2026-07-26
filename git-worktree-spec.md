# Feature Spec: MuriArc 永久数据兼容基础

> 此分支建立 1.0 前必须冻结的兼容契约；升级控制器、交付 Driver 与历史 Fixture 在后续分支实现。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/server-upgrade-data-compatibility` |
| 基于提交 | `origin/main@ce223d819129de2d9fad6cbee2a691304c97a53d` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-server-upgrade-data-compatibility` |
| 建立日期 | `2026-07-26` |

## 目标

从 `preview_epoch_0` 建立可验证的版本、Epoch、Backend State、Gateway Contract、Generation、
Persistent Data Registry 和 migration checksum 基础，使 Server/Desktop 普通启动能够拒绝错误
数据库、缺失 generation、密钥或附件恢复集合，而不再静默把旧库原地改造成最新版。

## 实现范围

- [x] 建立应用版本、Data Epoch、Backend State Digest、Gateway revision 和只追加 Release Catalog。
- [x] 建立代码化 Persistent Data Registry、M0-M3/UI 影响分类和未知值保留 decoder。
- [x] 为 SQLite/PostgreSQL 增加 deployment state、generation、upgrade operation、write lease 和首次写入标记。
- [x] 为两套 Store 增加 compiled/applied migration 精确核对、generation adoption 和恢复集合 inventory。
- [x] Server/Desktop 普通启动改为 fail-closed 核对；仅保留显式预发布 bootstrap 入口。
- [x] 已有密文但 Master Key 缺失、已有附件但根目录缺失、generation manifest 不一致时阻断。
- [x] 建立 migration checksum 清单与门禁，补充双后端兼容 contract tests。
- [x] 更新架构、安全、迁移和 1.0 兼容契约文档。

## 验收标准

- 未执行升级控制面的普通进程不能修改 schema 或 deployment identity。
- migration 被修改、缺少或额外出现时，兼容核对必须失败并给出稳定问题 code。
- 数据库 generation 与 data-root manifest 不一致时，Server/Desktop 不开放业务入口。
- 有 AI 密文却没有正确 Master Key、或有附件 metadata 却没有附件目录时不能生成替代状态。
- SQLite/PostgreSQL 使用同一核心兼容类型和相同判定语义。

## 技术约束

- `core` 不依赖 Tauri、Axum、SQLx 或 Provider；Store adapter 负责读取各自 migration ledger。
- migration 只追加；既有 SQL 由仓库内 checksum manifest 保护。
- 不把数据库、附件、密钥、真实用户数据或恢复点加入 Git。
- 兼容失败必须 fail closed；不得提供跳过备份、校验、Epoch 或 Digest 的普通 force 参数。

## 跨分支备注

本分支先合并。`feature/upgrade-engine-control-plane` 与 `feature/release-fixtures-gates`
以这里的公共类型和数据库状态表为基础；交付、Desktop 与 Cloudflare 分支不得复制兼容判定。

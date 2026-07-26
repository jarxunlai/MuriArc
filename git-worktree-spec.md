# Feature Spec: 基因鉴定批次与胶图证据

> 此分支实现用户确认的批量基因鉴定录入：表格结果、批次元数据与多张胶图共同绑定并可追溯。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/genotyping-batch-evidence` |
| 基于提交 | `main@552dc0324f4c1d9bd70251854849294eb606f4d6` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-genotyping-batch-evidence` |
| 建立日期 | `2026-07-25` |

## 目标

在“动物数据”中新增独立的基因鉴定批次工作流。用户可一次选择鉴定结果表格和一张或多张胶图，
预览动物/基因型结果与证据文件，确认后形成正式批次；每条 Genetics v2 `GenotypingRecord`、
原始胶图 Attachment、操作者、时间、hash、Audit 与 Provenance 均能从批次追溯。

## 实现范围

- [x] 建立 `GenotypingBatch` 领域聚合及草稿/已提交生命周期，记录实验室、可选项目、批次编号、鉴定时间、方法、备注、创建者和 revision。
- [x] 使用 `genotyping_batch_records` 显式关系表关联批次与既有 `GenotypingRecord`，并保证已提交批次、记录、附件关系、AnimalEvent、Audit 与 Provenance 由 Store adapter 原子确认。
- [x] 增加 SQLite/PostgreSQL 纯向前迁移、Store port、两套 adapter 和共享 contract tests；扩展 snapshot。
- [x] 增加 Server REST 与 Tauri commands：创建/读取/列出批次、上传/列出胶图证据、预览结果表格、确认或取消草稿。
- [x] 在“动物数据”增加“动物登记 / 基因鉴定批次”入口；支持 CSV/XLSX、批次元数据、多图选择、缩略预览、映射/校验、确认收据和最近批次。
- [x] 在动物档案的基因型记录中展示批次来源，并可回到批次查看胶图证据。
- [x] 更新架构、安全、迁移与普通导入边界文档。
- [x] 完成任务相关 Rust、UI、Store 与合成浏览器工作流验证。
- [x] 从干净 feature commit 重建本机可人工使用的 Server 标准服务，并通过标准数据与部署核验。
- [ ] 合并前使用独立 run-id 执行完整正式验收；本分支的可复用标准服务不替代独立 `all` 验收。

## 验收标准

- 用户能在同一工作流选择一份鉴定结果表格和多张胶图；确认前可删除或替换任一证据。
- 表格至少包含动物业务编号和检测状态，基因型定义、鉴定时间与方法由批次元数据统一指定；系统拒绝不存在/不可见动物、未知定义、重复动物和跨实验室引用。
- 确认成功后关系表把全部检测记录关联到同一批次，批次能列出全部记录与胶图；胶图保存文件名、MIME、大小、SHA-256 和版本。
- 确认失败不会产生部分 GenotypingRecord；草稿及已上传证据保持可恢复或可取消，不伪装成正式结果。
- Server 与 Desktop 语义一致；Editor 只能在项目范围读取，AnimalManager/LabAdmin 才能创建实验室级鉴定批次。
- 所有正式写入包含 actor/source/revision/Audit/Provenance，删除或更正不覆盖历史事实。

## 技术约束

- `core` 不依赖 Tauri、Axum、SQLx 或 Provider；入口保持薄。
- 不把胶图 Base64、路径、密钥或真实动物数据写入日志、审计、测试 fixture 或 Git。
- 不用通用 EAV、任意 SQL 或前端多次写入模拟事务。
- Genetics v2 是正式事实源；旧自由文本 `Genotype` 不作为批次结果写入目标。
- Attachment 内容与数据库 metadata 继续使用现有安全检查、hash 与私有文件库。
- SQLite/PostgreSQL adapter 必须通过同一 Store contract。

## 跨分支备注

单一纵向 feature，无跨分支依赖。合并前应重新基于最新 main 验证迁移编号与 UI 冲突。

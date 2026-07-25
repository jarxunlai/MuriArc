# Legacy migration

## Source of truth

- 主迁移基线由项目所有者在 Git 外的受控位置管理。
- 实施时只读取安全副本，绝不修改源文件。
- 每次迁移记录源文件 SHA-256、schema 版本、开始/结束时间和工具版本。

## Private acceptance baseline

真实源库的路径、SHA-256、动物与笼位数量、基因型数量、重复编号、缓存计数差异及孤立关系
统计只保存在项目所有者管理的私有验收报告中，不进入源代码仓库。迁移验收必须从该受控
manifest 读取预期值，并在迁移前后核对源文件指纹。

## Rules

1. 迁移前先执行 audit；阻断 schema 不兼容、不可读日期和损坏外键。
2. 每一旧表行获得确定性的 legacy identifier，不能用可能重复的 mouse.id 作为身份。
3. 重复显示编号保留原值并进入 conflict report，不自动重命名或合并动物。
4. Cage 实际数量按 mouse.cage_id 关系重算；旧 mice_count 只作为 provenance 保留。
5. 无法解析或孤立的谱系关系写入 rejected-records 报告，不伪造父母关系。
6. 目标数据库必须是新文件或空 schema；V1 不覆盖已存在的冲突 UUID。
7. 完成后核对实体数量、引用完整性与源文件 SHA-256，并导出 JSON 报告。

## Research extension schema migrations

SQLite 与 PostgreSQL 按相同顺序应用以下共享业务迁移：

1. `0012_genetics_v2.sql`：新增 GenotypeDefinition、组件与 GenotypingRecord；保留旧
   GeneLocus/Allele/Genotype 表和语义。
2. `0013_breeding.sql`：新增 BreedingLine、Colony、BreedingPair/Member、MatingEvent、
   Litter 与 AnimalDraft。
3. `0014_observations.sql`：新增 ExperimentEvent、ObservationDefinition、Observation 与
   追加式 ObservationValueRecord。
4. `0015_participation_genotype_snapshot.sql`：为 Participation 增加入组时基因检测快照。
5. `0016_audit_operations_ai_runtime.sql`：为 Audit 增加稳定 operation code/参数快照，并
   补充实验室级 AI runtime 配置。
6. `0017_workspace_assets_multimodal.sql`：新增 Attachment link/derivative、私有 AI 图片和
   提取草稿等 Workspace 资产关系。
7. Desktop/SQLite `0030_genotyping_batches.sql` 与 Server/PostgreSQL
   `0032_genotyping_batches.sql`：新增 `genotyping_batches` 与 `genotyping_batch_records`
   关系表。迁移只建结构，不从既有单条鉴定记录或附件猜测批次归属。

这些迁移不从旧自由文本或旧单基因 Genotype 猜测 Genetics v2 定义，也不自动创建配对、
交配事件或 Observation。需要转换的历史数据必须通过单独、可复核、带 provenance 的迁移
计划处理。

## Server-only credential lifecycle migration

`migrations/postgres/0018_auth_credential_lifecycle.sql` 只属于 Server/PostgreSQL。它为已有
`user_credentials` 追加 `must_change_password BOOLEAN NOT NULL DEFAULT FALSE` 与
`revision BIGINT NOT NULL DEFAULT 1`，并建立强制改密索引。增量升级保留原
`password_hash`，历史账号默认不被突然强制改密；管理员随后可以通过临时密码重置将其纳入
首次改密生命周期。

Desktop/SQLite 不创建 Credential、Session、token 或认证迁移；无密码“进入本地空间”只使用
`sessionStorage` 记录当前 WebView 会话已确认，不能被解释为数据加密或安全锁。AI Provider
endpoint 在 PostgreSQL 使用 0019、SQLite 使用 0018；项目动物关系与对话级 AI 自主授权在
PostgreSQL 使用 0020/0021、SQLite 使用 0019/0020。Server-only 技术日志保留表使用
PostgreSQL 0022；Genetics v2 检测记录生命周期在 PostgreSQL 使用 0023、SQLite 使用 0021。
平台专属迁移造成编号偏移，但共享业务迁移仍保持相同的相对顺序与语义。

## Versioned AI model profile migrations

多模型升级只新增迁移，不改写已发布 SQL：

| 语义 | PostgreSQL | SQLite |
| --- | --- | --- |
| 旧用户 Provider 设置兼容层 | `0024_ai_user_provider_settings.sql` | `0022_ai_user_provider_settings.sql` |
| 模型档案、不可变版本、默认引用与会话绑定 | `0025_ai_model_profiles.sql` | `0023_ai_model_profiles.sql` |
| 视觉图片、提取候选与审批证据 | `0026_ai_vision_data_entry.sql` | `0024_ai_vision_data_entry.sql` |
| 无效默认引用的纯向前兼容修复 | `0027_ai_provider_compatibility_finalize.sql` | `0025_ai_provider_compatibility_finalize.sql` |

旧设置迁移按 owner 幂等投影为模型档案和版本，保留自由文本模型、视觉模型、参数、密钥版本与
默认选择；不会根据模型名称重写能力或参数。新会话绑定精确的
`model_profile_id + model_profile_version`。无法建立安全绑定的旧会话继续可读并标记为
legacy read-only，不会伪造档案身份后继续调用 Provider。

默认对话/视觉档案引用必须属于同一用户、未软删除且未归档；视觉默认还必须显式声明视觉能力。
升级发现无效默认时使用后续纯向前修复迁移清空对应引用并推进 defaults revision，不删除档案、
版本、凭据或会话。档案停用同样只阻止新调用与会话追加，历史记录和 Trace 保持可读。

视觉数据迁移新增私有图片、候选草稿、图片关系与审批事务所需结构。它不扫描旧附件猜测证据
用途，也不自动批准或创建 Observation；只有后续的人工作业能把候选提升为正式研究记录。

## Upgrade and rollback policy

- PostgreSQL 迁移门禁必须在一次性真实 PostgreSQL 17 上覆盖空库完整应用、重复应用，以及从
  旧版本增量应用；SQLite 使用独立临时数据库执行相同 Store contract。未提供测试数据库而被
  skip 不算通过。
- 旧 Provider 表、旧设置行、旧配置文件、旧档案版本、旧密钥版本和 Desktop 旧 Keyring 项
  本轮一律保留。兼容读取可以停止写入旧结构，但不得用删除来“完成迁移”。
- 真实数据库完成升级后只允许追加新的修复迁移。禁止编辑或回滚已执行迁移、回退 schema
  version，或从生产数据猜测重建凭据。
- `MURIARC_AI_MASTER_KEY_VERSION` 只有在全部既有密文已用旧版本解密并重新加密后才能推进；
  轮换失败必须阻断启动并保留原数据，不能清空密钥行或要求用户在无报告的情况下重输。

## Ordinary import/export boundary

- 普通 Import 仅支持两种显式类型：Lab Animal Registry 与指定 Experiment 的 Measurement。
- 普通 Export 仅支持 Animal Registry CSV/XLSX，可按 Project 作用域过滤；它不包含完整
  Breeding、Observation、Audit、Provenance 或附件数据。
- Import/Export 是用户选择的数据产品，不是 Desktop ↔ Server 迁移、数据库恢复或通用实体
  同步工具。任何未列出的资源都应拒绝或根本不出现在选择项中，禁止静默遗漏后仍称为全量导出。

## Snapshot boundary and reserved conflict policy

当前 `0.1.0` 预发布实现支持生成和校验 snapshot（manifest、JSONL、附件、大小与 SHA-256），
以及同一 artifact 的幂等写入与内容冲突阻断。业务 JSONL 明确包括：

- Lab、Project、Cage、Animal、AnimalEvent、Pedigree；
- GeneLocus、Allele、旧 Genotype、GenotypeDefinition、GenotypingRecord；
- BreedingLine、Colony、BreedingPair（内含成员）、MatingEvent、Litter、AnimalDraft；
- ExperimentTemplateVersion、Experiment、Cohort、Participation（内含入组基因检测快照）、
  Procedure、Measurement、Sample；
- ExperimentEvent、ObservationDefinition、Observation、ObservationValue；
- Attachment 元数据与内容、Audit、Provenance。

账号、Membership、session、token、AI secret、运行时 Job 和部署配置不属于业务 snapshot；它也
不是可启动的数据库备份。当前尚未开放 snapshot restore/apply。正式恢复仍应使用经演练的
SQLite/PostgreSQL 与附件联合备份。

未来只有在 typed JSONL 全量预检、跨实体与附件统一事务、snapshot 应用账本、canonical record hash、Lab 映射及 Audit/Provenance 语义全部冻结后，才可开放 restore。届时必须遵循：

- 相同 snapshot_id 重复应用：幂等跳过；
- UUID 不存在：新增；
- UUID 存在且 canonical hash 相同：跳过；
- UUID 存在且内容不同：整次操作停止并报告，禁止部分写入、静默覆盖和字段级自动合并。

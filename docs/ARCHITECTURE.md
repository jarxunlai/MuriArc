# Architecture

## Runtime topology

```text
Vue UI ── LocalTauriGateway ── Tauri commands ──┐
                                                ├─ Application services ── Domain ── Store ports
Vue UI ── RemoteHttpGateway ── Axum /api/v1 ────┘                                      ├─ SQLite
AI drawer/workspace ── approved domain tools ──────────────────────────────────────────└─ PostgreSQL
External clients ── REST or MCP + scoped token ────────────────────────────────────────────────┘
```

Desktop 与 Server 共享核心领域模型、主要业务用例和同一套响应式前端。Desktop 提供无密码的个人本地工作流：每次 WebView 会话先显示“进入本地空间”，`sessionStorage` 只避免刷新重复，不能充当安全锁，也不创建认证表。Server 额外提供认证、用户治理、项目权限与共享部署能力。部署差异明确收敛在 transport、认证、凭据管理、密钥和 Store adapter，不在 Vue 页面复制领域规则。

## Application services

`crates/application` 位于 transport 与 `core` 之间，负责用例级输入规范化、领域对象构造和持久化意图编排。Tauri/Axum 只负责 DTO、传输格式、身份认证和权限门禁；`core` 继续保持对 transport、SQLx adapter 与模型 Provider 的零依赖。首个共享纵向切片为 `CreateAnimal`，后续用例按相同边界逐步迁移，而不是一次性拆解现有 Store。

## Domain invariants

- `AnimalId` 为 UUID；旧编号只是带命名空间的 identifier。
- Animal 属于实验室统一 Registry，通过 Participation 进入 Project/Experiment。
- AnimalEvent 是状态变化事实；Animal.current_state 是可重建的查询投影。
- 已发布 ExperimentTemplateVersion 不可修改，新定义必须创建新版本。
- Measurement 必须有显式 value type；数值测量同时记录 unit。
- Attachment 内容在文件库，数据库记录 hash、版本和关联。
- 正式写入必须包含 Provenance 与 Audit；删除默认为 soft delete。
- Genetics v2 以 `GenotypeDefinition` 聚合一个或多个显式组件；`GenotypingRecord` 表示某只
  Animal 在某一时点对定义的检测状态。旧 `Genotype` 保留原语义，不在迁移中自动折叠或重写。
- BreedingPair 必须恰好一个雄性成员和至少一个雌性成员；同一动物不能同时进入冲突的活跃
  配对。MatingEvent 是人工确认的事实，不由预测或 AI 自动生成。
- Litter 与其存活 offspring Draft 在一个事务中创建；Draft → Animal 同时写入 Animal、
  父母 Pedigree、Registered/Born 事件、Audit 与 Provenance。
- Observation 的值类型由 ObservationDefinition 决定。初值与 Observation 原子创建；修订
  只追加 ObservationValueRecord 并推进 current version，不覆盖历史值。
- Participation 入组在与基因检测写入相同的 animal-scoped 锁/事务序列中，捕获每个
  GenotypeDefinition 当时最新的检测记录；快照之后不可被新检测回写。

## API boundaries

- REST 统一位于 `/api/v1`，错误使用稳定 code、message、details、request_id。
- 并发写入携带 revision，过期 revision 返回冲突而非覆盖。
- 长任务进入 Job：parse → validate → preview → confirm → transaction → report。
- Genetics、Breeding 与 Observation 分别使用 `/genotype-definitions`、`/genotyping-records`，
  `/breeding-*`、`/mating-events`、`/litters`、`/animal-drafts`，以及
  `/experiment-events`、`/observation-*` REST 资源；Tauri Gateway 提供同等应用语义。
- 普通导入只接受 Animal Registry 或 Experiment Measurement CSV/XLSX；普通结构化导出只
  生成 Animal Registry CSV/XLSX。两者是受控数据产品，不是通用迁移协议。
- Snapshot 是 Lab 业务归档边界，包含新增 Genetics/Breeding/Observation 聚合、既有业务
  实体、Audit、Provenance 和附件；当前只生成和校验，不支持 restore/apply。
- MCP/AI 工具只能调用 application use case；禁止数据库连接、raw SQL、任意 HTTP、自动
  MatingEvent 和直接 Animal 修改。当前 MutationDraft 仅能提出结构化 Measurement 草稿。

## Runtime identity hierarchy

Server 的身份层级固定为：

1. **Environment Root**：由宿主机 `.env` 和部署生命周期管理；在应用权限上拥有 LabAdmin 能力，只有它能创建、修改、停用、降级或重置其他 LabAdmin。
2. **LabAdmin**：治理实验室业务与所有非 LabAdmin 账号，不能修改环境、代码、Environment Root 或同级 LabAdmin。
3. **ProjectAdmin**：只在获授权 Project 内拥有项目管理和业务权限。
4. **AnimalManager / Editor / Viewer**：分别承担 Lab Animal Registry、项目写入和项目只读职责。

Environment Root 不是新增的领域角色枚举，而是“配置声明的唯一 User ID + 实时 LabAdmin membership”的部署身份。这样现有 Permission/Store contract 保持共享，同时用户治理层能在 LabAdmin 之上实施不可绕过的层级检查。Server 启动同步、Session/external token、Argon2id 凭据和 PostgreSQL 属于 Server adapter；Desktop 不依赖这些组件。

## Tenancy and transactions

- V1 以 `Lab` 作为可执行的 Workspace/tenant 边界；`Project` 是 Lab 内的授权与协作范围，不改变 Animal 的 Lab Registry 所有权。
- Server 的 `lab_id`、actor 与权限来自认证主体，不接受客户端自行声明；Desktop 使用固定本地 Lab 与 LocalOperator。
- Application service 描述完整写入意图；SQLite/PostgreSQL Store adapter 在单事务内执行关系校验、领域记录、事件、Audit 与 Provenance。
- 跨多个持久化步骤的新用例必须扩展原子 Store port 或明确 Unit of Work，不允许 transport/application 通过多次独立写入模拟事务。

决策细节见 [ADR-0001](adr/0001-application-layer.md)、[ADR-0002](adr/0002-workspace-tenancy.md)、
[ADR-0003](adr/0003-transaction-boundaries.md)、[ADR-0004](adr/0004-genetics-v2-compatibility.md)、
[ADR-0005](adr/0005-breeding-atomicity.md)、[ADR-0006](adr/0006-observation-version-policy.md)、
[ADR-0007](adr/0007-enrollment-genotype-snapshot.md) 与
[ADR-0008](adr/0008-runtime-identity-and-account-security.md)。

## Deployment

- Desktop：SQLite 和附件目录位于 OS application data；密钥位于 OS keyring。
- Server：PostgreSQL、附件 volume、加密 secret store；Axum 位于 HTTPS reverse proxy 后。
- V1 不做 Desktop 与 Server 的实时同步。当前 snapshot 用于版本化完整业务归档、离线留存与
  完整性校验，不提供自动合并、导入或恢复入口；CSV/XLSX Export 也不能替代部署备份。

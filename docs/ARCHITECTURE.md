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

Desktop 启动时先由 `StorageRootState` 解析 OS application data 中的 locator，并在任何 SQLite
pool、附件服务或 AI 设置服务初始化前完成 pending migration。config root 只保存
`storage-location.json`、`storage-migration.json` 等定位控制信息；active data root 统一
保存 SQLite、附件、数据产物与非敏感 AI 配置。Vue 只能通过原生 folder picker 获得一次性
selection token，不能把任意路径作为迁移命令输入。迁移以 SQLite 完整性检查、WAL checkpoint、
staging 文件树 SHA-256 和目标只读复检为切换门禁；失败、自定义磁盘缺失或 metadata 无效均
fail closed，不创建空库。OS keyring 凭据和 WebView cache 不属于 data root。

## Application services

`crates/application` 位于 transport 与 `core` 之间，负责用例级输入规范化、领域对象构造和持久化意图编排。Tauri/Axum 只负责 DTO、传输格式、身份认证和权限门禁；`core` 继续保持对 transport、SQLx adapter 与模型 Provider 的零依赖。首个共享纵向切片为 `CreateAnimal`，后续用例按相同边界逐步迁移，而不是一次性拆解现有 Store。

## Domain invariants

- `AnimalId` 为 UUID；旧编号只是带命名空间的 identifier。
- Animal 属于实验室统一 Registry。`ProjectAnimalAssignment` 是项目可见性的显式授权关系，
  只能由实验室动物管理角色批量建立或移除；已分配动物再通过 Participation 进入具体
  Experiment。Participation 不再承担隐式项目授权。
- AnimalEvent 是状态变化事实；Animal.current_state 是可重建的查询投影。
- 已发布 ExperimentTemplateVersion 不可修改，新定义必须创建新版本。
- Measurement 必须有显式 value type；数值测量同时记录 unit。
- Attachment 内容在文件库，数据库记录 hash、版本和关联。
- 正式写入必须包含 Provenance 与 Audit；删除默认为 soft delete。
- Genetics v2 以 `GenotypeDefinition` 聚合一个或多个显式组件；`GenotypingRecord` 表示某只
  Animal 在某一时点对定义的检测状态。`GenotypingBatch` 通过显式关系表绑定一组记录、一份
  CSV/XLSX 原始结果和至少一张胶图；确认事务同时写入记录、关系、AnimalEvent、Audit 与
  Provenance，失败时保留草稿和附件但不产生部分正式记录。旧 `Genotype` 保留原语义，不在迁移中自动折叠或重写。
- BreedingPair 必须恰好一个雄性成员和至少一个雌性成员；同一动物不能同时进入冲突的活跃
  配对。MatingEvent 是人工确认的事实，不由预测或 AI 自动生成。
- Litter 与其存活 offspring Draft 在一个事务中创建；Draft → Animal 同时写入 Animal、
  父母 Pedigree、Registered/Born 事件、Audit 与 Provenance。
- Observation 的值类型由 ObservationDefinition 决定。初值与 Observation 原子创建；修订
  只追加 ObservationValueRecord 并推进 current version，不覆盖历史值。
- Participation 入组在与基因检测写入相同的 animal-scoped 锁/事务序列中，捕获每个
  GenotypeDefinition 当时最新的检测记录；快照之后不可被新检测回写。
- 项目成员可读取本项目已分配动物、这些动物所在笼位及本项目实验。项目笼位投影不得暴露
  同笼但未分配给该项目的动物或由它们推导出的占用统计。ProjectAdmin 可治理本项目成员和
  实验，但动物跨项目分配、笼位转移等实验室级动作仍由 LabAdmin/AnimalManager 执行。

## API boundaries

- REST 统一位于 `/api/v1`，错误使用稳定 code、message、details、request_id。
- 并发写入携带 revision，过期 revision 返回冲突而非覆盖。
- 长任务进入 Job：parse → validate → preview → confirm → transaction → report。
- Genetics、Breeding 与 Observation 分别使用 `/genotype-definitions`、`/genotyping-records`，
  `/breeding-*`、`/mating-events`、`/litters`、`/animal-drafts`，以及
  `/experiment-events`、`/observation-*` REST 资源；Tauri Gateway 提供同等应用语义。
- 普通导入只接受 Animal Registry 或 Experiment Measurement CSV/XLSX；基因鉴定使用独立的
  `GenotypingBatch` 工作流，不能绕过胶图、预览哈希和原子确认。普通结构化导出只生成 Animal
  Registry CSV/XLSX。这些都是受控数据产品，不是通用迁移协议。
- Snapshot 是 Lab 业务归档边界，包含新增 Genetics/Breeding/Observation 聚合、既有业务
  实体、Audit、Provenance 和附件；当前只生成和校验，不支持 restore/apply。
- MCP/AI 工具只能调用 application use case；禁止数据库连接、raw SQL、任意 HTTP、自动
  MatingEvent 和直接 Animal 修改。当前 MutationDraft 仅能提出结构化 Measurement 草稿。
- AI 自主度是绑定 conversation、user、project 与运行时 session 的持久授权，不是角色。
  有效能力取角色权限、项目范围、实验室上限、会话授权和外部 token scope 的交集。Ask 为
  默认；Auto 只放行受控读取、产物和可逆草稿；Full 需要 step-up，30 分钟空闲后失效。
  所有 AI 写入仍必须经过结构化 draft/preview/approval，不得把 Full 解释为最终签署权限。

## AI configuration and runtime boundaries

AI 配置按实验室治理、用户模型档案、不可变档案版本和独立凭据四层持久化：

- 实验室级只保存 AI 总开关、最大自主模式和允许的 Provider 出口。出口以“协议 + 规范化
  Base URL”登记；Environment Root 与 LabAdmin 不能读取、替换或调用用户凭据。
- 用户可以创建任意数量的模型档案，自由填写模型 ID，并选择
  `openai_chat_completions`、`openai_responses` 或 `anthropic_messages`。档案保存可变展示
  状态；每次影响调用语义的修改创建不可变版本。协议适配器负责在 Base URL 上拼接标准端点，
  页面和 Store 不按模型名推断或覆盖能力与参数。
- 凭据与档案版本分离。Server 使用部署 Master Key 以 AES-256-GCM 加密，并把 user ID、
  profile ID、profile version 与 Master Key version 绑定到 AAD；Desktop 为每个档案版本
  使用独立 OS keyring 项。协议或 Base URL 变化时必须提供新 Key；其他编辑留空 Key 表示
  保留现有绑定。API 只返回 `hasKey`。
- 用户显式指定默认对话档案和至多一个默认视觉档案。默认引用无效、档案停用或缺少密钥时
  都在 Provider 调用前失败，不会静默选择列表第一项。迁移生成的旧设置投影保留兼容读取，
  不再可安全续写的旧会话标为只读。

对话创建时绑定 `model_profile_id + model_profile_version`，同一会话不可改变这组身份。空会话
切换模型可原地生效；已有消息时切换会创建新会话，只保留项目范围和未发送输入，不复制消息、
工具结果、草稿或 Full 授权。档案归档后历史仍可读，但追加消息、工具结果或授权一律拒绝。

每次文本调用从所绑定版本构造 `AssistantRuntimeConfig`。`maxInputTokens +
maxOutputTokens` 必须不超过 `contextWindowTokens`。调用前的输入 Token 是明确标注的保守估算，
覆盖系统提示、历史、工具定义/结果与当前用户消息；超限时只从最旧的完整
user/assistant 历史轮次开始裁剪，tool call/result 组保持配对，当前问题绝不静默截断。响应
Trace 分开记录估算输入、裁剪原因与 Provider 返回的真实 usage。

Provider HTTP redirects 被禁用；云出口要求 HTTPS，只有 LabAdmin 精确登记的 HTTP 开发出口
可使用明文协议。出口匹配始终同时比较协议和规范化地址。内置厂商信息只作为非敏感建议，
不是模型 allowlist，也不提供共享账号。

## Vision routing and image evidence

- 当前对话模型声明视觉能力时直接接收图片；否则由默认或用户明确指定的视觉档案生成长度受限
  的结构化文本观察，再交给最终对话模型。没有有效视觉默认时必须要求选择，不得暗选首个档案。
- Trace 分别记录视觉阶段和最终阶段的档案 ID、不可变版本、用途、Token usage 与图片
  SHA-256；中转文本采用受控 JSON envelope，不能把视觉模型输出当系统指令。
- AI 图片只接受经过服务端重编码的 JPEG、PNG、WebP 或静态 GIF，执行大小、尺寸、像素和
  帧预算，并移除元数据。未经批准的图片、提取候选、Audit 与 Provenance 保持私有且不进入
  项目 snapshot。
- “录入实验数据”允许多图生成当前数据单元的候选草稿。拒绝可以恢复编辑；只有具备权限的
  人工批准才能在一个 Store 事务中创建 Observation、项目附件关系、领域关联、Audit 与
  Provenance。模型和 transport 都不能直接写入正式实验数据。

## Operational records

- 面向成员的“操作动态”只投影动物分配/移除、转移、实验入组、测量/样本等关键业务事件，
  同一 request 的批量动作聚合展示，不直接暴露原始 JSON diff 或内部 UUID。
- Audit 与 Provenance 是正式不可变记录，不能由保留策略或 root 清理动作删除。
- Server 技术访问日志与正式审计分表存储。默认最多保留 20,000 行且至少保留 30 天；自动
  清理和 Environment Root 的手动清理都只能删除“超过数量上限且已超过最短天数”的交集，
  手动清理必须先预览并用同一 policy revision 确认。Desktop 继续使用固定滚动日志文件，
  不把技术日志并入业务数据库或 snapshot。

## Runtime identity hierarchy

Server 的身份层级固定为：

1. **Environment Root**：由宿主机 `.env` 和部署生命周期管理；在应用权限上拥有 LabAdmin 能力，只有它能创建、修改、停用、降级或重置其他 LabAdmin。
2. **LabAdmin**：治理实验室业务与所有非 LabAdmin 账号，不能修改环境、代码、Environment Root 或同级 LabAdmin。
3. **ProjectAdmin**：只在获授权 Project 内拥有项目管理和业务权限，可管理该项目成员并授予
   ProjectAdmin；不得管理实验室账号或其他项目，且系统必须阻止移除最后一名有效项目管理员。
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

- Desktop：Windows Tauri WebView 安装包为正式本地交付目标；SQLite、附件、数据产物和非敏感 AI 配置位于同一个 active data root（默认是 OS application data，可由用户选择本机固定磁盘上的独立空目录），密钥位于 OS keyring。Desktop 不通过 VNC/noVNC、浏览器远程桌面或 Server Docker 交付。
- Server：PostgreSQL、附件 volume、加密 secret store；Axum 位于 HTTPS reverse proxy 后。
- V1 不做 Local Web、本地 Axum+SQLite 浏览器服务，也不做 Desktop 与 Server 的实时同步。当前 snapshot 用于版本化完整业务归档、离线留存与
  完整性校验，不提供自动合并、导入或恢复入口；CSV/XLSX Export 也不能替代部署备份。

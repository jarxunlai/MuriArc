# MuriArc 0.1.0 完整工作区、双运行形态与账号安全交付验收

本文件覆盖既有六阶段工作区、Genetics v2、Breeding、Observation、双运行形态、Server
账号安全，以及版本化多模型、对话切换、视觉路由和图片证据审批的最终交付边界与逐步验收
清单。默认验收服务地址为 `http://127.0.0.1:8787`；账号凭据和模型 Key 使用单独的本地
交付信息，不写入仓库。

## 1. Delivered scope

- `GenotypeDefinition` 支持 Diploid、Hemizygous、TransgenePresence、Conditional 多组件定义。
- `GenotypingRecord` 将 Expected/Confirmed/Rejected/Unknown 检测状态与 Animal 关联。
- `BreedingLine`、`Colony`、一雄多雌 `BreedingPair`、退役、`MatingEvent`、`Litter` 与
  `AnimalDraft` 已贯通 Domain、Application、SQLite、PostgreSQL、Tauri、REST 与 Vue。
- Draft → Animal 同时生成正式动物、父母 Pedigree、Registered/Born 事件、Audit 与 Provenance。
- `ExperimentEvent`、`ObservationDefinition`、动态类型 `Observation`、初值和追加式修订历史已贯通。
- Participation 入组时保存每个 GenotypeDefinition 当时最新的检测快照。
- Snapshot 已纳入全部上述研究实体，并保持确定性 ID 排序、manifest record count、附件与 SHA-256。
- Desktop/Tauri/SQLite 与 Server/Axum/PostgreSQL 保持单代码库，共享 Domain、Application、Store contract 和主体 Vue UI；Desktop 正式本地交付为 Windows Tauri WebView 安装包，不以 VNC/noVNC 作为部署方式，不新增认证表，每个 WebView 会话只显示一次无密码欢迎页。
- Server 启动从环境配置单事务同步唯一 Environment Root、LabAdmin membership 与 Argon2id Credential；冲突阻断启动，配置变化撤销 Root Session，Audit 不含密码/hash。
- 新成员使用临时密码并强制首次改密；自助改密保留当前 Session、撤销其他 Session，管理员重置撤销目标全部 Session/external token，并受 Root/LabAdmin 层级约束。
- 用户可创建多个版本化模型档案，自由填写模型 ID，并分别选择 OpenAI Chat Completions、
  OpenAI Responses 或 Anthropic Messages；默认对话/视觉档案显式配置，凭据按档案版本隔离。
- 会话绑定不可变档案版本，档案归档或旧会话无法安全绑定时保留历史读取但禁止继续发送；
  Ask、Auto、Full 同时显示请求模式和实验室上限裁剪后的实际模式。
- 图片可由对话模型直接处理或经明确视觉档案中转；实验数据多图识别只生成当前数据单元的
  私有候选，人工批准后才原子创建 Observation、附件关系、Audit 与 Provenance。

## 2. REST surface

所有路径位于 `/api/v1`，身份、Lab 和 Project 作用域由 Server 会话决定。

| 领域 | 方法与资源 |
| --- | --- |
| Genetics v2 | `GET/POST /genotype-definitions`；`GET /genotype-definitions/{id}`；`GET/POST /genotyping-records`；`GET /genotyping-records/{id}` |
| Genetics compatibility | `GET/POST /gene-loci`、`/alleles`、`/genotypes`、`/pedigrees` 及各自 `/{id}` |
| Breeding structure | `GET/POST /breeding-lines`、`/colonies`、`/breeding-pairs`；对应 `GET /{id}` |
| Breeding facts | `POST /breeding-pairs/{id}/retire`；`GET/POST /mating-events`；`GET/POST /litters` |
| Offspring drafts | `GET /animal-drafts`；`GET /animal-drafts/{id}`；`POST /animal-drafts/{id}/register` |
| Prediction | `POST /breeding-predictions`，只返回确定性孟德尔概率，不写数据 |
| Experiment events | `GET/POST /experiment-events`；`GET /experiment-events/{id}` |
| Observation definitions | `GET/POST /observation-definitions`；`GET /observation-definitions/{id}` |
| Observations | `GET/POST /observations`；`GET /observations/{id}`；`GET /observations/{id}/values`；`POST /observations/{id}/revisions` |
| Enrollment snapshot | 既有 Participation DTO 新增 `genotypeSnapshot`；创建 Participation 时由 Store 原子填充 |
| Data products | `POST /data/imports` 仅 Animal/Measurement；`POST /data/exports` 仅 Animal Registry；`POST /data/snapshots` 为 Lab 业务归档 |
| AI model profiles | `GET/POST /ai/models`；`GET/PUT /ai/models/{id}`；`POST /ai/models/validate`；`DELETE /ai/models/{id}/key`；`POST /ai/models/{id}/archive`；`GET/PUT /ai/models/defaults` |
| AI conversations | `GET/POST /ai/conversations`；`GET /ai/conversations/{id}`；`GET/PUT /ai/conversations/{id}/autonomy`；消息发送仍受绑定档案、项目和授权 preflight |
| AI image evidence | 私有图片流式上传/读取；`GET/POST /ai/extractions`；`GET /ai/extractions/{id}`；`POST /ai/extractions/{id}/approve` 或 `/reject` |
| Self-service account | `GET /auth/session`、`GET /auth/csrf`、`PATCH /auth/profile`、`POST /auth/password/change`、`POST /auth/logout` |
| User governance | `GET/POST /admin/users`；`PATCH /admin/users/{id}/profile`；`POST /admin/users/{id}/password-reset` 及状态/权限操作 |

Tauri 的 `LocalTauriGateway` 和 Server 的 `RemoteHttpGateway` 暴露相同前端操作；DemoGateway 用于
无后端浏览器测试，不代表持久化实现。

## 3. Data and AI boundaries

| 功能 | 当前支持 | 明确不支持 |
| --- | --- | --- |
| Import | Animal Registry CSV/XLSX；指定 Experiment 的 Measurement CSV/XLSX | 任意实体通用导入、Snapshot apply |
| Export | Lab/Project Animal Registry CSV/XLSX | Breeding/Observation/Audit/附件全量迁移 |
| Snapshot | 业务 JSONL、附件、manifest、record count、大小、SHA-256 | restore/apply、自动合并、实时同步、账号/session/secret 备份 |
| AI breeding | 基于可见数据进行分析、预测和建议 | 自动创建 MatingEvent、直接修改 Animal、绕过人工审批 |
| AI mutation | 结构化 Measurement 只能形成待审批草稿 | raw SQL、任意 HTTP、Breeding/Animal mutation operation |
| AI providers | 三种显式协议、自由模型 ID、版本化档案与本地 Mock 验证 | 静态模型 allowlist、按模型名覆盖参数、CI 调用真实厂商 |
| AI vision | 当前模型直接视觉或经明确视觉档案生成受控观察 | 静默选择第一个视觉模型、把视觉输出当系统指令 |
| Image data entry | 多图候选、编辑/拒绝/批准、事务性证据提升 | AI 直接正式写入、批准前进入项目 snapshot |

普通 Export 是用户选择的数据产品，Snapshot 是业务归档边界；二者都不能替代 PostgreSQL/SQLite
与附件目录的经演练部署备份。

## 4. Automated verification record

### 2026-07-24 multi-model compatibility addendum

阶段五候选源码在当前 feature worktree 内完成以下门禁；Cargo target 位于仓库外，Provider
测试只使用 Mock/本机 upstream，未调用真实厂商：

```text
git diff --check + cargo fmt --all -- --check                   PASS
Locked workspace metadata                                       PASS
Tauri Linux image strict Clippy, all targets/features            PASS
Rust workspace, all targets/features, disposable PostgreSQL 17   PASS
Rust workspace planned command + doc tests, fresh PostgreSQL 17  PASS
AI: 77 unit + 10 integration                                     PASS
Desktop: 66 tests                                                PASS
Server: 124 library + 2 binary                                   PASS
PostgreSQL: 2 migration + 6 contract + 2 integration             PASS
SQLite: 6 migration + 4 profile + 2 operation + 8 other tests    PASS
UI Vitest: 20 files / 158 tests                                  PASS
TypeScript vue-tsc                                               PASS
Remote and Local Vite production builds                          PASS
Playwright Desktop/Tablet/Mobile: 40 pass / 8 conditional skip   PASS
Branding integrity                                               PASS
pnpm high-severity audit against official registry               PASS
```

PostgreSQL 0027 与 SQLite 0025 的兼容迁移从前一版本 schema 增量应用并重放迁移账本：无效的
归档、软删除、跨 owner、缺失版本或非视觉默认引用被清空，每个受影响 defaults 行 revision
只推进一次；有效默认的 revision/时间戳不变。旧 `ai_provider_settings` 密钥版本与密文、
档案/不可变版本、Server profile secret、Desktop keyring reference、legacy 会话和固定版本会话
均保持可读。归档档案的历史仍可读取，但 SQLite/PostgreSQL 同一 Store contract 都在写前拒绝
追加 turn、tool run 或 autonomy grant。

Server 额外验证 Master Key v1 → v2 只有“旧 runtime 解密，再由新 runtime 显式重加密”路径
成功；错误密钥、旧密文配新版本以及新密文配旧 runtime 都 fail closed。Desktop 验证 schema
v1 JSON 和单一旧 Keyring 项幂等投影为文本/视觉档案版本，并保留旧文件/旧 Keyring 项。UI
验证失效默认不会回退首个档案，`legacy_model_unknown`、`model_archived` 和
`model_unavailable` 三类历史均可读但即使程序化调用 `send()` 也不会请求 Provider。

8 个 Playwright skip 来自场景中明确的设备条件，不是浏览器、依赖或环境缺失。生产构建仍报告
已知的主入口 chunk 大于 500 KiB 警告；它不影响本轮功能门禁，继续列在 Known limitations。
Windows 安装包和原生运行时不能由 WSL/Linux 结果替代：阶段五提交后必须生成写明 40 位 commit
的仓库外交接，Draft PR 的 Windows Tauri smoke 必须通过，安装/Keyring/升级/release bundle
人工结果按 `docs/DESKTOP_DELIVERY.md` 记录，未执行项不得标记为 PASS。

以下门禁在 2026-07-21 的交付工作树上实际执行：

```text
cargo fmt --all -- --check                                        PASS
Non-Desktop workspace strict Clippy                               PASS
Server tests: 85 passed                                           PASS
PostgreSQL 17 Store runtime: 4 contracts + 1 research test        PASS
PostgreSQL fresh/reapply and 0017 -> 0023 migration tests         PASS
Tauri Linux check image: strict Clippy + 28 tests                 PASS
UI Vitest: 14 files / 91 tests                                    PASS
TypeScript vue-tsc + remote Vite production build                 PASS
Playwright: desktop/tablet/mobile 34 passed / 2 expected skips    PASS
Compose Root/account lifecycle and persistence smoke              PASS
Deployed Chromium remote login/account navigation smoke           PASS
Credential redaction scan: JSON responses/Audit/Server logs       PASS
Real HTTP research smoke: full chain + Snapshot download          PASS
Snapshot source diagnosis: 57 checked / 0 failed                  PASS
```

PostgreSQL contract 使用无挂载卷的临时 PostgreSQL 17 实例并设置真实
`MURIARC_TEST_DATABASE_URL`，不是“环境变量未设置”的跳过结果。SQLite 与 PostgreSQL 都覆盖：

- Genetics v2、Breeding、Observation 的读写与 Audit；
- Litter + Draft、Draft → Animal、Observation + initial value、Observation revision 的原子性；
- recorded_by 与 human audit actor 不一致时拒绝且无部分写入；
- Participation genotype snapshot；
- PostgreSQL `TIMESTAMPTZ` 微秒精度下的跨 adapter round trip；
- `user_credential`、`auth_session`、`external_token` 与 `revoke` 安全生命周期 Audit 的兼容读取，
  未知 Audit 枚举值会返回带具体列名和原始枚举错误的诊断，而不是模糊的 JSON 行列错误。

PostgreSQL migration 门禁从空库真实执行 `0001`–`0023`，再次执行验证幂等，并从 `0017` 状态增量
应用到当前版本；`0018_auth_credential_lifecycle.sql` 升级后既有 password hash 保留，
`must_change_password=false`、`revision=1` 默认值符合升级契约。后续迁移依次覆盖 Provider endpoint、
项目动物关系、AI 自主授权、Server 技术日志与 Genetics v2 检测记录生命周期；SQLite 对应业务链为
`0018`–`0021`。宿主 Linux 缺少 Tauri 的 GTK/WebKit 系统开发库，因此 Desktop
Clippy/test 使用隔离的 Tauri Linux 工具镜像执行；strict Clippy 与 28 个测试均通过，不是跳过或以
非 Desktop 结果代替。

最终服务还连续执行了两轮真实 HTTP smoke。第二轮 Snapshot 已包含上一轮 logout 产生的 `revoke`
Audit；两轮都完成登录、Project、Animal、Genetics v2、Breeding、Draft → Animal、遗传预测、
Experiment/Participation 快照、ExperimentEvent、Observation 初值与修订、Snapshot 创建与下载，且下载
字节数与元数据一致。

账号与部署验收使用 Compose 的真实 PostgreSQL 17 volume 和 release Server：完成 health、Environment
Root 登录、临时密码首次改密门禁、自助改密、其他 Session 撤销、external token 创建与访问、管理员
重置以及目标全部 Session/token 撤销。随后修改临时 Root 环境密码与名称并仅强制重建 Server；数据库
容器 ID 和 `muriarc_acceptance_postgres_data` volume 均保持不变，普通成员仍存在，新密码生效、旧密码
和旧 Root Session 失效。以相同配置再次重建 Server 后 Root Session 保持有效，Root credential update
Audit 计数不变，证明同步幂等。真实 Chromium 又完成远程登录、设置页“由部署配置管理”和成员管理导航；
最终扫描 JSON 响应、Audit 与三轮 Server 日志，未发现测试密码、Argon2id hash、Session/CSRF 或
external token 明文。验收停止时只执行 `docker compose down`，未使用 `--volumes`。

## 5. Manual acceptance checklist

### A. 双运行形态

1. 以 Windows Desktop 安装包或等价的 Tauri 本地启动方式打开，确认出现原生 MuriArc 窗口；窗口外不应出现浏览器地址栏、noVNC toolbar、远程桌面边框或额外缩放层。
2. 以 Desktop/local gateway 启动，确认显示 Lab、LocalOperator 和“进入本地空间”，页面没有密码字段，
   并明确提示它不是安全锁。
3. 进入后刷新页面，确认同一 `sessionStorage` 生命周期不重复显示；关闭并新建 WebView 会话后再次显示。
4. 新建笼位、动物并上传附件，关闭后重新启动，确认 SQLite 数据、附件和数据任务文件仍位于 OS application data 管理的本地目录。
5. 以 remote gateway 打开 Server，确认显示正式登录页，不出现 Desktop 欢迎页。
6. 登录普通账号，确认笼位、动物、繁育、实验、数据中心和设置页可访问，刷新后 HttpOnly Session 仍有效。

### B. Environment Root 与账号安全

1. 使用宿主机交付的 Root 凭据登录，确认成员页显示 Environment Root/LabAdmin 徽标；设置页仅说明
   “由部署配置管理”，不存在修改 Root profile/password、停用、降级或重置操作。
2. 保持 `MURIARC_ROOT_USER_ID` 不变重启 Server，确认同步幂等；在受控验收配置中修改 Root
   邮箱、名称或密码后仅替换 Server，确认新身份生效、旧密码失效、旧 Session 被撤销。
3. 由 Root 创建 LabAdmin；再以普通 LabAdmin 登录，确认不能治理 Root 或同级 LabAdmin，但可治理
   ProjectAdmin、AnimalManager、Editor 和 Viewer。
4. 创建普通成员并设置临时密码。首次登录后应只显示专用改密页；直接输入业务 URL、管理 token
   或使用 bearer 均返回 `password_change_required`。
5. 输入至少 8 个字符的新密码，观察“过短/弱/中/强”建议但不要求字符组合；成功后业务导航恢复，
   当前 Session 保留，其他 Session 失效，所有密码字段被清空。
6. 在“账号与安全”修改自己的显示名称和密码；确认不能查看现有密码，改密后其他 Session 被撤销。
7. 管理员修改下级邮箱/名称并执行强制密码重置；确认需要管理员当前密码与 credential revision，
   目标全部 Session/external token 失效，下一次登录再次强制改密。
8. 检查 Root 同步、自助改密、资料修改、管理员重置和凭据撤销 Audit：operation code 稳定，内容不含
   明文密码、Argon2id hash、Session 或 token 明文。

### C. Genetics v2

1. 进入“繁育管理 → 基因定义与检测”。
2. 创建一个 locus 和至少两个 allele。
3. 创建含一个 Diploid 组件的 GenotypeDefinition；再创建多组件定义，确认组件顺序和模式保留。
4. 为一只动物记录 Confirmed 检测，必须填写检测时间；确认列表显示状态、方法和时间。
5. 检查旧谱系/旧 Genotype 页面仍可读取，未被新定义替换。

### D. Breeding

1. 创建 BreedingLine 并关联 GenotypeDefinition，再创建 Colony。
2. 创建一个包含恰好一只雄鼠和至少一只雌鼠的活跃配对；尝试缺少雄鼠或重复活跃成员应失败。
3. 人工创建 MatingEvent，选择配对内的一只雌鼠。
4. 创建 Litter 和存活 Draft；确认 Draft 数量与 `size_alive` 一致。
5. 将一个 Draft 登记为正式 Animal，确认新动物可在 Registry 搜索，并有父、母两条 Pedigree 与
   Registered/Born 时间线事件。
6. 退役配对，确认成员离开时间和 revision 更新；再次退役应冲突。
7. 运行遗传预测，确认概率和为 1，且预测不会新增 MatingEvent 或动物。

### E. Experiment observations and enrollment snapshot

1. 为已有 Experiment 创建 ExperimentEvent。
2. 创建 Number ObservationDefinition 并填写单位；无单位应被拒绝。
3. 对已入组动物录入 version 1，随后修订为 version 2；确认历史值都可见且 current version 为 2。
4. 创建 Immutable 定义并录入初值；尝试修订应被拒绝。
5. 在动物已有检测记录后新建 Participation，记录其 `genotypeSnapshot`。
6. 再为动物新增检测记录，刷新既有 Participation，确认旧快照不变化；新 Participation 可捕获新记录。

### F. Import, export and snapshot

1. 数据中心仅显示“动物登记/实验测量”两种导入类型；文件必须先预览、重映射和确认。
2. “导出动物 Registry”下载 CSV/XLSX，确认界面没有暗示这是全实体迁移包。
3. “创建完整归档快照”触发下载，确认界面明确标注当前不可 `restore/apply`。
4. 不删除现有数据库或 volume；部署恢复演练使用 `docs/DEPLOYMENT.md` 的数据库+附件流程。

### G. AI models, conversations, vision and audit

1. 新建三个模型档案，协议分别为 `openai_chat_completions`、`openai_responses` 和
   `anthropic_messages`，使用任意自由模型 ID。本机 Mock upstream 必须捕获各自标准端点、
   鉴权 header、请求体和 usage；错误响应映射稳定，测试不得调用真实厂商。
2. 对同一用户设置默认对话档案和至多一个默认视觉档案；删除、归档、跨 owner 或非视觉默认
   引用后刷新，确认引用被 fail-closed 清除或拒绝且不会暗选首项。未保存表单可验证，验证不
   落库也不是保存前置条件。
3. 编辑名称、模型 ID 或 Token 参数且 Key 留空，确认现有绑定保留；修改协议或 Base URL 时
   必须重新输入 Key。读取接口与页面只显示 `hasKey`，任何日志、Audit、错误、snapshot 和
   前端状态都不得出现 Key。
4. 创建空会话并切换模型，确认原会话绑定更新；发送消息后再切换，确认经提示创建新会话，
   只保留项目范围和未发送输入，不继承消息、工具结果、草稿或 Full 授权。旧会话、归档档案
   会话和 legacy read-only 会话保持可读但不能继续发送。
5. 新会话首轮选择 Full：Server 必须用当前密码完成 session-bound step-up，Desktop 必须用
   本次启动声明和独立 Session UUID；取消或验证失败时 Mock upstream 收到零请求。实验室上限
   继续裁剪实际模式，界面同时展示请求模式与实际模式。
6. 上传便携格式图片。当前档案支持视觉时直接处理；不支持时选择视觉档案中转。清除视觉默认
   后必须显式选择，不能使用列表第一项。Trace 分别核对视觉/最终档案版本、用途、Token usage
   与图片 SHA-256。
7. 上传多张实验数据图片，检查预览和移除；AI 只生成当前数据单元候选。拒绝后可以恢复编辑；
   批准前图片、草稿及相关 Audit/Provenance 不进入项目 snapshot，批准后 Observation、项目
   附件关系、领域关联、Audit 与 Provenance 同时出现。制造第二张证据失败，确认整次回滚。
8. 验证单图 10 MiB、尺寸、总像素、帧数和请求总预算；EXIF 等元数据必须被重编码移除。非
   JPEG/PNG/WebP/静态 GIF、伪造 MIME 或解码炸弹必须在 Provider 调用前拒绝。
9. 从旧用户设置和旧会话执行增量升级，确认旧模型/参数、凭据版本和历史不丢失；执行 Master
   Key 轮换时确认旧密文先解密、全部重加密后才推进版本。轮换失败必须保留旧数据并阻断，不能
   清空凭据要求用户无报告重输。
10. 让 AI 建议繁育方案；确认只返回分析/预测/建议。尝试要求 AI 创建 `MatingEvent`、更新
    Animal 或直接批准图片候选，应因固定工具面和人工审批边界而无法执行。

### H. Desktop Windows 本地交付

1. 正式发布包必须来自 Tauri bundle；`pnpm run dev`、Vite preview、VNC/noVNC 和远程桌面会话不能替代 Desktop 安装包验收。
2. 在干净 Windows 机器上安装后启动，确认 WebView2 runtime 策略有效：应用可正常显示或给出明确安装提示，不要求用户连接 noVNC。
3. 为多个档案保存独立 Key，确认 Key 不进入业务 snapshot、日志、审计或前端状态；重启后各
   档案 Keyring 状态不串用，清除当前档案 Key 不影响其他档案。
4. 卸载、重装或升级不得静默删除 application data；清除本地数据必须是用户通过 OS 或明确产品流程主动执行。
5. 用固定 commit 从旧配置和旧 Keyring 状态升级，确认旧项保留；归档档案后历史会话可读但
   不能继续发送。Debug `--no-bundle` 可保留控制台，正式 release bundle 启动不应伴随额外
   PowerShell 窗口。

## 6. Known limitations

- Snapshot restore/apply 尚未实现；Snapshot 也不包含账号、Membership、session、token、AI secret、
  Job 或部署配置。
- 普通 Import/Export 没有通用资源选择器，支持范围严格限于上表。
- V1 不提供 Local Web、本地 Axum+SQLite 浏览器服务；Desktop 与 Server 不实时同步，Snapshot 也不是同步协议。
- Desktop 欢迎页不是登录、磁盘加密或操作系统权限边界；需要设备安全时必须依赖 OS 账号与磁盘加密。
- VNC/noVNC 只能作为临时远程预览或人工协助工具，不能作为正式 Desktop 部署方式。
- Environment Root 明文密码按确认方案位于宿主机 `.env`；权限 600 不能防护宿主机管理员、Docker daemon/inspect、进程环境采集或未加密备份。
- AI 没有 Breeding 写工具；这是安全边界，不是遗漏。实际配对和交配必须人工确认。
- 当前 Web 主入口 chunk 有大于 500 KiB 的构建告警，但不阻断功能；后续可继续按页面/组件拆分。
- Windows Tauri WebView 安装包是首发 Desktop 本地交付平台；macOS 发布前仍需真实设备验证。

# MuriArc 0.1.0 完整工作区、双运行形态与账号安全交付验收

本文件覆盖既有六阶段工作区、Genetics v2、Breeding、Observation、双运行形态与 Server 账号安全的最终交付边界和逐步验收清单。默认验收服务地址为 `http://127.0.0.1:8787`；账号凭据使用单独的本地交付信息，不写入仓库。

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

普通 Export 是用户选择的数据产品，Snapshot 是业务归档边界；二者都不能替代 PostgreSQL/SQLite
与附件目录的经演练部署备份。

## 4. Automated verification record

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

### G. AI and audit

1. 在全新 Server 数据卷启动普通 Compose 部署；确认 diagnostics 为 `runtimeConfigured=true`、`labEnabled=true`、`userEnabled=true`、`providerPresetsAvailable=true`，无个人 Key 时状态为 `waiting_for_personal_api_key`。确认只存在 Server/PostgreSQL 容器，没有 Ollama 容器、模型下载或外部 AI 请求。
2. 分别以 Root、Editor、Viewer 保存 DeepSeek/智谱 GLM/Moonshot-Kimi、不同模型和不同测试 Key；确认读取接口只返回 `hasKey`，Root 配置不出现在其他用户页面。Fake upstream 必须分别捕获预期 Authorization、model、`max_tokens` 与 Temperature。未配置 Key 的用户不能触发 upstream。
3. 修改 Root 模型与 Token 参数，刷新设置页后再次调用；确认参数保持并实际进入 Provider 请求，Editor/Viewer 不变。切换 Provider 或 Base URL 且不输入新 Key 时，旧 Key 必须清除而不是复用。
4. 验证 `maxInputTokens + maxOutputTokens <= contextWindowTokens`；超限 UI 禁止保存。构造历史与工具结果压力，确认只裁剪最旧完整历史、tool call/result 不拆分、当前问题不截断，Trace 区分估算输入、裁剪原因与 Provider 真实 usage。
5. 让 AI 建议繁育方案；确认只返回分析/预测/建议，没有创建交配事件按钮或自动结果。尝试要求 AI 直接创建 `MatingEvent` 或更新 Animal，应因固定工具面而无法执行。
6. 在审计/来源页面抽查 AI 设置修改及 Definition、检测、配对、窝次、Draft 注册、Observation 初值/修订，确认 actor、source、entity ID 与 revision 可追溯；任何日志、Audit、错误与前端状态都不得包含 API Key。

### H. Desktop Windows 本地交付

1. 正式发布包必须来自 Tauri bundle；`pnpm run dev`、Vite preview、VNC/noVNC 和远程桌面会话不能替代 Desktop 安装包验收。
2. 在干净 Windows 机器上安装后启动，确认 WebView2 runtime 策略有效：应用可正常显示或给出明确安装提示，不要求用户连接 noVNC。
3. 保存 AI Provider 设置后确认 API key 不进入项目数据库、日志、审计、快照或前端状态；清除 key 后重启仍保持清除状态。
4. 卸载、重装或升级不得静默删除 application data；清除本地数据必须是用户通过 OS 或明确产品流程主动执行。

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

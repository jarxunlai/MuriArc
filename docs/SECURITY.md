# Security

## Trust boundaries

- Browser、外部 REST/MCP 客户端和模型输出均视为不可信输入。
- Server Web 使用持久安全 session；外部 REST/MCP 使用绑定用户和 scopes 的可撤销 token。
- 本地模式不创建账号、密码或认证表；每次启动的“进入本地空间”只是操作者确认而非安全锁，但所有写入仍记录 LocalOperator。

Server 普通用户密码以 Argon2id PHC hash 保存。随机 session、会话派生 CSRF 和外部 token
只将 SHA-256 digest 写入 PostgreSQL；明文 session 仅存在于 HttpOnly cookie，
CSRF 可在有效会话内安全恢复但不持久化明文，外部 token 仅在创建响应中显示一次。身份验证时实时读取 User、Credential 与 Membership，
因此 suspended、软删除、角色调整、强制改密、token 到期或撤销立即生效。管理员和 API 永远不能读取任何用户的现有密码或 Argon2id hash。

## Environment Root and account lifecycle

- Server 必须配置 `MURIARC_LAB_NAME`、`MURIARC_ROOT_USER_ID`、`MURIARC_ROOT_USER_EMAIL`、`MURIARC_ROOT_USER_NAME` 和 `MURIARC_ROOT_PASSWORD`；`MURIARC_LAB_ID` 仍是 tenant ID。
- Root 明文密码按产品决策保存在宿主机 `.env`，真实文件必须 Git ignore 且建议 `chmod 600 .env`。权限 600 只能限制普通本机用户：宿主机管理员、Docker daemon/`docker inspect`、进程环境采集和未加密备份仍可能看到它。备份 `.env` 时必须加密并限制访问。
- Server 每次启动在 PostgreSQL 单事务和 advisory lock 内核对 Lab、Root User、唯一 LabAdmin membership 与 Argon2id credential。Root 邮箱或名称变化会同步；密码与 hash 不匹配会更新 hash、`password_changed_at` 和 revision；身份或凭据变化会撤销 Root Session。重复邮箱、跨 Lab User ID、软删除身份/Root membership 或不支持的 hash 会阻止启动。
- Root 密码只能由 Environment Root 操作者编辑 `.env` 后重启修改。应用内禁止修改 Root profile/password，禁止停用、降级、重置或撤销 Root membership。
- Root 是配置声明的唯一 User ID，不新增第二套 Permission 枚举。只有 Root 能治理 LabAdmin；LabAdmin 只能治理非 LabAdmin 账号；ProjectAdmin 仅治理获授权项目。
- 新账号只接受临时密码并设置 `must_change_password=true`。强制期只开放登录后的 Session/CSRF 查询、退出和自助改密；业务 API 与 external bearer token 返回稳定的 `password_change_required`。
- 密码验收仅要求至少 8 个 Unicode 字符、最多 1024 bytes、无控制字符且新旧不同；不强制字符组合，也不定期过期。前端“弱/中/强”只是建议。
- 自助改密撤销除当前 Session 以外的其他 Session。管理员重置设置新临时密码、强制下次改密，并撤销目标全部 Session 与 external token；每次凭据与撤销写入使用稳定 operation code 和脱敏 Audit。

## Required controls

- 密码使用 Argon2id；生产 cookie 默认设置 Secure、HttpOnly、SameSite=Strict。
- 所有 cookie-auth mutation 强制验证 `X-CSRF-Token`；bearer token 不从 cookie 读取，避免混淆代理问题。
- 页面刷新后只可通过有效 HttpOnly session 调用安全的 `GET /api/v1/auth/csrf` 恢复 CSRF；该端点拒绝 bearer 身份并返回 `Cache-Control: no-store`。
- 登录失败统一返回安全错误，不区分未知 email、错误密码、停用或删除账号。
- 持久外部 token 具有 scopes、到期时间和撤销时间；有效权限始终为用户实时角色与 scopes 的交集，数据库不保存明文 token。
- AI key：Desktop 为每个模型档案版本使用独立 OS keyring 项；Server 优先使用环境注入的
  32-byte Master Key，未注入时在数据卷的 `secrets/ai-master-key` 首次生成并跨重启复用。
  Server 以 AES-256-GCM、随机 nonce 和绑定 user ID、profile ID、profile version、
  Master Key version 的 AAD 独立加密；PostgreSQL 不保存 Master Key。
- `MURIARC_AI_MASTER_KEY` 或生成的 key 文件是部署级密钥材料；生成一次后保持稳定并纳入受控备份。文件无效或不可写时启动失败。只有在完成所有既有用户 AI 凭据重新加密后才递增 `MURIARC_AI_MASTER_KEY_VERSION`。
- API 只返回 `hasKey`。Root/LabAdmin 只能管理实验室总开关、自主度上限和按“协议 +
  规范化 Base URL”登记的非敏感出口，不能读取、解密、替换或调用其他用户的 Key、模型、
  Base URL 和 Token 参数。档案设置写入要求 actor user ID 与 owner user ID 相同。
- AI Key 不进入业务 snapshot、前端状态、日志、审计或错误响应。协议或 Base URL 改变时
  必须提供新 Key；其他档案参数修改时 Key 留空表示保留。清除或归档只影响当前用户，且不会
  删除旧档案版本、旧密钥行、旧配置文件或旧 keyring 项。
- 无有效默认档案、档案已归档、会话是 legacy read-only、档案版本不属于当前用户或缺少
  Key 时均在任何 Provider 请求前失败。历史会话仍可读，但不得追加消息、工具结果或授权。
- 内置厂商信息只提供非敏感建议，不构成模型 allowlist。云出口必须 HTTPS；只有 LabAdmin
  精确登记的 HTTP 开发出口例外。Provider redirects 被禁用，出口校验始终同时比较协议与
  规范化地址。
- AI Token 预算强制满足 `maxInputTokens + maxOutputTokens <= contextWindowTokens`。输入估算明确标记为 estimate；超限只裁剪最旧完整历史轮次并保持 tool call/result 配对，不截断当前问题。Provider 返回的真实 usage 与估算值分开记录。
- Lab-wide AI 会话只读；产生写入草稿前必须显式绑定 Project。客户端声明的
  step-up 状态不受信任，外部 token 不得修改 AI 设置或代替研究者审批。
- AI 对话授权默认 Ask，可按对话提升为 Auto 或 Full；它不是可继承的用户角色。Server 的
  Full 必须重新验证当前用户密码并绑定当前 session，30 分钟无使用即失效；Desktop 必须在
  当前本地会话明确声明。实验室 AI 设置给出最大模式，外部 REST/MCP 永远封顶 Ask。
- 当前模型具备视觉能力时可直接处理图片；否则只能由默认或用户明确选择的视觉档案生成受控
  文本观察，再交给对话模型。系统不得静默使用模型列表第一项。两个阶段分别记录档案版本、
  用途、Token usage 和图片 SHA-256，视觉输出不能提升为系统指令。
- AI 图片上传只接受服务端重编码后的 JPEG、PNG、WebP 或静态 GIF；单文件限制 10 MiB，
  同时强制尺寸、总像素和帧预算并移除 EXIF 等元数据。对 Provider 的 Base64 载荷还受请求
  总预算限制，不能用多图绕过单文件或请求上限。
- 图片、提取候选及其 Audit/Provenance 在人工批准前属于创建者私有数据，不进入项目
  snapshot。AI 只能生成当前数据单元的候选；批准必须由有权限的研究者触发，并在同一事务中
  创建 Observation、项目附件关系、领域关联、Audit 与 Provenance。失败必须整体回滚。
- 无论 AI 模式如何，正式签署、动物转移/死亡、删除/批量导入、权限与账号治理、root 日志
  清理以及繁育科学事实都是硬边界；模型只能准备草稿、解释或引导人工进入专用工作流。
- 附件名称不得决定磁盘路径；使用 UUID/hash，阻止目录穿越。
- 查询 DSL 使用资源、字段、操作符 allowlist，并限制分页和执行成本。
- MCP 仅接受带 AI scopes 的外部身份，普通 Web session 不可隐式升级；浏览器 `Origin` 默认拒绝并只支持精确 allowlist。
- MCP 首版只暴露固定只读领域工具，不接受任意 SQL 或任意 HTTP 请求。
- 普通 JSON API 请求体显式限制为 1 MiB，MCP 限制为 128 KiB；CSV/XLSX 导入使用已实现的专用流式上传端点，单文件独立限制为 32 MiB。
- 旧持久管理员 bootstrap seed 已从生产启动面移除；升级部署必须显式提供新的 Root 环境变量，不能静默复用遗留 bootstrap 密码。
- 可选 bootstrap bearer 只供受控预览，正常运行应留空；它不创建持久管理员、不替代 Root、持久账号或可撤销 external token。
- 高风险操作（删除、批量导入、权限、迁移）必须加强确认。
- 科研测量由 AI 提取时先进入 draft，只有授权研究者可签署为正式记录。
- Master Key 轮换必须先用旧版本成功解密并将所有现有凭据重新加密到新版本；只修改版本号、
  丢失旧密钥或对生产数据库降级都必须阻断。数据库升级只允许通过新的向前修复迁移，不修改
  已执行迁移，也不以清理旧表、旧设置或旧 Keyring 项作为升级步骤。
- 项目成员只能读取显式分配给项目的动物及相应实验；项目笼位视图必须过滤同笼的其他项目
  动物。ProjectAdmin 的成员治理只作用于当前项目，不能查看凭据状态、实验室角色或其他项目
  membership，并且不能移除最后一名有效 ProjectAdmin。
- 技术访问日志不记录请求体、query、密码、token、AI key 或动物业务内容。只有 Environment
  Root 可更改保留策略、预览并确认清理；清理本身必须写入不可删除的正式 Audit。

## Reporting

请不要在公开 Issue 中提交真实动物数据、数据库、密钥或附件。安全问题应通过仓库维护者提供的私下渠道报告。

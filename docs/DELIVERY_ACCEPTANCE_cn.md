# 交付验收

> [English](DELIVERY_ACCEPTANCE.md) | 简体中文

## 状态与证据规则

本文列出已实现产品范围和开发/人工验收证据。MuriArc 当前仍是 `0.1.0 / preview_epoch_0`，本文任何结果都不得解释为正式 `1.0.0 / E0001` RC PASS。

自动门禁、dirty main 开发服务、unsigned Tester 包或源码 Compose 都可以作为开发证据，但不是最终制品验收。

## 已交付范围

- 动物、笼位、生命周期、转笼、项目分配、附件、Audit 与 Provenance。
- 结构化 Genetics definition/record 和带证据 genotyping batch。
- 繁育品系、Colony、一雄多雌配对、退役、交配事件、窝次、AnimalDraft 与 Pedigree。
- 版本化实验模板、Experiment、Cohort、Participation、入组基因型快照、Procedure、typed Observation、Measurement 与 Sample。
- 有边界动物/测量 Import、作用域 Animal Registry Export 与 checksum 业务 Snapshot。
- Desktop SQLite/Tauri 与 Server PostgreSQL/Axum 共享 Application/Core/Store contract 和 Vue 行为。
- Server 持久账号/角色、Environment Root、Session/CSRF、可撤销 Token 与技术日志保留。
- 用户隔离、版本化多 Provider AI 档案、会话、引用、受控工具、自主限制、视觉路由、私有图片与人工批准候选。
- 兼容身份、generation manifest、Write Lease、Upgrade Engine、签名交付合同与 fail-closed RC definition。

## 自动门禁

影响交付的变更在准确 clean commit 上运行适用门禁。

### Rust 与数据库

- migration checksum 不可变与 locked Cargo metadata；
- format 与 Clippy 零 warning；
- Core/Application/AI/data/import/snapshot tests；
- SQLite 与真实 PostgreSQL 17 Store contract；
- Server account/API/MCP/AI tests；
- Upgrade、delivery、release-evidence 与 verifier tests；
- fresh、幂等、增量、中断/resume 和无残留测试数据库。

PostgreSQL 因缺配置而 skip 不算 PASS。

### UI 与 Desktop

- 品牌一致性；
- 依赖 high-severity audit；
- Vue 单测/typecheck；
- remote/local production build；
- Desktop/Tablet/Mobile Playwright；
- Windows 缺 updater key 负向测试、Desktop strict Clippy/tests 与 Tauri smoke build。

### Container 与文档

- Compose config、image build、health、持久登录与干净 teardown（不删除持久验收数据）；
- 双语文件名/状态合同和 Markdown 本地链接；
- 过期来源声明与敏感/生成文件扫描。

## 人工验收

使用合成数据和一次性账号/环境；报告中不复制访问凭据。

### A. 双运行形态

1. Desktop 打开原生 Tauri 窗口、使用 SQLite，并在没有 Server 时工作。
2. Server 通过 Axum/PostgreSQL 提供响应式 UI、登录、CSRF、退出和角色。
3. 两种形态在支持范围内行为一致，同时保留各自身份/安全模型。

### B. 账号与隔离

1. 核对 Environment Root，并在密码轮换后确认旧 Session 撤销。
2. 验证 Root/LabAdmin/ProjectAdmin/AnimalManager/Editor/Viewer 边界。
3. 验证停用、软删除、强制改密、外部 Token 过期/撤销和项目隔离。
4. 确认响应/日志/Audit 不暴露密码、hash、Cookie、CSRF、Token、Key 或 object path。

### C. 动物、遗传与繁育

1. 登记动物/笼位；按 revision/capacity 转笼并生成可审计生命周期事件。
2. 分配动物到项目，确认其他项目成员看不到同笼未分配动物。
3. 创建多组件 genotype definition/record，旧/未知值保持显式。
4. 创建繁育配对、交配、窝次、offspring draft，并原子登记 Animal、双亲和 Provenance。
5. 创建/确认/拒绝/void 带证据 genotyping batch，核对附件关系。

### D. 实验与记录

1. 发布版本化模板并拒绝修改已发布版本。
2. 动物入组后，后续基因鉴定不能回写当时 genotype evidence snapshot。
3. 创建 Procedure、typed Observation、Measurement draft/signature、Sample 与 Attachment。
4. 验证 immutable/mutable/versioned Observation policy 与历史值。

### E. 数据操作

1. Preview 动物/测量 Import，覆盖 field mapping、歧义、单位、重复和项目作用域。
2. 合法 Import 原子确认；冲突/拒绝输入无部分状态。
3. 导出作用域 Animal Registry，验证 spreadsheet formula neutralization。
4. 创建/验证业务 Snapshot，确认排除私有 AI operation 与账号秘密。
5. Snapshot 不提供通用 restore，Import/Export 不标为 migration。

### F. AI

1. 为至少两个用户创建独立档案，证明 profile/model/参数/secret 隔离。
2. 只使用 mock upstream 验证 OpenAI Chat Completions、Responses、Anthropic mapping。
3. 验证无 Key、归档模型、stale default、legacy read-only 会话、timeout、output limit 与脱敏 Provider error。
4. Ask/Auto/Full 不能超过人的权限或敏感动作禁止项。
5. 数据读取有界并带引用；写入成为可审阅草稿，不直接产生科研事实。
6. 上传/净化私有图片，验证直接视觉/显式中转，并由人 reject/approve candidate。

### G. Desktop 交付

1. 在一次性 Windows 账号/VM 从准确 clean GitHub commit 构建。
2. 验证数据根迁移、重启、完整性和磁盘缺失 fail closed。
3. 验证 OS-keyring 隔离，SQLite/备份/报告中无 Key 字节。
4. 验证旧数据更新、中断/resume、首次写入前转交已验证旧 executable，以及首次写入后拒绝降级。

### H. Server 交付与公网 Profile

1. 从最终签名 package 分别验证 Native/systemd 与 Managed Compose。
2. 联合备份并实际隔离恢复，运行 Candidate 七层。
3. 验证 drain/freeze、原子激活、只读 gate、新 Write Lease 和回滚边界。
4. Cloudflare staging 验证直接 Origin 拒绝、HTTPS/Cookie/CSRF、登录退避、WAF/限流/cache、95 MiB edge 与外部 API 双控制。

## 已知限制

- 当前仓库尚未完成真实 `1.0.0 / E0001` RC。
- 业务 Snapshot 没有通用 restore/apply。
- 普通 Import/Export 有意保持窄范围，不是数据库迁移或同步。
- macOS 正式打包/验收未完成。
- Cloudflare no-MFA Profile 仍有账号接管风险，不能描述为 MFA。
- CI 不使用真实 Provider Key；项目所有者可在私有环境选择性测试真实 Provider。

## 正式 1.0 RC

正式 RC 使用最终签名 Native/systemd、Managed Compose、Windows Desktop 制品及同一个 `artifact-lock.json`/Release Manifest。同一 digest 生成 E0001 SQLite/PostgreSQL fixture，并完成完整历史、恢复、故障注入、首次写入、签名攻击和 Cloudflare staging，要求 `FAIL=0`、`SKIP=0`。

只有这批不重建的制品可以发布为 `v1.0.0`；Tester prerelease 和 dirty-main 开发服务永远不能满足该门禁。

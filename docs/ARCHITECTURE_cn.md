# 架构

> [English](ARCHITECTURE.md) | 简体中文

## 范围

MuriArc 是具有两种运行形态的同一个产品。Desktop 与 Server 共享 Vue 界面、Application service、领域模型、Store port、导入/Snapshot 服务、AI 安全层和兼容合同；各运行时只提供 transport、认证、秘密存储、持久化和交付 adapter。

```text
Vue UI ── LocalTauriGateway ── Tauri commands ──┐
                                                ├─ Application ── Core/Domain ── Store ports
Vue UI ── RemoteHttpGateway ── Axum /api/v1 ────┘                               ├─ SQLite
AI workspace ── approved domain tools ──────────────────────────────────────────└─ PostgreSQL
External client ── REST or MCP + scoped token ────────────────────────────────────────┘
```

## 分层

### UI 与 transport

- Vue 页面只负责渲染状态、收集用户意图、展示校验与审批结果，不持有业务不变量。
- `LocalTauriGateway` 将同一套 UI 合同映射到 Tauri command，不开放本地 HTTP Server。
- `RemoteHttpGateway` 使用 Axum API、HttpOnly Session、CSRF，以及有大小边界的 JSON/流式端点。
- REST/MCP handler 只做认证、授权、反序列化和调用 Application service，禁止在入口中独立拼装多步领域写入。

### Application

Application service 规范化输入并编排跨 Domain/Store port 的用例。只要运行边界允许，同一公共行为就应由 Desktop 和 Server 复用一条 Application 路径。

典型职责包括动物登记与转笼、项目动物分配、繁育状态转换、实验发布/入组、测量草稿、导入确认、Snapshot、AI 会话和人工审批。

### Core 与 Domain

Core 不依赖 Tauri、Axum、SQLx 具体数据库或模型 Provider，包含：

- 强类型 ID、revision、actor、source、Audit 与 Provenance；
- 动物、笼位、生命周期事件和项目动物分配；
- 基因型定义/记录/证据批次、谱系、繁育品系、配对、交配事件、窝次与草稿；
- 实验模板、Cohort、Participation、Procedure、Observation、Measurement、Sample 与 Attachment；
- 权限、AI operation 合同、兼容身份和 Release Manifest 类型。

领域不变量在持久化前拒绝非法转换，例如终末动物不可复活、写入必须核对 revision、科研记录只能签署一次、已发布模板不可变、项目作用域必须显式、繁育成员组成合法、观察值历史追加保留。

### Store port 与 adapter

SQLite 和 PostgreSQL adapter 实现相同 Store contract，并运行共享合同测试。SQL、事务边界、数据库约束和 migration primitive 属于 adapter；Core 不感知具体数据库。

所有正式写入都要求 actor、source、revision、Audit，并在适用时记录 Provenance。核心记录默认软删除，只有明确记录的技术日志保留策略例外。

## 事务边界

一个科研意图对应一个事务，例如：

- 动物登记同时写 Animal、生命周期事件、Audit 与 Provenance；
- 转笼锁定并校验源/目标笼位，推进 Animal revision，写事件和 Audit；
- 将 offspring draft 登记为 Animal 时，Animal、双亲谱系、生命周期、Audit、Provenance 一次提交；
- 实验入组同时保存 Participation 与当时采用的基因鉴定证据快照；
- 批准测量或鉴定批次时保留人工审批、证据关系和 Provenance。

Transport 禁止用多个无关 Store 调用模拟这些事务。

## 数据与资产边界

数据库保存附件元数据和内容 hash，大文件字节位于对应运行形态的附件库。需要完整性的读取必须校验对象身份、大小/hash。私有 AI 来源图片和提取候选在人工批准正式关系前，不进入普通项目可见范围。

业务 Snapshot 是带类型和 checksum 的完整性/离线留存归档，不是可启动数据库备份，当前也没有通用 restore/apply。部署恢复必须联合数据库、附件、数据产物、配置、generation manifest、密钥材料与 AI 状态。

## AI 架构

每个用户拥有版本化模型档案。会话绑定不可变档案版本；默认模型是显式引用，不会回退到“第一个模型”。Provider 构造会精确解析该版本的协议、规范化 endpoint、模型 ID、能力、参数与用户级秘密。

模型可见工具是以下条件的交集：

1. 当前人的权限；
2. Lab/Project 作用域；
3. 外部 Token scope（如存在）；
4. 会话声明的自主等级；
5. 当前 executor 实际提供的能力。

raw SQL 和安全/transport 证明会在 Provider 请求前被拒绝。读取返回有界 projection 与引用；写入只返回可审阅草稿，敏感动作和科研签署必须由人完成。

视觉请求使用当前会话模型，或用户显式选择的视觉中转模型。图片先执行大小、类型、尺寸校验和净化，私有保存后才生成候选；模型不能为正式写入自行决定权威 Animal/Experiment ID。

## 身份与租户

Server 层级为 Environment → Lab → Project。`EnvironmentRoot` 是部署所有者的恢复/治理权限，不会静默取代 Lab/Project 授权。用户具有实验室角色和可选项目角色。项目读取不能泄露同笼其他项目动物或无关实验。

Desktop 使用本地操作者资料而非 Server 凭据表。无密码进入只在可信 OS 账号内确认操作者，不能描述为数据加密或访问控制边界。

## 运行与交付边界

- **Desktop**：Tauri v2、内置 Vue、SQLite、本地数据根和 OS keyring 引用；正式交付为 Windows WebView 安装包，不是 VNC/noVNC。
- **Server**：Axum、PostgreSQL、响应式 Web、默认 loopback 入口；长期运行的 `muriarc-server` 不拥有 Docker socket、systemd、发布签名、备份编排或 raw DDL 权限。
- **升级控制面**：`muriarcctl` 与共享 Upgrade Engine 负责签名目标、freeze/drain、备份/实际恢复、Candidate 验证、原子激活和 Write Lease。

当前候选源码身份已是 `1.0.0 / E0001 / permanent-upgrade`。只有同一批最终制品完成完整真实 RC，永久兼容和正式交付承诺才会生效。

## 架构决策

中文内部 ADR 记录详细取舍：[ADR-0001](adr/0001-application-layer_cn.md)、[ADR-0002](adr/0002-workspace-tenancy_cn.md)、[ADR-0003](adr/0003-transaction-boundaries_cn.md)、[ADR-0004](adr/0004-genetics-v2-compatibility_cn.md)、[ADR-0005](adr/0005-breeding-atomicity_cn.md)、[ADR-0006](adr/0006-observation-version-policy_cn.md)、[ADR-0007](adr/0007-enrollment-genotype-snapshot_cn.md)和 [ADR-0008](adr/0008-runtime-identity-and-account-security_cn.md)。

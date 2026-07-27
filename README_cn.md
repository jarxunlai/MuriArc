# MuriArc

> [English](README.md) | 简体中文

<div align="center">
  <img src="branding/logo-master.png" alt="MuriArc" width="128">

  **动物管理优先 · 实验数据原生关联 · AI 辅助操作**
</div>

## 产品定位

MuriArc 是面向科研场景的动物全生命周期、繁育、实验数据与 AI 服务管理平台。系统通过明确的领域关系连接笼位、动物、谱系、繁育事实、实验、观察、测量、样本、附件、Audit 与 Provenance，不把科研数据退化为无约束表格、任意 SQL 或 EAV。

MuriArc 由 `jarxunlai` 独立开发和维护，工程实现由 AI 辅助。AI 不作为法律作者或版权主体。

## 发布状态

> [!IMPORTANT]
> 仓库当前仍是 **`0.1.0 / preview_epoch_0`**，正在准备 **`1.0.0 / E0001`** 发布候选，但尚未完成正式 1.0 RC。Preview 构建、源码 checkout、本机测试服务和 unsigned Tester 包均不是生产发布。

永久兼容承诺从通过验证的 `1.0.0 / E0001` 制品开始。同一批已签名制品的 digest 必须通过完整私有 RC 矩阵，之后才能不重建制品、原样发布为 `v1.0.0`。

## 核心能力

下列能力已经由当前源码或验收测试覆盖：

- **动物登记与生命周期**：动物、笼位、转笼、生命周期事件、项目动物分配、附件、Audit 与 Provenance。
- **繁育与遗传**：繁育品系、Colony、一雄多雌配对、交配事件、窝次、动物草稿、谱系、结构化基因型定义、逐动物鉴定记录，以及带胶图证据的基因鉴定批次。
- **实验与科研记录**：版本化实验模板、Cohort、Participation、入组基因型快照、Procedure、Observation、Measurement、Sample 和带类型的观察值版本历史。
- **数据操作**：有边界的动物/测量导入、按作用域导出 Animal Registry、附件完整性校验和可验证业务 Snapshot。
- **多 Provider AI**：用户自有、版本化模型档案；支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 与显式配置的兼容端点；提供受控工具、审批、引用、多模态路由和私有图片候选。
- **运维与治理**：Server 账号和角色、Environment Root 恢复权限、技术日志保留、签名升级控制面合同，以及 SQLite/PostgreSQL 双 Store contract。

Snapshot 当前不是通用 restore 格式；普通 Import/Export 不是 Desktop 到 Server 的迁移工具；macOS 正式交付、生产公网托管和“1.0 RC 已通过”均不属于当前已完成声明。

## Desktop 与 Server 形态

| 形态 | 运行时 | 使用场景 | 安全与存储边界 |
| --- | --- | --- | --- |
| **Desktop** | Tauri v2 + Vue + SQLite | 单个可信 Windows 账号下的研究者 | 原生 WebView 窗口、本地数据根、附件库、OS keyring 保存 API Key；无密码“进入本地空间”只是操作者确认，不是操作系统安全边界 |
| **Server** | Axum + Vue + PostgreSQL | 单一实验室内的多用户、多项目协作 | Argon2id 凭据、HttpOnly Session、CSRF、可撤销作用域 Token、按用户加密 Provider 密钥、默认仅 loopback 暴露 |

Desktop 不通过 VNC/noVNC 或浏览器远程桌面交付；Server 也不替代 Desktop 的本地 SQLite 形态。

## AI 安全边界

- Provider 凭据和模型设置按用户、档案版本隔离。Server 加密秘密；Desktop 仅通过 OS keyring 引用秘密。
- 缺少凭据、档案归属错误、档案归档、默认档案过期或 legacy read-only 会话，都会在发出 Provider 请求前失败。
- 模型不能执行 raw SQL，也不能绕过项目作用域、权限、revision、预览、审批或科研人员签署。
- 普通写入先成为可审阅草稿。动物转移/死亡、删除、批量导入、权限与账号、科研签署、图片证据批准等敏感动作始终由人完成。
- Provider 错误、Audit、日志和 UI 状态不得包含 API Key、密码、Session、CSRF、Token 或私钥。

完整信任模型见[安全文档](docs/SECURITY_cn.md)。

## 快速开始

### 前置要求

- Rust `1.88`
- Node.js `>=22.13`
- 通过 Corepack 使用 pnpm `11.5.0`
- Server 集成测试和部署使用 PostgreSQL `17`
- Desktop 开发需要 Windows WebView2 与 Tauri 构建依赖

### 开发门禁

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install --frozen-lockfile

cargo fmt --all -- --check
cargo clippy --locked -p muriarc-core -p muriarc-server --all-targets --all-features -- -D warnings
cargo test --locked -p muriarc-core -p muriarc-server --all-targets --all-features
pnpm --dir ui run test
pnpm --dir ui run typecheck
VITE_MURIARC_GATEWAY=remote pnpm --dir ui run build
```

PostgreSQL Store 测试必须通过 `MURIARC_TEST_DATABASE_URL` 连接独立 PostgreSQL 17；因变量缺失而 skip 不算通过。每个 worktree 必须使用自己的 UI 依赖和运行数据。详见[环境文档](docs/ENVIRONMENTS_cn.md)。

### Preview 部署

根目录 Compose 和源码命令只用于开发/preview。复制示例环境文件后必须替换所有占位值、保持 PostgreSQL 私有，并在可信反向代理终止 TLS。源码构建的 Compose 不能冒充签名 1.0 制品。

```bash
cp .env.example .env
# 只在本机编辑 .env；禁止提交或把秘密粘贴到 issue。
docker compose config --quiet
docker compose up -d --build --wait
```

进入非开发环境前，请阅读[Server 部署](docs/DEPLOYMENT_cn.md)、[Desktop 交付](docs/DESKTOP_DELIVERY_cn.md)和[Server 正式交付](docs/SERVER_DELIVERY_cn.md)。

## 数据与隐私

- Git 只跟踪源码、迁移、测试、小型合成 fixture、文档、依赖锁和公开发布定义。
- Git 不跟踪运行数据库、附件、Snapshot、恢复副本、凭据、AI Key、Session、Token、私钥或真实动物/科研数据。
- 数据库、附件、数据产物、部署配置、generation manifest、密钥材料与 AI 状态构成一个恢复集合，必须联合备份和恢复。
- 既有数据只通过可验证、可恢复的原位升级流程推进；禁止通过清库或手改 migration SQL 让升级“通过”。
- standard-v1 只包含合成验收数据；真实研究数据必须执行项目所有者批准的隐私、备份与权限制度。

## 文档导航

从[中文文档首页](docs/README_cn.md)开始。主要公开文档包括：

- [架构](docs/ARCHITECTURE_cn.md)与[安全](docs/SECURITY_cn.md)
- [环境](docs/ENVIRONMENTS_cn.md)与[Server 部署](docs/DEPLOYMENT_cn.md)
- [Desktop 交付](docs/DESKTOP_DELIVERY_cn.md)与[Server 正式交付](docs/SERVER_DELIVERY_cn.md)
- [MuriArc 数据迁移](docs/MIGRATION_cn.md)、[Upgrade Engine](docs/UPGRADE_ENGINE_cn.md)与[兼容合同](docs/UPGRADE_COMPATIBILITY_cn.md)
- [Cloudflare 公网 Profile](docs/CLOUDFLARE_PUBLIC_PROFILE_cn.md)与[交付验收](docs/DELIVERY_ACCEPTANCE_cn.md)

## 开发与贡献

修改公共行为前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 和相关架构/安全文档。保持 transport 入口薄、业务规则集中在 Application/Core、SQLite/PostgreSQL contract 一致，并为每项公共行为变更补充测试。

功能开发和缺陷修复应使用干净的非 main worktree。禁止提交构建产物、`node_modules`、Cargo target、数据库、秘密或本机验收证据。

## 许可证

Copyright 2026 `jarxunlai`。

MuriArc 使用 [Apache License 2.0](LICENSE) 发布，项目声明见 [NOTICE](NOTICE)。

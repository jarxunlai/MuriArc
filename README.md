# MuriArc

<div align="center">
  <img src="branding/logo-master.png" alt="MuriArc" width="128">

  **动物管理优先 · 实验数据原生关联 · AI 辅助操作**
</div>

MuriArc 是面向个人研究者和共享实验室的实验动物研究管理平台。它以动物全生命周期为主线，将笼位、繁育、基因型、实验执行、测量、样本、附件和审计组织在同一条可追溯数据链中。

> MuriArc 基于开源项目 [MurisPro / animal_lab](https://github.com/lanternx/animal_lab) 继续开发。法律归属见 [LICENSE](LICENSE) 与 [NOTICE](NOTICE)。

## 运行形态

- **Desktop**：Tauri v2 + SQLite，面向个人用户；正式本地交付为 Windows Tauri WebView 安装包，不通过 VNC/noVNC 或浏览器远程桌面部署。每次启动显示无密码“进入本地空间”，它只确认实验室和操作者、不是安全锁，进入后可完全离线使用。数据库、附件、数据产物和非敏感 AI 配置使用同一个本地数据根，用户可从设置中选择本机固定磁盘上的独立空目录并在重启前安全迁移；API Key 始终保留在 OS keyring。
- **Server**：Axum + PostgreSQL + 响应式 Web，面向一个实验室内的多用户、多项目协作。
- **AI**：每位用户可建立多个版本化模型档案，自由填写模型 ID，并分别使用 OpenAI Chat
  Completions、OpenAI Responses 或 Anthropic Messages 协议；档案密钥独立保存，未配置密钥
  时不会发出外部请求或产生费用。对话绑定不可变档案版本，可在实验室上限内使用 Ask /
  Auto / Full 委托。视觉请求可由当前模型直接处理，或经用户明确选择的视觉模型生成受控观察
  后转交对话模型；AI 只访问受控领域工具，实验数据图片只能生成私有候选草稿，正式写入仍需
  人工编辑或批准。

## 工程结构

```text
crates/application/       共享应用用例、输入规范化与工作流编排
crates/core/              领域模型与 Store ports
crates/store-sqlite/      本地 SQLite adapter
crates/store-postgres/    Server PostgreSQL adapter
crates/server/            Axum REST、认证、MCP 与 Job
crates/ai/                AI DSL、工具权限与审批
crates/importer/          CSV/XLSX 解析、校验与事务导入计划
crates/data/              导入、导出、附件与快照生成/校验服务
crates/snapshot/          可校验快照格式
crates/legacy-migrator/   旧库只读审计与迁移
src-tauri/                桌面入口
ui/                       Vue 3 + Vite 响应式前端
branding/                 MuriArc 品牌源文件
migrations/               数据库迁移
```

详细边界见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，关键架构取舍见
[docs/adr/](docs/adr/)。

## 已实现核心能力

- **Genetics v2**：位点、等位基因、结构化多组件 `GenotypeDefinition` 与逐动物
  `GenotypingRecord`；旧 `Genotype` 记录保持兼容，不被静默重解释。
- **Breeding**：品系、Colony、一雄多雌配对、配对退役、交配事件、窝次与存活
  `AnimalDraft`；Draft 登记为正式 Animal 时，Animal、双亲 Pedigree、生命周期事件、
  Audit 与 Provenance 在一个事务中完成。
- **Experiment observations**：实验事件、带值类型的 ObservationDefinition、动态观察录入，
  以及 Immutable / Mutable / Versioned 策略和追加式值版本历史。
- **Enrollment snapshot**：动物入组实验时，按基因型定义捕获当时最新检测记录；之后的检测
  不回写既有 Participation。
- **项目动物范围**：动物由实验室角色通过显式 `ProjectAnimalAssignment` 批量分配；项目成员
  可查看本项目动物所在笼位和实验，但不会看到同笼的其他项目动物或实验室总占用。
- **双运行形态**：Desktop/Tauri/SQLite 与 Server/Axum/PostgreSQL 共享 Application、Domain、
  Store contract 和 Vue 页面。
- **受控 AI**：用户拥有多个模型档案，档案的协议、规范化 Base URL、自由模型 ID、能力与
  参数形成不可变版本；默认对话/视觉模型必须显式设置，停用档案只保留历史读取能力。Server
  加密密钥和 Desktop OS keyring 均按档案版本隔离。固定工具、最小权限、结构化 Trace 和
  对话级 Ask / Auto / Full 授权继续约束调用；科研签署、图片证据批准、动物转移/死亡、
  删除/批量导入、权限账号、技术日志清理与繁育事实始终由人工完成。
- **分层操作记录**：成员日常只看聚合后的关键活动；正式 Audit/Provenance 永久保留，Server
  技术日志按数量与最短天数自动清理，只有 Environment Root 能预览、调整策略或手动清理。

## 开发

要求：Rust 1.88、Node.js >=22.13、pnpm 11.5.0（可通过 Corepack 启用）。PostgreSQL 只在 Server 集成测试和部署时需要。

```bash
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p muriarc-server --features postgres

# Web UI
cd ui
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm install --frozen-lockfile
pnpm test
pnpm run dev
pnpm run build
pnpm run test:e2e
```

Tauri 开发入口和环境变量以 `src-tauri/tauri.conf.json`、`.env.example` 为准。Desktop
本地交付边界见 [docs/DESKTOP_DELIVERY.md](docs/DESKTOP_DELIVERY.md)；VNC/noVNC 只能作为临时远程预览手段，不能作为正式 Desktop 部署方式。

共享 Server 使用 Argon2id 账号、HttpOnly/SameSite session、CSRF 防护和
可撤销外部 token。部署必须通过 `MURIARC_ROOT_*` 环境变量声明唯一 Environment Root；
Server 每次启动在单事务中核对身份、LabAdmin membership 与凭据，Root 密码只能编辑宿主机
`.env` 后重启修改。角色与升级流程见 [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)。

## 数据安全

- 数据库、附件、密钥、快照和旧库副本不进入 Git。
- 安全升级由独立 muriarcctl 和共享 Upgrade Engine 编排；Server 不持有 Docker/systemd/DDL
  权限，未实际恢复验证完整备份、未完成 Candidate 七层验证时不得激活。
- 旧数据库只以只读方式扫描；迁移目标始终是新文件。
- 大附件存文件库，数据库只保存元数据和 SHA-256。
- Server 所有正式写入均记录操作者、来源、revision 与审计事件。
- 普通 CSV/XLSX 导入只支持动物登记与实验测量；批量基因鉴定使用专用工作流，将一份结果
  表和一张或多张胶图绑定到可审计批次后原子确认。普通 CSV/XLSX 导出只生成 Animal Registry
  数据产品，不能作为迁移工具。
- Snapshot 是实验室完整业务归档与完整性校验边界，但当前没有 restore/apply；部署恢复仍需
  经演练的数据库与附件联合备份。

迁移规则见 [docs/MIGRATION.md](docs/MIGRATION.md)，安全边界见 [docs/SECURITY.md](docs/SECURITY.md)，共享版部署见 [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)，接口与逐步验收见 [docs/DELIVERY_ACCEPTANCE.md](docs/DELIVERY_ACCEPTANCE.md)。
从正式版 1.0 开始的永久数据兼容、Generation、Write Lease 与 fail-closed 启动契约见
[docs/UPGRADE_COMPATIBILITY.md](docs/UPGRADE_COMPATIBILITY.md)。

## 当前状态

MuriArc 0.1.0 为预发布重构版本：Genetics v2、Breeding、Observation、Desktop、Server、
完整业务 Snapshot、受控数据任务、AI 审批和响应式 UI 已落地，尚未宣告正式发布。

- SQLite 与真实 PostgreSQL 17 已运行同一 Store contract；Rust workspace、UI 单测、生产构建与
  Desktop/Tablet/Mobile Playwright 场景均纳入交付门禁。
- Snapshot 生成并校验 manifest、全量业务 JSONL、附件、大小与 SHA-256；不提供
  restore/apply、实时同步或自动合并。
- 普通导入/导出没有通用实体选择器：导入仅动物登记/实验测量，导出仅 Animal Registry；
  基因鉴定批次是独立的结果表 + 胶图证据工作流。
- Windows Desktop 安装包为首发本地交付目标；macOS 必须在真实设备验证后发布。
- 公开发布前必须完成遗留凭据轮换与自有远程 Git 历史中的敏感数据清理，并再次执行发布审计。

# Feature Spec: AI 全业务上下文与科研工作台

> 此文档记录 `codex/AI_context` 单一综合分支的已确认范围、实现约束和验收门禁。

## 分支信息

| 项目 | 值 |
|------|-----|
| 分支名称 | `codex/AI_context` |
| 基于提交 | `origin/main@c842e64d0f5109314d1a1f9e2f3ea3c717bbce8a` |
| Worktree 路径 | `/home/ljx/Github/.worktrees/animal_lab/codex-AI_context` |
| 建立日期 | `2026-07-23` |

## 目标

将现有 AI 抽屉升级为 Server 与 Desktop 共用的多会话科研工作台；让助手通过受权限约束、
可分页、带引用的业务读取层查询动物全生命周期和项目数据；允许用户拖入结构化文件、文本、
PDF 与图片形成可复核的科研候选，最终仍由当前用户通过正式预览、加强确认或研究者签署执行。

## 已实现范围

- [x] 助手消息安全渲染 Markdown，用户文本保持纯文本。
- [x] 快捷 AI 工作台可移动、双向缩放、最大化/复位，并与完整 `/ai` 页面复用会话组件。
- [x] 全局会话列表支持项目筛选、标题搜索、新建、重命名、置顶和软归档。
- [x] Composer 支持拖拽 XLSX/CSV/TSV/TXT/MD/JSON、文本 PDF、带有界 embedded JPEG
  页图的扫描 PDF，以及 PNG/JPEG/TIFF；不宣称通用 OCR 或任意扫描 PDF 解码。
- [x] 增加共享 `BusinessReadService` 与分页安全投影，覆盖 Genetics v2、动物、项目、笼位、
  繁育、实验、观察、测量、样本、资料库和任务等正式资源；当前用户 AI 运行产物在聚合层仅
  暴露 owner-scoped Job，会话、草稿与 ToolRun 继续通过各自的 owner-scoped API 读取。
- [x] 向模型注册 `resource_search`、`genotyping_query`、`animal_context` 和
  `project_context`；活动、Audit 与 Provenance 查询只按当前 actor 的实时权限注册。
  用户选择的来源不注册模型可调用的 `source_inspect`，而由 Server/Tauri 的可信 transport
  重新读取并校验 owner、会话、项目、revision 与对象字节，再把有界、惰性的解析结果注入
  Provider 上下文。
- [x] 保留历史工具轨迹、旧 wire name 和现有 MCP 只读客户端的兼容解码；旧模型读取工具
  不再主动注册。
- [x] 文件先进入按用户、会话和至多一个项目隔离的私有暂存源；不把附件路径、对象摘要
  （SHA）或内部 Attachment ID 暴露给模型或公开历史 DTO。
- [x] 新增确定性实验分组计划，记录 seed、分层条件、协变量、排除项、输入快照摘要和平衡摘要，
  并由 Store adapter 原子应用 Cohort 与 Participation。
- [x] AI 写入能力使用显式窄 allowlist：单条测量草稿、确定性实验分组、项目绑定的来源测量
  普通导入，以及项目动物导出；均复用正式业务边界。实验室级会话严格只读，项目成员、权限、
  账号、Provider 与系统设置不可由 AI 写入。
- [x] 聊天内复用正式预览、确认或签署边界；AI、Auto 与 Full 都不能替代人工 actor。
- [x] 来源支持的测量导入成功后，在同一事务内提交领域记录、Job、Approval、ToolRun，
  归档原始来源并建立 Attachment/Provenance 关系；失败、并发拒绝、取消或 revision 漂移
  全体回滚，问答文件按保留策略和可重试清理队列处理。
- [x] Composer 只列出从未被任一历史消息引用的可用来源；SQLite/PostgreSQL 在存储层跨完整
  会话历史判定“已消费”，不会因客户端只加载最近消息而重新排队。用户可显式释放暂存原件，
  释放接口幂等，并使依赖该来源的待确认导入失效。
- [x] 批量测量导入草稿持久化正式预览摘要与至多 20 行安全投影；确认前重新生成预览并逐字段
  比对项目、实验、计数、问题、截断标记和行内容，缺失、错误或漂移均 fail closed。
- [x] 公开 Audit、Operations、资料库和业务快照使用安全投影：不暴露其他用户会话正文、
  ToolRun/Approval 内容、Provider 配置、AI 暂存对象路径或摘要；已正式归档到项目的来源附件
  及 Provenance 仍作为业务记录保留。
- [x] `ToolCallLimitExceeded` 不通过提高上限掩盖 N+1；已有成功结果时保留 ToolRun、引用和
  草稿，零进度时也持久化稳定 `incompleteReason` 与安全反馈，均可在会话恢复后读取。
- [x] 视觉提取的 Provider 响应只接受 observation definition key、值、置信度和来源标签；
  服务端将候选固定绑定当前已授权 Experiment，不接受模型提供任何实体或 definition UUID。
- [x] 工作台页签、当前项、引用和批量预览使用语义化控件与键盘导航；布局遵循高对比度、
  可见焦点、最小触控目标和 reduced-motion 边界。

## 明确后续 / 非本次范围

以下能力没有混入本次“已实现”声明，也不是本分支 Draft PR 可以宣称完成的内容：

- 逐单元格 AI 纠错，包括原值、新值、理由、置信度、用户逐项勾选、衍生 preview hash 与
  correction provenance。
- 多工作表混合工作簿的单一 typed plan，以及跨 sheet 依赖、revision、错误摘要和单项目约束。
- 动物、Genetics v2、实验、样本、附件、繁育、交配、窝次和谱系等全领域通用 AI 写入。
- 实验室级 Animal Registry 来源导入的 AI 确认。普通非 AI 动物导入仍按原流程可用；AI
  入口需先设计明确的项目归属或单独的实验室级审批边界，不能借项目会话绕过。
- 仅在用户明确询问 Audit 时才临时开放 `audit_query` 的交互意图门；本次只有实时权限门。
- 将旧 AI 读取实现和 `server/mcp.rs` 的 MCP 工具内部迁移到 `BusinessReadService`；本次只
  保证兼容解码与现有客户端可继续使用。
- 从图片解析 Animal、Sample 或 Artifact 主体，以及在授权范围内把可见动物编号唯一解析为
  正式实体；本次视觉提取仅支持服务端绑定当前 Experiment。
- 同一草稿同时满足“科研事实研究者签署”和“高风险加强确认”的复合双门禁；本次按现有
  `DraftKind` 分别执行研究者声明或加强确认。
- 非 embedded-JPEG 扫描 PDF、通用 PDF 页面渲染和 OCR。

## 验收标准

- 助手表格、列表、代码块和链接正确渲染；原始 HTML、脚本与危险 URI 无法执行。
- 375/768/1024/1440 宽度下无非预期横向滚动；所有窗口、会话和上传操作可通过键盘完成。
- 会话列表只展示当前用户数据；项目切换、归档和并发 revision 不会覆盖其他会话更新。
- 所有向模型开放的查询仅返回当前 actor 有权限访问的安全字段，分页结果明确说明是否完整并
  带精确引用。
- “找出待确认的基因型”使用 Genetics v2 当前有效记录，不依赖旧自由文本或旧 `Genotype`。
- 项目成员可见本项目动物、笼位和实验，但看不到同笼的项目外动物或实验室占用。
- 文件路径、凭据、Provider 私密配置、其他用户 AI 内容和原始 Audit 敏感值不进入模型。
- Excel 宏、公式、外部链接与 PDF 动作不执行；不受支持的扫描 PDF 安全失败，不静默进入模型。
- 视觉提取不接受模型编写的实体 UUID；候选只能由服务端绑定当前授权 Experiment。
- 对本次实际支持的测量、分组和项目测量导入，未解决错误、权限不足或 revision 漂移使草稿
  不可确认；导入领域数据、来源归档、Job、Approval 与 ToolRun 任一失败均整体回滚。
- 分组结果在相同 seed 和输入快照下可重复，且不会通过逐动物工具调用触发调用数上限。
- 研究计划和测量事实需要研究者声明；批量导入需要加强确认，二者均由当前人工 actor 完成。
- 达到工具调用上限时，无论是否已有成功工具结果，稳定 `incompleteReason`、用户反馈和已完成
  轨迹都进入会话持久化。
- 新增会话、来源保留、导入来源归档和实验分组行为由 SQLite/PostgreSQL 共用 Store contract；
  Server/Tauri 共用 DTO、workflow error code 和最终业务边界。
- 历史 ToolRun/会话响应与现有 MCP 客户端继续可读。

## 技术约束

- `core` 不依赖 Tauri、Axum、SQLx adapter 或模型 Provider；入口保持薄。
- 所有写入携带 actor、source、revision、Audit 与 Provenance；删除只允许正式软删除。
- 跨实体计划必须由 Store adapter 在单事务/明确 Unit of Work 中执行，不能由 Vue 或
  transport 循环模拟事务。
- 不允许 raw SQL、任意 HTTP、任意 EAV、任意附件路径或模型生成 UUID。
- Genetics v2 是正式基因型事实源；旧 `Genotype` 仅作明确标记的兼容读取。
- 会话正文搜索、跨项目原子计划和既有 Participation 重分组不属于首期。
- 真实数据库、附件、上传源、测试运行数据、密钥、报告和构建产物不得进入 Git。
- 当前 worktree 内完成全部测试；Cargo 产物写入仓库外的分支隔离目录。

## 交付门禁

以下是最终源码状态和精确提交的交付门禁。当前不因定向测试、已有测试代码或一次中间检查而
提前勾选：

- [x] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
      （Desktop 在当前 WSL 缺少 `pkg-config`、dbus/GTK/WebKit 开发库；排除 Desktop 的同等
      `--all-targets --all-features -- -D warnings` 检查已通过）
- [ ] `cargo test --workspace --all-features`
      （同一 Desktop 系统依赖阻断；`--workspace --exclude muriarc-desktop --all-features`
      已通过）
- [x] SQLite 新增 contract tests 在最终源码状态通过
- [ ] 配置 `MURIARC_TEST_DATABASE_URL` 后运行 PostgreSQL 新增 contract tests
- [x] `pnpm --dir ui run test`（21 个文件、153 项测试）
- [x] `pnpm --dir ui run build`
- [ ] 相关 Playwright 场景（当前 Chromium 在启动前缺少 `libnspr4.so`，42 个场景均未执行）
- [ ] 精确提交的 Windows Tauri 验收
- [x] 最终源码状态执行 `git diff --check`

提交和 Draft PR 属于此清单之后的交付动作，以 Git/GitHub 状态为准；不自行合并、rebase、
force-push 或删除分支。

## 当前验证状态

- `cargo fmt --all -- --check`、非 Desktop 全工作区 Rust 测试、非 Desktop 严格 Clippy、
  SQLite 共用 contracts、UI 153 项测试、UI 生产构建和 `git diff --check` 已通过。
- PostgreSQL adapter、迁移和共用 contract 可编译；当前没有配置
  `MURIARC_TEST_DATABASE_URL`，带 `_when_configured` 的真实 PostgreSQL 用例本轮未连库，
  不虚报为运行通过。
- 完整 Rust workspace/Clippy 在进入 Desktop 产品代码前，被当前 WSL 缺少 `pkg-config`
  及 dbus/GTK/WebKit 系统开发库阻断。
- Playwright 已完成前置 UI 构建，但 Chromium 因缺少 `libnspr4.so` 无法启动，42 个场景未
  执行。Windows Tauri 仍必须对最终精确提交验收，不能由 WSL 构建或旧产物代替。

# Feature Spec: Desktop 安全签名更新与 Candidate generation

> 本分支实现 Windows Tauri Desktop 的签名更新和完整 SQLite/data-root Candidate 激活，不修改 Server Native/Compose 规则。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/desktop-safe-updater` |
| 基于提交 | `feature/native-compose-delivery@04d3449` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-desktop-safe-updater` |
| 建立日期 | `2026-07-27` |

## 目标

Desktop 更新不得原地迁移用户唯一 SQLite 或数据目录。目标安装包必须先通过 Tauri 签名，旧 generation 完成 WAL checkpoint 和完整 Candidate 复制后，目标 Upgrade Engine 才能迁移、验证并原子切换。更新失败继续使用旧 generation；目标首次写入后禁止自动降级。

## 实现范围

- [x] 接入 Tauri updater plugin 与 fail-closed 签名配置，不允许未签名本地替换正式安装包。
- [x] 建立 Desktop activation pointer、generation manifest、hash-chain operation Journal 与恢复点布局。
- [x] 更新前对 SQLite 执行 WAL checkpoint，并复制 SQLite、attachments、artifacts、非敏感设置及 Keyring 引用；不复制明文 Provider API Key。
- [x] Candidate 使用独立同卷目录和目标 migration primitive，启动恢复阶段不会运行外部 Provider、后台任务或真实用户写入。
- [x] 目标版本验证 Candidate 的 Store/Application read surface、附件 SHA、AI 历史/secret reference、Audit inventory 和事务内继续写入后 rollback。
- [x] 激活前失败保留旧 generation；激活采用原子 locator；首次写入后拒绝自动降级。
- [x] 普通 Desktop 启动只核对精确 SQLite Epoch/Digest/Generation，不再隐式执行稳定版 migration。
- [x] 跨机恢复保留 Provider profile 但因 OS Keyring 不随 data root 复制而强制重新输入 API Key。
- [x] 增加 Rust/Windows CI、release 签名配置检查、Candidate 中断/恢复和旧数据继续写入测试。
- [x] 更新 Desktop 交付与恢复文档。

## 正式 RC 边界

- 本分支提供实现与合成测试，不宣称 Windows 正式 RC 已通过。
- 最终 1.0 集成必须使用旧正式安装包生成数据，再用目标 MSI/NSIS updater archive 与 `.sig`
  执行真实安装、进程退出/重启、WebView UI、故障注入和首次写入边界验证；任何 SKIP 阻断发布。
- recovery copy 默认保留。面向用户的显式 recovery restore/prune UI/CLI 由最终恢复编排接入，
  不得以自动删除恢复点代替。

## 验收标准

- 原始 SQLite、attachments、artifacts 和 Keyring 项在 Candidate 失败时保持不变。
- 缺少数据库、目录、generation manifest、Keyring 引用或签名元数据时启动/更新 fail closed。
- Desktop 安装包不能用源码 `cargo run` 或 `--no-bundle` 冒充正式 RC。
- 同一 generation 上的 SQLite 与文件树必须一一对应；禁止自动创建空目录掩盖漏恢复。
- 更新后旧账号、动物关系、实验/测量/样本、附件、AI 历史和 Audit 可读且可继续写入。

## 技术约束

- 复用共享兼容类型、Upgrade Engine 和 release-evidence，不复制迁移状态机。
- Key 继续保存在 OS Keyring；日志、Journal、备份和测试 fixture 不记录明文 Key。
- Candidate/备份/报告位于 Git 外；测试仅用合成 Keyring adapter 和合成数据。
- Tauri updater endpoint/public key 是发布配置；私钥只能存在于受保护的发布环境。

## 跨分支备注

依赖 `04d3449` 及其之前的兼容基础、Upgrade Engine、Release Fixture 门禁。Cloudflare 与 Desktop 无运行时依赖。最终 1.0 集成分支使用 Windows 正式安装包生成并验证 E0001 Desktop fixture。

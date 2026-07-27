# MuriArc Desktop 交付

> [English](DESKTOP_DELIVERY.md) | 简体中文

## 状态与运行形态

Desktop 正式目标是内置 Vue UI、通过 Tauri IPC 使用 `LocalTauriGateway` 的 Windows Tauri v2 WebView 安装包。它不开放本地 HTTP API，也不通过 VNC/noVNC、浏览器远程桌面或 Server Docker 交付。

仓库当前仍是 `0.1.0 / preview_epoch_0`。本地 debug build 或 unsigned Tester 包都不是签名 `1.0.0 / E0001` 发布。

- SQLite：`<data-root>/muriarc.sqlite3`
- 附件：`<data-root>/attachments/`
- 数据产物：`<data-root>/data/`
- Generation 身份：`<data-root>/deployment-generation.json`
- Provider Key：只在 OS keyring
- 本地入口：可信 Windows 账号内的操作者确认，不是密码/安全锁

## 数据根与迁移

OS application-data **config root** 保留 locator 和 migration intent；用户选择的 **data root** 联合保存 SQLite、附件、数据产物、非敏感 AI 配置与 generation 身份。

用户可选择本机固定磁盘上的空目录。安装目录、相对路径、UNC/网络盘、符号链接、当前 root 及其父/子重叠路径都被拒绝。原生 picker 返回一次性 selection token，Vue 不能提交任意文件系统路径。

迁移先登记 intent，下次启动在打开 SQLite pool、附件服务或 Provider 设置之前执行：

1. WAL checkpoint 与 integrity check；
2. 复制到隔离 staging；
3. 比较 SHA-256 文件树 manifest；
4. 打开并验证目标数据库；
5. 原子更新 locator。

失败时 source 继续 active，绝不创建替代空库；source 保留供显式恢复。OS keyring 与 WebView2 cache 不迁移。

## 签名更新与 Candidate 激活

正式更新使用 HTTPS updater metadata、Tauri/Minisign 签名、独立签名 Release Manifest，以及固定的制品大小/SHA-256。Release build 缺少或无法解析 updater 公钥时必须失败。

交给安装器前，Desktop 在 operation 专属恢复目录保存准确旧 executable，并记录大小/SHA-256。目标程序在打开业务存储前通过共享 Upgrade Engine 恢复：

1. 重验 target 与 Release Manifest；
2. 取得 host/backend lock，并核对持久 operation state 与 hash-chain Journal；
3. checkpoint/验证 source 并创建完整 recovery copy；
4. 从 recovery 实际恢复出隔离 Candidate；
5. 只迁移 Candidate；
6. 验证 integrity/FK、Store/Application 读取、附件字节、AI 历史/secret reference、Audit inventory、事务内继续写入和只读无副作用；
7. 在无 Write Lease 状态原子切换 locator；
8. 验证目标启动/readiness 后才打开新 Write Lease。

目标首次业务写入前，失败可以原子切回 source locator，并转交给已验证旧 executable。出现 `first_write_at` 后禁止自动降级，只允许 forward fix 或带数据损失确认的显式恢复。

Provider Key 字节不复制。同机更新继续使用 OS-keyring account；跨机恢复只还原档案/历史引用，用户必须重新输入 Key。

## Windows 精确 commit 构建

每个分发包必须来自准确、干净的 GitHub commit，不能使用旧 clone、移动中的 `origin/main` 或之前本地 build。构建记录至少包含：

- 40 位 commit SHA 与 clean-tree 证明；
- Rust/Node/pnpm/Tauri 版本；
- updater 公钥身份（不含私钥）；
- installer/bundle 名称、大小、SHA-256、签名与 provenance 证据；
- Git 外的 build/evidence 目录。

签名私钥和密码只存在于受保护发布环境，不得进入 Git、build transcript、Release Manifest custom metadata 或验收附件。

Release build 执行 Windows CI 等价门禁：local-gateway UI build、缺 updater key 的负向测试、Desktop strict Clippy、Desktop tests 与 Tauri no-bundle/build 打包门禁。

## 运行验收

使用无真实 MuriArc 数据和个人 AI Key 的一次性 Windows 账号/VM，验证：

- 原生窗口和内置 UI，不依赖源码 Server；
- 本地操作者确认与离线运行；
- fresh SQLite/data root，以及安全迁移/重启；
- Windows 用户与 MuriArc 档案版本之间的 keyring 隔离；
- 动物、笼位、繁育、实验、观察、测量、样本、附件、Audit 与 AI 档案；
- 旧数据升级、中断恢复、首次写入前旧 executable fallback，以及首次写入后拒绝降级；
- 卸载/重装不会静默删除用户数据或恢复点。

macOS 只有在真实设备完成同等级打包、keychain、更新、迁移与恢复验收后才能发布。

## Windows Tester 包

朋友测试包与正式 RC/Release 分离：

- 从已经合并、干净、可追溯的 GitHub commit 构建；
- 只携带合成 standard 数据；
- 归档前扫描 API Key、密码、Session、Token、CSRF、私钥和真实科研数据；
- 使用 tester 专属 prerelease tag；
- 标记 **unsigned**、**synthetic data**、**not for production**；
- 提供 package SHA-256。

Tester 制品/证据不得与最终 `v1.0.0` artifact lock 或 RC 报告混用。

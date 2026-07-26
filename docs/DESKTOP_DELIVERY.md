# MuriArc Desktop local delivery

MuriArc Desktop 的正式本地交付目标是 Windows Tauri WebView 安装包。用户启动的是
MuriArc 原生窗口，窗口内加载随应用打包的 Vue 前端，并通过 Tauri IPC 调用本地
`LocalTauriGateway`。Desktop 不通过 VNC、noVNC、浏览器远程桌面或 Server Docker
部署交付。

## Runtime shape

- UI：随 Tauri 包内置的 `ui/dist`，运行在 Windows WebView2 中。
- Transport：`LocalTauriGateway` 调用 Tauri commands；不开放本地 HTTP API。
- Data root：首次启动使用 OS application data；用户可以在 Desktop“设置 → 备份与迁移”中
  选择本机固定磁盘上的独立空目录作为完整业务数据根。
- Data：SQLite 位于当前 data root 下的 `muriarc.sqlite3`。
- Files：附件与数据任务文件位于同一 data root 下的 `attachments/` 和 `data/` 子目录。
- Secrets：Desktop AI key 写入 OS keyring；项目数据库、日志、审计、快照和前端状态不得记录
  API key。
- Identity：每个 WebView 会话显示一次无密码“进入本地空间”；这只是 Lab 与
  LocalOperator 确认，不是登录、磁盘加密或操作系统权限边界。

Server 是另一种运行形态：Axum + PostgreSQL + HTTPS reverse proxy + 账号与权限体系。
需要浏览器访问、多人协作或远程部署时使用 Server；需要个人本地离线使用时使用 Desktop
安装包。V1 不提供 Local Web、本地 Axum+SQLite 浏览器服务或 Desktop/Server 实时同步。

## Local data root and migration

Desktop 区分两个目录：

- **config root**：Tauri 提供的 OS application data。`storage-location.json` 与
  `storage-migration.json` 始终留在这里，用来定位当前 data root 和记录等待重启执行的迁移。
- **data root**：统一保存 `muriarc.sqlite3`、`attachments/`、`data/`、非敏感
  `ai-provider.json` 和 `deployment-generation.json`。数据库、文件树与 generation manifest
  不得单独迁移到不同位置。

用户操作流程：

1. 打开“设置 → 备份与迁移”，选择本机固定磁盘上的独立空目录，例如
   `D:\MuriArcData`。安装目录、相对路径、UNC、符号链接、当前目录及其父/子目录均被拒绝。
2. 原生 folder picker 返回一次性 selection token。Vue/Tauri IPC 的确认请求只提交 token，
   不接受前端构造的任意磁盘路径。
3. 确认后只写入 migration intent。用户必须保存工作、完全退出 MuriArc，再重新启动。
4. 下次启动在打开 SQLite pool、附件服务和 AI 设置前执行迁移：先
   `integrity_check` 与 WAL checkpoint，再复制到 staging，以 SHA-256 文件树 manifest
   校验，并只在目标数据库再次通过完整性检查后切换 locator。

迁移采用 fail-closed 语义。失败或自定义磁盘缺失时不会创建新的空数据库，locator 继续指向
旧 data root，并显示安全错误。源目录不会自动删除。恢复系统默认目录时，默认目录中的旧
managed entries 会先移入 `storage-backups/<migration-id>/`；当前自定义源同样保留。

OS keyring 中的 API Key 不属于 data root，不复制、不写入 locator、intent、marker、日志或
UI 状态。WebView2 cache 也不迁移。目标根的 `.muriarc-storage-root.json` 只记录迁移 ID、
文件数量、总字节数和聚合 SHA-256，不记录业务内容。

## Signed updater and Candidate activation

正式 Desktop 更新只通过 `tauri-plugin-updater` 的 HTTPS endpoint 和 Minisign 公钥完成。前端
capability 不授予 `updater:default`；Vue 只能调用 MuriArc 自己的 `check_desktop_update` 和
`apply_desktop_update` commands，不能绕过恢复验证直接安装。`latest.json` 除 Tauri 的
`version/platforms/signature` 外必须包含：

```json
{
  "muriarc_artifact_name": "desktop-windows-x86_64-nsis",
  "muriarc_release_manifest_signature": "<Base64-wrapped Minisign signature text>",
  "muriarc_release_manifest": {
    "format_version": 1,
    "application_version": "1.0.0",
    "data_epoch": "E0001"
  }
}
```

完整 `muriarc_release_manifest` 仍须包含双后端 digest、Gateway revision、控制协议、迁移等级
和所有制品摘要；上例仅展示额外字段的位置。发布流程必须先以 Rust `ReleaseManifest` 的紧凑
JSON 序列化结果生成独立 Minisign signature，再把 signature 文件文本整体 Base64 后写入上述
字段。MuriArc 用同一固定公钥验证 manifest，防止攻击者只改维护等级、Epoch 或 backend digest。
下载在返回 bytes 前再由 Tauri 验证安装包签名，随后 MuriArc 核对下载字节的 SHA-256/大小与
已验签 Release Manifest。任一值不一致都不会写入 upgrade intent，也不会启动安装器。

旧程序只记录已经验签的目标和操作 ID。安装器替换程序并完全退出后，新目标程序必须在打开
任何 SQLite pool、附件服务或 AI Provider 前恢复该操作：

- 用户必须先在“设置 → 软件更新”看到 M0–M3 维护等级、数据卷/控制目录空间预检和更新包
  大小，并显式确认完整恢复验证及首次写入边界；IPC 同时绑定目标版本与维护等级，前端不能只
  发送一个“确定”布尔值来重放其他更新。
- 启动安装器前，把当前正在运行的旧 executable 复制到 config root 下按 operation ID 隔离的
  `desktop-binary-recovery/`，记录大小和 SHA-256。该恢复副本不是数据恢复点，不随 data root
  迁移，也不会进入 Git、Snapshot 或业务备份。

1. 重新验证独立签名的 Release Manifest，并用旧 executable 恢复记录中的 digest 绑定完整
   intent；然后重新推导 source、recovery、Candidate 路径。Desktop `UpgradeDriver` 由共享
   `UpgradeEngine` 执行，取得 Host/backend 锁，并交叉验证 SQLite operation state 与
   hash-chain Journal；intent 中的路径不能直接成为任意复制目标。
2. 对 source 执行 WAL checkpoint 和完整性检查；在同卷兄弟目录生成完整 recovery copy，比较
   文件树 SHA-256，并从 recovery **实际恢复**出独立 Candidate。最后验证的 recovery 不自动删除。
3. 仅对 Candidate 调用目标 migration primitive，创建新的 generation manifest，并保持无
   Write Lease 的只读状态。普通 Desktop 启动仍只核对 Epoch/Digest/Generation，不迁移结构。
4. 对 Candidate 执行 SQLite integrity/foreign-key、Store/Application read surface、附件实际
   字节 SHA、AI history/secret reference、Audit inventory 和事务内写入后 rollback 验证。Candidate
   的受保护记录数量不得少于 source，文件验证前后不得产生副作用。
5. 验证通过后先在**无 Write Lease**状态原子更新 `storage-location.json`，完成只读激活和无副作用
   验证，最后才打开目标 Write Lease。切换前失败时 locator 继续指向 source；进程中断可根据
   SQLite operation state 与 Journal 幂等恢复。

若新目标程序在上述步骤中失败，且 Candidate 尚无首次业务写入，目标程序会先确认 active locator
仍指向 source（若刚切换则原子切回），再把已校验的旧 executable 标记为 fallback 并启动它。
此后安装目录中的失败版本只充当最小启动器：在打开数据库前转交给旧版本。因此不仅“旧数据仍在”，
用户实际仍能进入旧程序。旧 executable 被篡改、丢失、大小或 SHA-256 不符时禁止执行；若 Candidate
已出现首次写入，也禁止这一自动回退，只允许 forward-fix 或显式恢复并确认数据损失。

新 generation 的首次业务写入由数据库 trigger 记录。此后没有自动降级入口：只能保持只读并
forward-fix，或由用户显式选择恢复点并确认可能丢失新写入。OS keyring 不复制；同机更新继续
使用原 keyring account，跨机恢复保留 Provider profile/历史，但必须重新输入 Provider API Key。
恢复点只可由未来显式 recovery prune 操作删除，普通更新和应用卸载均不得自动删除。

## Windows exact-commit release build

正式发布包必须在 Windows 上从交接中指定的 40 位 commit 构建。验收人不得用
`origin/main`、PR 页面当前显示的最新 commit、旧 clone 或旧构建产物代替该 commit。建议在
独立验收 clone 和一次性 Windows 账号中执行，不接触真实数据库、附件、账号或 AI key。

Windows 发布环境要求：

- PowerShell 7。
- Rust 1.88，且安装 `rustfmt`、`clippy` 和 MSVC target。
- Node.js >=22.13。
- pnpm 11.5.0。
- Visual Studio C++ Build Tools、Windows SDK 与 Tauri Windows 打包依赖。
- WebView2 Runtime；仍需在不预装 WebView2 的干净 Windows 环境验证安装包的提示或安装策略。
- `MURIARC_DESKTOP_UPDATER_PUBLIC_KEY`：Tauri 格式，即把 UTF-8 Minisign public-key 文本整体
  Base64 后得到的环境变量值。`build.rs` 会实际解码并调用 Minisign parser；release build
  缺失或格式无效时直接失败，debug build 只保留不可用占位符。
- `TAURI_SIGNING_PRIVATE_KEY`：仅存在于受保护发布环境的 Minisign 私钥；不得写入 Git、构建
  transcript、Release Manifest 或验收附件。密码如有设置，通过
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 注入。

PR 合并前生成 Windows 交接时，把下面两个占位符替换为实际值：

- `<PHASE5_PR_NUMBER>`：待验收的 Draft PR 编号。
- `<PHASE5_COMMIT_SHA>`：该 PR 已通过 Linux/WSL 门禁的精确 40 位 commit。

以下 PowerShell 从验收 clone 的仓库根目录执行。它把编译产物和证据放到 Git 工作树以外；
有 `E:\Muriarc` 时沿用统一验收目录，否则回退到当前用户的临时目录。

```powershell
$ErrorActionPreference = "Stop"
$PrNumber = "<PHASE5_PR_NUMBER>"
$ExpectedCommit = "<PHASE5_COMMIT_SHA>"

if ($PrNumber -notmatch "^[0-9]+$") {
  throw "Replace <PHASE5_PR_NUMBER> with the Draft PR number."
}
if ($ExpectedCommit -notmatch "^[0-9a-f]{40}$") {
  throw "Replace <PHASE5_COMMIT_SHA> with the exact 40-character commit."
}

git fetch origin "pull/$PrNumber/head"
if ($LASTEXITCODE -ne 0) { throw "Unable to fetch the PR head." }
git switch --detach FETCH_HEAD
if ($LASTEXITCODE -ne 0) { throw "Unable to check out the PR head." }

$ActualCommit = (git rev-parse HEAD).Trim()
if ($ActualCommit -ne $ExpectedCommit) {
  throw "Commit mismatch: expected $ExpectedCommit, got $ActualCommit."
}
$Dirty = @(git status --porcelain=v1 --untracked-files=all)
if ($Dirty.Count -ne 0) {
  $Dirty | ForEach-Object { Write-Error $_ }
  throw "Acceptance source tree is not clean."
}

$RunId = "{0}-{1}" -f (Get-Date -Format "yyyyMMdd-HHmmss"), $ExpectedCommit.Substring(0, 12)
$AcceptanceBase = if (Test-Path "E:\Muriarc") {
  "E:\Muriarc\acceptance\runs"
} else {
  Join-Path $env:TEMP "MuriArc-acceptance"
}
$EvidenceRoot = Join-Path $AcceptanceBase $RunId
$env:CARGO_TARGET_DIR = if (Test-Path "E:\Muriarc") {
  "E:\Muriarc\builds\cargo-target\windows-acceptance-$($ExpectedCommit.Substring(0, 12))"
} else {
  Join-Path $env:LOCALAPPDATA "MuriArc\builds\cargo-target\windows-acceptance-$($ExpectedCommit.Substring(0, 12))"
}
New-Item -ItemType Directory -Force $EvidenceRoot, $env:CARGO_TARGET_DIR | Out-Null

Start-Transcript -Path (Join-Path $EvidenceRoot "windows-acceptance.log")
git show --no-patch --format=fuller HEAD
git status --short
rustc --version --verbose
cargo --version --verbose
rustup show active-toolchain
node --version
corepack --version
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --version
pnpm --dir ui install --frozen-lockfile
```

任何一条命令非零退出都必须记为 `FAIL` 并停止发布，不得把缺依赖、跳过或环境不可用写成
`PASS`。依赖安装完成后，在同一个 transcript 中运行门禁：

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked -p muriarc-desktop --all-features settings::tests
cargo test --locked -p muriarc-desktop --all-features model_profiles::tests
cargo test --locked -p muriarc-desktop --all-features ai::tests
cargo test --locked -p muriarc-desktop --all-features storage_root::tests
cargo test --locked -p muriarc-desktop --all-features desktop_upgrade::tests
cargo test --locked --workspace --all-features

pnpm --dir ui run test
pnpm --dir ui run typecheck
pnpm --dir ui run build
pnpm --dir ui exec playwright install chromium
pnpm --dir ui run test:e2e

& .\ui\node_modules\.bin\tauri.cmd build
if ($LASTEXITCODE -ne 0) { throw "Tauri release bundle failed." }
```

`src-tauri/tauri.conf.json` 会执行前端生产 build、嵌入 `ui/dist` 并生成 release bundle。若要
定位 Windows-only 启动问题，可另外执行
`.\ui\node_modules\.bin\tauri.cmd build --debug --no-bundle`；debug executable 出现控制台是
预期行为，不能把它作为正式交付物。正式交付只能使用上面 release 命令生成的 Tauri bundle，
不得用 `pnpm run dev`、Vite preview、VNC/noVNC 或远程桌面会话替代。

构建结束后必须同时收集安装包、updater archive、`.sig` 和 SHA-256；只有 MSI/EXE 而没有
`.sig` 视为发布失败：

```powershell
$BundleRoot = Join-Path $env:CARGO_TARGET_DIR "release\bundle"
$Artifacts = @(Get-ChildItem $BundleRoot -Recurse -File |
  Where-Object { $_.Extension -in ".msi", ".exe" })
if ($Artifacts.Count -eq 0) {
  throw "No MSI or NSIS release artifact was produced."
}
$Artifacts |
  Select-Object FullName, Length, LastWriteTimeUtc |
  Format-Table -AutoSize |
  Out-File (Join-Path $EvidenceRoot "bundle-files.txt")
$Artifacts |
  Get-FileHash -Algorithm SHA256 |
  Format-Table -AutoSize |
  Out-File (Join-Path $EvidenceRoot "bundle-sha256.txt")

@"
expected_commit=$ExpectedCommit
actual_commit=$ActualCommit
pr_number=$PrNumber
repo_root=$(Resolve-Path .)
cargo_target=$env:CARGO_TARGET_DIR
bundle_root=$BundleRoot
evidence_root=$EvidenceRoot
"@ | Set-Content (Join-Path $EvidenceRoot "source-and-paths.txt")

Stop-Transcript
```

## Windows runtime acceptance

先保存 transcript、`source-and-paths.txt`、`bundle-files.txt` 和 `bundle-sha256.txt`，再安装刚刚
哈希的 release bundle。运行验收使用一次性数据和无生产权限的测试 key；不得把 key 写入证据。

| 场景 | 必须观察到的结果 | 证据 |
|---|---|---|
| 原生启动 | 从开始菜单或安装目录打开 MuriArc 原生 WebView2 窗口；没有浏览器地址栏、noVNC toolbar 或远程桌面边框 | 窗口截图、安装包文件名与 SHA-256 |
| 本地持久化 | 新建笼位、动物和附件，关闭并重启后数据仍在；重新打开 WebView 时本地空间欢迎状态符合 `docs/DELIVERY_ACCEPTANCE.md` | 重启前后截图与测试记录 |
| 数据目录迁移 | 使用测试数据选择独立空目录，确认后当前进程不切换；完全退出并重启后 SQLite、附件、数据产物与非敏感 AI 配置整体可用，旧源仍存在 | 迁移前后脱敏路径、文件数量/大小/SHA-256 与功能截图 |
| 迁移失败关闭 | 目标磁盘缺失、目标含未知文件、校验失败或安装目录被选择时，不创建空数据库、不切换 locator，并显示安全错误 | 错误截图、旧源仍可读取的记录 |
| 恢复默认目录 | 从自定义目录恢复默认后完整数据可用，自定义源仍存在，默认目录旧 managed entries 位于 `storage-backups/<migration-id>/` | 脱敏目录清单与重启后功能截图 |
| 旧 JSON 升级 | 用旧版安装包在一次性账号生成 `ai-provider.json`，升级后文本/视觉配置迁移为模型档案；旧会话与旧文件仍保留 | 升级前后脱敏配置、模型列表截图 |
| 旧 Keyring 升级 | 旧单 Provider 测试 key 升级后只显示“密钥已配置”，文本与视觉档案可各自使用；旧 Keyring 项不被删除，新建无关档案不继承它 | 脱敏状态截图和步骤记录，不记录 key 值 |
| 密钥轮换与清除 | 同一档案轮换 key 后当前版本可用，历史版本绑定不被改写；清除后重启仍是未配置，残留旧项不能复活 | 模型详情截图、重启结果、相关测试日志 |
| 档案停用 | 停用档案后它不再出现在可选模型中；若它曾是默认对话/视觉模型，对应默认值被清空 | 停用前后截图 |
| 默认模型失效 | 默认 ID 缺失、停用或不可用时，新会话要求明确选择，不静默使用列表第一项，也不调用 Provider | 新会话截图与本地 Mock Provider 零请求记录 |
| 历史会话只读 | `legacy_model_unknown`、`model_archived`、`model_unavailable` 三类历史仍能读取消息，但 composer 禁用且不能发送 Provider 请求 | 三类会话截图与本地 Mock Provider 零请求记录 |
| 多模型与视觉 | 三种协议档案均能保存自由模型 ID；支持视觉时直接路由，不支持时必须明确选择视觉模型后中转 | 本地 Mock Provider 请求/响应记录，不用真实厂商 API |
| 数据与密钥边界 | SQLite、日志、audit、snapshot、locator、migration intent、marker、前端状态和证据文件中都没有 API key；图片与附件仍在当前 data root 范围 | 脱敏扫描结果与数据目录记录 |
| 签名 Candidate 升级 | 从旧正式安装包生成含附件、AI 历史和 Audit 的数据，使用 `latest.json` 指向的签名 updater 升级；旧 generation、完整 recovery copy 与新 Candidate 均可核对，切换后旧记录可读可继续写 | 两个安装包/更新制品 digest、Journal phase、脱敏 Expected Facts 与 UI 截图 |
| 更新故障恢复 | 分别在 recovery copy、Candidate migration、Candidate verification 和 locator 切换前中断；重启后幂等恢复或保持 source locator，禁止创建空数据库 | 每个故障点的 Journal、locator 与两份数据 SHA-256 |
| 首次写入边界 | 切换后未写入时可按恢复设计回到 source；完成一笔新版业务写入后自动降级必须被拒绝 | `first_write_at` 脱敏状态和 forward-fix/显式恢复提示 |
| 升级/重装 | 升级、卸载和重装不会静默删除 application data 或最后验证恢复点；清除数据只能由用户显式执行 | 操作记录与重装后截图 |

最终交接报告逐条写 `PASS` 或 `FAIL`，并包含：

- PR 编号、expected commit、actual commit 和 `git status --short`。
- Windows、PowerShell、Rust、Cargo、Node、pnpm、WebView2 版本。
- 每条测试命令的退出码、通过/失败/跳过数量；跳过项必须说明原因，且不计作通过。
- release bundle 的绝对路径、大小与 SHA-256。
- 上表每个场景的证据文件名。
- 任一失败的首个根因、完整错误日志位置和是否阻止发布。

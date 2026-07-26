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
- **data root**：统一保存 `muriarc.sqlite3`、`attachments/`、`data/` 和非敏感
  `ai-provider.json`。数据库、附件或数据产物不得单独迁移到不同位置。

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

构建结束后收集 release bundle 清单与 SHA-256：

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
| 升级/重装 | 升级、卸载和重装不会静默删除 application data；清除数据只能由用户显式执行 | 操作记录与重装后截图 |

最终交接报告逐条写 `PASS` 或 `FAIL`，并包含：

- PR 编号、expected commit、actual commit 和 `git status --short`。
- Windows、PowerShell、Rust、Cargo、Node、pnpm、WebView2 版本。
- 每条测试命令的退出码、通过/失败/跳过数量；跳过项必须说明原因，且不计作通过。
- release bundle 的绝对路径、大小与 SHA-256。
- 上表每个场景的证据文件名。
- 任一失败的首个根因、完整错误日志位置和是否阻止发布。

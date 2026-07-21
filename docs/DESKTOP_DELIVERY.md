# MuriArc Desktop local delivery

MuriArc Desktop 的正式本地交付目标是 Windows Tauri WebView 安装包。用户启动的是
MuriArc 原生窗口，窗口内加载随应用打包的 Vue 前端，并通过 Tauri IPC 调用本地
`LocalTauriGateway`。Desktop 不通过 VNC、noVNC、浏览器远程桌面或 Server Docker
部署交付。

## Runtime shape

- UI：随 Tauri 包内置的 `ui/dist`，运行在 Windows WebView2 中。
- Transport：`LocalTauriGateway` 调用 Tauri commands；不开放本地 HTTP API。
- Data：SQLite 位于 OS application data 下的 `muriarc.sqlite3`。
- Files：附件与数据任务文件位于同一 application data 根目录下的 `attachments/` 和
  `data/` 子目录。
- Secrets：Desktop AI key 写入 OS keyring；项目数据库、日志、审计、快照和前端状态不得记录
  API key。
- Identity：每个 WebView 会话显示一次无密码“进入本地空间”；这只是 Lab 与
  LocalOperator 确认，不是登录、磁盘加密或操作系统权限边界。

Server 是另一种运行形态：Axum + PostgreSQL + HTTPS reverse proxy + 账号与权限体系。
需要浏览器访问、多人协作或远程部署时使用 Server；需要个人本地离线使用时使用 Desktop
安装包。V1 不提供 Local Web、本地 Axum+SQLite 浏览器服务或 Desktop/Server 实时同步。

## Windows release build

正式发布包在发布/合并验收阶段由 Windows 环境构建和验收。常规分支检查只维护配置、文档和
非发布验证，不把 VNC 作为交付兜底。

Windows 发布环境要求：

- Rust 1.88。
- Node.js >=22.13。
- pnpm 11.5.0。
- Tauri Windows 打包依赖。
- WebView2 runtime 策略必须在干净 Windows 机器上验证：安装包应能安装或明确提示用户安装
  WebView2，不能退化为 VNC/noVNC 访问方式。

参考命令：

```powershell
corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install --frozen-lockfile
.\ui\node_modules\.bin\tauri.cmd build
```

`src-tauri/tauri.conf.json` 负责调用前端 build、嵌入 `ui/dist`、设置窗口、图标、产品名和
bundle 开关。发布产物必须来自 Tauri bundle，不得把 `pnpm run dev`、Vite preview、noVNC
或远程桌面会话作为 Desktop 安装包替代物。

## Acceptance

Desktop 本地交付验收至少覆盖：

1. 在 Windows 上安装正式包并从开始菜单或安装目录启动 MuriArc。
2. 首屏是原生 MuriArc 窗口内的本地空间欢迎页；没有浏览器地址栏、noVNC toolbar、远程桌面
   边框或缩放层。
3. 新建笼位、动物和附件后关闭应用，再重新启动，确认 SQLite 数据和附件仍存在。
4. 刷新或重开 WebView 会话时，“进入本地空间”的 `sessionStorage` 行为符合
   `docs/DELIVERY_ACCEPTANCE.md`。
5. 保存 AI 设置后确认 API key 不进入数据库、日志、审计、快照或前端状态；清除 key 后再次
   启动仍保持清除状态。
6. 卸载、重装或升级包不得静默删除 application data；任何清除本地数据的动作必须由用户在
   OS 或明确的产品流程中主动执行。

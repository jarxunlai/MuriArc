# MuriArc 配置使用指南

> [English](CONFIGURATION.md) | 简体中文

## 范围与发布状态

本文是选择、下载、配置、运行和备份 MuriArc Desktop/Server 的统一公开入口。当前源码身份为 `1.0.0 / E0001 / permanent-upgrade`，但真实 1.0 RC 尚未通过，也没有宣称已发布正式 `v1.0.0`。

现有 Windows Tester 与 Server Tester 都是未签名测试交付，不是生产 Release，也不构成正式 RC 证据。在建立真实数据隐私、权限和恢复制度前，只应使用合成数据。

## 1. 选择运行方式

| 需求 | 推荐形态 | 存储 | 网络 | 交付边界 |
| --- | --- | --- | --- | --- |
| 单个可信 Windows 账号下由一位研究者使用 | Windows Desktop Tester | 本地 SQLite、附件与 OS keyring | 不需要 Server | 未签名 Tester；仅合成测试数据 |
| 在一台电脑体验共享 Web UI | Docker Desktop/Linux 上的 Server Docker Tester | 私有 PostgreSQL 与 `server_data` 命名 volumes | Loopback `127.0.0.1` | 未签名；默认空库，可选 standard-v1 合成数据 |
| 可信局域网/VPN 内多人测试 | 私网 HTTPS 反向代理后的 Server Docker Tester | 同上，另有宿主机 `.env` | 仅私有 LAN/VPN | 仅评估；不是公网或生产 Profile |
| 开发或审查源码 | 干净 Git checkout/worktree | worktree 隔离运行数据 | 由开发者控制 | 源码构建，不是发布制品 |
| 生产、公网、签名升级或真实数据恢复 | 签名正式 Server 交付 | 数据库/数据/配置/密钥联合恢复集合 | 经批准的私网或 Cloudflare Profile | 尚未发布；遵循[Server 正式交付](SERVER_DELIVERY_cn.md) |

Desktop 是 Tauri + SQLite；Server 是 Axum + PostgreSQL。Windows x64 可通过 Docker Desktop Linux containers 运行 `linux/amd64` Server Tester；不提供 Windows 原生 Server 镜像。

## 2. 下载与 SHA-256 校验

进入 [GitHub Releases 页面](https://github.com/jarxunlai/MuriArc/releases)：

- Windows Tester：选择 tag 以 `tester-v1.0.0-standard-v1-` 开头的 prerelease；
- Server Docker Tester：选择 tag 以 `server-tester-v1.0.0-standard-v1-` 开头的 prerelease；
- 禁止用名称相似的第三方压缩包或浮动容器 tag 替代。

解压前先校验下载文件。

Linux/macOS/WSL：

```bash
sha256sum --check MuriArc-*.zip.sha256
```

PowerShell：

```powershell
$Expected = (Get-Content .\MuriArc-*.zip.sha256).Split(' ')[0]
$Actual = (Get-FileHash .\MuriArc-*.zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($Actual -ne $Expected) { throw 'SHA-256 mismatch' }
```

Server Tester ZIP 内还有 `CHECKSUMS.sha256`。选择并编辑环境模板后，运行 `muriarc-tester.sh verify` 或 `muriarc-tester.ps1 verify`；它会校验包内每个文件、Compose 安全策略和两个不可变镜像引用。

## 3. Windows Desktop 首次使用

1. 校验并把 Windows Tester ZIP 解压到当前用户拥有的目录。
2. 运行包内验证启动器；出现 checksum 或 manifest 错误时不得绕过。
3. 首次启动会把不可变 standard-v1 合成基线复制到当前 Windows 用户的 LocalAppData；后续人工修改保存在用户数据根，不会回写解压目录。
4. Desktop 使用 SQLite 和本地附件/数据根，不是浏览器 Server；不得把数据库放在网络共享盘供多人同时打开。
5. 关闭 MuriArc 后备份完整 Desktop 数据根；SQLite、附件、数据产物、storage marker 与密钥引用必须一起保存。
6. 每位 Windows 用户在设置页添加自己的 AI Provider 档案与 API Key。Key 由 OS keyring 引用，不进入项目文件、截图或共享配置。

unsigned Tester 只用于评估。正式 Desktop 交付还要求签名安装包/Updater、干净 GitHub commit 和已接受的 Release 证据。详见 [Desktop 交付](DESKTOP_DELIVERY_cn.md)。

## 4. Server Docker 前置条件

安装：

- Docker Engine，或启用 Linux containers 的 Docker Desktop；
- Docker Compose v2（`docker compose version`）；
- amd64 CPU/运行时、至少 4 GiB 可用内存和足够的持久磁盘；
- Bash 方式需要 `curl`、`sha256sum`，Windows 可使用 PowerShell 脚本。

Docker 用户可以控制数据库并查看进程配置，因此必须限制 Docker 组成员，禁止在不可信共享宿主机运行本 Tester。

解压 Server Tester ZIP 后选择一种模板：

```bash
cp .env.empty.example .env  # 推荐：全新空数据库
# 或
cp .env.demo.example .env   # 显式加载 standard-v1 合成演示数据
chmod 600 .env
```

替换所有 `REPLACE_` 值。同机运行多个副本时使用不同 Compose project name 与宿主端口。密码使用 URL-safe 字符，避免破坏 PostgreSQL URL：

```bash
openssl rand -hex 32  # PostgreSQL 密码
openssl rand -hex 32  # Environment Root 使用另一个值
```

禁止把 `.env`、解析后的 `docker compose config`、`docker inspect`、Cookie、CSRF、Token、Provider 请求体或含私有路径的日志粘贴到 Issue。

## 5. 初始化空库或演示数据

### 空数据库（默认）

```bash
./muriarc-tester.sh verify
./muriarc-tester.sh init-empty
```

PowerShell：

```powershell
.\muriarc-tester.ps1 verify
.\muriarc-tester.ps1 init-empty
```

`init-empty` 只要发现当前 project 的容器或命名 volume 已存在就拒绝执行。只有证明两个 volumes 都不存在后，才临时启用 preview bootstrap；等待 `/readyz` 后停止 Server，再用 bootstrap=false 启动同一部署。禁止手动长期保持 bootstrap=true。

### standard-v1 合成演示数据

```bash
./muriarc-tester.sh verify
./muriarc-tester.sh init-demo
```

Demo 同样只允许全新 volumes。脚本先启动私有 PostgreSQL，再运行镜像内严格 PostgreSQL Seeder，核对固定 dataset digest 和领域数量，安装匹配的 generation manifest，最后以 bootstrap=false 启动 Server。

Demo 的固定身份不得修改：

- Lab：`4d555249-4152-4300-0000-000000000001`
- Environment Root/User：`4d555249-4152-4300-0000-000000000002`

Root 邮箱、显示名和密码由用户填写。Server 启动时会把已有合成 User 核对为 Environment Root，并创建 LabAdmin membership 与凭据。第二次初始化会被拒绝。Seed/verify 失败时保留现场，禁止清库或 SQL 修补。

## 6. Server Tester 完整环境变量表

| 变量 | 必填/默认 | 安全等级 | 修改影响 |
| --- | --- | --- | --- |
| `MURIARC_TESTER_DATASET_MODE` | 必填：`empty` 或 `demo` | 公开 | 必须与一次性 init 命令一致；已有部署禁止切换 |
| `MURIARC_COMPOSE_PROJECT_NAME` | 模板给默认值 | 公开 | 决定容器、网络、volume 名称；修改等于选择另一套部署 |
| `MURIARC_TESTER_SOURCE_COMMIT` | 由包固定 | 公开 | 必须与镜像/fixture commit 一致，禁止修改 |
| `MURIARC_TESTER_SERVER_PORT` | `8787` | 公开 | 只改变 loopback 宿主端口 |
| `MURIARC_POSTGRES_DB` | `muriarc` | 内部 | 初始化后修改会指向另一数据库 |
| `MURIARC_POSTGRES_USER` | `muriarc` | 内部 | 必须与 PostgreSQL volume 所有者/配置一致 |
| `MURIARC_POSTGRES_PASSWORD` | 必填，32+ URL-safe 字符 | 秘密 | DB/Server 同时需要；修改必须协调轮换 PostgreSQL 凭据 |
| `MURIARC_DATA_ROOT` | 模板固定路径 | 内部 | 属于恢复/generation 边界，初始化后禁止修改 |
| `MURIARC_ATTACHMENT_ROOT` | 模板固定路径 | 内部 | 必须与 PostgreSQL 联合备份；错误修改会表现为数据丢失 |
| `MURIARC_AI_MASTER_KEY_FILE` | 模板固定路径 | 关键秘密路径 | 自动生成文件用于解密按用户 Provider 凭据；丢失会阻断 AI 凭据恢复 |
| `MURIARC_LAB_ID` | 稳定 UUID；Demo 固定 | 内部身份 | 修改可能与既有记录冲突并 fail closed |
| `MURIARC_LAB_NAME` | 必填显示名 | 实验室内公开 | 每次 Server 启动核对 |
| `MURIARC_ROOT_USER_ID` | 稳定 UUID，须不同于 Lab；Demo 固定 | 内部身份 | 修改会选择/核对另一 Root，可能撤销 Session |
| `MURIARC_ROOT_USER_EMAIL` | 必填 | 个人/敏感 | 登录标识，规范化后必须唯一 |
| `MURIARC_ROOT_USER_NAME` | 必填 | 个人/敏感 | 显示于 UI 和 Audit actor |
| `MURIARC_ROOT_PASSWORD` | 必填，32+ URL-safe 字符且唯一 | 关键秘密 | 核对时轮换凭据并撤销相关 Session |
| `MURIARC_SESSION_COOKIE_SECURE` | 本机 HTTP 为 `false` | 安全控制 | HTTPS 后必须为 `true`；Secure Cookie 不会通过明文 HTTP 发送 |
| `MURIARC_SESSION_TTL_HOURS` | `12`，允许 `1..720` | 安全控制 | 修改未来 Session 的过期策略 |
| `RUST_LOG` | 默认 Server/info | 运维 | 更详细日志增加泄露风险，禁止开启 body/secret 日志 |

镜像引用直接以不可变 `@sha256:` 写入 `compose.yaml`，不创建 `latest`。PostgreSQL 不发布宿主端口，Server 只发布 `127.0.0.1:<port>`。

禁止向 Tester `.env`/Compose 添加 `MURIARC_AI_MASTER_KEY` 明文、bootstrap bearer Token、Provider API Key、Cloudflare secret 或 Docker socket mount。

## 7. Environment Root、Session 与私网反向代理

Environment Root 是恢复管理员。每次 read-write Server 启动时，MuriArc 都会在事务中核对 Lab、Root User、LabAdmin membership 和 Argon2id 凭据；身份碰撞、跨 Lab 归属、软删除记录、非法邮箱或不支持的凭据会 fail closed。

浏览器登录返回 HttpOnly Session Cookie 和 CSRF Token；两者都禁止复制。Logout、密码轮换、账号停用、membership 撤销、过期和强制改密状态都会通过持久状态执行。

本机直接使用：

```text
http://127.0.0.1:8787
MURIARC_SESSION_COOKIE_SECURE=false
```

可信 LAN/VPN 使用时，容器端口继续保持 loopback，在宿主机增加自主管理的反向代理：终止 HTTPS、只允许指定私网 hostname、禁止缓存认证 API、设置 `MURIARC_SESSION_COOKIE_SECURE=true`，并限制防火墙/VPN。unsigned Tester 不支持公网。公网 Cloudflare 部署属于正式 [Cloudflare Public Profile](CLOUDFLARE_PUBLIC_PROFILE_cn.md)，不能照搬到本包。

## 8. 每用户 AI Provider 配置

MuriArc 不提供共享 Provider Key。每位已登录 Server 用户在 AI 设置页创建自己拥有的 Provider/模型档案，配置：

- 协议与精确 Base URL；
- Model ID 与能力标记；
- Context、Input、Output 与 History budget；
- Timeout/Temperature；
- 该用户自己的 API Key。

Key 由部署 AI Master Key 加密，并绑定用户/档案版本。验证 timeout 或预算错误会保持表单未保存，也不意味着日常对话 context window 等于同一验证预算。Provider Key 禁止进入 `.env`、Lab preset、截图、日志或朋友测试包。

## 9. 状态、日志、停止与恢复启动

```bash
./muriarc-tester.sh status
./muriarc-tester.sh logs
./muriarc-tester.sh down  # 停止容器，命名 volumes 保留
./muriarc-tester.sh up
```

PowerShell 提供同名命令。本包没有 `destroy`；`up` 会拒绝未初始化环境；`down` 永不传递 `--volumes`。

有效 Server 备份必须联合包含：

1. PostgreSQL volume/数据库；
2. `server_data` 中的附件、generation manifest、数据产物和自动生成的 AI Master Key；
3. 精确 `.env`、Compose ZIP manifest、source commit 与 image digest。

恢复必须使用隔离 project name，验证登录、`/readyz`、项目/动物、附件字节、Audit/Provenance、AI 历史引用和继续写入。普通业务 Snapshot 不能替代 Server 灾难恢复备份。

## 10. 故障排查与危险操作

| 现象 | 安全处理 |
| --- | --- |
| `verify` 报 checksum mismatch | 删除当前解压目录，重新下载 Release 资产并校验外层 SHA-256 |
| 无法匿名 inspect 镜像 | 确认 GHCR package 为 Public 且 digest 与 manifest 一致；禁止换成 tag |
| 端口被占用 | 停止另一服务，或在初始化前修改 `MURIARC_TESTER_SERVER_PORT` |
| `init-*` 报 volume 已存在 | 该部署使用 `up`；新测试改用新的 project name |
| `/readyz` 失败 | 执行 `status`、查看脱敏 `logs`；保留 volumes 并记录 Release tag/commit |
| Demo verify 报基线漂移 | 停止并保留现场；禁止再次 seed、清行或 SQL 修补 |
| AI 凭据无法解密 | 用匹配的 PostgreSQL 备份恢复同一 `server_data`/Master Key |
| HTTPS 反代后登录循环 | 核对 proxy scheme/host 与 `MURIARC_SESSION_COOKIE_SECURE=true` |

禁止执行 `docker compose down --volumes`、删除命名 volumes、清空 PostgreSQL、修改 migration 历史、手改 standard-v1 行、挂载 `/var/run/docker.sock`、暴露 `5432`、把 `8787` 发布到 `0.0.0.0`，或把 unsigned 包宣称为生产/RC 证据。

源码开发者必须使用干净 worktree，并阅读[环境](ENVIRONMENTS_cn.md)和[Server 部署](DEPLOYMENT_cn.md)。正式 Server 制品与升级控制另见[Server 正式交付](SERVER_DELIVERY_cn.md)和 [Upgrade Engine](UPGRADE_ENGINE_cn.md)。

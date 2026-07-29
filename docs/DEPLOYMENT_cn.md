# MuriArc Server 部署

> [English](DEPLOYMENT.md) | 简体中文

## 范围与状态

普通用户在 Windows Desktop、本机 Docker 与远程私网 Server 之间选择时，应先阅读[配置使用指南](CONFIGURATION_cn.md)。其中的 Server Docker Tester 是未签名评估制品；本文继续说明源码 checkout 与正式交付边界。

本文覆盖共享 Server 形态的源码 checkout 开发。当前候选源码身份为 `1.0.0 / E0001 / permanent-upgrade`，但根目录 Compose 不是签名正式制品，真实 RC 也尚未通过。稳定 Native/systemd 和 Managed Compose 合同见 [Server 正式交付](SERVER_DELIVERY_cn.md)。

MuriArc Server 由 Axum + PostgreSQL + 响应式 Vue UI 组成。应用端口默认只发布到 loopback；PostgreSQL 必须保持私有，生产 TLS 在可信反向代理或文档规定的 Cloudflare Tunnel 终止。

Desktop 是 Tauri + SQLite 的另一种形态，不通过 Docker、VNC 或 noVNC 部署。

## 1. 准备配置

```bash
cp .env.example .env
chmod 600 .env
```

替换所有占位值，至少包括：

- PostgreSQL database、role 和密码；
- 持久化 data/attachment root；
- 稳定的 Lab 与 Environment Root UUID、显示信息和 Root 密码；
- Cookie 安全与生命周期；
- AI Master Key 来源/版本；
- 默认关闭、且仅可用于一次性全新本地空栈的源码开发 bootstrap；
- 可选外部 API 与 MCP origin。

使用独立随机值，不复用个人密码：

```bash
openssl rand -hex 32
openssl rand -base64 24
uuidgen  # Lab ID
uuidgen  # Root user ID；必须与 Lab ID 不同
```

`600` 只能阻止普通用户，不能阻止宿主管理员、Docker daemon、进程环境采集或未加密备份。配置备份必须加密并控制访问，限制 Docker 组成员；禁止把 `.env`、`docker inspect` 或解析后的 Compose 输出附到 issue。

### Environment Root

Server 每次启动都在 PostgreSQL 事务与 advisory lock 下核对 Root，创建或验证 Lab、User、LabAdmin membership 与 Argon2id credential。身份冲突、软删除、重复规范化邮箱、跨 Lab 归属或不支持的 hash 会 fail closed。

轮换 Root 身份/密码时只编辑宿主环境并重启 Server；成功变更会撤销旧 Root Session。UI 不能读取旧密码，也不能静默改写部署文件。

### AI Master Key

真正空部署在未注入环境 Key 时，可以在受保护 data-root secrets 目录生成一个稳定 32-byte Base64 Key。该文件必须与 PostgreSQL、附件、配置和 generation metadata 联合备份。

若数据库已有加密凭据但原 Key 不可用，启动必须失败，不能生成替代 Key。`MURIARC_AI_MASTER_KEY_VERSION` 保持不变，直到文档化轮换已重新加密全部用户/档案秘密。每个用户使用自己的 Provider Key；未配置 Key 时不发外部请求。

## 2. 校验并启动源码 stack

```bash
docker compose config --quiet
docker compose build server
docker compose up -d --wait --wait-timeout 180
curl --noproxy '*' --fail http://127.0.0.1:8787/api/v1/health
```

根 Compose 只用于开发验收，并默认关闭 bootstrap。只有一次性、全新的本地空栈才可设置 `MURIARC_PREVIEW_BOOTSTRAP=true`；严禁用它重标或修补既有数据，也不得绕过稳定版 `muriarcctl` 升级控制。

使用 `docker compose ps` 和脱敏应用日志诊断。禁止把环境、Cookie、CSRF、Token、密码、Master Key、Provider body 或私有 object path 写入日志/工单。

## 3. 浏览器 Session 与 CSRF

登录返回 opaque HttpOnly Cookie 和当前 Session 的 CSRF。UI 只在内存保存 CSRF，并在状态变更请求中发送。生产环境必须使用 HTTPS 和 `MURIARC_SESSION_COOKIE_SECURE=true`。

退出撤销当前 Session。改密和 Root 环境核对会撤销受影响的其他 Session。停用/删除用户、撤销 membership、Token 过期和强制改密状态在每个认证请求实时生效。

## 4. 反向代理与 Origin 边界

常规反向代理应：

- 终止 TLS；
- 只转发指定应用 host；
- 保留请求大小/超时边界；
- 保持 PostgreSQL 和容器网络私有；
- 只在需要时转发 WebSocket/stream；
- 不缓存认证 API 或私有附件响应。

不得信任任意 forwarded host/proto。浏览器 MCP 使用精确 trusted origin；非浏览器 MCP 仍需可撤销 AI scope Token。

公网暴露必须使用 [Cloudflare 公网 Profile](CLOUDFLARE_PUBLIC_PROFILE_cn.md)，不得把 8787 直接开放到 Internet。

## 5. 外部 Token 与 MCP

持久外部 Token 绑定用户、可撤销、可过期且 scope 有限，只能进一步收窄实时用户权限。生产/公网 Profile 默认关闭外部 bearer REST/MCP。

Bootstrap bearer 只是 preview adapter，不是生产凭据。临时启用时使用彼此独立的高熵值、保留在 Git 外，并在持久登录/Token 流程可用后移除。

## 6. 备份与恢复

一个恢复集合必须联合包含：

- PostgreSQL；
- data 与 attachment root；
- 部署配置；
- `deployment-generation.json` 与 control state；
- AI Master Key/Keyset 和非明文 AI 状态。

备份只有在隔离环境实际恢复并验证 Storage、Store/Application、真实 API/UI 读取、附件字节、AI 历史引用、Audit/Provenance 和继续写入不变量后才有效。禁止在唯一在线副本上测试恢复。

普通业务 Snapshot 不能替代数据库/附件恢复集合。

## 7. 运维清单

非开发启动前：

1. 固定准确源码/制品身份和当前发布状态。
2. 确认所有占位符已替换，秘密文件 ownership/mode 受限。
3. PostgreSQL 保持私有，只向 proxy/Tunnel 暴露 loopback。
4. 核对 Secure Cookie、精确 origin、Session 生命周期和外部 API 策略。
5. 验证 Root 登录、强制改密、退出、CSRF、停用与 Token 撤销。
6. 使用 mock Provider 验证 AI 用户隔离，禁止把 Root Key 借给其他用户。
7. 创建并实际恢复联合恢复集合。
8. 脱敏记录 health、compatibility、storage、UI 与 generation 结果。

1.0+ 签名升级和维护窗口继续阅读 [Server 正式交付](SERVER_DELIVERY_cn.md)与 [Upgrade Engine](UPGRADE_ENGINE_cn.md)。

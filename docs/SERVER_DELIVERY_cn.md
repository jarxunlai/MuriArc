# MuriArc Server 正式交付

> [English](SERVER_DELIVERY.md) | 简体中文

## 状态

本文定义 `1.0.0 / E0001 / permanent-upgrade` 候选及后续版本的正式 Server 交付边界。合同测试、候选源码身份和模板通过不等于真实物理 RC 已通过。

根目录开发 Compose 只用于源码/preview。正式安装只能从一个签名 Server bundle 选择 `native-system` 或 `managed-compose`，二者不能混用。

## 权限与恢复边界

长期运行的 `muriarc-server` 是低权限应用进程：

- 不拥有 systemd、Docker socket、发布签名、备份编排或数据库 DDL；
- Native 读取不可变 release/config，只写当前 generation 数据；
- Managed Compose 只挂应用数据，不挂 Docker socket；
- 一次性 upgrade executor 只对隔离 Candidate 执行 DDL；
- 宿主 `muriarcctl` 承担安装、备份、恢复、激活与服务控制。

可恢复 generation 必须联合 PostgreSQL、data/attachments、配置、Keyset/Master Key、AI 状态和 `deployment-generation.json`。只恢复数据库或 volume 都不完整。

## 签名 bundle

Bundle packager 只接收已经构建的最终二进制和 UI 资产，拒绝 symlink、空资产、路径逃逸、已存在输出及 Git 工作树内输出，并生成闭合 `bundle-manifest.json`；发布流水线把其 object digest 固定到签名 target metadata。

外部 Release Manifest 在 Native、Compose、Desktop、provenance 和签名证据的最终 digest 已知后生成，不嵌入它描述的 bundle，避免 digest 自引用。安装同时验证签名外层 target 和闭合内层 bundle manifest。

## Native/systemd Profile

固定布局：

```text
/opt/muriarc/releases/<version>/       不可变 release
/opt/muriarc/current                   原子 release symlink
/etc/muriarc/server.env                root:muriarc 0640
/var/lib/muriarc/control/active.env    root:root 0600
/var/lib/muriarc/generations/<uuid>/   generation data/attachments/keyset
/var/lib/muriarc/backups/              已验证恢复点
/var/lib/muriarc/candidates/           Candidate control data
```

安装验证并 stage bundle，安装 systemd/sysusers/tmpfiles，enable 但不自动 start。管理员建立真实受保护 config/control、准备匹配 generation、运行 `muriarcctl doctor` 后再启动。

`/livez` 只表示进程存活；`/readyz` 还要求准确 Epoch/Digest/Generation、有效 activation/lease、真实 data/attachment root、可用 AI Master Key 和 UI 资产。

## Managed Compose Profile

Managed Compose 使用绝对 install root、digest-pinned image、宿主机 `server.env`/`active.env` 和固定控制器调用，禁止 `build:`、浮动 tag、Watchtower、绕过控制器直接 `pull/up`，以及 Docker-socket mount。

PostgreSQL 不发布 host port；Server 只绑定 loopback。不同 generation/Candidate 使用不重叠的 PostgreSQL database 与 data path；Candidate 关闭外部 Provider、后台 Job 与真实用户写入。

## Physical Driver 配置

`muriarcctl` 默认从经 receipt 验证的不可变安装 bundle 加载 `bin/muriarc-physical-driver`。`MURIARCCTL_PHYSICAL_DRIVER` 仅用于受控测试，必须指向绝对路径、非 symlink 的控制文件；正式生产交付保持未设置并使用已安装 Driver。

运行 `muriarcctl` 的宿主机必须提供 位于 `/usr/lib/postgresql/17/bin/pg_dump` 和 `/usr/lib/postgresql/17/bin/pg_restore` 的 PostgreSQL 17+ clients，并配置：

| 变量 | 要求 |
|---|---|
| `MURIARCCTL_BACKUP_RECIPIENT_FILE` | age 加密 recipient 文件的绝对路径，文件必须非空。 |
| `MURIARCCTL_BACKUP_IDENTITY_FILE` | 私有 age identity 的绝对路径；必须是非 symlink 文件且仅所有者可读写（`0600`）。 |
| `MURIARCCTL_AGE_EXECUTABLE` | 可选的 age 可执行文件绝对路径；默认 `/usr/bin/age`。 |
| `MURIARCCTL_PG_DUMP_EXECUTABLE` | 可选的 PostgreSQL 17+ `pg_dump` 绝对路径；默认 `/usr/lib/postgresql/17/bin/pg_dump`。 |
| `MURIARCCTL_PG_RESTORE_EXECUTABLE` | 可选的 PostgreSQL 17+ `pg_restore` 绝对路径；默认 `/usr/lib/postgresql/17/bin/pg_restore`。 |
| `MURIARCCTL_POSTGRES_ADMIN_URL` | 仅 Native/systemd：用于建立、clone 和删除隔离数据库的受保护 PostgreSQL admin URL。 |

Managed Compose 同样要求宿主侧 PostgreSQL clients、age、recipient 与 identity；仅有 Compose 服务正在运行不能证明 backup/restore ready。`muriarcctl doctor` 只报告 readiness 布尔值，不输出这些路径或内容。admin URL 和私有 identity 必须放在 root 控制配置中；密码、identity、Token 或任何密钥材料不得复制到工单、对话、日志或 Git。

## 升级与维护窗口

固定顺序为：签名 target 验证、三锁、preflight、drain、冻结 Write Lease、联合备份、实际隔离恢复、Candidate migration、七层验证、原子激活、只读启动验证、开放新 Write Lease。

单节点不承诺零停机：

- M0：UI/无 schema，通常只有短切换；
- M1：短时冻结写入；
- M2：明确只读维护窗口；
- M3：离线结构迁移。

只读激活是控制面状态，不是普通业务服务；Session touch 也可能写入，因此公共流量保持 gate，直到最终 readiness。

新 generation 首次写入前可以恢复已验证 source；首次写入后永久拒绝自动降级。

## BYO 与恢复点

BYO PostgreSQL/存储只有在证明 PostgreSQL 17、隔离 Candidate database、完整 dump/restore、generation 目录复制、DDL executor、七层 verifier 和服务控制后才接受。缺少任一能力时 `doctor/upgrade` 失败，不回退在线原地 migration。

最后一个已实际恢复并验证的 recovery point 不会自动 prune。显式 `muriarcctl recovery prune` 只能删除指定更早恢复点。dump、附件、Key、Journal 和报告保留在 Git 外。

## Cloudflare 与最终 RC

公网入口使用独立宿主 `cloudflared` 模板和 [Cloudflare 公网 Profile](CLOUDFLARE_PUBLIC_PROFILE_cn.md)，禁止直接暴露 Origin。

最终 RC 把 Native/systemd、Managed Compose、Windows Desktop、Release Manifest、artifact lock、签名/provenance、E0001 SQLite/PostgreSQL fixture、完整历史矩阵、恢复/故障注入、首次写入边界、签名攻击和 Cloudflare staging 绑定到同一 digest。任何 FAIL、SKIP、缺少真实 Driver 或空 Candidate Catalog 都阻断发布。

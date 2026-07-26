# Feature Spec: Native/systemd 与 Managed Compose 安全交付

> 本分支只实现 Server 两种正式交付 Driver、制品布局和服务生命周期，不复制 Upgrade Engine 状态机，也不把 Docker/systemd 权限交给长期运行的 Server。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/native-compose-delivery` |
| 基于提交 | `feature/release-fixtures-gates@0461187` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-native-compose-delivery` |
| 建立日期 | `2026-07-27` |

## 目标

提供可签名、可验证、权限分离的 Native/systemd 与 Managed Compose bundle；让 `muriarcctl` 能检查和安装 profile，并让正式升级 Driver 通过共享 Engine 执行服务控制、联合备份、Candidate 与切换。任何未满足的 BYO PostgreSQL/存储能力必须 fail closed。

## 实现范围

- [x] 新增共享 delivery crate，固定 Bundle manifest、文件 digest、路径与权限契约。
- [x] Native 布局使用 `/opt/muriarc/releases/<version>`、`/opt/muriarc/current`、`/etc/muriarc`、`/var/lib/muriarc` 和专用系统用户。
- [x] 提供 systemd service、sysusers、tmpfiles 和环境模板；Server 仅有业务权限，控制器/执行器保持 root 管理权限。
- [x] Managed Compose 使用 digest-pinned GHCR 镜像和签名 bundle；应用/迁移容器不挂 Docker socket，不使用 latest/Watchtower/build。
- [x] 增加 `muriarcctl install/doctor/status` 的 profile Driver 接入；升级能力缺少签名目标、备份或 Candidate 条件时必须失败，不得模拟成功。
- [x] 实现服务 drain/stop/start、健康检查和 profile capability 探测的强类型接口与 fake-runner contract tests。
- [x] Server 接入 SIGTERM/SIGINT graceful shutdown；`/livez`、`/readyz`、compatibility health 保持稳定。
- [x] 增加 signed portable bundle 构建/验证脚本，包含 server、ctl、目标执行器、verifier、UI、Release Manifest 和模板。
- [x] 更新部署文档与 CI，检查模板、Compose policy、制品清单和低权限边界。

## 明确的 fail-closed 边界

- [ ] 在最终签名制品与真实 PostgreSQL/volume RC 环境中接通 physical backup/restore、
  Candidate database/directory、七层 verifier 和 activation pointer 的完整 `UpgradeDriver`。

当前 `muriarcctl upgrade` 对该未验证边界返回 `prerequisite_failed`，而不是执行在线原地
migration 或伪造 typed evidence。这个未完成项只能由后续 1.0 集成分支在真实 Native/Compose
环境中关闭；本分支已经提供所需的不可变 bundle、root-only pointer、一次性 DDL executor、
服务生命周期和 capability 探测接口。

## 验收标准

- Native/Compose 模板都只暴露 `127.0.0.1:8787`，PostgreSQL 无宿主机端口。
- Server unit/container 没有 Docker socket、systemd、DDL/升级控制凭据；升级只从宿主机 `muriarcctl` 发起。
- Bundle 存在 symlink、额外/缺失文件、digest 不符、可变镜像引用或错误权限时拒绝安装。
- BYO 环境无法创建隔离 Candidate、实际恢复备份或取得必要权限时 doctor/upgrade 明确失败。
- 收到停止信号后停止接收新流量并等待在途请求；readiness 验证数据库、Epoch/Generation、Crypto、附件根和 UI。

## 技术约束

- 不修改 Upgrade Engine 的固定 phase 顺序；Driver 方法必须幂等并返回 typed evidence。
- 不把数据库、附件、密钥、备份、Journal 或真实 `.env` 加入 Git。
- Compose 生产 bundle 不含 `build:`、`latest`、Watchtower 或 Docker socket mount。
- Native 的 `muriarc-server` 使用 `muriarc` 用户；`muriarcctl` 与目标执行器不在 Server unit 内运行。

## 跨分支备注

本分支依赖兼容基础、Upgrade Engine 和 release-evidence。Desktop 使用独立 Driver；Cloudflare 只消费本分支的 loopback origin 和 health contract。最终 1.0 集成分支生成真实签名制品并执行 systemd/Compose RC。

# MuriArc 文档

> [English](README.md) | 简体中文

MuriArc 当前候选源码身份为 `1.0.0 / E0001 / permanent-upgrade`，但真实 RC 尚未通过，`v1.0.0` 也尚未发布。所有文档都必须保留这一区分。

## 产品与工程

| 文档 | 用途 |
| --- | --- |
| [架构](ARCHITECTURE_cn.md) | 运行拓扑、分层、领域边界、事务与 adapter |
| [安全](SECURITY_cn.md) | 信任边界、账号、秘密、AI 控制、审计与安全报告 |
| [环境](ENVIRONMENTS_cn.md) | 支持的工具链、隔离 worktree 与本地/CI 门禁 |
| [MuriArc 数据迁移](MIGRATION_cn.md) | MuriArc schema/数据根升级、恢复，以及导入/Snapshot 边界 |

## 交付与运维

| 文档 | 用途 |
| --- | --- |
| [Server 部署](DEPLOYMENT_cn.md) | 源码/preview Server 部署与日常运维 |
| [Desktop 交付](DESKTOP_DELIVERY_cn.md) | Windows Tauri 运行形态、数据根、签名更新与验收 |
| [Server 正式交付](SERVER_DELIVERY_cn.md) | 1.0+ 签名 Native/systemd 与 Managed Compose 合同 |
| [Upgrade Engine](UPGRADE_ENGINE_cn.md) | `muriarcctl`、固定状态机、三锁、Journal 与信任链 |
| [升级兼容](UPGRADE_COMPATIBILITY_cn.md) | Epoch 身份、Write Lease、迁移不可变与回滚边界 |
| [Cloudflare 公网 Profile](CLOUDFLARE_PUBLIC_PROFILE_cn.md) | 可选公网入口控制与剩余风险 |
| [交付验收](DELIVERY_ACCEPTANCE_cn.md) | 自动/人工验收范围，不提前宣称 RC PASS |

## 架构决策

ADR 是中文内部工程记录，统一使用 `_cn.md` 后缀。从 [ADR-0001](adr/0001-application-layer_cn.md) 开始，当前连续到 ADR-0008。

## 工程归档

`DATA_MANIFEST.md`、`log/PROJECT_LOG.md` 与 `RELEASE_EVIDENCE_cn.md` 属于维护者归档或内部证据说明，故意不放入上方普通用户主导航，也不得把它们解释为公开发布证书。

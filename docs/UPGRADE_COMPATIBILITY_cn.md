# 升级兼容合同

> [English](UPGRADE_COMPATIBILITY.md) | 简体中文

当前候选源码已携带 `1.0.0 / E0001 / permanent-upgrade` 身份；只有同一批最终制品完成完整真实 RC，永久兼容承诺和正式发布状态才开始。

## 兼容下限

永久兼容承诺从首个稳定 `1.0 / E0001` 开始。预发布 `0.1.0` 仍登记为 `preview_epoch_0`，用于显式采用、测试并转换为首个不可变 fixture。管理员可以从一个稳定版本直接请求升级到最新稳定版本，控制器内部可跨多个 Epoch。

只有升级后的 Application/API/真实 UI 能读取旧记录、继续写入、获取附件字节与 AI 历史，并保持 Audit/Provenance 连续性，才算数据保留。“数据库文件仍存在”不是验收证据。

## 四维身份

每个可运行 generation 由以下验证值标识：

- `ApplicationVersion`；
- `DataEpoch`；
- backend-specific `BackendStateDigest`：由有序 SQLx migration version、description、SHA-384 checksum 组成，再以 SHA-256 汇总；
- `GatewayContractRevision`。

`ReleaseManifest` 还固定 SQLite/PostgreSQL 身份、PostgreSQL major、Bootstrap/Controller protocol range、migration class 及每个制品的 SHA-256/大小。代码内 Release Catalog 与 Persistent Data Registry 只能追加。

## 持久部署状态

SQLite `0031` 与 PostgreSQL `0033` 增加 generation set、upgrade operation、Write Lease 和 singleton deployment state。Active generation 必须持有未过期 active lease。

数据库 trigger 覆盖当前业务表，记录首次写入，并在 lease 被撤销后用 `muriarc_write_lease_required` 拒绝 INSERT/UPDATE/DELETE。未来新增业务表也必须安装 fence；控制面表和 SQLx ledger 排除。

PostgreSQL `0034` 追加 credential policy revision 与 HMAC-keyed 登录退避，并重新安装 fence。它不保存被探测邮箱或 Cloudflare secret。

Data root 保存 `deployment-generation.json`。Server/Desktop 在打开 Application 前比较其中 generation、Epoch、Backend digest 与数据库。普通启动不会重建缺失/不匹配 manifest。

## 启动与 preview 采用

长期 Server/Desktop 启动调用 `compatibility_report`，不调用 migration runner。以下情况 fail closed：migration 缺失/改变/未知、身份漂移、inactive generation、无效 lease、缺 generation manifest 或附件根。

仅 `preview_epoch_0` 可用 `MURIARC_PREVIEW_BOOTSTRAP=true` 显式采用：应用 preview migrations，创建初始 generation/lease/manifest。稳定 managed profile 必须移除该逃生口，由 `muriarcctl` 管理安装升级。Desktop 可以无 flag bootstrap 可证明为空的 fresh data root；既有 Desktop 数据库需要显式 preview flag 或签名 updater。

存在加密凭据行时 Server 拒绝生成替代 AI Master Key；存在附件 metadata 时也拒绝缺失/空附件根。普通启动不运行旧设置 materializer。

## Migration 不可变

`migrations/checksums.json` 锁定 migration。新结构只追加并使用 `Expand → Backfill → Switch → Contract`；不确定变更属于 M3。

持久 enum/JSON 使用 `PreservedValue`/`VersionedJson`；未知 raw value 保留并标记 `needs_review`，不得静默映射到正常业务状态。

## 恢复集合与回滚边界

PostgreSQL/SQLite、附件/data、配置、generation manifest、Keyset 与 AI 状态组成一个恢复集合。只有隔离 restore 和 verifier 通过的备份才有效。

新 generation 首次写入前，控制器可以原子恢复前一 generation。设置 `first_write_at` 后禁止自动降级；必须 forward fix，或执行记录操作者“可能丢失数据”确认的显式恢复。

共享 Upgrade Engine 和独立 `muriarcctl` 实现固定 transition/evidence、三锁、hash-chain Journal、PostgreSQL fencing、TUF-compatible metadata 与固定 Bootstrap Protocol，详见 [Upgrade Engine](UPGRADE_ENGINE_cn.md)。Native/Compose/Desktop Driver 都实现同一 `UpgradeDriver`，不得复制或重排状态机。

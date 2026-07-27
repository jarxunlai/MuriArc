# Upgrade Engine 与 muriarcctl

> [English](UPGRADE_ENGINE.md) | 简体中文

控制面合同与交付 Driver 已实现，候选源码身份也已切换为 `1.0.0 / E0001 / permanent-upgrade`；但只有真实物理 Driver 和同一批最终签名制品生成完整证据时升级与发布才成功，当前 RC 尚未通过。

## 权限边界

`muriarc-server` 是低权限长期应用进程，不拥有 systemd、Docker socket、发布签名、备份编排或 raw DDL。`muriarcctl` 是独立本机管理员进程。Native、Managed Compose、Desktop 实现 `UpgradeDriver`，不复制/重排 `muriarc-upgrade` 共享状态机。

需要物理 Driver 的命令在 Driver 缺失时 fail closed，绝不打印模拟成功。

## 固定状态机

唯一成功路径：

```text
Initialized
  -> LocksAcquired
  -> PreflightPassed
  -> Drained
  -> WritesFrozen
  -> BackupCreated
  -> BackupRestored
  -> CandidatePrepared
  -> CandidateMigrated
  -> CandidateVerified
  -> Switched
  -> ReadOnlyActivated
  -> ActivationVerified
  -> WriteLeaseOpened
  -> Completed
```

每次 transition 接收强类型 evidence：

- Preflight 固定签名 target、维护等级、空间和恢复前置条件；
- Drain 必须报告请求、Job、附件 writer、Provider request 全部为零；
- Freeze 记录被撤销 lease 与 fencing token；
- Backup 联合数据库、附件、数据、配置、Keyset、AI state 与 generation manifest；
- BackupRestored 要求 digest 匹配的实际隔离恢复，并在 Candidate 前登记 verified recovery point；
- Candidate 使用私有 endpoint，关闭 Provider、后台 work 和真实用户写入；
- Candidate verification 执行资产、Storage、Store/Application、真实 API、真实 UI、继续写入、只读无副作用七层；
- Generation switch 必须原子；目标先无 Write Lease、位于 traffic gate 后，readiness/no-write 通过才创建 fenced lease。

不存在跳阶段 API。Driver 调用在 crash 后可能重放，因此必须按 operation/revision 幂等。

## 三锁与持久恢复

1. control-state root 以 exclusive create 获取 `upgrade.lock`；
2. PostgreSQL 专用 Session 持有 `pg_try_advisory_lock(0x4d55524955504752)`；
3. `muriarc_upgrade_operations` 限制一个 running operation，业务 Write Lease 以递增 fencing token 在 active/draining/revoked 间转换。

数据库 operation JSON 是权威状态。mode-0600 append-only JSONL Journal 用 SHA-256 hash chain 镜像每个 revision。Resume 拒绝领先/冲突 Journal，但可用数据库 snapshot 修复落后的本地 Journal。

`recovery-points.json` 禁止删除最近一次实际恢复并验证的 recovery point。制品删除属于 Driver，只有显式 `muriarcctl recovery prune` 才执行。

目标首次写入前可以恢复 source generation 并签发更高 fencing token；出现 `first_write_at` 后 Engine 记录 `recovery_required` 并拒绝自动降级。显式 restore 必须携带操作者 data-loss confirmation。

## TUF-compatible 信任与 bootstrap

Trust client 验证 TUF Root/Timestamp/Snapshot/Targets chain，包括 Ed25519 threshold signature、canonical signed JSON、到期、metadata version 单调、父级 length/SHA-256、连续双签 Root rotation 与签名 Release Manifest。

`VerifiedRelease` 不能从外部反序列化伪造。固定 Bootstrap Protocol 在 Unix exec 或其他平台 child-process handoff 前再次核对 controller protocol、target length 和 SHA-256。

私钥、数据库 URL、密码、Token、Cookie、API Key 和真实恢复内容永不进入 Journal 或 Git。

## CLI surface

```text
muriarcctl install --profile native-system|managed-compose
muriarcctl doctor|status [--output json]
muriarcctl update check
muriarcctl upgrade [--to <version>]
muriarcctl backup create|verify
muriarcctl verify --read-only
muriarcctl recovery resume [--operation <uuid>]
muriarcctl recovery restore [--backup <uuid>] [--confirm-data-loss]
muriarcctl recovery prune --backup <uuid>
```

Parser 拒绝 raw migration、`--force` 和 skip-verification 选项。

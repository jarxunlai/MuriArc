# Feature Spec: Upgrade Engine 与 muriarcctl 控制面

> 本分支基于永久兼容 Foundation，建立所有 Native、Compose 与 Desktop Driver 共用的升级状态机；不在长期运行的 Server 内复制升级规则。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/upgrade-engine-control-plane` |
| 基于提交 | `feature/server-upgrade-data-compatibility@13bd246` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-upgrade-engine-control-plane` |
| 建立日期 | `2026-07-26` |

## 目标

提供独立、可恢复、fail-closed 的 Upgrade Engine 与 `muriarcctl`。引擎必须在迁移前冻结写入、创建联合恢复集合并实际恢复验证，在隔离 Candidate 上完成迁移与七层验证后才允许原子切换；新版首次写入后禁止自动降级。

## 实现范围

- [x] 新增共享 `muriarc-upgrade` crate，定义固定 phase/state machine、Journal、Driver/Store ports、错误码和恢复语义。
- [x] 实现宿主机独占锁、PostgreSQL advisory session lock、持久化 running operation 与 fencing Write Lease 三重互斥。
- [x] 实现 PostgreSQL 控制面：创建/恢复 operation、Drain、撤销 Lease、Candidate generation、原子激活、新 Lease 与首次写入降级屏障。
- [x] 将联合备份、实际恢复验证、Candidate 迁移、七层验证、只读激活验证建模为不可跳过的 typed evidence；缺证据不得进入下一 phase。
- [x] 实现目标失败时的 pre-write 自动回退；检测 `first_write_at` 后只允许 forward-fix 或显式恢复确认。
- [x] 实现只追加 JSONL Journal、幂等 resume、恢复点保留与显式 prune 安全约束。
- [x] 实现 TUF-compatible Root/Timestamp/Snapshot/Targets metadata 校验骨架，包括阈值签名、过期、冻结、rollback 与 artifact digest/length 验证。
- [x] 实现固定 Bootstrap Protocol：旧 ctl 验证目标 controller 协议和制品后 re-exec，不接受未签名或降级目标。
- [x] 新增独立 Rust 二进制 `muriarcctl`，提供 install/doctor/status/update/upgrade/backup/verify/recovery 命令树；尚无 Driver/权限时必须报告前置条件而非假成功。
- [x] 为状态转移、锁竞争、备份未恢复、Candidate 验证失败、断点 resume、首次写入后拒绝回退、TUF 攻击与 CLI 契约添加测试和文档。

## 验收标准

- 任意 phase 失败都保留可诊断 Journal；重跑不会重复越过已完成但未验证的边界。
- 未持有 host/advisory/persistent 三重锁时不能进入 Drain；未有 verified restore 时不能迁移 Candidate；七层验证不完整时不能激活。
- 激活前失败可恢复旧 generation；激活后且尚无首写可自动回退；存在首写则引擎拒绝自动降级。
- Engine 不直接依赖 systemd、Docker、Tauri UI 或具体 Provider；Driver 不可绕过 Engine transition guard。
- `muriarc-server` 不获得 Docker socket、systemd 或 DDL 权限；升级 CLI 不暴露 raw migration 或跳过验证参数。
- CLI JSON 输出稳定且不泄露密码、API key、Token、Cookie、数据库 URL 或 Journal 中的秘密。

## 技术约束

- 已发布 migration 仍只追加；本分支只能为控制面需要追加新 migration 并更新 checksum manifest。
- `core` 不依赖 SQLx/CLI/Driver；升级控制 Store 与业务 Store 权限和连接明确分离。
- Candidate 禁止外部 Provider、后台 Job/Cleanup、真实用户流量和附件写入。
- 备份必须覆盖 PostgreSQL/SQLite、附件、配置、密钥、AI 状态和 generation manifest；默认保留最后验证恢复点。
- 不把数据库、备份、密钥、TUF 私钥、运行 Journal 或真实数据加入 Git。

## 跨分支备注

本分支先于 `feature/release-fixtures-gates` 和交付 Driver 合并。Native/Compose、Desktop 只实现 Driver；历史 Fixture/Verifier 提供七层验证器，不得复制状态机或绕过证据门禁。

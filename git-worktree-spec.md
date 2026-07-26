# Feature Spec: MuriArc 1.0 发布集成与 Fail-Closed RC

> 本分支只实现正式发布编排与门禁，不把当前 `0.1.0 / preview_epoch_0` 冒充为 1.0，也不生成
> 虚假的 E0001 Fixture、Windows/systemd/Docker/Cloudflare 证据或签名结果。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/release-integration-1-0` |
| 基于提交 | `feature/cloudflare-public-profile@ef89402` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-release-integration-1-0` |
| 建立日期 | `2026-07-27` |

## 目标

建立可由正式发布流水线调用的 1.0 RC 编排：把最终 Native bundle、Managed Compose bundle、
Windows 安装包、Release Manifest、TUF/Sigstore/Tauri provenance、完整历史兼容矩阵和真实
systemd/Docker/Windows/Cloudflare/fault-injection 证据绑定到同一组 digest。Catalog 为空、E0001
不是由最终制品生成、任一层 `FAIL/SKIP`、源码运行或 DemoGateway 都必须阻断。

## 实现范围

- [x] 增加版本化 RC gate definition，固定 Native、Compose、Desktop、Cloudflare Public、恢复、
  故障注入和签名攻击场景及其最终制品映射。
- [x] 增加 Release readiness validator，验证源码正式身份、Release Manifest、最终 artifact lock、
  双后端 E0001 Fixture、完整 RC compatibility matrix 和场景证据的 digest 闭环。
- [x] 空 Catalog、缺少 SQLite/PostgreSQL 当前状态、错误 source artifact/provenance、任何
  `FAIL/SKIP`、非 final package、重复或额外场景均 fail closed。
- [x] 增加宿主编排脚本：先执行完整历史 RC matrix，再调用真实 RC Driver，最后运行 readiness
  validator；所有恢复数据、报告与制品必须在 Git 工作树之外。
- [x] 将 GitHub RC workflow 接入统一编排；PR/Nightly 保持分层矩阵，RC 缺少最终 driver、manifest、
  Fixture 或真实环境时明确失败。
- [x] 增加 Python/Rust/Workflow 合同测试和发布文档；测试只能构造临时合成控制文件，不能生成
  或登记真实 E0001 Fixture。

## 非目标

- 不把 workspace 版本改为 `1.0.0`，不把 Epoch 改为 `E0001`，不追加空壳 Catalog 条目。
- 不创建签名、GHCR OCI、Windows 安装包或 Cloudflare staging 通过记录。
- 不在 Git 中保存数据库、附件、Keyset、真实账号、Journal、Fixture 大资产或 RC 报告。
- 不允许源码 `cargo run`、DemoGateway、SKIP 或手工勾选替代最终制品证据。

## 验收标准

- 当前仓库运行正式 1.0 readiness 必须失败，并明确指出仍是 preview 或 Catalog 为空。
- 对完整合成控制面：只有版本/Epoch、双后端 Fixture 来源、artifact/provenance、全历史矩阵、
  required scenarios 和所有 digest 完全一致且全部 PASS/final_package 时 validator 才通过。
- 外部 `artifact-lock.json` 必须被 RC evidence 按 digest 引用；Driver 报告不能替换其中的 artifact
  size、provenance 或 signature evidence。
- 任意删除、重复、FAIL、SKIP、source_run、digest 篡改、错误 profile/artifact 映射均由测试证明失败。
- `run-release-candidate.sh` 拒绝工作树内输出、缺少真实 driver/verifier/manifest，并且不生成伪证据。

## 跨分支备注

依赖 `ef89402` 及此前兼容基础、Upgrade Engine、Fixture、Native/Compose、Desktop、Cloudflare 全部
阶段。该分支完成后只代表“发布门禁和编排已就绪”，不代表真实 1.0 RC 已通过；正式发布必须在
最终 release commit、最终 digest 和真实外部环境上另行执行。

# 不可变历史 Fixture 与兼容发布门禁

## 为什么需要独立证据链

数据库文件仍存在不等于升级成功。MuriArc 的兼容结论必须证明旧账号、权限、动物关系、实验、
Observation、样本、附件字节、AI 历史与密文、Audit/Provenance 都能被目标 Application/API/真实
Remote UI 读取，并且旧记录可以继续写入。用当前 HEAD 重新创建一个“看起来像旧版”的数据库，
无法证明旧 Release 当时真正写出的状态可升级，因此禁止作为正式 Fixture。

永久矩阵从 `1.0 / E0001` 开始。`0.1.0 / preview_epoch_0` 只允许通过精确的预发布接入路径进入
E0001；在最终 1.0 制品产生双后端 Fixture 前，`release-fixtures/catalog.json` 保持为空，RC
门禁必定失败。

## 三种互不混用的 digest

| 名称 | 保护对象 | 用途 |
|---|---|---|
| `backend_state_digest` | 某 backend 的有序 migration 状态 | 判断数据库结构身份 |
| `fixture_artifact_digest` | GHCR 中 OCI Artifact manifest | 固定下载对象与 cosign provenance |
| `expected_facts_digest` | canonical `expected-facts.json` | 固定业务事实断言 |

`fixture_manifest_digest` 另外固定 Bundle 内的文件清单；Catalog entry 自身再计算
`immutable_entry_digest`。任何 digest 都不能拿另一种 digest 替代。

## Fixture Bundle

每个 Bundle 根目录包含 `fixture-manifest.json`，并登记以下八类、至少一个非空普通文件：

1. database；
2. attachments；
3. data artifacts；
4. configuration manifest；
5. 仅用于合成测试的 Keyset；
6. AI state；
7. `deployment-generation.json`；
8. `expected-facts.json`。

Verifier 拒绝绝对路径、`..`、反斜线、symlink、特殊文件、未登记文件、缺失文件以及长度或
SHA-256 不一致。Generation ID、Epoch、Backend State 必须与 Fixture identity 一致。真实账号、
动物数据、Provider Key、生产密钥、Cookie、Token、Journal 和运行日志均不得进入 Fixture。

`expected-facts.json` 必须是 `serde_json::to_vec` 产生的无空白 canonical JSON；人工格式化、
未知字段或不同字节会导致 digest 不一致。它必须覆盖：

- 账号、Lab role、Project membership；
- 动物、父母关系和繁育；
- 实验、Observation、样本；
- 附件大小和内容 SHA-256；
- AI profile、conversation、message、approval、job、加密 envelope 与 key version；
- Audit 与 Provenance 连续性；
- 升级后针对既有记录执行一次受控写入所需 actor、revision 和预期增量。

## 只追加 Catalog

Catalog entry 同时绑定生成 Release 的 ApplicationVersion、DataEpoch、Gateway revision、
Backend State、Release artifact digest、Release provenance digest、Fixture/manifest/facts digest
与 OCI digest 引用。一个 backend state 只能登记一次。PR 会把 base branch 的 Catalog 作为前缀
比较；修改、删除、重排既有条目都会失败。

发布流程必须由**对应正式 Release 二进制或镜像**生成 Bundle，然后执行：

```bash
scripts/publish-release-fixture.sh \
  --fixture /secure/staging/e0001-postgres \
  --repository ghcr.io/jarxunlai/muriarc-fixtures \
  --tag 1.0.0-e0001-postgres \
  --manifest-digest sha256:...
```

脚本先做本地资产验证，再创建确定性 tar、推送 OCI、取得 registry digest、执行 cosign 签名并
输出 digest-pinned reference。它**不会**自动改 Catalog；维护者必须从受控生成报告中构造
entry，运行 Catalog 门禁后通过普通 PR 追加。这样 tag、上传动作或脚本都不能静默重写历史。

拉取端使用：

```bash
scripts/pull-release-fixture.sh \
  --reference ghcr.io/jarxunlai/muriarc-fixtures@sha256:... \
  --output /secure/cache/<fixture-id> \
  --manifest-digest sha256:...
```

脚本先验证 cosign identity，再拉到同文件系统临时目录，执行内置资产验证，最后原子切换缓存。

## 七层 Verifier

`muriarc-verifier` 固定以下七层，任一 `FAIL`、`SKIP`、缺 digest 或 Expected Facts 不一致都失败：

1. 资产恢复；
2. Storage；
3. Store/Application；
4. 真实 API；
5. 真实 RemoteHttpGateway UI；
6. 升级后继续写入；
7. 只读验证无副作用。

资产层由 verifier 自己完成。其余层由交付 Driver 输出强类型 evidence 文件；文件必须是普通
非 symlink JSON。Candidate 禁止真实用户流量、外部 Provider、后台 Job/Cleanup 与不受控写入。
只读层必须提供验证前后相同的 persistent-state digest；继续写入层必须明确证明 revision、Audit
和 Provenance 增量。

CLI：

```text
muriarc-verifier asset --root <fixture-dir> [--manifest-digest sha256:...]
muriarc-verifier run --request <run-request.json>
muriarc-verifier report --report <verification-report.json>
muriarc-verifier matrix --report <matrix-report.json> --definition <matrix.json> --catalog <catalog.json>
```

## PR、Nightly 与 RC

- PR：持久化、Store、Application、API、AI、migration、升级或证据链变化选择完整 Catalog；纯
  UI/文档等变化至少选择每个 backend 最新 State。被选 State 仍必须跑完整七层。
- Nightly：选择完整历史 Catalog，并按 `nightly_profiles` 运行。
- RC：Catalog 不得为空；选择完整历史和 Native/Compose/Desktop 三个 profile；Driver 报告
  必须声明 `final_package`，并由最终安装包/镜像 digest 产生。源码 `cargo run`、DemoGateway、
  `FAIL` 或 `SKIP` 均阻断。

`scripts/run-release-compatibility.sh` 是 fail-closed orchestrator。当 Catalog 非空但未配置真实
`MURIARC_COMPATIBILITY_DRIVER` 时直接失败，不生成“成功”报告。Native、Compose 和 Desktop
Driver 在各自交付分支实现；本证据分支不会用 mock 冒充正式门禁。Driver 的目标 Release
Manifest 必须使用 `native-system`、`managed-compose`、`desktop-windows` artifact key；
报告的目标 identity、backend state 和 artifact digest 会再次与该 manifest 交叉核对。

## 本地门禁

```bash
python3 scripts/check_fixture_catalog.py --catalog release-fixtures/catalog.json
python3 scripts/compatibility_matrix.py plan \
  --mode nightly --catalog release-fixtures/catalog.json \
  --definition release-fixtures/matrix.json --output /tmp/muriarc-plan.json
cargo test --locked -p muriarc-release-evidence -p muriarc-verifier --all-targets
cargo clippy --locked -p muriarc-release-evidence -p muriarc-verifier \
  --all-targets --all-features -- -D warnings
```

大 Fixture、报告和缓存必须放在仓库外。正式 RC 使用干净 worktree、独立 generation 和最终制品
digest；本地合成 contract 只验证证据协议本身，不能代替发布结论。

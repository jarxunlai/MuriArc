# 不可变历史 Fixture 与兼容发布门禁

> 简体中文 · 内部发布证据说明；本文件不是 RC PASS 证书。

## 正式 1.0 RC 的最终门禁

兼容矩阵只回答“每个历史 Backend State 能否被最终交付形态读取、继续写入并保持只读无副作用”。
正式 1.0 还必须把最终制品、签名 provenance、真实部署/恢复/故障注入和 Cloudflare staging 绑定到
同一组 digest。固定定义位于 `release-fixtures/rc-gate.json`，统一入口为：

最终 Native/Compose bundle 和 Windows 安装包先各自由签名流水线输出一个普通 JSON descriptor：
`format_version`、`artifact_name`、`media_type`、`digest`、`size_bytes`、`provenance_digest` 和
`signature_evidence_digest`。正式 release source 再用编译身份文件和这些 descriptor 组装外部
Release Manifest：

```bash
scripts/assemble_release_manifest.py \
  --identity /secure/release-identity.json \
  --artifact native-system=/secure/native-descriptor.json \
  --artifact managed-compose=/secure/compose-descriptor.json \
  --artifact desktop-windows=/secure/windows-descriptor.json \
  --release-provenance-digest sha256:... \
  --output-directory /secure/new/muriarc-1.0.0-manifest
```

Assembler 只做闭合 schema/digest 编排，不生成或验证签名；它拒绝 preview source、工作树内输出
和缺少三种最终制品。输出 `release-manifest.json` 与 `artifact-lock.json`，随后由 TUF/Tauri/发布
provenance 流程签名和消费。Release Manifest 不嵌入其描述的 bundle，从而不存在自引用 digest。

完整 RC 入口为：

```bash
export MURIARC_VERIFIER=/absolute/path/muriarc-verifier
export MURIARC_FIXTURE_PRODUCER=/absolute/path/real-final-artifact-fixture-producer
export MURIARC_COMPATIBILITY_DRIVER="$PWD/scripts/release_compatibility_driver.py"
export MURIARC_COMPATIBILITY_NATIVE_RUNNER=/absolute/path/native-compatibility-runner
export MURIARC_COMPATIBILITY_COMPOSE_RUNNER=/absolute/path/compose-compatibility-runner
export MURIARC_COMPATIBILITY_WINDOWS_RUNNER=/absolute/path/windows-compatibility-runner
export MURIARC_RC_DRIVER="$PWD/scripts/release_rc_driver.py"
export MURIARC_RC_NATIVE_RUNNER=/absolute/path/native-rc-runner
export MURIARC_RC_COMPOSE_RUNNER=/absolute/path/compose-rc-runner
export MURIARC_RC_WINDOWS_RUNNER=/absolute/path/windows-rc-runner
export MURIARC_RC_CLOUDFLARE_RUNNER=/absolute/path/cloudflare-rc-runner
export MURIARC_RELEASE_ARTIFACT_INPUTS=/absolute/path/artifact-inputs.json
export COSIGN_PUBLIC_KEY=/absolute/path/cosign.pub
export MURIARC_FIXTURE_CACHE=/absolute/path/outside-git/fixture-cache

scripts/run-release-candidate.sh \
  --release-manifest /absolute/path/final-release-manifest.json \
  --artifact-lock /absolute/path/signed-artifact-lock.json \
  --run-root /absolute/new/path/outside-git/muriarc-1.0.0-rc
```

编排器先要求 Fixture producer 从最终制品生成 append-only candidate Catalog，再运行全历史七层
RC matrix，并要求真实 RC Driver 对 Native/systemd、Managed Compose、Windows 安装包、两个
Cloudflare Public overlay、故障注入、激活前回退/首次写入后拒绝降级以及 TUF/Sigstore/Tauri
攻击执行固定场景，最后由 `scripts/check_release_readiness.py` 生成可签名的
`release-readiness-report.json`。它不生成证据；缺 driver、空 Catalog、任何 `FAIL/SKIP`、
`source_run`、DemoGateway、digest 不一致或工作树内报告都会失败。

`artifact-lock.json` 不是可选的旁路文件：RC evidence 必须同时引用它的 SHA-256，且其中每个制品的
digest、size、provenance 与 signature evidence 必须和 RC Driver 报告逐项一致。Fixture producer
也必须读取同一份 lock，不能从未受信任的运行参数自行填写 source provenance。

### 物理 Runner 闭合协议

仓库内的 `scripts/release_compatibility_driver.py` 和 `scripts/release_rc_driver.py` 是最终的
fail-closed 编排边界，不是物理环境替身。它们首先重新验证 Release Manifest、artifact lock、
release/artifact provenance、Cosign evidence、最终制品字节和由最终 commit 编译的 verifier；
只有随后调用的外部 runner 才可以操作真实 systemd、Docker、Windows 或 Cloudflare staging。
所有 runner 环境变量必须指向绝对路径的普通可执行文件，缺失时直接失败，不能降级为源码运行或
模拟 PASS。

Fixture 与交付形态按物理 backend 固定映射：

| Fixture backend | 允许的 profile |
|---|---|
| SQLite | `desktop-windows` |
| PostgreSQL | `native-system`、`managed-compose` |

矩阵编排器从 append-only Catalog 读取 `fixture_manifest_digest`，下载时验证一次，并通过
`--fixture-manifest-digest` 原样传入 Compatibility Driver。Driver 再把该值写入 verifier request
的 `expected_manifest_digest`；不得使用 `null` 或从已恢复目录重新猜测 Catalog 身份。

每个 Compatibility runner 必须在新的外部 evidence 目录内只生成以下六个普通、非 symlink JSON：
`storage.json`、`store_application.json`、`api.json`、`remote_ui.json`、
`continue_write.json` 和 `read_only_no_side_effects.json`，并生成闭合 schema 的
`runner-result.json`。结果必须绑定 Fixture、backend、profile、Release Manifest、artifact lock
和最终制品 digest/size，声明 `final_package`、`pass`、零 `FAIL/SKIP`；目录缺文件、多文件、
symlink 或任一 digest 漂移都会失败。资产恢复层始终由最终 verifier 自己执行，从而形成七层报告。

RC Driver 串行执行 `rc-gate.json` 的 14 个场景。环境与 runner 变量固定为：
`linux-systemd → MURIARC_RC_NATIVE_RUNNER`、`linux-docker → MURIARC_RC_COMPOSE_RUNNER`、
`windows-installer → MURIARC_RC_WINDOWS_RUNNER`、`cloudflare-staging →
MURIARC_RC_CLOUDFLARE_RUNNER`。每个场景必须返回 Driver 内固定的完整检查集合，并在指定
evidence 目录为每个 check 写入 `<check_id>.json`；场景根目录只能包含该 evidence 目录和
`scenario-evidence.json`。未知、缺失、额外或 symlink 文件，重复检查或 evidence digest，
越界时间戳，非最终制品或任何 `FAIL/SKIP` 都会阻断最终 `rc-evidence.json`。证据内容、凭据和
密钥留在受保护的仓库外运行目录，不进入 CI 日志或 Git。

当前候选源码已切换为 `1.0.0 / E0001 / permanent-upgrade`，但这只满足源码身份条件，不代表 readiness 或 RC 已通过。
只有同一 release commit 构建的最终签名制品实际生成双后端 E0001 Fixture，且其
`source_release_artifact_digest` 与 `source_release_provenance_digest` 指向这些制品时，readiness 才可能通过。
这项失败是安全边界，不得通过预造 HEAD Fixture 或修改 gate definition 绕过。

### 首个 E0001 的无循环引导

E0001 Fixture 不能先由 HEAD 生成再把应用版本改成 1.0；但如果先把 Fixture Catalog 提交进源码再
重建制品，artifact digest 又会变化，形成循环。因此正式编排采用以下固定顺序：

1. 从最终 release commit 构建并签名候选 Release Manifest 与三个最终制品；
2. `MURIARC_FIXTURE_PRODUCER` **实际运行这些 digest 固定的制品**，生成/发布双后端 Fixture，
   并在仓库外输出相对已提交 `catalog.json` 的 append-only `candidate-catalog.json`；
3. 全历史 matrix 和 readiness 使用这个 candidate Catalog，并把其 digest 写入最终报告；
4. RC 通过且正式制品按原 digest 发布后，将报告中的同一批 Catalog entry 原样追加回仓库，供
   后续版本 Nightly/RC 使用，不重建或替换已经发布的 1.0 制品。

这样 Catalog 控制文件的后置登记不会伪称它参与生成 1.0 artifact，也不会让“最终 artifact”因
登记动作发生变化。producer 缺失、candidate 非 append-only、未含 SQLite/PostgreSQL E0001 或
Fixture 的 source artifact/provenance 不匹配时，编排在兼容矩阵开始前失败。

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

# Feature Spec: 不可变历史 Fixture、Expected Facts 与发布门禁

> 本分支建立“旧数据真的来自旧 Release、最新版真的能完整读取并继续写入”的永久证据链，不用 HEAD 伪造历史数据库。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | feature/release-fixtures-gates |
| 基于提交 | feature/upgrade-engine-control-plane@7ff7c8c |
| Worktree 路径 | /home/ljx/Github/animal_lab-release-fixtures-gates |
| 建立日期 | 2026-07-26 |

## 目标

定义不可变 Fixture/Catalog、Expected Facts 和七层 Verifier，建立 PR、Nightly、RC 三种兼容矩阵门禁。每个正式 Backend State 只能由对应 Release 制品生成；RC 必须使用最终安装包/镜像与 digest，任何 FAIL/SKIP 阻断。

## 实现范围

- [x] 新增共享 release-evidence crate，定义 Fixture Bundle、只追加 Catalog、生成制品 provenance、三种独立 digest 和 Expected Facts 强类型 schema。
- [x] Expected Facts 覆盖账号/角色/项目、动物/繁育、实验/Observation/样本、附件 bytes/SHA、AI 历史与密文、Audit/Provenance，以及升级后继续写入。
- [x] 实现安全资产验证：固定相对路径、拒绝 symlink/额外文件、流式 SHA-256/长度、backend/epoch/generation/Keyset/config manifest 联合恢复集合核对。
- [x] 实现七层 Verifier runner：资产恢复、Storage、Store/Application、真实 API、真实 Remote UI、继续写入、只读无副作用；FAIL/SKIP 都不得伪装通过。
- [x] 生成可供 Upgrade Engine 使用的七层 VerificationEvidence，并校验 Expected Facts digest 与所有 evidence digest。
- [x] 实现历史 Catalog append-only/entry-self-digest 检查、Release 生成器版本绑定和“HEAD 不得重建旧 State”的 fail-closed 规则。
- [x] 新增独立 muriarc-verifier CLI，支持 asset、run、report 与 matrix；JSON 报告固定 schema 且不包含秘密。
- [x] 增加 GHCR OCI Artifact digest/cosign provenance 的发布与拉取脚本/工作流骨架；禁止 latest，Catalog 只记录 digest。
- [x] 实现 PR 影响选择、Nightly 全历史、RC 全历史/全 profile/最终制品矩阵定义；RC 对空 Catalog、非最终 digest、源码运行或任一 SKIP 失败。
- [x] 添加合成小型 fixture contract、篡改/路径逃逸/缺层/错误来源 Release/继续写入不一致等测试与文档。

## 验收标准

- Fixture 内数据库/附件/Keyset/config/Expected Facts 任一缺失、损坏、额外或路径不安全都 fail closed。
- Catalog 现有 entry 不能修改/删除/复用 Backend State；新增 entry 必须绑定生成它的 Release artifact 与 provenance digest。
- Verifier 必须覆盖七层，且 Candidate 的继续写入不会污染只读检查；Expected Facts 未覆盖的业务域不能被静默忽略。
- PR 可按影响缩小“历史 State 集合”，但被选中的每个 State 仍运行完整七层；Nightly/RC 不允许缩小。
- RC 报告必须绑定最终 Native/Compose/Desktop artifact digest；任何 FAIL/SKIP 或源码 cargo run 都阻断。
- 大数据库/附件不进入 Git；Git 只含 schema、小型合成 contract、Catalog digest 和工作流。

## 技术约束

- Fixture 生成器嵌入 ApplicationVersion/DataEpoch/BackendStateDigest；若与目标 Catalog entry 不同则拒绝发布。
- fixture_artifact_digest 不与 backend_state_digest 或 expected_facts_digest 混用。
- 所有测试数据均为合成数据；密钥仅为合成 Keyset，禁止真实账号、动物数据或 Provider Key。
- Verifier 使用真实 Store/Application/API/RemoteHttpGateway UI adapter；DemoGateway 不能形成 RC 通过。
- OCI 使用 digest 固定；cosign 私钥、OIDC Token、Registry 凭据和大资产不进入仓库。

## 跨分支备注

本分支输出 Fixture/Catalog/Verifier ports，供 Native/Compose、Desktop 与最终 1.0 集成分支调用。E0001 双后端正式 Fixture 在 release-integration-1-0 分支由最终 Release 制品生成，不在本分支用 HEAD 预造。

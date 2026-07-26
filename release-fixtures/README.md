# Immutable release fixture index

这个目录只保存小型、可审查的控制文件，不保存数据库、附件、Keyset、恢复点或运行报告。

- `catalog.json` 是只追加 Catalog。正式版 `1.0 / E0001` 发布时，SQLite 与 PostgreSQL 的每个
  唯一 Backend State 各追加一个条目；既有条目不得修改、删除、排序或复用。
- `matrix.json` 固定 PR、Nightly 与 RC profile。RC 必须覆盖完整历史 Catalog、三个正式交付
  profile，并且只能接受最终安装包或镜像产生的报告。
- `rc-gate.json` 固定 1.0 的最终 Native、Compose、Desktop、Cloudflare staging、故障注入、
  首次写入降级保护和 TUF/Sigstore/Tauri 攻击测试；这些场景不能通过减少 definition 绕过。
- 大 Fixture 保存为 GHCR OCI Artifact。Catalog 只接受
  `ghcr.io/...@sha256:<digest>`，不接受 tag、`latest` 或可变引用。

当前项目仍为 `0.1.0 / preview_epoch_0`，所以 Catalog 有意为空。空 Catalog 不代表兼容验证已
通过；RC 模式会明确失败。第一个 E0001 条目必须由最终 1.0 Release 制品实际生成，禁止用开发
分支 HEAD 预造“1.0 历史数据”。

首个 1.0 RC 为避免“先提交 Catalog 导致最终 artifact digest 改变”的循环，由正式 Fixture
producer 在仓库外生成 append-only `candidate-catalog.json`；完整 RC 报告绑定其 digest。正式
制品按原 digest 发布后，再把候选条目原样追加回本目录，不能因此重建 1.0 制品或改写条目。
producer 的 source artifact/provenance 必须来自同一次正式编排的外部签名
`artifact-lock.json`，不能仅凭 Release Manifest 或可变环境变量自行声明。

发布、拉取、验证与矩阵操作见 [`docs/RELEASE_EVIDENCE.md`](../docs/RELEASE_EVIDENCE.md)。

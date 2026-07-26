# Immutable release fixture index

这个目录只保存小型、可审查的控制文件，不保存数据库、附件、Keyset、恢复点或运行报告。

- `catalog.json` 是只追加 Catalog。正式版 `1.0 / E0001` 发布时，SQLite 与 PostgreSQL 的每个
  唯一 Backend State 各追加一个条目；既有条目不得修改、删除、排序或复用。
- `matrix.json` 固定 PR、Nightly 与 RC profile。RC 必须覆盖完整历史 Catalog、三个正式交付
  profile，并且只能接受最终安装包或镜像产生的报告。
- 大 Fixture 保存为 GHCR OCI Artifact。Catalog 只接受
  `ghcr.io/...@sha256:<digest>`，不接受 tag、`latest` 或可变引用。

当前项目仍为 `0.1.0 / preview_epoch_0`，所以 Catalog 有意为空。空 Catalog 不代表兼容验证已
通过；RC 模式会明确失败。第一个 E0001 条目必须由最终 1.0 Release 制品实际生成，禁止用开发
分支 HEAD 预造“1.0 历史数据”。

发布、拉取、验证与矩阵操作见 [`docs/RELEASE_EVIDENCE.md`](../docs/RELEASE_EVIDENCE.md)。

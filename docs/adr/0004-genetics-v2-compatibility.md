# ADR-0004：Genetics v2 追加兼容策略

- 状态：Accepted
- 日期：2026-07-19

## 背景

旧模型使用 `GeneLocus`、`Allele` 与逐动物 `Genotype` 表达单个位点结果。繁育计划需要能够
命名和复用多位点、半合子、转基因存在与条件等位基因组合，同时必须避免把旧自由文本或旧
单基因记录自动解释成未经科研人员确认的新定义。

## 决策

- 保留旧 `GeneLocus`、`Allele`、`Genotype` API 和表，不做破坏性重命名或数据回填。
- 新增 `GenotypeDefinition` 聚合；一个定义包含一个或多个显式 `GenotypeComponent`，组件
  指向既有 locus/allele，并声明 Diploid、Hemizygous、TransgenePresence 或 Conditional。
- 新增 `GenotypingRecord`，将 Animal、GenotypeDefinition、状态、检测时间、方法和备注分离。
  Confirmed/Rejected 必须有 `assessed_at`；Unknown/Expected 不伪装成已检测事实。
- `BreedingLine` 只引用经人工创建的 GenotypeDefinition ID，不从 Animal 的旧 Genotype 或
  strain 字符串自动推断。
- SQLite/PostgreSQL 使用相同 Store contract；Snapshot 同时保留旧 Genotype 和 Genetics v2
  数据，防止迁移时静默丢失任一语义层。

## 后果

- 旧客户端和旧数据仍可读取；新繁育工作流可以使用结构化、多组件定义。
- 两套表示在过渡期并存。任何旧 → v2 转换都必须是单独的预览、确认和 provenance 流程。
- 当前定义创建后没有就地重写入口；需要改变科研含义时创建新定义，保留历史引用稳定性。

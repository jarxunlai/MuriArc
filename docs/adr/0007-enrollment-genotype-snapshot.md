# ADR-0007：实验入组时冻结基因检测快照

- 状态：Accepted
- 日期：2026-07-19

## 背景

动物的基因检测会在实验期间继续补充或修订。如果 Participation 只动态关联“当前基因型”，
以后无法回答动物入组决策当时依据的是哪条检测记录，也无法稳定复现实验分组。

## 决策

- 创建 Participation 时，在同一个事务和 animal-scoped 写入序列中读取该动物全部未删除的
  GenotypingRecord。
- 对每个 GenotypeDefinition 选择创建顺序中最新的一条记录，保存
  `genotyping_record_id`、`genotype_definition_id`、`state` 与 `assessed_at`。
- SQLite 在捕获前取得 writer lock；PostgreSQL 对相同 animal key 使用 transaction advisory
  lock。基因检测写入使用同一顺序边界，因此检测与入组严格发生在彼此前或后。
- 快照作为 Participation 数据持久化并进入 Desktop/REST DTO 与业务 Snapshot；后续新增或
  修改检测记录不回写既有 Participation。
- 空快照是合法事实，明确表示入组时没有检测记录，而不是查询失败。

## 后果

- 实验入组依据可复现，历史 Participation 不随当前检测投影漂移。
- 同一 Animal 在不同 Experiment/时间入组可拥有不同快照，这是预期行为。
- 快照不复制方法和备注等完整检测内容；需要详情时可按保存的 record ID 查询原记录，业务
  Snapshot 同时归档两者。

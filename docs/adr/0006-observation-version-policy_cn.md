# ADR-0006：Observation 值策略与版本保留

> 简体中文 · 内部架构决策记录

- 状态：Accepted
- 日期：2026-07-19

## 背景

实验观察既可能是不可更改的原始事实，也可能需要纠错或形成显式时间版本。直接更新同一值行
会破坏可追溯性；仅依赖 Audit diff 又不足以作为可查询的科研值历史。

## 决策

- `ObservationDefinition` 固定 key、值类型、单位/类别和 Immutable、Mutable 或 Versioned 策略。
- 创建 Observation 时，Observation 与 version 1 `ObservationValueRecord` 在同一事务中写入。
- Immutable 拒绝任何修订。
- Mutable 与 Versioned 都不覆盖旧值，而是追加严格连续的新版本并原子推进
  `current_value_version`。Mutable 表示调用方通常只消费当前纠正值；Versioned 表示版本历史
  本身具有业务意义，但两者都保留完整记录。
- 修订必须携带 expected revision；过期 revision、跳号版本和值类型不匹配均整次拒绝。
- 人工 Audit actor 有 user ID 时，`ObservationValueRecord.recorded_by` 必须与之相同；SQLite
  与 PostgreSQL 在写事务内执行相同校验，失败不产生部分值或投影更新。

## 后果

- 当前值查询高效，历史值仍是一级科研记录而非审计文本。
- Mutable 不等于物理覆盖；存储成本增加，但纠错链可验证。
- 改变 Definition 语义需要新定义，不能让既有 Observation 的类型或策略漂移。

# ADR-0003：写入事务边界

> 简体中文 · 内部架构决策记录

- 状态：Accepted
- 日期：2026-07-19

## 背景

科研写入不仅包含主记录，还包含生命周期事件、Audit、Provenance、关系一致性与并发约束。
若 Application 或 transport 通过多次独立 Store 调用完成一次业务操作，中途失败会留下
不可追溯的部分状态，并使 SQLite/PostgreSQL 行为分叉。

## 决策

- Application service 描述并提交一个完整业务意图。
- Store port 的单个写方法定义原子事务边界；adapter 必须全部成功或全部回滚。
- 关系存在性、同 Lab/Project 校验、笼位容量和 revision 冲突在事务内执行，以避免
  check-then-write 竞态。
- 领域主记录、派生生命周期事件、Audit 与 Provenance 在同一事务中写入。
- 若新用例需要多个持久化步骤，应新增表达该意图的原子 Store 方法或明确 Unit of Work，
  不得由 Tauri/Axum/application 顺序调用多个独立写方法模拟事务。
- SQLite 与 PostgreSQL 必须通过同一 Store contract；并发能力不同可以有实现差异，
  对外语义必须一致。

## CreateAnimal 的应用

`CreateAnimal` Application service 负责规范化输入并构造 `Animal`。Store adapter 在一个
事务内：

1. 校验 Project identifier scope 和 initial Cage 与 Lab 一致；
2. 校验笼位容量；
3. 写入 Animal；
4. 写入 Registered、可选 Born、可选 Transferred 事件；
5. 写入 Audit 与 Provenance。

## 后果

- Application 不复制 SQL adapter 的并发和回滚细节。
- adapter contract 的实现成本略有增加，但失败状态可预测且可测试。
- 未来拆分 repository 时，事务语义优先于接口粒度。

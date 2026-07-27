# ADR-0005：Breeding 事实与原子事务边界

> 简体中文 · 内部架构决策记录

- 状态：Accepted
- 日期：2026-07-19

## 背景

配对、交配、窝次和 offspring 登记包含性别、活跃成员、双亲关系、动物生命周期、审计与
来源等多项约束。若由 UI 或 transport 分步写入，中途失败会产生无双亲 Animal、无 Draft
的 Litter 或只有一半审计链的不可恢复状态。

## 决策

- `BreedingPair` 必须恰好一个雄性成员和至少一个雌性成员，允许一雄多雌；同一动物不能
  同时参与冲突的活跃配对。
- 配对退役携带 expected revision，并在同一事务中关闭所有仍活跃的成员。
- `MatingEvent` 是科研人员确认的事实，只能引用配对中的雄性与某一雌性；遗传预测和 AI
  建议绝不自动创建事件。
- `create_litter` 在一个 Store 事务中创建 Litter 及所有存活 offspring `AnimalDraft`。
- `register_animal_draft` 在一个 Store 事务中完成 Draft 状态转换、正式 Animal、父母两条
  Pedigree、Registered/Born 生命周期事件、Audit 与 Provenance；任一步失败全部回滚。
- Tauri 与 Axum 调用同一 Application 用例；SQLite/PostgreSQL 通过共享 contract 验证相同
  成功、冲突和回滚语义。

## 后果

- Draft 是出生事实和正式 Registry 之间的明确缓冲，不会因部分写入产生幽灵动物。
- Store port 比 CRUD 更粗，但事务边界与科研意图一致。
- AI Breeding Planner 只能分析、预测和给出建议；实际配对、交配与动物登记始终是显式人工操作。

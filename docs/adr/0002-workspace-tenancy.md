# ADR-0002：V1 Workspace 与租户边界

- 状态：Accepted
- 日期：2026-07-19

## 背景

工程规范使用 Workspace → User → Role → Permission → Resource 描述多工作区模型；当前领域
与迁移已经使用 `Lab`、`Membership` 和 `Project` 表达同一执行边界。现在新增一套
Workspace 表会引入命名重复和高风险 schema 迁移，而首批 Application 收口并不需要它。

## 决策

V1 将 `Lab` 视为 Workspace/tenant 的领域实现：

- 所有 Lab 级资源必须携带 `lab_id`。
- Animal 属于 Lab Registry；Project 只提供 Lab 内授权、编号命名空间和实验参与范围，
  不改变 Animal 所有权。
- Server 的 `lab_id`、actor 和角色来自认证主体，不允许创建请求自行声明。
- Project-scoped 操作先进行权限门禁；跨 Lab 资源对调用者表现为 not found。
- Desktop 固定使用本地 Lab 与 LocalOperator，但仍写入 actor、source、revision、Audit
  和 Provenance。
- “用户属于多个 Workspace”通过一个 User 拥有多个 Lab Membership 的方向演进；本 ADR
  不要求立即修改当前身份 schema。

## 后果

- 首批架构收口无需数据库迁移。
- 文档中的 Workspace 与代码中的 Lab 有明确映射，不再把 Project 误当顶层租户。
- 将来若产品术语统一为 Workspace，可通过受控重命名或兼容 API 完成，而不是并存两套
  顶层所有权。

## 非目标

- 本阶段不实现跨 Lab 数据共享。
- 本阶段不实现 Desktop/Server 实时同步。
- 本阶段不修改 Genetics、Breeding 或 Observation schema。

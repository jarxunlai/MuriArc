# ADR-0001：引入共享 Application Layer

> 简体中文 · 内部架构决策记录

- 状态：Accepted
- 日期：2026-07-19

## 背景

Desktop 与 Server 已共享领域模型和 Store contract，但部分写入仍分别在 Tauri command
和 Axum handler 中构造领域对象。这会使输入规范化、默认值和关系意图逐渐分叉，也不符合
“API → Application Service → Domain → Repository”的工程规范。

## 决策

新增 `crates/application`，作为 transport 与 `muriarc-core` 之间的共享应用层。

- Application service 负责用例级输入规范化、领域对象构造和持久化意图编排。
- Tauri/Axum 负责 DTO、字符串/日期/UUID 解析、认证与权限门禁，并调用同一用例。
- `core` 仅保留领域模型、领域不变量和持久化 port，不依赖 Application、transport、
  SQLx adapter 或 AI Provider。
- 第一阶段以现有 `MuriArcStore` 作为兼容 port，按纵向切片逐步收口；不为形式完整性
  一次性拆分全部 context repository。
- 首个切片是 `CreateAnimal`。公共 DTO 暂不改变。

依赖方向固定为：

```text
Tauri / Axum / approved AI tools
              ↓
    muriarc-application
              ↓
       muriarc-core ports
              ↓
 SQLite / PostgreSQL adapters
```

## 后果

正向后果：

- Desktop 与 Server 共享同一创建动物规则。
- transport 入口变薄，后续 AI 工具也有明确且受控的调用面。
- 可以按用例添加测试，而无需复制两套业务断言。

代价与约束：

- 过渡期 Application 仍依赖较宽的 `MuriArcStore` port。
- transport-specific 的认证、权限与格式错误不会下沉到 Application。
- 后续拆分 repository 必须保持 SQLite/PostgreSQL contract 一致。
